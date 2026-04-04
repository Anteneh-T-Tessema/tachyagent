mod input;
mod render;

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use audit::{AuditEvent, AuditEventKind, AuditLogger, AuditSeverity, FileAuditSink};
use backend::{BackendRegistry, DynBackend};
use commands::handle_slash_command;
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use daemon::DaemonState;
use platform::{PlatformConfig, PlatformWorkspace};
use render::{Spinner, TerminalRenderer};
use runtime::{
    load_system_prompt, CompactionConfig, ContentBlock,
    ConversationRuntime, PermissionMode, PermissionPolicy,
    Session, ToolError, ToolExecutor,
};
use tools::{execute_tool, execute_tool_with_diff};

const DEFAULT_MODEL: &str = "gemma4:26b";
const DEFAULT_DATE: &str = "2026-04-03";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let action = parse_args(&args)?;

    // License check — skip for non-agent commands
    let needs_license = matches!(
        action,
        CliAction::Prompt { .. } | CliAction::Repl { .. } | CliAction::RunAgent { .. } | CliAction::Serve { .. }
    );
    if needs_license {
        let tachy_dir = env::current_dir()
            .unwrap_or_default()
            .join(".tachy");
        let license = audit::LicenseFile::load_or_create(&tachy_dir);
        let status = license.status();
        if !status.is_active() {
            eprintln!("⚠ {}", status.display());
            eprintln!();
            eprintln!("Purchase a license at https://tachy.dev/pricing");
            eprintln!("Then run: tachy activate <LICENSE_KEY>");
            std::process::exit(1);
        }
        // Show trial status in non-error cases
        if let audit::LicenseStatus::TrialActive { remaining_secs } = &status {
            let days = remaining_secs / 86400;
            if days <= 2 {
                eprintln!("⚠ Trial expires in {} — https://tachy.dev/pricing", status.display());
            }
        }
    }

    match action {
        CliAction::BootstrapPlan => print_bootstrap_plan(),
        CliAction::PrintSystemPrompt { cwd, date } => print_system_prompt(cwd, date),
        CliAction::ResumeSession {
            session_path,
            command,
        } => resume_session(&session_path, command),
        CliAction::Prompt { prompt, model } => {
            let mut cli = LiveCli::new(model, true)?;
            cli.run_turn(&prompt)?;
        }
        CliAction::Repl { model } => run_repl(model)?,
        CliAction::Init => init_workspace()?,
        CliAction::Setup => run_setup_wizard()?,
        CliAction::ListModels => list_models(),
        CliAction::ListModelsLocal => list_models_local(),
        CliAction::ListAgents => list_agents(),
        CliAction::Serve { addr, workspace } => run_serve(&addr, workspace.as_deref())?,
        CliAction::RunAgent { template, prompt, model } => run_agent_cmd(&template, &prompt, &model)?,
        CliAction::Doctor => run_doctor(),
        CliAction::Pull { model } => run_pull(&model)?,
        CliAction::VerifyAudit => verify_audit()?,
        CliAction::Warmup { model } => warmup_model(&model)?,
        CliAction::InstallOllama => {
            match install_ollama() {
                Ok(()) => {
                    println!("✓ Ollama installed");
                    println!("Starting server...");
                    let _ = start_ollama();
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    if backend::check_ollama("http://localhost:11434").0 {
                        println!("✓ Ollama server running");
                    } else {
                        println!("Server not responding yet — try: ollama serve");
                    }
                }
                Err(e) => eprintln!("✗ Install failed: {e}\n  Install manually: https://ollama.com/download"),
            }
        }
        CliAction::ListTools => list_tools(),
        CliAction::ListChannels => list_channels(),
        CliAction::McpServer => daemon::run_mcp_server(),
        CliAction::Activate { key } => activate_license(&key)?,
        CliAction::LicenseStatus => show_license_status()?,
        CliAction::Help => print_help(),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    BootstrapPlan,
    PrintSystemPrompt {
        cwd: PathBuf,
        date: String,
    },
    ResumeSession {
        session_path: PathBuf,
        command: Option<String>,
    },
    Prompt {
        prompt: String,
        model: String,
    },
    Repl {
        model: String,
    },
    Init,
    Setup,
    ListModels,
    ListModelsLocal,
    ListAgents,
    Serve { addr: String, workspace: Option<PathBuf> },
    RunAgent { template: String, prompt: String, model: String },
    Doctor,
    Pull { model: String },
    VerifyAudit,
    Warmup { model: String },
    InstallOllama,
    ListTools,
    ListChannels,
    McpServer,
    Activate { key: String },
    LicenseStatus,
    Help,
}

fn parse_args(args: &[String]) -> Result<CliAction, String> {
    let mut model = DEFAULT_MODEL.to_string();
    let mut rest = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--model" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --model".to_string())?;
                model = value.clone();
                index += 2;
            }
            flag if flag.starts_with("--model=") => {
                model = flag[8..].to_string();
                index += 1;
            }
            other => {
                rest.push(other.to_string());
                index += 1;
            }
        }
    }

    if rest.is_empty() {
        return Ok(CliAction::Repl { model });
    }
    if matches!(rest.first().map(String::as_str), Some("--help" | "-h")) {
        return Ok(CliAction::Help);
    }
    if rest.first().map(String::as_str) == Some("--resume") {
        return parse_resume_args(&rest[1..]);
    }
    if rest.first().map(String::as_str) == Some("--continue") {
        // Resume the most recent session
        if let Some(path) = LiveCli::last_session_path() {
            return Ok(CliAction::ResumeSession {
                session_path: path,
                command: None,
            });
        }
        return Err("no previous session found in .tachy/sessions/".to_string());
    }

    match rest[0].as_str() {
        "bootstrap-plan" => Ok(CliAction::BootstrapPlan),
        "system-prompt" => parse_system_prompt_args(&rest[1..]),
        "init" => Ok(CliAction::Init),
        "setup" => Ok(CliAction::Setup),
        "models" => {
            if rest.get(1).map(String::as_str) == Some("--local") {
                Ok(CliAction::ListModelsLocal)
            } else {
                Ok(CliAction::ListModels)
            }
        }
        "agents" => Ok(CliAction::ListAgents),
        "doctor" => Ok(CliAction::Doctor),
        "verify-audit" => Ok(CliAction::VerifyAudit),
        "warmup" => {
            let warmup_model = rest.get(1).cloned().unwrap_or_else(|| model.clone());
            Ok(CliAction::Warmup { model: warmup_model })
        }
        "install-ollama" => Ok(CliAction::InstallOllama),
        "tools" => Ok(CliAction::ListTools),
        "channels" => Ok(CliAction::ListChannels),
        "mcp-server" => Ok(CliAction::McpServer),
        "activate" => {
            let key = rest.get(1).ok_or("usage: activate <LICENSE_KEY>")?;
            Ok(CliAction::Activate { key: key.clone() })
        }
        "license" => Ok(CliAction::LicenseStatus),
        "pull" => {
            let model_name = rest.get(1).ok_or("usage: pull <model>")?;
            Ok(CliAction::Pull { model: model_name.clone() })
        }
        "serve" => {
            let mut addr = "127.0.0.1:7777".to_string();
            let mut workspace = None;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--workspace" | "-w" => {
                        workspace = rest.get(i + 1).map(|p| PathBuf::from(p));
                        i += 2;
                    }
                    other if !other.starts_with('-') && i == 1 => {
                        addr = other.to_string();
                        i += 1;
                    }
                    _ => { i += 1; }
                }
            }
            Ok(CliAction::Serve { addr, workspace })
        }
        "run-agent" => {
            if rest.len() < 3 {
                return Err("usage: run-agent <template> <prompt...>".to_string());
            }
            let template = rest[1].clone();
            let prompt = rest[2..].join(" ");
            Ok(CliAction::RunAgent { template, prompt, model })
        }
        "prompt" => {
            let prompt = rest[1..].join(" ");
            if prompt.trim().is_empty() {
                return Err("prompt subcommand requires a prompt string".to_string());
            }
            Ok(CliAction::Prompt { prompt, model })
        }
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn parse_system_prompt_args(args: &[String]) -> Result<CliAction, String> {
    let mut cwd = env::current_dir().map_err(|error| error.to_string())?;
    let mut date = DEFAULT_DATE.to_string();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--cwd" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --cwd".to_string())?;
                cwd = PathBuf::from(value);
                index += 2;
            }
            "--date" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --date".to_string())?;
                date.clone_from(value);
                index += 2;
            }
            other => return Err(format!("unknown system-prompt option: {other}")),
        }
    }

    Ok(CliAction::PrintSystemPrompt { cwd, date })
}

fn parse_resume_args(args: &[String]) -> Result<CliAction, String> {
    let session_path = args
        .first()
        .ok_or_else(|| "missing session path for --resume".to_string())
        .map(PathBuf::from)?;
    let command = args.get(1).cloned();
    if args.len() > 2 {
        return Err("--resume accepts at most one trailing slash command".to_string());
    }
    Ok(CliAction::ResumeSession {
        session_path,
        command,
    })
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

fn print_bootstrap_plan() {
    for phase in runtime::BootstrapPlan::default_plan().phases() {
        println!("- {phase:?}");
    }
}

fn print_system_prompt(cwd: PathBuf, date: String) {
    match load_system_prompt(cwd, date, env::consts::OS, "unknown") {
        Ok(sections) => println!("{}", sections.join("\n\n")),
        Err(error) => {
            eprintln!("failed to build system prompt: {error}");
            std::process::exit(1);
        }
    }
}

fn resume_session(session_path: &Path, command: Option<String>) {
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

fn run_setup_wizard() -> Result<(), Box<dyn std::error::Error>> {
    println!("⚡ Tachy Setup\n");

    let mut ollama_ok = false;
    let mut model_name = String::new();
    let mut manual_steps: Vec<String> = Vec::new();

    // Step 1: Check Ollama — install and start automatically if missing
    println!("Step 1/4: Checking Ollama...");
    let mut report = backend::run_health_check("http://localhost:11434");

    if report.ollama_running {
        println!("  ✓ Ollama running (v{})", report.ollama_version.as_deref().unwrap_or("unknown"));
        ollama_ok = true;
    } else {
        // Check if binary exists but server isn't running
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

        // Try to start
        println!("  Starting Ollama...");
        if let Err(e) = start_ollama() {
            eprintln!("  ⚠ Could not start: {e}");
        }

        // Wait for server with progress
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

    // Step 2: Pull model
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

    // Step 3: Initialize workspace
    println!("\nStep 3/4: Initializing workspace...");
    let cwd = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match platform::PlatformWorkspace::init(&cwd) {
        Ok(_) => println!("  ✓ Workspace initialized at {}", cwd.display()),
        Err(e) => {
            eprintln!("  ⚠ Workspace init failed: {e}");
            manual_steps.push("Initialize workspace: tachy init".to_string());
        }
    }

    // Step 4: Warm up model
    println!("\nStep 4/4: Warming up model...");
    if ollama_ok && !model_name.is_empty() {
        if let Err(e) = warmup_model(&model_name) {
            eprintln!("  ⚠ Warmup failed: {e} (model will load on first use)");
        }
    } else {
        println!("  ⚠ Skipping (no model available)");
    }

    // Summary
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
/// Handles: macOS (brew or direct), Linux (official script), Windows (silent installer).
fn install_ollama() -> Result<(), String> {
    if cfg!(target_os = "linux") {
        println!("    Downloading Ollama for Linux...");
        // The official script handles sudo internally and supports all distros
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("curl -fsSL https://ollama.com/install.sh | sh")
            .status()
            .map_err(|e| format!("failed to run installer: {e}"))?;
        if status.success() {
            return Ok(());
        }
        // Fallback: try downloading the binary directly
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
        // macOS: try Homebrew first (most reliable)
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

        // Fallback: download Ollama.app directly
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
        // Open the app once to install the CLI helper
        let _ = std::process::Command::new("open").arg("/Applications/Ollama.app").spawn();
        std::thread::sleep(std::time::Duration::from_secs(3));
        Ok(())
    } else if cfg!(target_os = "windows") {
        println!("    Downloading Ollama for Windows...");
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!(
                "$ProgressPreference = 'SilentlyContinue'; \
                 Invoke-WebRequest -Uri 'https://ollama.com/download/OllamaSetup.exe' -OutFile \"$env:TEMP\\OllamaSetup.exe\"; \
                 Start-Process -Wait -FilePath \"$env:TEMP\\OllamaSetup.exe\" -ArgumentList '/S'; \
                 Remove-Item \"$env:TEMP\\OllamaSetup.exe\" -ErrorAction SilentlyContinue"
            )])
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
fn start_ollama() -> Result<(), String> {
    // macOS: open the app (it manages its own server)
    if cfg!(target_os = "macos") {
        if std::path::Path::new("/Applications/Ollama.app").exists() {
            let _ = std::process::Command::new("open")
                .arg("/Applications/Ollama.app")
                .spawn();
            return Ok(());
        }
        // Try brew services
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

    // Linux: try systemd, then direct
    if cfg!(target_os = "linux") {
        // Try systemd (works if installed via official script)
        if std::process::Command::new("systemctl")
            .args(["start", "ollama"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        // Try sudo systemctl
        if std::process::Command::new("sudo")
            .args(["systemctl", "start", "ollama"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Ok(());
        }
    }

    // Windows: try starting the app
    if cfg!(target_os = "windows") {
        let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let ollama_exe = format!("{appdata}\\Programs\\Ollama\\ollama app.exe");
        if std::path::Path::new(&ollama_exe).exists() {
            let _ = std::process::Command::new(&ollama_exe).spawn();
            return Ok(());
        }
    }

    // Universal fallback: spawn ollama serve in background
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

fn init_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let ws = PlatformWorkspace::init(&cwd).map_err(|e| e.to_string())?;
    println!("Initialized workspace at {}", cwd.display());
    println!("  Config: {}", ws.config_path().display());
    println!("  Audit log: {}", ws.audit_log_path().display());
    println!("  Sessions: {}", ws.sessions_dir().display());
    println!("  Default model: {}", ws.config.default_model);
    println!("  Agent templates: {}", ws.config.agent_templates.len());
    println!("  Governance: destructive shell blocked={}", ws.config.governance.block_destructive_shell);

    // Project detection — create TACHY.md with project-specific instructions
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

struct ProjectInfo {
    language: String,
    test_command: Option<String>,
    build_command: Option<String>,
}

fn detect_project(cwd: &Path) -> Option<ProjectInfo> {
    // Rust
    if cwd.join("Cargo.toml").exists() {
        return Some(ProjectInfo {
            language: "Rust".to_string(),
            test_command: Some("cargo test".to_string()),
            build_command: Some("cargo build".to_string()),
        });
    }
    // Node.js / TypeScript
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
    // Python
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
    // Go
    if cwd.join("go.mod").exists() {
        return Some(ProjectInfo {
            language: "Go".to_string(),
            test_command: Some("go test ./...".to_string()),
            build_command: Some("go build ./...".to_string()),
        });
    }
    // Java / Kotlin
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
    // C/C++
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

fn list_channels() {
    let tachy_dir = env::current_dir().unwrap_or_default().join(".tachy");
    let channels = daemon::load_channels(&tachy_dir);
    if channels.is_empty() {
        println!("No messaging channels configured.");
        println!("Create .tachy/channels.yaml to add Slack, Discord, or Telegram integration.");
        println!("\nExample:");
        println!("  channels:");
        println!("    - type: slack");
        println!("      bot_token: $SLACK_BOT_TOKEN");
        println!("      channel: \"#ai-agent\"");
        println!("      template: chat");
        return;
    }
    println!("Configured channels:\n");
    for ch in &channels {
        let status = if ch.enabled { "enabled" } else { "disabled" };
        println!("  {:12} {:10} template={:15} [{}]",
            format!("{:?}", ch.r#type).to_lowercase(),
            if !ch.channel.is_empty() { &ch.channel } else { &ch.channel_id },
            if ch.template.is_empty() { "chat" } else { &ch.template },
            status,
        );
    }
}

fn list_tools() {
    println!("Built-in tools:\n");
    for spec in tools::mvp_tool_specs() {
        println!("  {:20} {}", spec.name, spec.description);
    }

    // Load custom tools
    let tachy_dir = env::current_dir().unwrap_or_default().join(".tachy");
    let custom = tools::CustomToolRegistry::load(&tachy_dir);
    if !custom.tools().is_empty() {
        println!("\nCustom tools (.tachy/tools.yaml):\n");
        for tool in custom.tools() {
            let type_str = match tool.r#type {
                tools::custom::ToolType::Shell => "shell",
                tools::custom::ToolType::Http => "http",
            };
            let approval = if tool.approval_required { " [approval required]" } else { "" };
            println!("  {:20} {} ({}){}", tool.name, tool.description, type_str, approval);
        }
    } else {
        println!("\nNo custom tools defined. Create .tachy/tools.yaml to add custom tools.");
    }
}

fn list_models() {
    let registry = BackendRegistry::with_defaults();
    println!("Available models:\n");
    for model in registry.list_models() {
        let tools = if model.supports_tool_use { "tools" } else { "no-tools" };
        println!(
            "  {:30} {:12} ctx={:>7}  {}",
            model.name,
            format!("{:?}", model.backend),
            model.context_window,
            tools,
        );
    }
}

fn list_models_local() {
    let models = backend::discover_local_models("http://localhost:11434");
    if models.is_empty() {
        println!("No models installed locally.");
        println!("  Run: tachy pull llama3.1:8b");
        return;
    }
    println!("Locally installed models ({}):\n", models.len());
    for model in &models {
        println!(
            "  {:30} {:>8}  {:6}  {}",
            model.name,
            model.size_human(),
            model.parameter_size,
            model.quantization,
        );
    }
}

fn run_doctor() {
    let base_url = "http://localhost:11434";
    let report = backend::run_health_check(base_url);
    report.print();

    // License status
    let tachy_dir = env::current_dir().unwrap_or_default().join(".tachy");
    let license = audit::LicenseFile::load_or_create(&tachy_dir);
    let status = license.status();
    println!();
    if status.is_active() {
        println!("  ✓ License: {}", status.display());
    } else {
        println!("  ✗ License: {}", status.display());
    }

    // Quick model test if Ollama is running and has models
    if report.ollama_running && !report.local_models.is_empty() {
        println!();
        let test_model = report.recommended_model.as_deref()
            .unwrap_or(&report.local_models[0].name);
        print!("  Testing {test_model} with tool call... ");
        io::stdout().flush().ok();

        let registry = BackendRegistry::with_defaults();
        // Try to create a client — if the model isn't in the registry, register it dynamically
        let client_result = registry.create_client(test_model, true)
            .or_else(|_| {
                // Model exists in Ollama but not in registry — create an ad-hoc client
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
                    Err(e) => {
                        println!("✘ failed: {e}");
                    }
                }
            }
            Err(e) => {
                println!("✘ could not create client: {e}");
            }
        }
    }
}

fn run_pull(model: &str) -> Result<(), Box<dyn std::error::Error>> {
    backend::pull_model(model).map_err(|e| e.into())
}

fn verify_audit() -> Result<(), Box<dyn std::error::Error>> {
    let audit_path = env::current_dir()?.join(".tachy").join("audit.jsonl");
    if !audit_path.exists() {
        println!("No audit log found. Run `tachy init` first.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&audit_path)?;
    let events: Vec<audit::AuditEvent> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    if events.is_empty() {
        println!("Audit log is empty.");
        return Ok(());
    }

    println!("Tachy Audit Verification Report");
    println!("================================\n");
    println!("  Log file:    {}", audit_path.display());
    println!("  Total events: {}", events.len());
    println!("  First event:  {}", events[0].timestamp);
    println!("  Last event:   {}", events[events.len() - 1].timestamp);

    // Count by type
    let mut kind_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut severity_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for event in &events {
        *kind_counts.entry(format!("{:?}", event.kind)).or_insert(0) += 1;
        *severity_counts.entry(format!("{:?}", event.severity)).or_insert(0) += 1;
    }

    println!("\n  Events by type:");
    for (kind, count) in &kind_counts {
        println!("    {kind:25} {count}");
    }

    println!("\n  Events by severity:");
    for (sev, count) in &severity_counts {
        println!("    {sev:25} {count}");
    }

    // Verify hash chain
    println!("\n  Hash chain verification:");
    match audit::verify_audit_chain(&events) {
        Ok(count) => {
            println!("    ✓ Chain intact: {count} events verified");
            println!("    ✓ Last hash: {}…", &events[count - 1].hash.get(..16).unwrap_or("unknown"));
            println!("\n  RESULT: PASS — audit trail is tamper-proof");
        }
        Err(broken_at) => {
            println!("    ✗ Chain BROKEN at event {broken_at}");
            if broken_at > 0 {
                println!("    ✗ Events 0-{} are valid", broken_at - 1);
            }
            println!("    ✗ The audit log has been tampered with or corrupted");
            println!("\n  RESULT: FAIL — audit trail integrity compromised");
        }
    }

    println!("\n================================");
    println!("This report can be provided to compliance auditors.");
    println!("The audit log is append-only JSONL with SHA-256 hash chain.");

    Ok(())
}

fn warmup_model(model: &str) -> Result<(), Box<dyn std::error::Error>> {
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

fn activate_license(key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let tachy_dir = env::current_dir()?.join(".tachy");
    std::fs::create_dir_all(&tachy_dir)?;
    let mut license = audit::LicenseFile::load_or_create(&tachy_dir);

    // The license secret — in production, this would be compiled in or fetched from a config
    // For now, use an env var. In the real product, embed the public key in the binary.
    let secret = env::var("TACHY_LICENSE_SECRET")
        .unwrap_or_else(|_| "tachy-license-secret-v1".to_string());

    match license.activate(key, &secret) {
        Ok(data) => {
            license.save(&tachy_dir)?;
            println!("✓ License activated!");
            println!("  Email: {}", data.email);
            println!("  Tier: {:?}", data.tier);
            if data.expires_at > 0 {
                println!("  Expires: {}s", data.expires_at);
            } else {
                println!("  Expires: never (perpetual)");
            }
        }
        Err(e) => {
            eprintln!("✗ Activation failed: {e}");
            eprintln!();
            eprintln!("If you purchased a license, check that you copied the full key.");
            eprintln!("Contact support@tachy.dev if the problem persists.");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn show_license_status() -> Result<(), Box<dyn std::error::Error>> {
    let tachy_dir = env::current_dir()?.join(".tachy");
    let license = audit::LicenseFile::load_or_create(&tachy_dir);
    let status = license.status();

    println!("Tachy License Status\n");
    println!("  Status:     {}", status.display());
    println!("  Machine ID: {}", license.machine_id);
    if !license.license_key.is_empty() {
        println!("  Key:        {}…", &license.license_key[..20.min(license.license_key.len())]);
    }
    if let Some(data) = &license.license {
        println!("  Email:      {}", data.email);
        println!("  Tier:       {:?}", data.tier);
    }
    Ok(())
}

fn list_agents() {
    let config = PlatformConfig::default();
    println!("Built-in agent templates:\n");
    for template in &config.agent_templates {
        println!("  {}", template.name);
        println!("    {}", template.description);
        println!("    model: {}  max_iterations: {}  approval: {}",
            template.model, template.max_iterations, template.requires_approval);
        println!("    tools: {}", template.allowed_tools.join(", "));
        println!();
    }
}

fn run_serve(addr: &str, workspace: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = workspace
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| env::current_dir().unwrap_or_default());
    let state = DaemonState::init(cwd.clone()).map_err(|e| e.to_string())?;
    let state = std::sync::Arc::new(std::sync::Mutex::new(state));

    eprintln!("Workspace: {}", cwd.display());
    eprintln!("State is auto-saved on every change. Ctrl+C to stop.");

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(daemon::serve(addr, state.clone()));

    // Flush on exit
    if let Ok(s) = state.lock() {
        s.save();
        s.audit_logger.flush();
    }

    result
}

fn run_agent_cmd(template: &str, prompt: &str, model: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let mut state = DaemonState::init(cwd).map_err(|e| e.to_string())?;

    let agent_id = state.create_agent(template, prompt).map_err(|e| e.to_string())?;
    let config = state.agents.get(&agent_id).unwrap().config.clone();

    // Override model if specified
    let mut config = config;
    if model != DEFAULT_MODEL {
        config.template.model = model.to_string();
    }

    println!("Running agent '{template}' ({})", config.template.model);
    println!("Prompt: {prompt}\n");

    let result = daemon::AgentEngine::run_agent(
        &agent_id,
        &config,
        prompt,
        &state.registry,
        &state.config.governance,
        &state.audit_logger,
        &state.config.intelligence,
        &state.workspace_root,
        None, // No shared file locks in single-agent CLI mode
    );

    if result.success {
        println!("✓ Agent completed ({} iterations, {} tool calls)\n", result.iterations, result.tool_invocations);
        println!("{}", result.summary);
    } else {
        eprintln!("✗ Agent failed: {}", result.summary);
    }

    state.audit_logger.flush();
    Ok(())
}

// ---------------------------------------------------------------------------
// REPL and LiveCli — now using BackendRegistry
// ---------------------------------------------------------------------------

fn run_repl(model: String) -> Result<(), Box<dyn std::error::Error>> {
    let registry = BackendRegistry::with_defaults();
    let mut cli = LiveCli::new(model, true)?;
    let editor = input::LineEditor::new("› ");

    // Show a clean, informative greeting
    let mut stdout = io::stdout();
    execute!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print("Tachy"),
        ResetColor,
        Print(format!(" — local AI coding agent — {}\n", cli.model)),
    )?;
    execute!(
        stdout,
        SetForegroundColor(Color::DarkGrey),
        Print("Tools: bash, read_file, write_file, edit_file, grep_search, glob_search, list_directory\n"),
        Print("Commands: /help /status /compact /model /exit\n"),
        Print(format!("Context: {}K tokens\n\n", 
            registry.find_model(&cli.model).map(|m| m.context_window / 1000).unwrap_or(8)
        )),
        ResetColor,
    )?;

    while let Some(input) = editor.read_line()? {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            "/exit" | "/quit" => break,
            "/help" => {
                println!("Available commands:");
                println!("  /help    Show help");
                println!("  /status  Show session status");
                println!("  /compact Compact session history");
                println!("  /save    Save session to disk");
                println!("  /undo    Undo last file edit");
                println!("  /model   Show current model");
                println!("  /audit   Show audit event count");
                println!("  /exit    Quit the REPL");
            }
            "/status" => cli.print_status(),
            "/compact" => cli.compact()?,
            "/save" => {
                match cli.save_session() {
                    Ok(path) => println!("Session saved to {path}"),
                    Err(e) => eprintln!("Failed to save: {e}"),
                }
            }
            "/undo" => {
                if let Ok(mut stack) = cli.undo_stack.lock() {
                    if let Some((path, content)) = stack.pop() {
                        match std::fs::write(&path, &content) {
                            Ok(()) => println!("Reverted {path}"),
                            Err(e) => eprintln!("Failed to undo: {e}"),
                        }
                    } else {
                        println!("Nothing to undo");
                    }
                }
            }
            "/model" => println!("Current model: {}", cli.model),
            "/audit" => println!("Audit events logged: {}", cli.audit_event_count),
            _ => cli.run_turn(trimmed)?,
        }
    }

    // Auto-save session on exit if there were any turns
    if cli.runtime.session().messages.len() > 0 {
        if let Ok(path) = cli.save_session() {
            eprintln!("Session saved to {path}");
        }
    }

    // Flush audit log on exit
    cli.audit_logger.flush();
    Ok(())
}

struct LiveCli {
    model: String,
    session_id: String,
    system_prompt: Vec<String>,
    runtime: ConversationRuntime<DynBackend, CliToolExecutor>,
    audit_logger: AuditLogger,
    audit_event_count: u32,
    governance: audit::GovernancePolicy,
    tool_invocation_counts: std::collections::BTreeMap<String, u32>,
    total_tool_invocations: u32,
    undo_stack: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

impl LiveCli {
    fn new(model: String, enable_tools: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let system_prompt = build_system_prompt()?;
        let session_id = format!("sess-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis());

        // Set up audit logging — resume hash chain from existing log
        let tachy_dir = env::current_dir()?.join(".tachy");
        let audit_path = tachy_dir.join("audit.jsonl");
        let mut audit_logger = AuditLogger::resume_from_file(&audit_path);
        if tachy_dir.exists() {
            if let Ok(sink) = FileAuditSink::new(&audit_path) {
                audit_logger.add_sink(sink);
            }
        }

        // Load governance policy
        let config = PlatformConfig::load(
            env::current_dir()?.join(".tachy").join("config.json"),
        );
        let governance = config.governance.clone();

        // Create backend from registry
        // Create backend — use OllamaBackend directly for streaming support
        let registry = BackendRegistry::with_defaults();
        let model_entry = registry.find_model(&model);
        let base_url = "http://localhost:11434".to_string();

        let backend = if model_entry.map(|e| format!("{:?}", e.backend)) == Some("Ollama".to_string()) {
            let mut ollama = backend::OllamaBackend::new(model.clone(), base_url, enable_tools)
                .map_err(|e| e.to_string())?;
            // Enable real-time token streaming to stdout
            ollama.set_stream_callback(|token| {
                use std::io::Write;
                let _ = io::stdout().write_all(token.as_bytes());
                let _ = io::stdout().flush();
            });
            DynBackend::new(Box::new(ollama))
        } else {
            let client = registry.create_client(&model, enable_tools)?;
            DynBackend::new(client)
        };

        let undo_stack = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let runtime = ConversationRuntime::new(
            Session::new(),
            backend,
            CliToolExecutor::with_undo_stack(undo_stack.clone()),
            permission_policy_from_env(),
            system_prompt.clone(),
        );

        // Log session start
        audit_logger.log(
            &AuditEvent::new(&session_id, AuditEventKind::SessionStart, "interactive session started")
                .with_model(&model),
        );

        Ok(Self {
            model,
            session_id,
            system_prompt,
            runtime,
            audit_logger,
            audit_event_count: 1,
            governance,
            tool_invocation_counts: std::collections::BTreeMap::new(),
            total_tool_invocations: 0,
            undo_stack,
        })
    }

    fn run_turn(&mut self, input: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Log user message
        self.audit_logger.log(
            &AuditEvent::new(&self.session_id, AuditEventKind::UserMessage, "user input")
                .with_redacted_payload(truncate(input, 200)),
        );
        self.audit_event_count += 1;

        let mut spinner = Spinner::new();
        let mut stdout = io::stdout();
        spinner.tick(
            "Thinking",
            TerminalRenderer::new().color_theme(),
            &mut stdout,
        )?;

        let result = self.runtime.run_turn(input, None);
        match result {
            Ok(summary) => {
                spinner.finish(
                    &format!("Done ({} iteration{}, {} tool call{})",
                        summary.iterations,
                        if summary.iterations == 1 { "" } else { "s" },
                        summary.tool_results.len(),
                        if summary.tool_results.len() == 1 { "" } else { "s" },
                    ),
                    TerminalRenderer::new().color_theme(),
                    &mut stdout,
                )?;

                // Show tool calls with clear formatting
                for msg in &summary.assistant_messages {
                    for block in &msg.blocks {
                        if let ContentBlock::ToolUse { name, input, .. } = block {
                            // Parse input to show key parameters
                            let params = summarize_tool_params(name, input);
                            execute!(
                                stdout,
                                SetForegroundColor(Color::Cyan),
                                Print(format!("  ▸ {name}")),
                                SetForegroundColor(Color::DarkGrey),
                                Print(format!("({params})\n")),
                                ResetColor
                            )?;
                        }
                    }
                }
                for tool_msg in &summary.tool_results {
                    for block in &tool_msg.blocks {
                        if let ContentBlock::ToolResult { tool_name, output, is_error, .. } = block {
                            if *is_error {
                                execute!(
                                    stdout,
                                    SetForegroundColor(Color::Red),
                                    Print(format!("  ✘ {tool_name}: {}\n", output.lines().next().unwrap_or(""))),
                                    ResetColor
                                )?;
                            }
                        }
                    }
                }

                // Print assistant response text
                let renderer = TerminalRenderer::new();
                for msg in &summary.assistant_messages {
                    for block in &msg.blocks {
                        if let ContentBlock::Text { text } = block {
                            if !text.trim().is_empty() {
                                let rendered = renderer.render_markdown(text);
                                println!("{rendered}");
                            }
                        }
                    }
                }

                // Log assistant response
                self.audit_logger.log(
                    &AuditEvent::new(
                        &self.session_id,
                        AuditEventKind::AssistantMessage,
                        format!("iterations={} tools={}", summary.iterations, summary.tool_results.len()),
                    )
                    .with_model(&self.model),
                );
                self.audit_event_count += 1;

                // Log each tool invocation
                for tool_msg in &summary.tool_results {
                    for block in &tool_msg.blocks {
                        if let ContentBlock::ToolResult { tool_name, is_error, .. } = block {
                            let count = self.tool_invocation_counts
                                .entry(tool_name.clone())
                                .or_insert(0);
                            *count += 1;
                            self.total_tool_invocations += 1;

                            // Check governance
                            if let Some(violation) = self.governance.check_tool_invocation(
                                tool_name,
                                "",
                                self.total_tool_invocations,
                                *count,
                            ) {
                                self.audit_logger.log(
                                    &violation.to_audit_event(&self.session_id),
                                );
                                self.audit_event_count += 1;
                                eprintln!("⚠ Governance: {}", violation.detail);
                            }

                            let severity = if *is_error {
                                AuditSeverity::Warning
                            } else {
                                AuditSeverity::Info
                            };
                            self.audit_logger.log(
                                &AuditEvent::new(
                                    &self.session_id,
                                    AuditEventKind::ToolResult,
                                    format!("tool={tool_name} error={is_error}"),
                                )
                                .with_severity(severity)
                                .with_tool(tool_name),
                            );
                            self.audit_event_count += 1;
                        }
                    }
                }

                println!();
                Ok(())
            }
            Err(error) => {
                spinner.fail(
                    "Request failed",
                    TerminalRenderer::new().color_theme(),
                    &mut stdout,
                )?;
                self.audit_logger.log(
                    &AuditEvent::new(&self.session_id, AuditEventKind::SessionEnd, error.to_string())
                        .with_severity(AuditSeverity::Warning),
                );
                self.audit_event_count += 1;
                Err(Box::new(error))
            }
        }
    }

    fn print_status(&self) {
        let usage = self.runtime.usage().cumulative_usage();
        println!(
            "model={} messages={} turns={} input_tokens={} output_tokens={} tool_calls={} audit_events={}",
            self.model,
            self.runtime.session().messages.len(),
            self.runtime.usage().turns(),
            usage.input_tokens,
            usage.output_tokens,
            self.total_tool_invocations,
            self.audit_event_count,
        );
    }

    fn compact(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let result = self.runtime.compact(CompactionConfig::default());
        let removed = result.removed_message_count;

        let registry = BackendRegistry::with_defaults();
        let client = registry.create_client(&self.model, true)?;
        let backend = DynBackend::new(client);

        self.runtime = ConversationRuntime::new(
            result.compacted_session,
            backend,
            CliToolExecutor::with_undo_stack(self.undo_stack.clone()),
            permission_policy_from_env(),
            self.system_prompt.clone(),
        );

        self.audit_logger.log(
            &AuditEvent::new(
                &self.session_id,
                AuditEventKind::SessionCompacted,
                format!("removed {removed} messages"),
            ),
        );
        self.audit_event_count += 1;

        println!("Compacted {removed} messages.");
        Ok(())
    }

    /// Save the current session to disk for later resumption.
    fn save_session(&self) -> Result<String, Box<dyn std::error::Error>> {
        let sessions_dir = env::current_dir()?.join(".tachy").join("sessions");
        std::fs::create_dir_all(&sessions_dir)?;
        let path = sessions_dir.join(format!("{}.json", self.session_id));
        let json = serde_json::to_string_pretty(self.runtime.session())?;
        std::fs::write(&path, json)?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// Show the last session file available for resumption.
    fn last_session_path() -> Option<std::path::PathBuf> {
        let sessions_dir = env::current_dir().ok()?.join(".tachy").join("sessions");
        let mut entries: Vec<_> = std::fs::read_dir(&sessions_dir).ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));
        entries.first().map(|e| e.path())
    }
}

// ---------------------------------------------------------------------------
// Tool executor and helpers
// ---------------------------------------------------------------------------

struct CliToolExecutor {
    /// Shared undo stack for /undo support: (path, previous_content)
    undo_stack: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

impl CliToolExecutor {
    #[allow(dead_code)]
    fn new() -> Self {
        Self {
            undo_stack: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn with_undo_stack(undo_stack: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>) -> Self {
        Self { undo_stack }
    }
}

impl ToolExecutor for CliToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        let value = serde_json::from_str(input)
            .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;

        // Show what tool is being called with a compact summary
        let input_summary = summarize_tool_input(tool_name, &value);
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!("  ⚡ {tool_name} {input_summary}\n")),
            ResetColor
        );

        // Use execute_tool_with_diff for write/edit to get diff previews
        let result = if tool_name == "write_file" || tool_name == "edit_file" {
            match execute_tool_with_diff(tool_name, &value) {
                Ok((output, Some(preview))) => {
                    // Show colored diff preview in the terminal
                    if !preview.diff_colored.is_empty() && (preview.additions > 0 || preview.deletions > 0) {
                        let _ = execute!(
                            stdout,
                            SetForegroundColor(Color::DarkYellow),
                            Print(format!("    ┌─ diff: {}\n", preview.summary)),
                            ResetColor
                        );
                        for line in preview.diff_colored.lines() {
                            let _ = execute!(stdout, Print(format!("    │ {line}\n")));
                        }
                        let _ = execute!(
                            stdout,
                            SetForegroundColor(Color::DarkYellow),
                            Print("    └─\n"),
                            ResetColor
                        );
                    }
                    Ok(output)
                }
                Ok((output, None)) => Ok(output),
                Err(e) => Err(e),
            }
        } else {
            execute_tool(tool_name, &value)
        };

        match result {
            Ok(output) => {
                // Track file modifications for /undo
                match tool_name {
                    "write_file" => {
                        if let Some(_path) = value.get("path").and_then(|v| v.as_str()) {
                            // For write_file, the file might not have existed before
                            // We store empty string to indicate "delete on undo"
                            // But if it existed, we'd need the old content — read it before write
                            // Since the tool already wrote, we can't get the old content here
                            // The edit_file output includes original_file though
                        }
                    }
                    "edit_file" => {
                        // edit_file output includes the original file content
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output) {
                            if let (Some(path), Some(original)) = (
                                parsed.get("filePath").and_then(|v| v.as_str()),
                                parsed.get("originalFile").and_then(|v| v.as_str()),
                            ) {
                                if let Ok(mut stack) = self.undo_stack.lock() {
                                    stack.push((path.to_string(), original.to_string()));
                                }
                            }
                        }
                    }
                    _ => {}
                }

                // For read_file, show a brief preview so the user knows what was read
                if tool_name == "read_file" {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output) {
                        if let Some(content) = parsed.get("content").and_then(|v| v.as_str()) {
                            let lines = content.lines().count();
                            let path = parsed.get("filePath").and_then(|v| v.as_str()).unwrap_or("?");
                            let _ = execute!(
                                stdout,
                                SetForegroundColor(Color::DarkGrey),
                                Print(format!("    ↳ {path} ({lines} lines)\n")),
                                ResetColor
                            );
                        }
                    }
                }
                Ok(output)
            }
            Err(error) => {
                let _ = execute!(
                    stdout,
                    SetForegroundColor(Color::Red),
                    Print(format!("    ✘ {error}\n")),
                    ResetColor
                );
                Err(ToolError::new(error))
            }
        }
    }
}

/// Create a compact one-line summary of tool input for display.
fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    match tool_name {
        "bash" => input.get("command")
            .and_then(|v| v.as_str())
            .map(|c| format!("`{}`", truncate(c, 80)))
            .unwrap_or_default(),
        "read_file" | "write_file" | "edit_file" => input.get("path")
            .and_then(|v| v.as_str())
            .map(|p| p.to_string())
            .unwrap_or_default(),
        "grep_search" => input.get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| format!("/{p}/"))
            .unwrap_or_default(),
        "glob_search" => input.get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| p.to_string())
            .unwrap_or_default(),
        "list_directory" => input.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string(),
        _ => String::new(),
    }
}

fn permission_policy_from_env() -> PermissionPolicy {
    let mode =
        env::var("TACHY_PERMISSION_MODE").unwrap_or_else(|_| "workspace-write".to_string());
    match mode.as_str() {
        "read-only" => PermissionPolicy::new(PermissionMode::Deny)
            .with_tool_mode("read_file", PermissionMode::Allow)
            .with_tool_mode("glob_search", PermissionMode::Allow)
            .with_tool_mode("grep_search", PermissionMode::Allow),
        "deny-all" => PermissionPolicy::new(PermissionMode::Deny),
        _ => PermissionPolicy::new(PermissionMode::Allow),
    }
}

fn build_system_prompt() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    Ok(load_system_prompt(
        env::current_dir()?,
        DEFAULT_DATE,
        env::consts::OS,
        "unknown",
    )?)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

/// Summarize tool call parameters for display (e.g. "path: src/main.rs" or "command: ls -la").
fn summarize_tool_params(tool_name: &str, input_json: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(input_json) {
        Ok(v) => v,
        Err(_) => return truncate(input_json, 60),
    };
    let obj = match parsed.as_object() {
        Some(o) => o,
        None => return truncate(input_json, 60),
    };

    match tool_name {
        "bash" => obj.get("command")
            .and_then(|v| v.as_str())
            .map(|s| truncate(s, 80))
            .unwrap_or_default(),
        "read_file" | "write_file" | "list_directory" => obj.get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| ".".to_string()),
        "edit_file" => {
            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let old = obj.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
            let preview = old.lines().next().unwrap_or("");
            format!("{path}: {}", truncate(preview, 50))
        }
        "grep_search" => obj.get("pattern")
            .and_then(|v| v.as_str())
            .map(|s| format!("/{s}/"))
            .unwrap_or_default(),
        "glob_search" => obj.get("pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default(),
        _ => {
            // Generic: show first string value
            obj.values()
                .find_map(|v| v.as_str())
                .map(|s| truncate(s, 60))
                .unwrap_or_default()
        }
    }
}

fn print_help() {
    println!("tachy — Local AI Coding Agent\n");
    println!("Usage:");
    println!("  tachy init                                Initialize workspace (.tachy/)");
    println!("  tachy setup                               Full setup: check Ollama, pull model, init workspace");
    println!("  tachy doctor                              Check Ollama, GPU, models, test tool calling");
    println!("  tachy pull <model>                        Pull a model via Ollama");
    println!("  tachy models                              List registered models");
    println!("  tachy models --local                      List locally installed models");
    println!("  tachy agents                              List agent templates");
    println!("  tachy verify-audit                        Verify audit trail integrity");
    println!("  tachy warmup [MODEL]                      Pre-load model into GPU memory");
    println!("  tachy [--model MODEL]                     Start interactive REPL");
    println!("  tachy [--model MODEL] prompt TEXT          Send one prompt (streams output)");
    println!("  tachy run-agent <template> <prompt...>     Run an agent template");
    println!("  tachy serve [ADDR]                         Start HTTP daemon + web UI (default 127.0.0.1:7777)");
    println!("  tachy --continue                           Resume last session");
    println!("  tachy --resume SESSION.json [/compact]     Resume a specific session");
    println!("\nREPL commands:");
    println!("  /help /status /compact /save /undo /model /audit /exit");
    println!("\nHTTP API (when running `tachy serve`):");
    println!("  GET  /health              Health check");
    println!("  GET  /api/models          List models");
    println!("  GET  /api/templates       List agent templates");
    println!("  GET  /api/agents          List all agents");
    println!("  GET  /api/agents/:id      Get agent status (poll for async results)");
    println!("  POST /api/agents/run      Start agent (async, returns 202)");
    println!("  POST /api/tasks/schedule  Schedule recurring agent");
    println!("\nEnvironment:");
    println!("  TACHY_PERMISSION_MODE   read-only | workspace-write | deny-all");
    println!("  TACHY_API_KEY           API key for HTTP daemon authentication");
    println!("  OLLAMA_HOST             Ollama URL (default http://localhost:11434)");
    println!("\nDefault model: gemma4:26b (Gemma 4 26B MoE — native tool calling, 256K context)");
}

#[cfg(test)]
mod tests {
    use super::{parse_args, CliAction, DEFAULT_MODEL};
    use std::path::PathBuf;

    #[test]
    fn defaults_to_repl_when_no_args() {
        assert_eq!(
            parse_args(&[]).expect("args should parse"),
            CliAction::Repl {
                model: DEFAULT_MODEL.to_string(),
            }
        );
    }

    #[test]
    fn parses_prompt_subcommand() {
        let args = vec![
            "prompt".to_string(),
            "hello".to_string(),
            "world".to_string(),
        ];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Prompt {
                prompt: "hello world".to_string(),
                model: DEFAULT_MODEL.to_string(),
            }
        );
    }

    #[test]
    fn parses_init_command() {
        let args = vec!["init".to_string()];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Init,
        );
    }

    #[test]
    fn parses_models_command() {
        let args = vec!["models".to_string()];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::ListModels,
        );
    }

    #[test]
    fn parses_model_override() {
        let args = vec![
            "--model".to_string(),
            "qwen2.5-coder:7b".to_string(),
            "prompt".to_string(),
            "hello".to_string(),
        ];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Prompt {
                prompt: "hello".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            }
        );
    }

    #[test]
    fn parses_resume_flag_with_slash_command() {
        let args = vec![
            "--resume".to_string(),
            "session.json".to_string(),
            "/compact".to_string(),
        ];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::ResumeSession {
                session_path: PathBuf::from("session.json"),
                command: Some("/compact".to_string()),
            }
        );
    }
}
