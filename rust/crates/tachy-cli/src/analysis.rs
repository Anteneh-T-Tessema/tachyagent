//! Analysis and reporting commands: graph, monorepo, dashboard, search,
//! export-audit, policy, swarm, finetune.

use std::env;
use std::io::{self, Write};

use crate::DEFAULT_MODEL;

pub(crate) fn run_graph(format: &str, file: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let graph = intelligence::DependencyGraph::build(&cwd);

    if let Some(f) = file {
        let deps = graph.transitive_dependents(f);
        let node = graph.nodes.get(f);
        let out = serde_json::json!({
            "file": f,
            "direct_imports": node.map(|n| &n.imports).cloned().unwrap_or_default(),
            "imported_by": node.map(|n| &n.imported_by).cloned().unwrap_or_default(),
            "transitive_dependents": deps,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    match format {
        "json" | "json-pretty" => {
            println!("{}", serde_json::to_string_pretty(&graph)?);
        }
        "summary" => {
            println!("Dependency graph — {} nodes, {} edges", graph.nodes.len(), graph.edge_count);
            let mut langs: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
            for node in graph.nodes.values() {
                *langs.entry(node.language.as_str()).or_default() += 1;
            }
            for (lang, count) in &langs {
                println!("  {lang}: {count} files");
            }
        }
        _ => return Err(format!("unknown graph format: {format}\n  supported: json, summary").into()),
    }
    Ok(())
}

pub(crate) fn run_monorepo() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let manifest = intelligence::MonorepoManifest::detect(&cwd);

    if manifest.is_monorepo {
        println!("Monorepo detected: {:?}", manifest.kind);
        println!("  {} members:", manifest.members.len());
        for m in &manifest.members {
            println!("    {}  ({})", m.name, m.path);
        }
    } else {
        println!("Single-project workspace (no monorepo structure detected)");
        println!("  Toolchain checked: Cargo, npm/yarn/pnpm, Turborepo, Nx, Go modules, Python");
    }
    println!();
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

pub(crate) fn run_dashboard() -> Result<(), Box<dyn std::error::Error>> {
    let daemon_url = "http://127.0.0.1:7777";
    let resp = std::process::Command::new("curl")
        .args(["-sf", &format!("{daemon_url}/api/dashboard")])
        .output();

    if let Ok(out) = resp {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                println!("⚡ Tachy Performance Dashboard");
                println!();
                println!("  Total requests : {}", json["total_requests"]);
                println!("  Total tokens   : {}", json["total_tokens"]);
                println!("  Avg tokens/sec : {:.1}", json["avg_tokens_per_sec"].as_f64().unwrap_or(0.0));
                println!("  Last tokens/sec: {:.1}", json["last_tokens_per_sec"].as_f64().unwrap_or(0.0));
                println!("  p50 TTFT       : {:.0}ms", json["p50_ttft_ms"].as_f64().unwrap_or(0.0));
                println!("  p95 TTFT       : {:.0}ms", json["p95_ttft_ms"].as_f64().unwrap_or(0.0));
                let cost = json["estimated_cost_usd"].as_f64().unwrap_or(0.0);
                println!("  Est. cost      : ${cost:.4}  (local compute proxy at $0.002/1k tokens)");
                println!();
                if let Some(models) = json["models"].as_array() {
                    if !models.is_empty() {
                        println!("  Model leaderboard:");
                        for m in models {
                            println!(
                                "    {:30} {:6.1} tok/s  {:8} tokens",
                                m["name"].as_str().unwrap_or("?"),
                                m["avg_tps"].as_f64().unwrap_or(0.0),
                                m["tokens"].as_u64().unwrap_or(0),
                            );
                        }
                    }
                }
                return Ok(());
            }
        }
    }
    eprintln!("⚠ Daemon not running — start with: tachy serve");
    Ok(())
}

pub(crate) fn run_search(query: &str, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;

    let daemon_url = "http://127.0.0.1:7777";
    let resp = std::process::Command::new("curl")
        .args([
            "-sf",
            &format!("{daemon_url}/api/search?q={}&limit={limit}", urlencoding_simple(query)),
        ])
        .output();

    if let Ok(out) = resp {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                print_search_results(query, &json);
                return Ok(());
            }
        }
    }

    println!("Searching codebase for: {query}\n");
    let cfg = intelligence::IndexerConfig::default();
    let index = if let Ok(i) = intelligence::CodebaseIndexer::load_index(&cwd) { i } else {
        print!("Building index... ");
        io::stdout().flush().ok();
        let idx = intelligence::CodebaseIndexer::build_index(&cwd, &cfg)?;
        let _ = intelligence::CodebaseIndexer::save_index(&cwd, &idx);
        println!("done ({} files)", idx.project.total_files);
        idx
    };

    let results = intelligence::CodebaseIndexer::search(&index, query, limit);
    if results.is_empty() {
        println!("No results found for \"{query}\"");
        return Ok(());
    }

    for (i, entry) in results.iter().enumerate() {
        println!("  {}. {} ({})", i + 1, entry.path, entry.language);
        if !entry.exports.is_empty() {
            let exports = entry.exports[..entry.exports.len().min(5)].join(", ");
            println!("     exports: {exports}");
        }
        if !entry.summary.is_empty() {
            let summary = entry.summary.lines().next().unwrap_or("").trim();
            if !summary.is_empty() {
                println!("     {summary}");
            }
        }
        println!();
    }

    Ok(())
}

pub(crate) fn urlencoding_simple(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn print_search_results(query: &str, json: &serde_json::Value) {
    let results = json.get("results").and_then(|r| r.as_array());
    match results {
        Some(list) if list.is_empty() => println!("No results found for \"{query}\""),
        Some(list) => {
            println!("Search results for \"{query}\":\n");
            for (i, item) in list.iter().enumerate() {
                let path = item.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                let lang = item.get("language").and_then(|v| v.as_str()).unwrap_or("");
                let summary = item.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                let exports: Vec<_> = item.get("exports")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter()
                        .filter_map(|e| e.as_str())
                        .take(5)
                        .collect())
                    .unwrap_or_default();
                println!("  {}. {} ({})", i + 1, path, lang);
                if !exports.is_empty() {
                    println!("     exports: {}", exports.join(", "));
                }
                if !summary.is_empty() {
                    let line = summary.lines().next().unwrap_or("").trim();
                    if !line.is_empty() {
                        println!("     {line}");
                    }
                }
                println!();
            }
        }
        None => println!("Unexpected response format"),
    }
}

pub(crate) fn run_export_audit(format: &str, output: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let audit_path = cwd.join(".tachy").join("audit.log");

    if !audit_path.exists() {
        return Err("audit log not found — has tachy been run in this workspace?".into());
    }

    let raw = std::fs::read_to_string(&audit_path)?;
    let events: Vec<serde_json::Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    let outfile = output.map(std::path::Path::new).unwrap_or_else(|| {
        match format {
            "soc2" => std::path::Path::new("tachy-soc2-report.json"),
            "csv"  => std::path::Path::new("tachy-audit.csv"),
            _      => std::path::Path::new("tachy-audit-export.json"),
        }
    });

    match format {
        "soc2" => {
            let report = serde_json::json!({
                "report_type": "SOC 2 Type II — Change Management Evidence",
                "generated_at": chrono_now_str_iso(),
                "workspace": cwd.display().to_string(),
                "total_events": events.len(),
                "controls": {
                    "CC6": "Logical and Physical Access Controls",
                    "CC8": "Change Management",
                    "A1": "Availability",
                },
                "evidence": events,
            });
            std::fs::write(outfile, serde_json::to_string_pretty(&report)?)?;
            println!("✓ SOC 2 evidence report written to {}", outfile.display());
        }
        "csv" => {
            let mut csv = "timestamp,kind,severity,description\n".to_string();
            for e in &events {
                let ts    = e["timestamp"].as_str().unwrap_or("-").replace(',', " ");
                let kind  = e["kind"].as_str().unwrap_or("-").replace(',', " ");
                let sev   = e["severity"].as_str().unwrap_or("-").replace(',', " ");
                let desc  = e["description"].as_str().unwrap_or("-").replace(',', " ");
                csv.push_str(&format!("{ts},{kind},{sev},{desc}\n"));
            }
            std::fs::write(outfile, csv)?;
            println!("✓ Audit log exported to {}", outfile.display());
        }
        _ => {
            std::fs::write(outfile, serde_json::to_string_pretty(&events)?)?;
            println!("✓ Audit log exported to {}", outfile.display());
        }
    }
    Ok(())
}

fn chrono_now_str_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}Z (unix {secs})")
}

pub(crate) fn run_policy(subcommand: &str, file: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let default_policy_path = cwd.join("tachy-policy.yaml");
    let policy_path = file.map(std::path::Path::new).unwrap_or(&default_policy_path);

    match subcommand {
        "show" => {
            let pf = audit::PolicyFile::load(policy_path)
                .unwrap_or_else(|_| audit::PolicyFile::enterprise_default());
            println!("{}", serde_json::to_string_pretty(&pf)?);
        }
        "init" => {
            if policy_path.exists() {
                return Err(format!("{} already exists — remove it first", policy_path.display()).into());
            }
            let pf = audit::PolicyFile::enterprise_default();
            pf.save(policy_path).map_err(|e| format!("failed to write policy: {e}"))?;
            println!("Created {}", policy_path.display());
            println!("Edit it, then run: tachy policy validate");
        }
        "validate" => {
            match audit::PolicyFile::load(policy_path) {
                Ok(pf) => {
                    println!("Policy '{}' is valid.", policy_path.display());
                    let json = serde_json::to_string_pretty(&pf)?;
                    let rule_count = json.matches("allow").count() + json.matches("deny").count();
                    println!("  {rule_count} allow/deny rules found");
                }
                Err(e) => return Err(format!("invalid policy: {e}").into()),
            }
        }
        "set" => {
            let pf = audit::PolicyFile::load(policy_path)
                .map_err(|e| format!("cannot load policy '{}': {e}", policy_path.display()))?;
            let body = serde_json::to_string(&pf)?;
            let out = std::process::Command::new("curl")
                .args([
                    "-sf", "-X", "POST",
                    "-H", "Content-Type: application/json",
                    "-d", &body,
                    "http://127.0.0.1:7777/api/policy",
                ])
                .output();
            match out {
                Ok(o) if o.status.success() => println!("Policy pushed to daemon."),
                Ok(o) => eprintln!("Daemon error: {}", String::from_utf8_lossy(&o.stderr)),
                Err(_) => eprintln!("Daemon not running — start with: tachy serve"),
            }
        }
        other => return Err(format!("unknown policy subcommand: {other}\n  usage: tachy policy show|init|validate|set").into()),
    }
    Ok(())
}

pub(crate) fn run_swarm(goal: &str, files: &[String], model: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;

    let resolved_files: Vec<String> = if files.is_empty() {
        let mut found = Vec::new();
        fn collect(dir: &std::path::Path, out: &mut Vec<String>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        if !matches!(name.as_str(), "target" | "node_modules" | ".git" | ".tachy") {
                            collect(&p, out);
                        }
                    } else if let Some(ext) = p.extension() {
                        if matches!(ext.to_str().unwrap_or(""), "rs" | "ts" | "tsx" | "js" | "py" | "go") {
                            if let Ok(rel) = p.strip_prefix(std::env::current_dir().unwrap_or_default()) {
                                out.push(rel.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
        collect(&cwd, &mut found);
        found
    } else {
        files.to_vec()
    };

    if resolved_files.is_empty() {
        return Err("no source files found — pass file paths explicitly or run from a project root".into());
    }

    let planner_model = model.unwrap_or("gemma4:26b").to_string();
    let input = intelligence::SwarmRefactorInput {
        goal: goal.to_string(),
        files: resolved_files.clone(),
        use_llm_planner: true,
        planner_model: planner_model.clone(),
        coordinator: Some(backend::CoordinatorConfig::from_env()),
    };

    let plan = intelligence::plan_swarm_refactor(&input);
    println!("Swarm Plan ({:?}, {} tasks):", plan.planner, plan.tasks.len());
    for task in &plan.tasks {
        let deps = if task.deps.is_empty() {
            String::new()
        } else {
            format!(" [after: {}]", task.deps.join(", "))
        };
        println!("  [{}] template={}{}", task.id, task.template, deps);
        let preview = task.prompt.lines().next().unwrap_or("").chars().take(80).collect::<String>();
        println!("      {preview}…");
    }
    println!();

    let body = serde_json::to_string(&serde_json::json!({
        "goal": goal,
        "files": resolved_files,
        "use_llm_planner": true,
        "planner_model": planner_model,
    }))?;

    let out = std::process::Command::new("curl")
        .args(["-sf", "-X", "POST", "-H", "Content-Type: application/json", "-d", &body,
               "http://127.0.0.1:7777/api/swarm/runs"])
        .output();

    match out {
        Ok(o) if o.status.success() => {
            let resp = String::from_utf8_lossy(&o.stdout);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                println!("Swarm run submitted: run_id={}", v["run_id"].as_str().unwrap_or("?"));
                println!("  Poll status: tachy dashboard (or GET http://127.0.0.1:7777/api/swarm/runs)");
            }
        }
        _ => {
            println!("Daemon not running — plan printed above. Start the daemon with: tachy serve");
        }
    }

    Ok(())
}

pub(crate) fn run_finetune(output: Option<&str>, base_model: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let sessions_dir = cwd.join(".tachy").join("sessions");

    if !sessions_dir.exists() {
        return Err("no session history found in .tachy/sessions/ — chat with tachy first to generate training data".into());
    }

    let dataset = intelligence::FinetuneDataset::from_sessions(&sessions_dir);
    if dataset.entries.is_empty() {
        println!("⚠ No (user, assistant) pairs found in session history.");
        return Ok(());
    }

    let out_dir = output.unwrap_or("tachy-finetune");
    std::fs::create_dir_all(out_dir)?;

    let jsonl_path = std::path::Path::new(out_dir).join("dataset.jsonl");
    dataset.save_jsonl(&jsonl_path)?;
    println!("✓ Dataset: {} training pairs from {} sessions", dataset.total_pairs, dataset.source_sessions);
    println!("  Written to {}", jsonl_path.display());

    let base = base_model.unwrap_or(DEFAULT_MODEL);
    let script_content = intelligence::generate_training_script(base, "dataset.jsonl", out_dir);
    let script_path = std::path::Path::new(out_dir).join("train.sh");
    std::fs::write(&script_path, &script_content)?;
    println!("  Training script: {}", script_path.display());

    let mf_content = intelligence::generate_modelfile(
        base,
        "./adapter.gguf",
        "You are Tachy, a fast local AI coding agent optimised for this codebase.",
    );
    let mf_path = std::path::Path::new(out_dir).join("Modelfile");
    std::fs::write(&mf_path, &mf_content)?;
    println!("  Modelfile:       {}", mf_path.display());

    println!();
    println!("Next steps:");
    println!("  1. pip install unsloth torch trl datasets transformers");
    println!("  2. bash {}", script_path.display());
    println!("  3. ollama create my-tachy -f {}", mf_path.display());
    println!("  4. tachy --model my-tachy");

    Ok(())
}

pub(crate) fn print_help() {
    println!("tachy — Local AI Coding Agent\n");
    println!("Usage:");
    println!("  tachy init                                Initialize workspace (.tachy/)");
    println!("  tachy setup                               Full setup: check Ollama, pull model, init workspace");
    println!("  tachy doctor                              Check Ollama, GPU, models, test tool calling");
    println!("  tachy pull <model>                        Pull a model via Ollama");
    println!("  tachy models                              List registered models");
    println!("  tachy models --local                      List locally installed models");
    println!("  tachy agents                              List agent templates");
    println!("  tachy search <query>                      Search the indexed codebase");
    println!("  tachy pipeline run <pipeline.yaml>        Run an agent pipeline from YAML");
    println!("  tachy pipeline validate <pipeline.yaml>   Validate pipeline without running");
    println!("  tachy pipeline init [output.yaml]         Generate a starter pipeline YAML");
    println!("  tachy graph [--format json|summary]       Print dependency/call graph (C1)");
    println!("  tachy graph --file <path>                 Show transitive dependents of a file");
    println!("  tachy monorepo                            Detect monorepo structure and members (C3)");
    println!("  tachy dashboard                           Show live performance dashboard (E2)");
    println!("  tachy export-audit [--format json|soc2|csv] [--output FILE]  Export audit log (F1)");
    println!("  tachy finetune [--output DIR] [--base-model MODEL]  Generate LoRA training data (F3)");
    println!("  tachy verify-audit                        Verify audit trail integrity");
    println!("  tachy warmup [MODEL]                      Pre-load model into GPU memory");
    println!("  tachy [--model MODEL]                     Start interactive REPL");
    println!("  tachy [--model MODEL] prompt TEXT          Send one prompt (streams output)");
    println!("  tachy run-agent <template> <prompt...>     Run an agent template");
    println!("  tachy serve [ADDR]                         Start HTTP daemon + web UI (default 127.0.0.1:7777)");
    println!("  tachy --continue                           Resume last session");
    println!("  tachy --resume SESSION.json [/compact]     Resume a specific session");
    println!("\nREPL commands:");
    println!("  /help /status /compact /save /undo /model [name] /sessions /audit /exit");
    println!("\nHTTP API (when running `tachy serve`):");
    println!("  GET  /health              Health check");
    println!("  GET  /api/models          List models");
    println!("  GET  /api/templates       List agent templates");
    println!("  GET  /api/agents          List all agents");
    println!("  GET  /api/agents/:id      Get agent status (poll for async results)");
    println!("  GET  /api/search?q=<q>    Search indexed codebase");
    println!("  GET  /api/graph           Full dependency graph (add ?file=<path> for per-file view)");
    println!("  GET  /api/monorepo        Monorepo workspace structure");
    println!("  GET  /api/dashboard       Live performance stats + cost estimate");
    println!("  GET  /api/policy          Get current tachy-policy.yaml");
    println!("  POST /api/policy          Update tachy-policy.yaml");
    println!("  POST /api/agents/run      Start agent (async, returns 202)");
    println!("  POST /api/tasks/schedule  Schedule recurring agent");
    println!("\nEnvironment:");
    println!("  TACHY_PERMISSION_MODE   read-only | workspace-write | deny-all");
    println!("  TACHY_API_KEY           API key for HTTP daemon authentication");
    println!("  TACHY_AIR_GAP           Set to 1 to disable all outbound connections (F1)");
    println!("  TACHY_TELEMETRY         Set to 0 to disable usage telemetry");
    println!("  OLLAMA_HOST             Ollama URL (default http://localhost:11434)");
    println!("\nDefault model: gemma4:26b (Gemma 4 26B MoE — native tool calling, 256K context)");
}
