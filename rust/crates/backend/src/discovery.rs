use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

/// Information about a locally installed Ollama model.
#[derive(Debug, Clone)]
pub struct LocalModel {
    pub name: String,
    pub size_bytes: u64,
    pub parameter_size: String,
    pub quantization: String,
    pub family: String,
}

impl LocalModel {
    #[must_use]
    pub fn size_human(&self) -> String {
        let gb = self.size_bytes as f64 / 1_073_741_824.0;
        if gb >= 1.0 {
            format!("{gb:.1} GB")
        } else {
            let mb = self.size_bytes as f64 / 1_048_576.0;
            format!("{mb:.0} MB")
        }
    }
}

/// Result of a system health check.
#[derive(Debug, Clone)]
pub struct HealthReport {
    pub ollama_running: bool,
    pub ollama_version: Option<String>,
    pub ollama_url: String,
    pub local_models: Vec<LocalModel>,
    pub gpu_info: Option<String>,
    pub recommended_model: Option<String>,
}

impl HealthReport {
    pub fn print(&self) {
        println!("Tachy System Check\n");

        // Ollama status
        if self.ollama_running {
            let version = self.ollama_version.as_deref().unwrap_or("unknown");
            println!("  ✓ Ollama running at {} (v{})", self.ollama_url, version);
        } else {
            println!("  ✗ Ollama not running at {}", self.ollama_url);
            println!("    Install: curl -fsSL https://ollama.com/install.sh | sh");
            println!("    Start:   ollama serve");
        }

        // GPU
        if let Some(gpu) = &self.gpu_info {
            println!("  ✓ GPU: {gpu}");
        } else {
            println!("  ⚠ No GPU detected — models will run on CPU (slower)");
        }

        // Models
        println!();
        if self.local_models.is_empty() {
            println!("  No models installed.");
            println!("    Run: tachy pull llama3.1:8b");
        } else {
            println!("  Installed models ({}):", self.local_models.len());
            for model in &self.local_models {
                println!(
                    "    {:30} {:>8}  {}  {}",
                    model.name,
                    model.size_human(),
                    model.parameter_size,
                    model.quantization,
                );
            }
        }

        // Recommendation
        println!();
        if let Some(rec) = &self.recommended_model {
            println!("  Recommended: tachy --model {rec}");
        } else if !self.local_models.is_empty() {
            println!("  Recommended: tachy --model {}", self.local_models[0].name);
        } else {
            println!("  Get started: tachy pull llama3.1:8b && tachy --model llama3.1:8b");
        }
    }
}

/// Query Ollama for locally installed models.
pub fn discover_local_models(base_url: &str) -> Vec<LocalModel> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let response = match client.get(format!("{base_url}/api/tags")).send() {
        Ok(r) if r.status().is_success() => r,
        _ => return Vec::new(),
    };

    let body: OllamaTagsResponse = match response.json() {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };

    body.models
        .into_iter()
        .map(|m| LocalModel {
            name: m.name,
            size_bytes: m.size,
            parameter_size: m.details.parameter_size,
            quantization: m.details.quantization_level,
            family: m.details.family,
        })
        .collect()
}

/// Check if Ollama is running and get its version.
pub fn check_ollama(base_url: &str) -> (bool, Option<String>) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return (false, None),
    };

    match client.get(format!("{base_url}/api/version")).send() {
        Ok(r) if r.status().is_success() => {
            let version = r
                .json::<serde_json::Value>()
                .ok()
                .and_then(|v| v.get("version")?.as_str().map(String::from));
            (true, version)
        }
        _ => (false, None),
    }
}

/// Detect GPU information (macOS Metal / Linux NVIDIA).
pub fn detect_gpu() -> Option<String> {
    // macOS: check for Apple Silicon
    if cfg!(target_os = "macos") {
        if let Ok(output) = Command::new("sysctl").arg("-n").arg("machdep.cpu.brand_string").output() {
            let brand = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !brand.is_empty() {
                // Get memory
                let mem = Command::new("sysctl")
                    .arg("-n")
                    .arg("hw.memsize")
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
                    .map(|bytes| format!(" ({} GB unified)", bytes / 1_073_741_824))
                    .unwrap_or_default();
                return Some(format!("{brand}{mem}"));
            }
        }
    }

    // Linux: check for NVIDIA
    if cfg!(target_os = "linux") {
        if let Ok(output) = Command::new("nvidia-smi")
            .arg("--query-gpu=name,memory.total")
            .arg("--format=csv,noheader,nounits")
            .output()
        {
            let info = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !info.is_empty() {
                return Some(info);
            }
        }
    }

    None
}

/// Pick the best model based on available RAM/VRAM.
pub fn recommend_model(models: &[LocalModel], gpu_info: &Option<String>) -> Option<String> {
    if models.is_empty() {
        return None;
    }

    // Prefer models with tool support and good coding ability
    let preferred_order = [
        "qwen3-coder",
        "qwen3:",
        "qwen2.5-coder",
        "llama3.1:",
        "mistral:",
        "codestral:",
        "llama3:",
        "codellama:",
    ];

    for prefix in &preferred_order {
        if let Some(model) = models.iter().find(|m| m.name.starts_with(prefix)) {
            return Some(model.name.clone());
        }
    }

    // Fall back to first model
    Some(models[0].name.clone())
}

/// Run a full health check.
pub fn run_health_check(base_url: &str) -> HealthReport {
    let (ollama_running, ollama_version) = check_ollama(base_url);
    let local_models = if ollama_running {
        discover_local_models(base_url)
    } else {
        Vec::new()
    };
    let gpu_info = detect_gpu();
    let recommended_model = recommend_model(&local_models, &gpu_info);

    HealthReport {
        ollama_running,
        ollama_version,
        ollama_url: base_url.to_string(),
        local_models,
        gpu_info,
        recommended_model,
    }
}

/// Pull a model via Ollama CLI.
pub fn pull_model(model_name: &str) -> Result<(), String> {
    println!("Pulling {model_name} via Ollama...\n");

    let status = Command::new("ollama")
        .arg("pull")
        .arg(model_name)
        .status()
        .map_err(|e| format!("failed to run `ollama pull`: {e}. Is Ollama installed?"))?;

    if status.success() {
        println!("\n✓ Model {model_name} ready");
        println!("  Run: tachy --model {model_name}");
        Ok(())
    } else {
        Err(format!("ollama pull {model_name} failed with exit code {status}"))
    }
}

// --- Ollama API types for discovery ---

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelEntry {
    name: String,
    size: u64,
    details: OllamaModelDetails,
}

#[derive(Debug, Deserialize)]
struct OllamaModelDetails {
    #[serde(default)]
    family: String,
    #[serde(default)]
    parameter_size: String,
    #[serde(default)]
    quantization_level: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_model_size_formatting() {
        let model = LocalModel {
            name: "test".to_string(),
            size_bytes: 4_920_753_328,
            parameter_size: "8B".to_string(),
            quantization: "Q4_K_M".to_string(),
            family: "llama".to_string(),
        };
        assert_eq!(model.size_human(), "4.6 GB");

        let small = LocalModel {
            size_bytes: 274_302_450,
            ..model.clone()
        };
        assert_eq!(small.size_human(), "262 MB");
    }

    #[test]
    fn recommend_model_prefers_coding_models() {
        let models = vec![
            LocalModel {
                name: "llama3:latest".to_string(),
                size_bytes: 4_000_000_000,
                parameter_size: "8B".to_string(),
                quantization: "Q4_0".to_string(),
                family: "llama".to_string(),
            },
            LocalModel {
                name: "qwen3-coder:30b".to_string(),
                size_bytes: 18_000_000_000,
                parameter_size: "30B".to_string(),
                quantization: "Q4_K_M".to_string(),
                family: "qwen3moe".to_string(),
            },
            LocalModel {
                name: "mistral:7b".to_string(),
                size_bytes: 4_000_000_000,
                parameter_size: "7B".to_string(),
                quantization: "Q4_K_M".to_string(),
                family: "llama".to_string(),
            },
        ];

        let rec = recommend_model(&models, &None);
        assert_eq!(rec.as_deref(), Some("qwen3-coder:30b"));
    }

    #[test]
    fn recommend_model_returns_none_for_empty() {
        assert!(recommend_model(&[], &None).is_none());
    }
}
