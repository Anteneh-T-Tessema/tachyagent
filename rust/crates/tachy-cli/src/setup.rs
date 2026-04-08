//! Workspace setup: bootstrap-plan printing, session resumption, setup wizard,
//! Ollama install/start, workspace init, project detection, model warmup.

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use backend::BackendRegistry;
use platform::PlatformWorkspace;
use runtime::{load_system_prompt, CompactionConfig, Session};
use commands::handle_slash_command;

pub(crate) fn print_bootstrap_plan() {
    for phase in runtime::BootstrapPlan::default_plan().phases() {
        println!("- {phase:?}");
    }
}

pub(crate) fn print_system_prompt(cwd: PathBuf, date: String) {
    match load_system_prompt(cwd, date, env::consts::OS, "unknown") {
        Ok(sections) => println!("{}", sections.join("\n\n")),
        Err(error) => {
            eprintln!("failed to build system prompt: {error}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn resume_session(session_path: &Path, command: Option<String>) {
    let session = match Session::load_from_path(session_path) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("failed to restore session: {error}");
            std::process::exit(1);
        }
    };

    match command {
        Some(command) if command.starts_with('/') => {
            let Some(result) = handle_slash_command(
                &command,
                &session,
                CompactionConfig {
                    max_estimated_tokens: 0,
                    ..CompactionConfig::default()
                },
            ) else {
                eprintln!("unknown slash command: {command}");
                std::process::exit(2);
            };
            if let Err(error) = result.session.save_to_path(session_path) {
                eprintln!("failed to persist resumed session: {error}");
                std::process::exit(1);
            }
            println!("{}", result.message);
        }
        Some(other) => {
            eprintln!("unsupported resumed command: {other}");
            std::process::exit(2);
        }
        None => {
            println!(
                "Restored session from {} ({} messages).",
                session_path.display(),
                session.messages.len()
            );
        }
    }
}

pub(crate) fn run_setup_wizard() -> Result<(), Box<dyn std::error::Error>> {
    println!("⚡ Tachy Setup\n");

    let mut ollama_ok = false;
    let mut model_name = String::new();
    let mut manual_steps: Vec<String> = Vec::new();

    println!("Step 1/4: Checking Ollama...");
    let mut report = backend::run_health_check("http://localhost:11434");

    if report.ollama_running {
        println!("  ✓ Ollama running (v{})", report.ollama_version.as_deref().unwrap_or("unknown"));
        ollama_ok = true;
    } else {
        let ollama_installed = std::process::Command::new("ollama")
            .arg("--version")
            .output()
            .is_ok();

        if !ollama_installed {
            println!("  Ollama not found. Installing...");
            match install_ollama() {
                Ok(()) => println!("  ✓ Ollama installed"),
                Err(e) => {
                    eprintln!("  ⚠ Auto-install failed: {e}");
                    manual_steps.push("Install Ollama: https://ollama.com/download".to_string());
                }
            }
        }

        println!("  Starting Ollama...");
        if let Err(e) = start_ollama() {
            eprintln!("  ⚠ Could not start: {e}");
        }

        for i in 0..20 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            report = backend::run_health_check("http://localhost:11434");
            if report.ollama_running {
                println!("  ✓ Ollama running (v{})", report.ollama_version.as_deref().unwrap_or("unknown"));
                ollama_ok = true;
                break;
            }
            if i % 5 == 4 {
                println!("  Waiting for Ollama... ({}s)", i + 1);
            }
        }

        if !ollama_ok {
            eprintln!("  ⚠ Ollama not responding after 20s");
            manual_steps.push("Start Ollama: ollama serve".to_string());
        }
    }

    println!("\nStep 2/4: Pulling default model...");
    if ollama_ok {
        if let Some(rec) = report.recommended_model.clone() {
            println!("  ✓ Found recommended model: {rec}");
            model_name = rec;
        } else {
            let ram_gb = backend::detect_system_ram_gb_public();
            let model_to_pull = if ram_gb >= 32 { "gemma4:26b" }
                else if ram_gb >= 16 { "qwen3:8b" }
                else { "gemma4:e4b" };
            println!("  Detected {ram_gb} GB RAM — pulling {model_to_pull}...");
            match backend::pull_model(model_to_pull) {
                Ok(()) => {
                    println!("  ✓ Model ready");
                    model_name = model_to_pull.to_string();
                }
                Err(e) => {
                    eprintln!("  ⚠ Pull failed: {e}");
                    manual_steps.push(format!("Pull model: ollama pull {model_to_pull}"));
                    model_name = model_to_pull.to_string();
                }
            }
        }
    } else {
        println!("  ⚠ Skipping (Ollama not running)");
        manual_steps.push("Pull a model: ollama pull gemma4:26b".to_string());
    }

    println!("\nStep 3/4: Initializing workspace...");
    let cwd = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match platform::PlatformWorkspace::init(&cwd) {
        Ok(_) => println!("  ✓ Workspace initialized at {}", cwd.display()),
        Err(e) => {
            eprintln!("  ⚠ Workspace init failed: {e}");
            manual_steps.push("Initialize workspace: tachy init".to_string());
        }
    }

    println!("\nStep 4/4: Warming up model...");
    if ollama_ok && !model_name.is_empty() {
        if let Err(e) = warmup_model(&model_name) {
            eprintln!("  ⚠ Warmup failed: {e} (model will load on first use)");
        }
    } else {
        println!("  ⚠ Skipping (no model available)");
    }

    if manual_steps.is_empty() {
        println!("\n✅ Setup complete!\n");
        let m = if model_name.is_empty() { "gemma4:26b" } else { &model_name };
        println!("Quick start:");
        println!("  tachy --model {m}          Interactive REPL");
        println!("  tachy serve                             Start web UI at http://localhost:7777");
        println!("  tachy doctor                            Check system status");
    } else {
        println!("\n⚠ Setup partially complete. Fix these manually:\n");
        for (i, step) in manual_steps.iter().enumerate() {
            println!("  {}. {step}", i + 1);
        }
        println!("\nThen run: tachy setup");
    }

    Ok(())
}

/// Install Ollama automatically based on the current platform.
pub(crate) fn install_ollama() -> Result<(), String> {
    if cfg!(target_os = "linux") {
        println!("    Downloading Ollama for Linux...");
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("curl -fsSL https://ollama.com/install.sh | sh")
            .status()
            .map_err(|e| format!("failed to run installer: {e}"))?;
        if status.success() {
            return Ok(());
        }
        println!("    Script failed, trying direct binary download...");
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };
        let url = format!("https://ollama.com/download/ollama-linux-{arch}");
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "curl -fsSL '{url}' -o /usr/local/bin/ollama 2>/dev/null || \
                 curl -fsSL '{url}' -o $HOME/.local/bin/ollama && \
                 chmod +x /usr/local/bin/ollama 2>/dev/null || chmod +x $HOME/.local/bin/ollama"
            ))
            .status()
            .map_err(|e| format!("direct download failed: {e}"))?;
        if !status.success() {
            return Err("all Linux install methods failed".to_string());
        }
        Ok(())
    } else if cfg!(target_os = "macos") {
        let brew_path = if std::path::Path::new("/opt/homebrew/bin/brew").exists() {
            Some("/opt/homebrew/bin/brew")
        } else if std::path::Path::new("/usr/local/bin/brew").exists() {
            Some("/usr/local/bin/brew")
        } else {
            None
        };

        if let Some(brew) = brew_path {
            println!("    Installing via Homebrew...");
            let status = std::process::Command::new(brew)
                .args(["install", "ollama"])
                .status()
                .map_err(|e| format!("brew install failed: {e}"))?;
            if status.success() {
                return Ok(());
            }
        }

        println!("    Downloading Ollama.app...");
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(concat!(
                "curl -fsSL https://ollama.com/download/Ollama-darwin.zip -o /tmp/ollama-dl.zip && ",
                "unzip -oq /tmp/ollama-dl.zip -d /tmp/ollama-app && ",
                "rm -rf /Applications/Ollama.app && ",
                "mv /tmp/ollama-app/Ollama.app /Applications/ && ",
                "rm -rf /tmp/ollama-dl.zip /tmp/ollama-app && ",
                "echo 'Installed Ollama.app'"
            ))
            .status()
            .map_err(|e| format!("download failed: {e}"))?;
        if !status.success() {
            return Err("macOS Ollama download failed".to_string());
        }
        let _ = std::process::Command::new("open").arg("/Applications/Ollama.app").spawn();
        std::thread::sleep(std::time::Duration::from_secs(3));
        Ok(())
    } else if cfg!(target_os = "windows") {
        println!("    Downloading Ollama for Windows...");
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "$ProgressPreference = 'SilentlyContinue'; \
                 Invoke-WebRequest -Uri 'https://ollama.com/download/OllamaSetup.exe' -OutFile \"$env:TEMP\\OllamaSetup.exe\"; \
                 Start-Process -Wait -FilePath \"$env:TEMP\\OllamaSetup.exe\" -ArgumentList '/S'; \
                 Remove-Item \"$env:TEMP\\OllamaSetup.exe\" -ErrorAction SilentlyContinue"])
            .status()
            .map_err(|e| format!("download failed: {e}"))?;
        if !status.success() {
            return Err("Windows Ollama installer failed".to_string());
        }
        Ok(())
    } else {
        Err(format!("unsupported platform: {}", std::env::consts::OS))
    }
}

/// Start the Ollama server in the background.
pub(crate) fn start_ollama() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        if std::path::Path::new("/Applications/Ollama.app").exists() {
            let _ = std::process::Command::new("open")
                .arg("/Applications/Ollama.app")
                .spawn();
            return Ok(());
        }
        for brew in ["/opt/homebrew/bin/brew", "/usr/local/bin/brew", "brew"] {
            if std::process::Command::new(brew)
                .args(["services", "start", "ollama"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return Ok(());
            }
        }
    }

    if cfg!(target_os = "linux") {
        if std::process::Command::new("systemctl")
            .args(["start", "ollama"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        if std::process::Command::new("sudo")
            .args(["systemctl", "start", "ollama"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Ok(());
        }
    }

    if cfg!(target_os = "windows") {
        let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let ollama_exe = format!("{appdata}\\Programs\\Ollama\\ollama app.exe");
        if std::path::Path::new(&ollama_exe).exists() {
            let _ = std::process::Command::new(&ollama_exe).spawn();
            return Ok(());
        }
    }

    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let args: Vec<&str> = if cfg!(target_os = "windows") {
        vec!["/C", "start /B ollama serve"]
    } else {
        vec!["-c", "ollama serve &"]
    };

    std::process::Command::new(shell)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to start ollama: {e}"))?;

    Ok(())
}

pub(crate) fn init_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let ws = PlatformWorkspace::init(&cwd).map_err(|e| e.to_string())?;
    println!("Initialized workspace at {}", cwd.display());
    println!("  Config: {}", ws.config_path().display());
    println!("  Audit log: {}", ws.audit_log_path().display());
    println!("  Sessions: {}", ws.sessions_dir().display());
    println!("  Default model: {}", ws.config.default_model);
    println!("  Agent templates: {}", ws.config.agent_templates.len());
    println!("  Governance: destructive shell blocked={}", ws.config.governance.block_destructive_shell);

    let tachy_md_path = cwd.join("TACHY.md");
    if !tachy_md_path.exists() {
        if let Some(project_info) = detect_project(&cwd) {
            let content = format!(
                "# Project Instructions for Tachy\n\n\
                 Language: {}\n\
                 {}\
                 {}\
                 \n\
                 ## Guidelines\n\n\
                 - Follow existing code style and conventions\n\
                 - Run tests after making changes\n\
                 - Keep changes minimal and focused\n",
                project_info.language,
                if let Some(tc) = &project_info.test_command {
                    format!("Test command: `{tc}`\n")
                } else { String::new() },
                if let Some(bc) = &project_info.build_command {
                    format!("Build command: `{bc}`\n")
                } else { String::new() },
            );
            std::fs::write(&tachy_md_path, &content)?;
            println!("  ✓ Created TACHY.md ({} project detected)", project_info.language);
            if let Some(tc) = &project_info.test_command {
                println!("    Test command: {tc}");
            }
        }
    }

    Ok(())
}

pub(crate) struct ProjectInfo {
    pub(crate) language: String,
    pub(crate) test_command: Option<String>,
    pub(crate) build_command: Option<String>,
}

pub(crate) fn detect_project(cwd: &Path) -> Option<ProjectInfo> {
    if cwd.join("Cargo.toml").exists() {
        return Some(ProjectInfo {
            language: "Rust".to_string(),
            test_command: Some("cargo test".to_string()),
            build_command: Some("cargo build".to_string()),
        });
    }
    if cwd.join("package.json").exists() {
        let test_cmd = if cwd.join("vitest.config.ts").exists() || cwd.join("vitest.config.js").exists() {
            Some("npx vitest --run".to_string())
        } else if cwd.join("jest.config.js").exists() || cwd.join("jest.config.ts").exists() {
            Some("npx jest".to_string())
        } else {
            Some("npm test".to_string())
        };
        let lang = if cwd.join("tsconfig.json").exists() { "TypeScript" } else { "JavaScript" };
        return Some(ProjectInfo {
            language: lang.to_string(),
            test_command: test_cmd,
            build_command: Some("npm run build".to_string()),
        });
    }
    if cwd.join("pyproject.toml").exists() || cwd.join("setup.py").exists() {
        let test_cmd = if cwd.join("pytest.ini").exists() || cwd.join("pyproject.toml").exists() {
            Some("pytest".to_string())
        } else {
            Some("python -m pytest".to_string())
        };
        return Some(ProjectInfo {
            language: "Python".to_string(),
            test_command: test_cmd,
            build_command: None,
        });
    }
    if cwd.join("go.mod").exists() {
        return Some(ProjectInfo {
            language: "Go".to_string(),
            test_command: Some("go test ./...".to_string()),
            build_command: Some("go build ./...".to_string()),
        });
    }
    if cwd.join("pom.xml").exists() {
        return Some(ProjectInfo {
            language: "Java (Maven)".to_string(),
            test_command: Some("mvn test".to_string()),
            build_command: Some("mvn package".to_string()),
        });
    }
    if cwd.join("build.gradle").exists() || cwd.join("build.gradle.kts").exists() {
        return Some(ProjectInfo {
            language: "Java/Kotlin (Gradle)".to_string(),
            test_command: Some("./gradlew test".to_string()),
            build_command: Some("./gradlew build".to_string()),
        });
    }
    if cwd.join("CMakeLists.txt").exists() {
        return Some(ProjectInfo {
            language: "C/C++ (CMake)".to_string(),
            test_command: Some("cmake --build build && ctest --test-dir build".to_string()),
            build_command: Some("cmake --build build".to_string()),
        });
    }
    if cwd.join("Makefile").exists() {
        return Some(ProjectInfo {
            language: "C/C++ (Make)".to_string(),
            test_command: Some("make test".to_string()),
            build_command: Some("make".to_string()),
        });
    }
    None
}

pub(crate) fn warmup_model(model: &str) -> Result<(), Box<dyn std::error::Error>> {
    print!("Warming up {model}... ");
    io::stdout().flush()?;

    let start = std::time::Instant::now();
    let registry = BackendRegistry::with_defaults();
    let mut client = registry.create_client(model, false)
        .or_else(|_| {
            backend::OllamaBackend::new(model.to_string(), "http://localhost:11434".to_string(), false)
                .map(|b| Box::new(b) as Box<dyn runtime::ApiClient>)
                .map_err(|e| runtime::RuntimeError::new(e.to_string()))
        })?;

    let request = runtime::ApiRequest {
        system_prompt: vec!["You are helpful.".to_string()],
        messages: vec![runtime::ConversationMessage::user_text("hi")],
        format: runtime::ResponseFormat::default(),
    };

    match client.stream(request) {
        Ok(_) => {
            let elapsed = start.elapsed();
            println!("✓ ready ({:.1}s)", elapsed.as_secs_f64());
        }
        Err(e) => {
            println!("✗ failed: {e}");
        }
    }
    Ok(())
}
