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
use render::{approval_prompt, ApprovalChoice, Spinner, TerminalRenderer};
use runtime::{
    load_system_prompt, preview_edit_file, preview_write_file, CompactionConfig, ContentBlock,
    ConversationRuntime, PermissionMode, PermissionPolicy,
    RuntimeEvent, Session, ToolError, ToolExecutor,
};
use tools::{execute_tool, execute_tool_with_diff};

const DEFAULT_MODEL: &str = "gemma4:26b";
const DEFAULT_DATE: &str = "2026-04-03";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}


async fn run() -> Result<(), Box<dyn std::error::Error>> {
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
            cli.run_turn(&prompt)?
        }
        CliAction::Repl { model } => run_repl(model)?,

        CliAction::Init => init_workspace()?,
        CliAction::Setup => run_setup_wizard()?,
        CliAction::ListModels => list_models(),
        CliAction::ListModelsLocal => list_models_local(),
        CliAction::ListAgents => list_agents(),
        CliAction::Serve { addr, workspace } => run_serve(&addr, workspace.as_deref())?,
        CliAction::RunAgent { template, prompt, model } => run_agent_cmd(&template, &prompt, &model)?,
        CliAction::Doctor { json } => run_doctor(json),
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
        CliAction::McpConnect { server } => run_mcp_connect(server.as_deref())?,
        CliAction::PublishAgent { path } => publish_agent(&path)?,
        CliAction::Deploy { profile, region } => run_deploy(profile.as_deref(), region.as_deref())?,
        CliAction::Activate { key } => activate_license(&key)?,
        CliAction::LicenseStatus => show_license_status()?,
        CliAction::Help => print_help(),
        CliAction::Search { query, limit } => run_search(&query, limit)?,
        CliAction::Pipeline { subcommand, path, dry_run } => run_pipeline(&subcommand, &path, dry_run)?,
        CliAction::Graph { format, file } => run_graph(&format, file.as_deref())?,
        CliAction::Monorepo => run_monorepo()?,
        CliAction::Dashboard => run_dashboard()?,
        CliAction::ExportAudit { format, output } => run_export_audit(&format, output.as_deref())?,
        CliAction::Finetune { output, base_model } => run_finetune(output.as_deref(), base_model.as_deref())?,
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
        model: Option<String>,
    },
    Init,
    Setup,
    ListModels,
    ListModelsLocal,
    ListAgents,
    Serve { addr: String, workspace: Option<PathBuf> },
    RunAgent { template: String, prompt: String, model: String },
    Doctor { json: bool },
    Pull { model: String },
    VerifyAudit,
    Warmup { model: String },
    InstallOllama,
    ListTools,
    ListChannels,
    McpServer,
    McpConnect { server: Option<String> },
    PublishAgent { path: String },
    Activate { key: String },
    Deploy { profile: Option<String>, region: Option<String> },
    LicenseStatus,
    Help,
    Search { query: String, limit: usize },
    Pipeline { subcommand: String, path: String, dry_run: bool },
    /// C1: dependency/call graph
    Graph { format: String, file: Option<String> },
    /// C3: monorepo workspace detection
    Monorepo,
    /// E2: dashboard metrics
    Dashboard,
    /// F1: export audit log in various compliance formats
    ExportAudit { format: String, output: Option<String> },
    /// F3: fine-tuning dataset extraction + LoRA script generation
    Finetune { output: Option<String>, base_model: Option<String> },
}

fn parse_args(args: &[String]) -> Result<CliAction, String> {
    let mut model = DEFAULT_MODEL.to_string();
    let mut model_explicit = false;
    let mut rest = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--model" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --model".to_string())?;
                model = value.clone();
                model_explicit = true;
                index += 2;
            }
            flag if flag.starts_with("--model=") => {
                model = flag[8..].to_string();
                model_explicit = true;
                index += 1;
            }
            "--json" => {
                // Set global JSON output mode
                std::env::set_var("TACHY_OUTPUT_JSON", "1");
                index += 1;
            }
            other => {
                rest.push(other.to_string());
                index += 1;
            }
        }
    }

    if rest.is_empty() {
        return Ok(CliAction::Repl { model: if model_explicit { Some(model) } else { None } });
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
        "doctor" => {
            let json = rest.iter().any(|a| a == "--json");
            Ok(CliAction::Doctor { json })
        }
        "verify-audit" => Ok(CliAction::VerifyAudit),
        "warmup" => {
            let warmup_model = rest.get(1).cloned().unwrap_or_else(|| model.clone());
            Ok(CliAction::Warmup { model: warmup_model })
        }
        "install-ollama" => Ok(CliAction::InstallOllama),
        "tools" => Ok(CliAction::ListTools),
        "channels" => Ok(CliAction::ListChannels),
        "mcp-server" => Ok(CliAction::McpServer),
        "mcp-connect" => {
            let server = rest.get(1).map(|s| s.to_string());
            Ok(CliAction::McpConnect { server })
        }
        "publish" => {
            let path = rest.get(1).ok_or("usage: publish <agent.yaml>")?;
            Ok(CliAction::PublishAgent { path: path.to_string() })
        }
        "activate" => {
            let key = rest.get(1).ok_or("usage: activate <LICENSE_KEY>")?;
            Ok(CliAction::Activate { key: key.clone() })
        }
        "license" => Ok(CliAction::LicenseStatus),
        "pull" => {
            let model_name = rest.get(1).ok_or("usage: pull <model>")?;
            Ok(CliAction::Pull { model: model_name.clone() })
        }
        "deploy" => {
            let mut profile = None;
            let mut region = None;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--profile" | "-p" => { profile = rest.get(i+1).cloned(); i += 2; }
                    "--region" | "-r" => { region = rest.get(i+1).cloned(); i += 2; }
                    _ => { i += 1; }
                }
            }
            Ok(CliAction::Deploy { profile, region })
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
        "search" => {
            let query = rest[1..].join(" ");
            if query.trim().is_empty() {
                return Err("usage: tachy search <query>".to_string());
            }
            let limit = 10;
            Ok(CliAction::Search { query, limit })
        }
        "pipeline" => {
            let subcommand = rest.get(1).cloned().unwrap_or_else(|| "run".to_string());
            let path = rest.get(2).cloned().unwrap_or_else(|| "pipeline.yaml".to_string());
            let dry_run = rest.iter().any(|a| a == "--dry-run");
            Ok(CliAction::Pipeline { subcommand, path, dry_run })
        }
        "graph" => {
            let format = rest.iter()
                .find(|a| a.starts_with("--format="))
                .map(|a| a[9..].to_string())
                .or_else(|| {
                    rest.iter().position(|a| a == "--format")
                        .and_then(|i| rest.get(i + 1).cloned())
                })
                .unwrap_or_else(|| "json".to_string());
            let file = rest.iter()
                .find(|a| a.starts_with("--file="))
                .map(|a| a[7..].to_string())
                .or_else(|| {
                    rest.iter().position(|a| a == "--file")
                        .and_then(|i| rest.get(i + 1).cloned())
                });
            Ok(CliAction::Graph { format, file })
        }
        "monorepo" => Ok(CliAction::Monorepo),
        "dashboard" => Ok(CliAction::Dashboard),
        "export-audit" => {
            let format = rest.iter()
                .find(|a| a.starts_with("--format="))
                .map(|a| a[9..].to_string())
                .or_else(|| {
                    rest.iter().position(|a| a == "--format")
                        .and_then(|i| rest.get(i + 1).cloned())
                })
                .unwrap_or_else(|| "json".to_string());
            let output_opt = rest.iter()
                .find(|a| a.starts_with("--output="))
                .map(|a| a[9..].to_string())
                .or_else(|| {
                    rest.iter().position(|a| a == "--output" || a == "-o")
                        .and_then(|i| rest.get(i + 1).cloned())
                });
            Ok(CliAction::ExportAudit { format, output: output_opt })
        }
        "finetune" => {
            let output_opt = rest.iter()
                .find(|a| a.starts_with("--output="))
                .map(|a| a[9..].to_string())
                .or_else(|| {
                    rest.iter().position(|a| a == "--output" || a == "-o")
                        .and_then(|i| rest.get(i + 1).cloned())
                });
            let base_model = rest.iter()
                .find(|a| a.starts_with("--base-model="))
                .map(|a| a[13..].to_string())
                .or_else(|| {
                    rest.iter().position(|a| a == "--base-model")
                        .and_then(|i| rest.get(i + 1).cloned())
                });
            Ok(CliAction::Finetune { output: output_opt, base_model })
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

fn json_output_enabled() -> bool {
    std::env::var("TACHY_OUTPUT_JSON").map(|v| v == "1").unwrap_or(false)
}

fn list_models() {
    let registry = BackendRegistry::with_defaults();
    if json_output_enabled() {
        let models: Vec<serde_json::Value> = registry.list_models().iter().map(|m| {
            serde_json::json!({
                "name": m.name,
                "backend": format!("{:?}", m.backend),
                "context_window": m.context_window,
                "supports_tool_use": m.supports_tool_use,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&models).unwrap_or_default());
        return;
    }
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

fn run_doctor(json: bool) {
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
        // GPU info for JSON output
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
                    Err(e) => {
                        println!("✘ failed: {e}");
                    }
                }
            }
            Err(e) => {
                println!("✘ could not create client: {e}");
            }
        }

        // Throughput benchmark — count tokens in a short generation
        let registry2 = BackendRegistry::with_defaults();
        let client2 = registry2.create_client(test_model, true)
            .or_else(|_| {
                backend::OllamaBackend::new(
                    test_model.to_string(),
                    base_url.to_string(),
                    true,
                )
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
                    // Rough estimate: 1 token ≈ 4 characters
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

    // Disk space
    println!();
    match disk_free_gb {
        Some(gb) if gb < 5 => println!("  ⚠ Disk free: {gb} GB  (low — consider freeing space)"),
        Some(gb) => println!("  ✓ Disk free: {gb} GB"),
        None => println!("  ? Disk free: unable to determine"),
    }

    // ── GPU / VRAM stats ─────────────────────────────────────────────────
    println!();
    print_gpu_stats(json);
}

/// Query GPU and VRAM availability.
/// - macOS: uses `system_profiler SPDisplaysDataType`
/// - Linux/CUDA: uses `nvidia-smi`
/// Falls back gracefully if neither tool is present.
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
            // Apple Silicon — unified memory, no separate VRAM budget reported
            println!("  ✓ GPU: {name} (Apple Silicon — unified memory)");
        }
        _ => {
            println!("  ? GPU: not detected (nvidia-smi unavailable — running in CPU mode)");
        }
    }
}

/// Returns `(gpu_name, vram_total_mb, vram_free_mb, utilization_pct)`.
fn query_gpu_info() -> (Option<String>, Option<u64>, Option<u64>, Option<u64>) {
    // NVIDIA via nvidia-smi
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

    // Apple Silicon via system_profiler
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

fn run_pull(model: &str) -> Result<(), Box<dyn std::error::Error>> {
    backend::pull_model(model).map_err(|e| e.into())
}

fn publish_agent(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::path::Path::new(path);
    if !source.exists() {
        return Err(format!("file not found: {path}").into());
    }

    let content = std::fs::read_to_string(source)?;

    // Extract name from YAML (simple: look for "name: <value>")
    let name = content.lines()
        .find(|l| l.trim_start().starts_with("name:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .or_else(|| {
            // Try JSON
            serde_json::from_str::<serde_json::Value>(&content).ok()
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        })
        .ok_or("agent file must have a 'name' field")?;

    // Copy to .tachy/agents/
    let agents_dir = env::current_dir()?.join(".tachy").join("agents");
    std::fs::create_dir_all(&agents_dir)?;
    let dest = agents_dir.join(format!("{name}.yaml"));
    std::fs::copy(source, &dest)?;

    if json_output_enabled() {
        println!("{}", serde_json::json!({"name": name, "path": dest.to_string_lossy()}));
    } else {
        println!("Published agent '{}' to {}", name, dest.display());
    }
    Ok(())
}

fn run_mcp_connect(server_filter: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let tachy_dir = env::current_dir()?.join(".tachy");
    let config_path = tachy_dir.join("config.json");

    // Load MCP server configs from .tachy/config.json
    let configs: Vec<daemon::McpServerConfig> = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let parsed: serde_json::Value = serde_json::from_str(&content)?;
        if let Some(servers) = parsed.get("mcp_servers").and_then(|v| v.as_array()) {
            servers.iter()
                .filter_map(|s| serde_json::from_value(s.clone()).ok())
                .filter(|s: &daemon::McpServerConfig| {
                    server_filter.map_or(true, |f| s.name == f)
                })
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    if configs.is_empty() {
        if server_filter.is_some() {
            println!("No MCP server matching '{}' found in .tachy/config.json", server_filter.unwrap());
        } else {
            println!("No MCP servers configured. Add to .tachy/config.json:");
            println!(r#"  "mcp_servers": [{{"name": "example", "command": "uvx", "args": ["mcp-server-example"]}}]"#);
        }
        return Ok(());
    }

    println!("Connecting to {} MCP server(s)...\n", configs.len());
    let mgr = daemon::McpClientManager::connect_all(&configs);
    let tools = mgr.all_tools();

    if tools.is_empty() {
        println!("Connected but no tools discovered.");
    } else {
        println!("Discovered {} tool(s):\n", tools.len());
        for tool in &tools {
            println!("  {} — {}", tool.qualified_name(), tool.description);
        }
    }
    println!("\nMCP connection test complete.");
    Ok(())
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
    if json_output_enabled() {
        let agents: Vec<serde_json::Value> = config.agent_templates.iter().map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "model": t.model,
                "max_iterations": t.max_iterations,
                "requires_approval": t.requires_approval,
                "allowed_tools": t.allowed_tools,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&agents).unwrap_or_default());
        return;
    }
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
        None, // No shared daemon state in CLI direct mode
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

fn run_deploy(profile: Option<&str>, region: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Tachy Enterprise: Cloud Bridge (AWS Batch)");
    println!("==========================================");

    let cwd = env::current_dir()?;
    let tachy_dir = cwd.join(".tachy");
    if !tachy_dir.exists() {
        return Err("Workspace not initialized — run: tachy init".into());
    }

    // 1. Bundle Workspace
    print!("📦 Bundling workspace... ");
    io::stdout().flush()?;
    let version = "0.1.0"; // Should match Cargo.toml
    let bundle_name = format!("tachy-workspace-{}.tar.gz", chrono_now_secs());
    
    // In a production scenario, we'd use 'tar' or a native library to exclude .git/node_modules/target
    println!("✓ (ready for packaging)");

    // 2. AWS Pre-flight
    println!("☁ Checking AWS environment:");
    if let Some(p) = profile { println!("  Profile: {}", p); }
    if let Some(r) = region { println!("  Region:  {}", r); }
    
    // Check if aws cli is available as a fallback for missing SDK
    let has_aws_cli = std::process::Command::new("aws").arg("--version").output().is_ok();
    if !has_aws_cli {
        println!("  ⚠ Warning: 'aws' CLI not found. Job submission will require native SDK.");
    } else {
        println!("  ✓ AWS CLI detected");
    }

    // 3. Containerization (OCI)
    print!("🐳 Packaging OCI container... ");
    io::stdout().flush()?;
    let has_docker = std::process::Command::new("docker").arg("--version").output().is_ok();
    if has_docker {
        println!("✓ Docker ready");
        // We'd run: docker build -t tachy-agent:latest .
    } else {
        println!("⚠ Docker not found (required for local image builds)");
    }

    println!("\nNext Steps for Phase 4.2:");
    println!("  1. Upload bundle to S3: s3://tachy-deployments/{bundle_name}");
    println!("  2. Submit Batch Job: SubmitJob(jobDefinition='tachy-agent-{}", version);
    println!("  3. Monitor progress at http://localhost:7777/dashboard (Cloud Tab)");
    
    println!("\nDeployment engine initialized. Waiting for storage credentials.");
    Ok(())
}

fn chrono_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// REPL and LiveCli — now using BackendRegistry
// ---------------------------------------------------------------------------

fn run_repl(model: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve model: explicit CLI arg > persisted .tachy/config.json > default
    let model = model.unwrap_or_else(|| {
        std::env::current_dir().ok()
            .map(|d| d.join(".tachy").join("config.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["model"].as_str().map(str::to_string))
            .unwrap_or_else(|| DEFAULT_MODEL.to_string())
    });
    let registry = BackendRegistry::with_defaults();
    let mut cli = LiveCli::new(model, true)?;
    let editor = input::LineEditor::new("› ");

    // ── Auto-resume: prompt if there is a recent saved session ───────────
    let mut stdout = io::stdout();
    if let Some(last_path) = LiveCli::last_session_path() {
        if let Ok(meta) = last_path.metadata() {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    let secs = elapsed.as_secs();
                    let age = if secs < 120 {
                        format!("{secs}s ago")
                    } else if secs < 7200 {
                        format!("{}m ago", secs / 60)
                    } else if secs < 172800 {
                        format!("{}h ago", secs / 3600)
                    } else {
                        format!("{}d ago", secs / 86400)
                    };
                    execute!(
                        stdout,
                        SetForegroundColor(Color::DarkYellow),
                        Print(format!("Resume session from {}? [y/N] ", age)),
                        ResetColor,
                    )?;
                    stdout.flush()?;
                    let mut answer = String::new();
                    io::stdin().read_line(&mut answer).ok();
                    if answer.trim().eq_ignore_ascii_case("y") {
                        if let Ok(content) = std::fs::read_to_string(&last_path) {
                            if let Ok(session) = serde_json::from_str::<Session>(&content) {
                                let msg_count = session.messages.len();
                                cli.runtime.restore_session(session);
                                execute!(
                                    stdout,
                                    SetForegroundColor(Color::Green),
                                    Print(format!("  ✓ Resumed ({msg_count} messages)\n\n")),
                                    ResetColor,
                                )?;
                            }
                        }
                    } else {
                        println!();
                    }
                }
            }
        }
    }

    // ── Greeting ──────────────────────────────────────────────────────────
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
        Print("Commands: /help /status /compact /model /sessions /fix /test /review /commit /explain /exit\n"),
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
                println!("  /help             Show this help");
                println!("  /status           Show session status and stats");
                println!("  /compact          Compact session history (saves tokens)");
                println!("  /save             Save session to disk");
                println!("  /undo             Undo last file edit");
                println!("  /model [name]     Show current model / switch to a different one");
                println!("  /sessions         List saved sessions");
                println!("  /audit            Show audit event count");
                println!("  /fix <desc>       Fix an issue (describe it or leave blank for last error)");
                println!("  /test             Run the project test suite and report failures");
                println!("  /review           Review staged git changes");
                println!("  /commit           Generate a conventional commit message");
                println!("  /explain [file]   Explain a file or the current directory");
                println!("  /exit             Quit the REPL");
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
            s if s.starts_with("/model") => {
                let arg = s[6..].trim();
                if arg.is_empty() {
                    println!("Current model: {}", cli.model);
                    let reg = BackendRegistry::with_defaults();
                    println!("Available models:");
                    for m in reg.list_models() {
                        let marker = if m.name == cli.model { "▶" } else { " " };
                        println!("  {marker} {} ({}K ctx)", m.name, m.context_window / 1000);
                    }
                } else {
                    match cli.switch_model(arg) {
                        Ok(()) => println!("Switched to {arg}"),
                        Err(e) => eprintln!("Could not switch model: {e}"),
                    }
                }
            }
            "/audit" => println!("Audit events logged: {}", cli.audit_event_count),
            "/sessions" => {
                let sessions_dir = std::env::current_dir()
                    .unwrap_or_default()
                    .join(".tachy")
                    .join("sessions");
                match std::fs::read_dir(&sessions_dir) {
                    Ok(entries) => {
                        let mut sessions: Vec<_> = entries
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                e.path().extension().map(|x| x == "json").unwrap_or(false)
                            })
                            .collect();
                        sessions.sort_by_key(|e| std::cmp::Reverse(
                            e.metadata().ok().and_then(|m| m.modified().ok())
                        ));
                        if sessions.is_empty() {
                            println!("No saved sessions.");
                        } else {
                            println!("Saved sessions (most recent first):");
                            for (i, entry) in sessions.iter().take(10).enumerate() {
                                let name = entry.file_name();
                                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                                println!("  {}. {} ({:.1} KB)", i + 1,
                                    name.to_string_lossy(), size as f64 / 1024.0);
                            }
                            println!("\nResume with: tachy --resume <session-id>");
                        }
                    }
                    Err(_) => println!("No sessions directory found."),
                }
            }
            // ── Productive slash commands ──────────────────────────────────
            s if s.starts_with("/fix") => {
                let desc = s[4..].trim();
                let prompt = if desc.is_empty() {
                    "Find and fix the most obvious bug or error in this codebase. \
                     Run the tests after fixing to verify the fix works.".to_string()
                } else {
                    format!("Fix this issue: {desc}\n\n\
                             Steps: 1) Locate the relevant code. 2) Apply the fix. \
                             3) Run the tests to verify.")
                };
                cli.run_turn(&prompt)?;
            }
            "/test" => {
                cli.run_turn(
                    "Run the project's test suite. Show the full output. \
                     If any tests fail, analyze each failure and explain exactly what needs to be fixed."
                )?;
            }
            "/review" => {
                cli.run_turn(
                    "Review the staged git changes (run: git diff --staged). \
                     Provide: 1) A concise summary of what changed and why, \
                     2) Any potential bugs or correctness issues, \
                     3) Style and naming feedback, \
                     4) Concrete suggestions for improvement."
                )?;
            }
            "/commit" => {
                cli.run_turn(
                    "Generate a conventional commit message for the staged changes \
                     (run: git diff --staged to see them). \
                     Format: type(scope): short description\n\
                     Types: feat/fix/docs/style/refactor/test/chore\n\
                     Keep the subject line under 72 characters. \
                     Add a body if needed to explain motivation or breaking changes."
                )?;
            }
            s if s.starts_with("/explain") => {
                let target = s[8..].trim();
                let prompt = if target.is_empty() {
                    "Explain what this project does: list the main source files, \
                     describe each module's purpose in one sentence, and explain \
                     how they connect to each other.".to_string()
                } else {
                    format!("Explain `{target}` in plain English — its purpose, \
                             key functions/types, and how it fits into the broader codebase.")
                };
                cli.run_turn(&prompt)?;
            }
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

// ---------------------------------------------------------------------------
// LiveCli — interactive agent session holder
// ---------------------------------------------------------------------------

struct LiveCli {
    model: String,
    session_id: String,
    system_prompt: Vec<String>,
    runtime: ConversationRuntime<DynBackend, CliToolExecutor>,
    audit_logger: AuditLogger,
    audit_event_count: u64,
    governance: audit::GovernancePolicy,
    tool_invocation_counts: std::collections::BTreeMap<String, u32>,
    total_tool_invocations: u32,
    undo_stack: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    /// Last-turn latency in milliseconds (updated after each run_turn call).
    last_latency_ms: u64,
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
        let registry = BackendRegistry::with_defaults();
        let _model_entry = registry.find_model(&model);
        let _base_url = "http://localhost:11434".to_string();

        let client = registry.create_client(&model, enable_tools)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        let backend = DynBackend::new(client);

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
            last_latency_ms: 0,
        })
    }


    fn run_turn(&mut self, input: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Log user message
        self.audit_logger.log(
            &AuditEvent::new(&self.session_id, AuditEventKind::UserMessage, "user input")
                .with_redacted_payload(truncate(input, 200)),
        );
        self.audit_event_count += 1;

        // ── Streaming setup ────────────────────────────────────────────────
        // Create a per-turn channel so we can print tokens as they arrive.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<RuntimeEvent>();
        self.runtime.set_event_tx(tx);

        // Shared flag: set to true once the first TextDelta arrives so the
        // spinner can be cleared exactly once before token output begins.
        let first_token_seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first_token_clone = first_token_seen.clone();

        // Spawn a background thread to drain the channel and echo text tokens.
        // We create a minimal tokio runtime inside the thread because the
        // receiver's async recv() requires one.
        let stream_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tachy stream rt");
            rt.block_on(async move {
                let mut rx = rx;
                let mut stdout = io::stdout();
                while let Some(event) = rx.recv().await {
                    match event {
                        RuntimeEvent::TextDelta(delta) => {
                            // Clear the spinner line on the very first token
                            if !first_token_clone.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                let _ = execute!(
                                    stdout,
                                    crossterm::cursor::MoveToColumn(0),
                                    crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                                );
                            }
                            print!("{delta}");
                            let _ = stdout.flush();
                        }
                        RuntimeEvent::Finished(_) => break,
                        _ => {}
                    }
                }
            });
        });

        // ── Spinner (runs until first token or turn completes) ─────────────
        let mut spinner = Spinner::new();
        let mut stdout = io::stdout();
        let renderer = TerminalRenderer::new();
        let theme = renderer.color_theme().clone();

        let turn_start = std::time::Instant::now();

        // Tick the spinner in a thread while the turn runs synchronously.
        // We'll just draw it once before the blocking call and update after.
        spinner.tick("Thinking", &theme, &mut stdout)?;

        let result = self.runtime.run_turn(input, None);

        // Wait for the stream thread to flush all tokens
        let _ = stream_thread.join();

        let elapsed_ms = turn_start.elapsed().as_millis() as u64;
        self.last_latency_ms = elapsed_ms;

        match result {
            Ok(summary) => {
                // If no text tokens were streamed, show the completion banner.
                // If tokens were streamed, just print a newline for separation.
                if first_token_seen.load(std::sync::atomic::Ordering::SeqCst) {
                    println!();
                } else {
                    spinner.finish(
                        &format!("Done ({} iteration{}, {} tool call{}, {}ms)",
                            summary.iterations,
                            if summary.iterations == 1 { "" } else { "s" },
                            summary.tool_results.len(),
                            if summary.tool_results.len() == 1 { "" } else { "s" },
                            elapsed_ms,
                        ),
                        &theme,
                        &mut stdout,
                    )?;
                }

                // Show tool calls with clear formatting
                for msg in &summary.assistant_messages {
                    for block in &msg.blocks {
                        if let ContentBlock::ToolUse { name, input, .. } = block {
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

                // Print assistant response text (only if not already streamed token-by-token)
                if !first_token_seen.load(std::sync::atomic::Ordering::SeqCst) {
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
                }

                // Log assistant response
                self.audit_logger.log(
                    &AuditEvent::new(
                        &self.session_id,
                        AuditEventKind::AssistantMessage,
                        format!("iterations={} tools={} latency_ms={}", summary.iterations, summary.tool_results.len(), elapsed_ms),
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

                            let severity = if *is_error { AuditSeverity::Warning } else { AuditSeverity::Info };
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
                    &theme,
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

    /// Switch to a different model mid-session, preserving conversation history.
    fn switch_model(&mut self, new_model: &str) -> Result<(), Box<dyn std::error::Error>> {
        let registry = BackendRegistry::with_defaults();
        let client = registry.create_client(new_model, true)?;
        let backend = DynBackend::new(client);
        let session = self.runtime.session().clone();

        self.runtime = ConversationRuntime::new(
            session,
            backend,
            CliToolExecutor::with_undo_stack(self.undo_stack.clone()),
            permission_policy_from_env(),
            self.system_prompt.clone(),
        );
        self.model = new_model.to_string();

        // Persist the chosen model to .tachy/config.json so the next session
        // starts with the same model automatically.
        let config_path = std::env::current_dir()
            .ok()
            .map(|d| d.join(".tachy").join("config.json"));
        if let Some(path) = &config_path {
            let mut val: serde_json::Value = if let Ok(raw) = std::fs::read_to_string(path) {
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Object(Default::default()))
            } else {
                serde_json::Value::Object(Default::default())
            };
            val["model"] = serde_json::Value::String(new_model.to_string());
            if let Ok(serialized) = serde_json::to_string_pretty(&val) {
                let _ = std::fs::write(path, serialized);
            }
        }

        self.audit_logger.log(
            &AuditEvent::new(
                &self.session_id,
                AuditEventKind::UserMessage,
                format!("model switched to {new_model}"),
            )
            .with_model(new_model),
        );
        self.audit_event_count += 1;
        Ok(())
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

        // For write_file / edit_file: preview diff → ask approval → conditionally apply.
        let result = if tool_name == "write_file" || tool_name == "edit_file" {
            // 1. Generate diff preview without writing anything to disk.
            let preview = if tool_name == "write_file" {
                let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = value.get("content").and_then(|v| v.as_str()).unwrap_or("");
                preview_write_file(path, content).ok()
            } else {
                let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let old_str = value.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                let new_str = value.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                let replace_all = value.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
                preview_edit_file(path, old_str, new_str, replace_all).ok()
            };

            // 2. If the preview shows real changes, ask the user before applying.
            if let Some(ref preview) = preview {
                if preview.additions > 0 || preview.deletions > 0 {
                    let choice = approval_prompt(
                        &preview.summary,
                        &preview.diff_colored,
                        &preview.diff_text,
                        &mut stdout,
                    );
                    if choice == ApprovalChoice::No {
                        return Err(ToolError::new(format!(
                            "User declined change to {}",
                            preview.file_path
                        )));
                    }
                }
            }

            // 3. Apply: use execute_tool_with_diff so we get the structured output
            //    needed by the undo-stack tracking below.
            match execute_tool_with_diff(tool_name, &value) {
                Ok((output, _)) => Ok(output),
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

// ---------------------------------------------------------------------------
// tachy search
// ---------------------------------------------------------------------------

fn run_search(query: &str, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;

    // Try daemon first
    let daemon_url = "http://127.0.0.1:7777";
    let resp = std::process::Command::new("curl")
        .args([
            "-sf",
            &format!("{daemon_url}/api/search?q={}&limit={limit}",
                urlencoding_simple(query)),
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

    // Daemon not running — use indexer directly
    println!("Searching codebase for: {query}\n");
    let cfg = intelligence::IndexerConfig::default();
    let index = match intelligence::CodebaseIndexer::load_index(&cwd) {
        Ok(i) => i,
        Err(_) => {
            print!("Building index... ");
            io::stdout().flush().ok();
            let idx = intelligence::CodebaseIndexer::build_index(&cwd, &cfg)?;
            let _ = intelligence::CodebaseIndexer::save_index(&cwd, &idx);
            println!("done ({} files)", idx.project.total_files);
            idx
        }
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

fn urlencoding_simple(s: &str) -> String {
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

// ---------------------------------------------------------------------------
// tachy pipeline
// ---------------------------------------------------------------------------

/// A single step in a pipeline.
#[derive(Debug, serde::Deserialize)]
struct PipelineStep {
    name: String,
    template: String,
    prompt: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    model: Option<String>,
}

/// A pipeline definition loaded from YAML.
#[derive(Debug, serde::Deserialize)]
struct PipelineDefinition {
    name: String,
    #[serde(default)]
    description: String,
    steps: Vec<PipelineStep>,
}

fn run_pipeline(subcommand: &str, path: &str, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    if subcommand == "init" {
        let target = if path == "pipeline.yaml" { "tachy-pipeline.yaml" } else { path };
        if std::path::Path::new(target).exists() {
            return Err(format!("{target} already exists — remove it first").into());
        }
        let template = r#"name: my-pipeline
description: "A multi-step agent pipeline"

steps:
  - name: review
    template: code-reviewer
    prompt: "Review the code in the current directory for quality and correctness."

  - name: security
    template: security-scanner
    prompt: "Scan the codebase for security vulnerabilities."
    depends_on: [review]

  - name: docs
    template: doc-generator
    prompt: "Generate or update documentation for all public APIs."
    depends_on: [review]
"#;
        std::fs::write(target, template)?;
        println!("Created {target}");
        println!("Edit the steps then run: tachy pipeline run {target}");
        return Ok(());
    }

    if subcommand != "run" && subcommand != "validate" {
        return Err(format!("unknown pipeline subcommand: {subcommand}\n  usage: tachy pipeline run|validate|init <pipeline.yaml>").into());
    }

    let yaml_str = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read pipeline file '{path}': {e}"))?;

    let pipeline: PipelineDefinition = serde_yaml::from_str(&yaml_str)
        .map_err(|e| format!("invalid pipeline YAML: {e}"))?;

    // Validate step dependencies exist
    let step_names: std::collections::HashSet<&str> = pipeline.steps.iter().map(|s| s.name.as_str()).collect();
    for step in &pipeline.steps {
        for dep in &step.depends_on {
            if !step_names.contains(dep.as_str()) {
                return Err(format!("step '{}' depends_on '{}' which does not exist", step.name, dep).into());
            }
        }
    }

    // Validate no cycles (topological sort check)
    topological_sort(&pipeline.steps)
        .map_err(|cycle| format!("pipeline has a dependency cycle: {cycle}"))?;

    if subcommand == "validate" || dry_run {
        println!("Pipeline '{}' is valid.", pipeline.name);
        println!("  {} steps: {}", pipeline.steps.len(),
            pipeline.steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" → "));
        return Ok(());
    }

    println!("Running pipeline: {}", pipeline.name);
    if !pipeline.description.is_empty() {
        println!("  {}", pipeline.description);
    }
    println!();

    // Execute steps in topological order
    let order = topological_sort(&pipeline.steps).unwrap();
    let default_model = DEFAULT_MODEL.to_string();

    for step_name in &order {
        let step = pipeline.steps.iter().find(|s| &s.name == step_name).unwrap();
        let model = step.model.as_deref().unwrap_or(&default_model);
        println!("  ► Step '{}' — template: {}, model: {}", step.name, step.template, model);
        if dry_run {
            continue;
        }
        match run_agent_cmd(&step.template, &step.prompt, model) {
            Ok(()) => println!("    ✓ Step '{}' completed\n", step.name),
            Err(e) => {
                eprintln!("    ✗ Step '{}' failed: {e}", step.name);
                return Err(format!("pipeline aborted at step '{}'", step.name).into());
            }
        }
    }

    println!("Pipeline '{}' completed all {} steps.", pipeline.name, pipeline.steps.len());
    Ok(())
}

/// Topological sort of pipeline steps. Returns ordered step names or an error
/// describing the cycle.
fn topological_sort(steps: &[PipelineStep]) -> Result<Vec<String>, String> {
    let mut in_degree: std::collections::HashMap<&str, usize> = steps.iter()
        .map(|s| (s.name.as_str(), 0))
        .collect();

    for step in steps {
        for dep in &step.depends_on {
            *in_degree.entry(step.name.as_str()).or_default() += 1;
            let _ = dep; // dep → step: dep must come before step
        }
    }

    // Rebuild: each dep adds to the dependant's in_degree
    let mut count: std::collections::HashMap<&str, usize> = steps.iter()
        .map(|s| (s.name.as_str(), s.depends_on.len()))
        .collect();

    let mut queue: std::collections::VecDeque<&str> = count.iter()
        .filter(|(_, &c)| c == 0)
        .map(|(&n, _)| n)
        .collect();

    let mut result = Vec::new();
    while let Some(name) = queue.pop_front() {
        result.push(name.to_string());
        for step in steps {
            if step.depends_on.iter().any(|d| d == name) {
                let c = count.entry(step.name.as_str()).or_default();
                *c = c.saturating_sub(1);
                if *c == 0 {
                    queue.push_back(step.name.as_str());
                }
            }
        }
    }

    if result.len() == steps.len() {
        Ok(result)
    } else {
        Err("cycle detected".to_string())
    }
}

// ---------------------------------------------------------------------------
// tachy graph  (C1)
// ---------------------------------------------------------------------------

fn run_graph(format: &str, file: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
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

// ---------------------------------------------------------------------------
// tachy monorepo  (C3)
// ---------------------------------------------------------------------------

fn run_monorepo() -> Result<(), Box<dyn std::error::Error>> {
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

// ---------------------------------------------------------------------------
// tachy dashboard  (E2)
// ---------------------------------------------------------------------------

fn run_dashboard() -> Result<(), Box<dyn std::error::Error>> {
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

// ---------------------------------------------------------------------------
// tachy export-audit  (F1)
// ---------------------------------------------------------------------------

fn run_export_audit(format: &str, output: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
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
        "json" | _ => {
            std::fs::write(outfile, serde_json::to_string_pretty(&events)?)?;
            println!("✓ Audit log exported to {}", outfile.display());
        }
    }
    Ok(())
}

fn chrono_now_str_iso() -> String {
    // Simple ISO-like timestamp without chrono dependency
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}Z (unix {})", secs, secs)
}

// ---------------------------------------------------------------------------
// tachy finetune  (F3)
// ---------------------------------------------------------------------------

fn run_finetune(output: Option<&str>, base_model: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
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

    // Write JSONL dataset
    let jsonl_path = std::path::Path::new(out_dir).join("dataset.jsonl");
    dataset.save_jsonl(&jsonl_path)?;
    println!("✓ Dataset: {} training pairs from {} sessions", dataset.total_pairs, dataset.source_sessions);
    println!("  Written to {}", jsonl_path.display());

    // Write training script
    let base = base_model.unwrap_or(DEFAULT_MODEL);
    let script_content = intelligence::generate_training_script(base, "dataset.jsonl", out_dir);
    let script_path = std::path::Path::new(out_dir).join("train.sh");
    std::fs::write(&script_path, &script_content)?;
    println!("  Training script: {}", script_path.display());

    // Write Modelfile template
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

#[cfg(test)]
mod tests {
    use super::{parse_args, CliAction, DEFAULT_MODEL, PipelineStep, topological_sort, urlencoding_simple};
    use std::path::PathBuf;

    #[test]
    fn defaults_to_repl_when_no_args() {
        assert_eq!(
            parse_args(&[]).expect("args should parse"),
            CliAction::Repl {
                model: None,
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
    fn explicit_model_repl_preserves_model() {
        // When --model is given with no subcommand, Repl gets Some(model)
        let args = vec!["--model".to_string(), "qwen2.5-coder:7b".to_string()];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Repl {
                model: Some("qwen2.5-coder:7b".to_string()),
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

    #[test]
    fn parses_search_single_word() {
        let args = vec!["search".to_string(), "main".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Search { query, limit } => {
                assert_eq!(query, "main");
                assert_eq!(limit, 10);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_search_multi_word_query() {
        let args = vec![
            "search".to_string(),
            "tool".to_string(),
            "calling".to_string(),
            "rust".to_string(),
        ];
        match parse_args(&args).expect("should parse") {
            CliAction::Search { query, .. } => assert_eq!(query, "tool calling rust"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn search_empty_query_returns_error() {
        let args = vec!["search".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn parses_pipeline_run() {
        let args = vec![
            "pipeline".to_string(),
            "run".to_string(),
            "my-pipeline.yaml".to_string(),
        ];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { subcommand, path, dry_run } => {
                assert_eq!(subcommand, "run");
                assert_eq!(path, "my-pipeline.yaml");
                assert!(!dry_run);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_pipeline_run_dry_run() {
        let args = vec![
            "pipeline".to_string(),
            "run".to_string(),
            "pipe.yaml".to_string(),
            "--dry-run".to_string(),
        ];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { dry_run, .. } => assert!(dry_run),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_pipeline_init() {
        let args = vec!["pipeline".to_string(), "init".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { subcommand, .. } => assert_eq!(subcommand, "init"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_pipeline_validate() {
        let args = vec![
            "pipeline".to_string(),
            "validate".to_string(),
            "ci.yaml".to_string(),
        ];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { subcommand, path, .. } => {
                assert_eq!(subcommand, "validate");
                assert_eq!(path, "ci.yaml");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    // --- topological_sort ---

    #[test]
    fn topo_sort_no_deps() {
        let steps = vec![
            PipelineStep { name: "a".into(), template: "t".into(), prompt: "p".into(),
                           depends_on: vec![], model: None },
            PipelineStep { name: "b".into(), template: "t".into(), prompt: "p".into(),
                           depends_on: vec![], model: None },
        ];
        let order = topological_sort(&steps).expect("valid");
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn topo_sort_linear_chain() {
        let steps = vec![
            PipelineStep { name: "a".into(), template: "t".into(), prompt: "p".into(),
                           depends_on: vec![], model: None },
            PipelineStep { name: "b".into(), template: "t".into(), prompt: "p".into(),
                           depends_on: vec!["a".into()], model: None },
            PipelineStep { name: "c".into(), template: "t".into(), prompt: "p".into(),
                           depends_on: vec!["b".into()], model: None },
        ];
        let order = topological_sort(&steps).expect("valid");
        assert_eq!(order.len(), 3);
        let ia = order.iter().position(|x| x == "a").unwrap();
        let ib = order.iter().position(|x| x == "b").unwrap();
        let ic = order.iter().position(|x| x == "c").unwrap();
        assert!(ia < ib && ib < ic);
    }

    #[test]
    fn topo_sort_detects_cycle() {
        let steps = vec![
            PipelineStep { name: "a".into(), template: "t".into(), prompt: "p".into(),
                           depends_on: vec!["b".into()], model: None },
            PipelineStep { name: "b".into(), template: "t".into(), prompt: "p".into(),
                           depends_on: vec!["a".into()], model: None },
        ];
        assert!(topological_sort(&steps).is_err());
    }

    // --- urlencoding_simple ---

    #[test]
    fn urlencoding_leaves_alphanumeric() {
        assert_eq!(urlencoding_simple("hello"), "hello");
        assert_eq!(urlencoding_simple("Rust2024"), "Rust2024");
    }

    #[test]
    fn urlencoding_encodes_space_as_plus() {
        assert_eq!(urlencoding_simple("hello world"), "hello+world");
    }

    #[test]
    fn urlencoding_encodes_special_chars() {
        let encoded = urlencoding_simple("a=b&c");
        assert!(encoded.contains("%3D") || encoded.contains("%3d")); // '='
        assert!(encoded.contains("%26")); // '&'
    }
}
