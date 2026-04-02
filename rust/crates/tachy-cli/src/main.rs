mod input;
mod render;

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use audit::{AuditEvent, AuditEventKind, AuditLogger, AuditSeverity, FileAuditSink};
use backend::{BackendRegistry, DynBackend};
use commands::handle_slash_command;
use compat_harness::{extract_manifest, UpstreamPaths};
use daemon::DaemonState;
use platform::{PlatformConfig, PlatformWorkspace};
use render::{Spinner, TerminalRenderer};
use runtime::{
    load_system_prompt, CompactionConfig, ContentBlock,
    ConversationRuntime, PermissionMode, PermissionPolicy,
    Session, ToolError, ToolExecutor,
};
use tools::execute_tool;

const DEFAULT_MODEL: &str = "llama3.1:8b";
const DEFAULT_DATE: &str = "2026-03-31";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    match parse_args(&args)? {
        CliAction::DumpManifests => dump_manifests(),
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
        CliAction::ListModels => list_models(),
        CliAction::ListModelsLocal => list_models_local(),
        CliAction::ListAgents => list_agents(),
        CliAction::Serve { addr } => run_serve(&addr)?,
        CliAction::RunAgent { template, prompt, model } => run_agent_cmd(&template, &prompt, &model)?,
        CliAction::Doctor => run_doctor(),
        CliAction::Pull { model } => run_pull(&model)?,
        CliAction::Help => print_help(),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    DumpManifests,
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
    ListModels,
    ListModelsLocal,
    ListAgents,
    Serve { addr: String },
    RunAgent { template: String, prompt: String, model: String },
    Doctor,
    Pull { model: String },
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

    match rest[0].as_str() {
        "dump-manifests" => Ok(CliAction::DumpManifests),
        "bootstrap-plan" => Ok(CliAction::BootstrapPlan),
        "system-prompt" => parse_system_prompt_args(&rest[1..]),
        "init" => Ok(CliAction::Init),
        "models" => {
            if rest.get(1).map(String::as_str) == Some("--local") {
                Ok(CliAction::ListModelsLocal)
            } else {
                Ok(CliAction::ListModels)
            }
        }
        "agents" => Ok(CliAction::ListAgents),
        "doctor" => Ok(CliAction::Doctor),
        "pull" => {
            let model_name = rest.get(1).ok_or("usage: pull <model>")?;
            Ok(CliAction::Pull { model: model_name.clone() })
        }
        "serve" => {
            let addr = rest.get(1).cloned().unwrap_or_else(|| "127.0.0.1:7777".to_string());
            Ok(CliAction::Serve { addr })
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

fn dump_manifests() {
    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = UpstreamPaths::from_workspace_dir(&workspace_dir);
    match extract_manifest(&paths) {
        Ok(manifest) => {
            println!("commands: {}", manifest.commands.entries().len());
            println!("tools: {}", manifest.tools.entries().len());
            println!("bootstrap phases: {}", manifest.bootstrap.phases().len());
        }
        Err(error) => {
            eprintln!("failed to extract manifests: {error}");
            std::process::exit(1);
        }
    }
}

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
    Ok(())
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
    let report = backend::run_health_check("http://localhost:11434");
    report.print();
}

fn run_pull(model: &str) -> Result<(), Box<dyn std::error::Error>> {
    backend::pull_model(model).map_err(|e| e.into())
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

fn run_serve(addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let state = DaemonState::init(cwd).map_err(|e| e.to_string())?;
    let state = std::sync::Arc::new(std::sync::Mutex::new(state));

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(daemon::serve(addr, state))?;
    Ok(())
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
    let mut cli = LiveCli::new(model, true)?;
    let editor = input::LineEditor::new("› ");
    println!("Tachy — Enterprise AI Agent Platform — interactive mode");
    println!("Model: {}", cli.model);
    println!("Type /help for commands. Shift+Enter or Ctrl+J inserts a newline.\n");

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
                println!("  /model   Show current model");
                println!("  /audit   Show audit event count");
                println!("  /exit    Quit the REPL");
            }
            "/status" => cli.print_status(),
            "/compact" => cli.compact()?,
            "/model" => println!("Current model: {}", cli.model),
            "/audit" => println!("Audit events logged: {}", cli.audit_event_count),
            _ => cli.run_turn(trimmed)?,
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
}

impl LiveCli {
    fn new(model: String, enable_tools: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let system_prompt = build_system_prompt()?;
        let session_id = format!("sess-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis());

        // Set up audit logging
        let mut audit_logger = AuditLogger::new();
        let tachy_dir = env::current_dir()?.join(".tachy");
        if tachy_dir.exists() {
            if let Ok(sink) = FileAuditSink::new(tachy_dir.join("audit.jsonl")) {
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
        let client = registry.create_client(&model, enable_tools)?;
        let backend = DynBackend::new(client);

        let runtime = ConversationRuntime::new(
            Session::new(),
            backend,
            CliToolExecutor::new(),
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
                    "Done",
                    TerminalRenderer::new().color_theme(),
                    &mut stdout,
                )?;

                // Print assistant response text
                for msg in &summary.assistant_messages {
                    for block in &msg.blocks {
                        if let ContentBlock::Text { text } = block {
                            let rendered = TerminalRenderer::new().render_markdown(text);
                            print!("{rendered}");
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
            CliToolExecutor::new(),
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
}

// ---------------------------------------------------------------------------
// Tool executor and helpers
// ---------------------------------------------------------------------------

struct CliToolExecutor {
    renderer: TerminalRenderer,
}

impl CliToolExecutor {
    fn new() -> Self {
        Self {
            renderer: TerminalRenderer::new(),
        }
    }
}

impl ToolExecutor for CliToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        let value = serde_json::from_str(input)
            .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;
        match execute_tool(tool_name, &value) {
            Ok(output) => {
                let markdown = format!("### Tool `{tool_name}`\n\n```json\n{output}\n```\n");
                self.renderer
                    .stream_markdown(&markdown, &mut io::stdout())
                    .map_err(|error| ToolError::new(error.to_string()))?;
                Ok(output)
            }
            Err(error) => Err(ToolError::new(error)),
        }
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

fn print_help() {
    println!("tachy — Enterprise AI Agent Platform\n");
    println!("Usage:");
    println!("  tachy init                                Initialize workspace (.tachy/)");
    println!("  tachy doctor                              Check Ollama, GPU, models");
    println!("  tachy pull <model>                        Pull a model via Ollama");
    println!("  tachy models                              List registered models");
    println!("  tachy models --local                      List locally installed models");
    println!("  tachy agents                              List agent templates");
    println!("  tachy [--model MODEL]                     Start interactive REPL");
    println!("  tachy [--model MODEL] prompt TEXT          Send one prompt");
    println!("  tachy run-agent <template> <prompt...>     Run an agent template");
    println!("  tachy serve [ADDR]                         Start HTTP daemon (default 127.0.0.1:7777)");
    println!("  tachy --resume SESSION.json [/compact]     Resume a session");
    println!("\nHTTP API (when running `tachy serve`):");
    println!("  GET  /health              Health check");
    println!("  GET  /api/models          List models");
    println!("  GET  /api/templates       List agent templates");
    println!("  GET  /api/agents          List agent instances");
    println!("  GET  /api/tasks           List scheduled tasks");
    println!("  POST /api/agents/run      Run an agent  {{\"template\":\"...\",\"prompt\":\"...\"}}");
    println!("  POST /api/tasks/schedule  Schedule agent {{\"template\":\"...\",\"name\":\"...\",\"interval_seconds\":N}}");
    println!("\nEnvironment:");
    println!("  TACHY_PERMISSION_MODE   read-only | workspace-write | deny-all");
    println!("  OLLAMA_HOST             Ollama URL (default http://localhost:11434)");
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
