//! `tachy doctor` — system health check, GPU stats, and model benchmark.

use std::io::{self, Write};
use std::env;

use backend::BackendRegistry;

pub(crate) fn run_doctor(json: bool) {
    let base_url = "http://localhost:11434";
    let report = backend::run_health_check(base_url);

    // Disk free via `df`
    let disk_free_gb: Option<u64> = std::process::Command::new("df")
        .args(["-k", "."])
        .output()
        .ok()
        .and_then(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.lines().nth(1).and_then(|line| {
                line.split_whitespace().nth(3).and_then(|kb| kb.parse::<u64>().ok())
            })
        })
        .map(|kb| kb / 1_048_576); // KB → GB

    if json {
        let tachy_dir = env::current_dir().unwrap_or_default().join(".tachy");
        let license = audit::LicenseFile::load_or_create(&tachy_dir);
        let status = license.status();
        let (gpu_name, vram_total_mb, vram_free_mb, gpu_util_pct) = query_gpu_info();
        let obj = serde_json::json!({
            "ollama_running": report.ollama_running,
            "local_models": report.local_models.iter().map(|m| &m.name).collect::<Vec<_>>(),
            "recommended_model": report.recommended_model,
            "license_active": status.is_active(),
            "license": status.display(),
            "disk_free_gb": disk_free_gb,
            "gpu_name": gpu_name,
            "vram_total_mb": vram_total_mb,
            "vram_free_mb": vram_free_mb,
            "gpu_utilization_pct": gpu_util_pct,
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
        return;
    }

    report.print();

    let tachy_dir = env::current_dir().unwrap_or_default().join(".tachy");
    let license = audit::LicenseFile::load_or_create(&tachy_dir);
    let status = license.status();
    println!();
    if status.is_active() {
        println!("  ✓ License: {}", status.display());
    } else {
        println!("  ✗ License: {}", status.display());
    }

    if report.ollama_running && !report.local_models.is_empty() {
        println!();
        let test_model = report.recommended_model.as_deref()
            .unwrap_or(&report.local_models[0].name);
        print!("  Testing {test_model} with tool call... ");
        io::stdout().flush().ok();

        let registry = BackendRegistry::with_defaults();
        let client_result = registry.create_client(test_model, true)
            .or_else(|_| {
                backend::OllamaBackend::new(
                    test_model.to_string(),
                    base_url.to_string(),
                    true,
                )
                .map(|b| Box::new(b) as Box<dyn runtime::ApiClient>)
                .map_err(|e| runtime::RuntimeError::new(e.to_string()))
            });

        match client_result {
            Ok(mut client) => {
                let request = runtime::ApiRequest {
                    system_prompt: vec!["You are a helpful assistant. Use the bash tool to run: echo tachy-ok".to_string()],
                    messages: vec![runtime::ConversationMessage::user_text("Run echo tachy-ok")],
                    format: runtime::ResponseFormat::default(),
                };
                let start = std::time::Instant::now();
                match client.stream(request) {
                    Ok(events) => {
                        let elapsed = start.elapsed();
                        let has_tool = events.iter().any(|e| matches!(e, runtime::AssistantEvent::ToolUse { .. }));
                        let has_text = events.iter().any(|e| matches!(e, runtime::AssistantEvent::TextDelta(_)));
                        if has_tool {
                            println!("✓ tool calling works ({:.1}s)", elapsed.as_secs_f64());
                        } else if has_text {
                            println!("⚠ responded with text only, no tool call ({:.1}s)", elapsed.as_secs_f64());
                            println!("    This model may not support tool calling reliably.");
                        } else {
                            println!("⚠ empty response ({:.1}s)", elapsed.as_secs_f64());
                        }
                    }
                    Err(e) => println!("✘ failed: {e}"),
                }
            }
            Err(e) => println!("✘ could not create client: {e}"),
        }

        let registry2 = BackendRegistry::with_defaults();
        let client2 = registry2.create_client(test_model, true)
            .or_else(|_| {
                backend::OllamaBackend::new(test_model.to_string(), base_url.to_string(), true)
                    .map(|b| Box::new(b) as Box<dyn runtime::ApiClient>)
                    .map_err(|e| runtime::RuntimeError::new(e.to_string()))
            });
        if let Ok(mut client2) = client2 {
            print!("  Benchmarking {test_model} throughput... ");
            io::stdout().flush().ok();
            let bench_req = runtime::ApiRequest {
                system_prompt: vec!["You are a coding assistant.".to_string()],
                messages: vec![runtime::ConversationMessage::user_text(
                    "List ten common Rust iterator methods in one sentence each.",
                )],
                format: runtime::ResponseFormat::default(),
            };
            let t0 = std::time::Instant::now();
            match client2.stream(bench_req) {
                Ok(events) => {
                    let elapsed = t0.elapsed().as_secs_f64();
                    let total_chars: usize = events.iter()
                        .filter_map(|e| if let runtime::AssistantEvent::TextDelta(s) = e { Some(s.len()) } else { None })
                        .sum();
                    let approx_tokens = (total_chars as f64 / 4.0).max(1.0);
                    let tps = approx_tokens / elapsed.max(0.001);
                    print!("{tps:.0} tokens/sec  ");
                    if tps < 5.0 {
                        println!("(⚠ very slow — try a smaller model)");
                    } else if tps < 15.0 {
                        println!("(moderate)");
                    } else if tps < 40.0 {
                        println!("(✓ good)");
                    } else {
                        println!("(⚡ fast)");
                    }
                }
                Err(e) => println!("benchmark skipped: {e}"),
            }
        }
    }

    println!();
    match disk_free_gb {
        Some(gb) if gb < 5 => println!("  ⚠ Disk free: {gb} GB  (low — consider freeing space)"),
        Some(gb) => println!("  ✓ Disk free: {gb} GB"),
        None => println!("  ? Disk free: unable to determine"),
    }

    println!();
    print_gpu_stats(json);
}

fn print_gpu_stats(_json: bool) {
    let (name_opt, vram_total, vram_free, util) = query_gpu_info();
    match (name_opt.as_deref(), vram_total, vram_free, util) {
        (Some(name), Some(total), Some(free), _) => {
            println!("  ✓ GPU: {name}");
            let used = total.saturating_sub(free);
            let pct = if total > 0 { used * 100 / total } else { 0 };
            println!("  ✓ VRAM: {used} / {total} MB used  ({pct}%)");
            if let Some(u) = util {
                println!("  ✓ GPU utilization: {u}%");
            }
        }
        (Some(name), None, _, _) => {
            println!("  ✓ GPU: {name} (Apple Silicon — unified memory)");
        }
        _ => {
            println!("  ? GPU: not detected (nvidia-smi unavailable — running in CPU mode)");
        }
    }
}

fn query_gpu_info() -> (Option<String>, Option<u64>, Option<u64>, Option<u64>) {
    if let Ok(out) = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,memory.free,utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            let line = text.lines().next().unwrap_or("").trim();
            let parts: Vec<&str> = line.split(',').map(str::trim).collect();
            if parts.len() >= 4 {
                return (
                    Some(parts[0].to_string()),
                    parts[1].parse().ok(),
                    parts[2].parse().ok(),
                    parts[3].parse().ok(),
                );
            }
        }
    }

    if let Ok(out) = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(arr) = v["SPDisplaysDataType"].as_array() {
                    if let Some(gpu) = arr.first() {
                        let name = gpu["sppci_model"].as_str().map(str::to_string);
                        return (name, None, None, None);
                    }
                }
            }
        }
    }

    (None, None, None, None)
}
