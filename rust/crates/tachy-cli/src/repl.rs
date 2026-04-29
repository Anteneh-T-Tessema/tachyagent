//! Interactive REPL: `LiveCli` session holder, `CliToolExecutor`, streaming token display,
//! `run_repl` loop, session save/restore, tool-executor helpers.

use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::render::{approval_prompt, ApprovalChoice, Spinner, TerminalRenderer};
use audit::{AuditEvent, AuditEventKind, AuditLogger, AuditSeverity, FileAuditSink};
use backend::{BackendRegistry, DynBackend};
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use platform::PlatformConfig;
use runtime::{
    load_system_prompt, preview_edit_file, preview_write_file, CompactionConfig, ContentBlock,
    ConversationRuntime, PermissionMode, PermissionPolicy, RuntimeEvent, Session, ToolError,
    ToolExecutor,
};
use tools::{execute_tool, execute_tool_with_diff};

use crate::setup::detect_project;
use crate::DEFAULT_DATE;
use crate::DEFAULT_MODEL;

// ---------------------------------------------------------------------------
// LiveCli — interactive agent session holder
// ---------------------------------------------------------------------------

pub(crate) struct LiveCli {
    pub(crate) model: String,
    pub(crate) session_id: String,
    pub(crate) system_prompt: Vec<String>,
    pub(crate) runtime: ConversationRuntime<DynBackend, CliToolExecutor>,
    pub(crate) audit_logger: AuditLogger,
    pub(crate) audit_event_count: u64,
    pub(crate) governance: audit::GovernancePolicy,
    pub(crate) tool_invocation_counts: std::collections::BTreeMap<String, u32>,
    pub(crate) total_tool_invocations: u32,
    pub(crate) undo_stack: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    /// Last-turn latency in milliseconds.
    pub(crate) last_latency_ms: u64,
}

impl LiveCli {
    pub(crate) fn new(
        model: String,
        enable_tools: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let system_prompt = build_system_prompt()?;
        let session_id = format!(
            "sess-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        let tachy_dir = env::current_dir()?.join(".tachy");
        let audit_path = tachy_dir.join("audit.jsonl");
        let mut audit_logger = AuditLogger::resume_from_file(&audit_path);
        if tachy_dir.exists() {
            if let Ok(sink) = FileAuditSink::new(&audit_path) {
                audit_logger.add_sink(sink);
            }
        }

        let config = PlatformConfig::load(env::current_dir()?.join(".tachy").join("config.json"));
        let governance = config.governance.clone();

        let registry = BackendRegistry::with_defaults();
        let client = registry
            .create_client(&model, enable_tools)
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

        audit_logger.log(
            &AuditEvent::new(
                &session_id,
                AuditEventKind::SessionStart,
                "interactive session started",
            )
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

    pub(crate) fn run_turn(&mut self, input: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.audit_logger.log(
            &AuditEvent::new(&self.session_id, AuditEventKind::UserMessage, "user input")
                .with_redacted_payload(truncate(input, 200)),
        );
        self.audit_event_count += 1;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<RuntimeEvent>();
        self.runtime.set_event_tx(tx);

        let first_token_seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first_token_clone = first_token_seen.clone();

        let stream_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tachy stream rt");
            rt.block_on(async move {
                let mut rx = rx;
                let mut stdout = io::stdout();
                let renderer = TerminalRenderer::new();
                let mut in_code_block = false;
                let mut code_lang = String::new();
                let mut code_buf = String::new();
                let mut text_buf = String::new();
                while let Some(event) = rx.recv().await {
                    match event {
                        RuntimeEvent::TextDelta(delta) => {
                            if !first_token_clone.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                let _ = execute!(
                                    stdout,
                                    crossterm::cursor::MoveToColumn(0),
                                    crossterm::terminal::Clear(
                                        crossterm::terminal::ClearType::CurrentLine
                                    ),
                                );
                            }
                            let mut i = 0;
                            let chars: Vec<char> = delta.chars().collect();
                            while i < chars.len() {
                                if !in_code_block && (chars[i] == '`' || chars[i] == '~') {
                                    let marker = chars[i];
                                    if i + 2 < chars.len()
                                        && chars[i + 1] == marker
                                        && chars[i + 2] == marker
                                    {
                                        if !text_buf.is_empty() {
                                            print!("{text_buf}");
                                            let _ = stdout.flush();
                                            text_buf.clear();
                                        }
                                        in_code_block = true;
                                        i += 3;
                                        let mut lang = String::new();
                                        while i < chars.len()
                                            && !chars[i].is_whitespace()
                                            && chars[i] != '\n'
                                        {
                                            lang.push(chars[i]);
                                            i += 1;
                                        }
                                        code_lang = lang.trim().to_string();
                                        if code_lang.is_empty() {
                                            print!("\n\x1b[36m╭─ code\x1b[0m\n");
                                        } else {
                                            print!("\n\x1b[36m╭─ {code_lang}\x1b[0m\n");
                                        }
                                        let _ = stdout.flush();
                                        continue;
                                    }
                                }
                                if in_code_block && (chars[i] == '`' || chars[i] == '~') {
                                    let marker = chars[i];
                                    if i + 2 < chars.len()
                                        && chars[i + 1] == marker
                                        && chars[i + 2] == marker
                                    {
                                        print!(
                                            "{}",
                                            renderer.highlight_code(&code_buf, &code_lang)
                                        );
                                        print!("\x1b[36m╰─\x1b[0m\n\n");
                                        let _ = stdout.flush();
                                        code_buf.clear();
                                        code_lang.clear();
                                        in_code_block = false;
                                        i += 3;
                                        continue;
                                    }
                                }
                                if in_code_block {
                                    code_buf.push(chars[i]);
                                } else {
                                    text_buf.push(chars[i]);
                                }
                                i += 1;
                            }
                            if !in_code_block && !text_buf.is_empty() {
                                print!("{text_buf}");
                                let _ = stdout.flush();
                                text_buf.clear();
                            }
                        }
                        RuntimeEvent::Finished(_) => {
                            if in_code_block && !code_buf.is_empty() {
                                print!("{}", renderer.highlight_code(&code_buf, &code_lang));
                                print!("\x1b[36m╰─\x1b[0m\n\n");
                                let _ = stdout.flush();
                            }
                            if !text_buf.is_empty() {
                                print!("{text_buf}");
                                let _ = stdout.flush();
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            });
        });

        let mut spinner = Spinner::new();
        let mut stdout = io::stdout();
        let renderer = TerminalRenderer::new();
        let theme = *renderer.color_theme();

        let turn_start = std::time::Instant::now();
        spinner.tick("Thinking", &theme, &mut stdout)?;

        let result = self.runtime.run_turn(input, None);
        let _ = stream_thread.join();

        let elapsed_ms = turn_start.elapsed().as_millis() as u64;
        self.last_latency_ms = elapsed_ms;

        match result {
            Ok(summary) => {
                if first_token_seen.load(std::sync::atomic::Ordering::SeqCst) {
                    println!();
                } else {
                    spinner.finish(
                        &format!(
                            "Done ({} iteration{}, {} tool call{}, {}ms)",
                            summary.iterations,
                            if summary.iterations == 1 { "" } else { "s" },
                            summary.tool_results.len(),
                            if summary.tool_results.len() == 1 {
                                ""
                            } else {
                                "s"
                            },
                            elapsed_ms,
                        ),
                        &theme,
                        &mut stdout,
                    )?;
                }

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
                        if let ContentBlock::ToolResult {
                            tool_name,
                            output,
                            is_error,
                            ..
                        } = block
                        {
                            if *is_error {
                                execute!(
                                    stdout,
                                    SetForegroundColor(Color::Red),
                                    Print(format!(
                                        "  ✘ {tool_name}: {}\n",
                                        output.lines().next().unwrap_or("")
                                    )),
                                    ResetColor
                                )?;
                            }
                        }
                    }
                }

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

                self.audit_logger.log(
                    &AuditEvent::new(
                        &self.session_id,
                        AuditEventKind::AssistantMessage,
                        format!(
                            "iterations={} tools={} latency_ms={}",
                            summary.iterations,
                            summary.tool_results.len(),
                            elapsed_ms
                        ),
                    )
                    .with_model(&self.model),
                );
                self.audit_event_count += 1;

                for tool_msg in &summary.tool_results {
                    for block in &tool_msg.blocks {
                        if let ContentBlock::ToolResult {
                            tool_name,
                            is_error,
                            ..
                        } = block
                        {
                            let count = self
                                .tool_invocation_counts
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
                                self.audit_logger
                                    .log(&violation.to_audit_event(&self.session_id));
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
                spinner.fail("Request failed", &theme, &mut stdout)?;
                self.audit_logger.log(
                    &AuditEvent::new(
                        &self.session_id,
                        AuditEventKind::SessionEnd,
                        error.to_string(),
                    )
                    .with_severity(AuditSeverity::Warning),
                );
                self.audit_event_count += 1;
                Err(Box::new(error))
            }
        }
    }

    pub(crate) fn print_status(&self) {
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

    pub(crate) fn switch_model(
        &mut self,
        new_model: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
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

        let config_path = std::env::current_dir()
            .ok()
            .map(|d| d.join(".tachy").join("config.json"));
        if let Some(path) = &config_path {
            let mut val: serde_json::Value = if let Ok(raw) = std::fs::read_to_string(path) {
                serde_json::from_str(&raw)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::default()))
            } else {
                serde_json::Value::Object(serde_json::Map::default())
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

    pub(crate) fn compact(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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

        self.audit_logger.log(&AuditEvent::new(
            &self.session_id,
            AuditEventKind::SessionCompacted,
            format!("removed {removed} messages"),
        ));
        self.audit_event_count += 1;
        println!("Compacted {removed} messages.");
        Ok(())
    }

    pub(crate) fn save_session(&self) -> Result<String, Box<dyn std::error::Error>> {
        let sessions_dir = env::current_dir()?.join(".tachy").join("sessions");
        std::fs::create_dir_all(&sessions_dir)?;
        let path = sessions_dir.join(format!("{}.json", self.session_id));
        let json = serde_json::to_string_pretty(self.runtime.session())?;
        std::fs::write(&path, json)?;
        Ok(path.to_string_lossy().into_owned())
    }
}

/// Find the most recent saved session file for auto-resume.
pub(crate) fn last_session_path() -> Option<PathBuf> {
    let sessions_dir = env::current_dir().ok()?.join(".tachy").join("sessions");
    let mut entries: Vec<_> = std::fs::read_dir(&sessions_dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));
    entries.first().map(std::fs::DirEntry::path)
}

// ---------------------------------------------------------------------------
// REPL main loop
// ---------------------------------------------------------------------------

pub(crate) fn run_repl(model: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let model = model.unwrap_or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|d| d.join(".tachy").join("config.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["model"].as_str().map(str::to_string))
            .unwrap_or_else(|| DEFAULT_MODEL.to_string())
    });
    let registry = BackendRegistry::with_defaults();
    let mut cli = LiveCli::new(model, true)?;
    let editor = crate::input::LineEditor::new("› ");

    // ── TACHY.md staleness check ──────────────────────────────────────────
    {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let tachy_md_path = cwd.join("TACHY.md");
        if tachy_md_path.exists() {
            let md_mtime = std::fs::metadata(&tachy_md_path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());

            let git_mtime = std::process::Command::new("git")
                .args(["log", "-1", "--format=%ct"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u64>().ok());

            if let (Some(md_t), Some(git_t)) = (md_mtime, git_mtime) {
                if git_t > md_t {
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
                            } else {
                                String::new()
                            },
                            if let Some(bc) = &project_info.build_command {
                                format!("Build command: `{bc}`\n")
                            } else {
                                String::new()
                            },
                        );
                        let _ = std::fs::write(&tachy_md_path, &content);
                        eprintln!("  ✓ TACHY.md refreshed (git HEAD newer than last sync)");
                    }
                }
            }
        }
    }

    // ── Auto-resume prompt ────────────────────────────────────────────────
    let mut stdout = io::stdout();
    if let Some(last_path) = last_session_path() {
        if let Ok(meta) = last_path.metadata() {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    let secs = elapsed.as_secs();
                    let age = if secs < 120 {
                        format!("{secs}s ago")
                    } else if secs < 7200 {
                        format!("{}m ago", secs / 60)
                    } else if secs < 172_800 {
                        format!("{}h ago", secs / 3600)
                    } else {
                        format!("{}d ago", secs / 86400)
                    };
                    execute!(
                        stdout,
                        SetForegroundColor(Color::DarkYellow),
                        Print(format!("Resume session from {age}? [y/N] ")),
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

    // ── Main REPL loop ────────────────────────────────────────────────────
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
                println!(
                    "  /fix <desc>       Fix an issue (describe it or leave blank for last error)"
                );
                println!("  /test             Run the project test suite and report failures");
                println!("  /review           Review staged git changes");
                println!("  /commit           Generate a conventional commit message");
                println!("  /explain [file]   Explain a file or the current directory");
                println!("  /exit             Quit the REPL");
            }
            "/status" => cli.print_status(),
            "/compact" => cli.compact()?,
            "/save" => match cli.save_session() {
                Ok(path) => println!("Session saved to {path}"),
                Err(e) => eprintln!("Failed to save: {e}"),
            },
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
                            .filter_map(std::result::Result::ok)
                            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                            .collect();
                        sessions.sort_by_key(|e| {
                            std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok()))
                        });
                        if sessions.is_empty() {
                            println!("No saved sessions.");
                        } else {
                            println!("Saved sessions (most recent first):");
                            for (i, entry) in sessions.iter().take(10).enumerate() {
                                let name = entry.file_name();
                                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                                println!(
                                    "  {}. {} ({:.1} KB)",
                                    i + 1,
                                    name.to_string_lossy(),
                                    size as f64 / 1024.0
                                );
                            }
                            println!("\nResume with: tachy --resume <session-id>");
                        }
                    }
                    Err(_) => println!("No sessions directory found."),
                }
            }
            s if s.starts_with("/fix") => {
                let desc = s[4..].trim();
                let prompt = if desc.is_empty() {
                    "Find and fix the most obvious bug or error in this codebase. \
                     Run the tests after fixing to verify the fix works."
                        .to_string()
                } else {
                    format!(
                        "Fix this issue: {desc}\n\n\
                             Steps: 1) Locate the relevant code. 2) Apply the fix. \
                             3) Run the tests to verify."
                    )
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
                     4) Concrete suggestions for improvement.",
                )?;
            }
            "/commit" => {
                cli.run_turn(
                    "Generate a conventional commit message for the staged changes \
                     (run: git diff --staged to see them). \
                     Format: type(scope): short description\n\
                     Types: feat/fix/docs/style/refactor/test/chore\n\
                     Keep the subject line under 72 characters. \
                     Add a body if needed to explain motivation or breaking changes.",
                )?;
            }
            s if s.starts_with("/explain") => {
                let target = s[8..].trim();
                let prompt = if target.is_empty() {
                    "Explain what this project does: list the main source files, \
                     describe each module's purpose in one sentence, and explain \
                     how they connect to each other."
                        .to_string()
                } else {
                    format!(
                        "Explain `{target}` in plain English — its purpose, \
                             key functions/types, and how it fits into the broader codebase."
                    )
                };
                cli.run_turn(&prompt)?;
            }
            _ => cli.run_turn(trimmed)?,
        }
    }

    if !cli.runtime.session().messages.is_empty() {
        if let Ok(path) = cli.save_session() {
            eprintln!("Session saved to {path}");
        }
    }

    cli.audit_logger.flush();
    Ok(())
}

// ---------------------------------------------------------------------------
// CliToolExecutor
// ---------------------------------------------------------------------------

pub(crate) struct CliToolExecutor {
    pub(crate) undo_stack: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

impl CliToolExecutor {
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self {
            undo_stack: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn with_undo_stack(
        undo_stack: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    ) -> Self {
        Self { undo_stack }
    }
}

impl ToolExecutor for CliToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        let value = serde_json::from_str(input)
            .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;

        let input_summary = summarize_tool_input(tool_name, &value);
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!("  ⚡ {tool_name} {input_summary}\n")),
            ResetColor
        );

        let result = if tool_name == "write_file" || tool_name == "edit_file" {
            let preview = if tool_name == "write_file" {
                let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = value.get("content").and_then(|v| v.as_str()).unwrap_or("");
                preview_write_file(path, content).ok()
            } else {
                let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let old_str = value
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new_str = value
                    .get("new_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let replace_all = value
                    .get("replace_all")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                preview_edit_file(path, old_str, new_str, replace_all).ok()
            };

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

            match execute_tool_with_diff(tool_name, &value) {
                Ok((output, _)) => Ok(output),
                Err(e) => Err(e),
            }
        } else {
            execute_tool(tool_name, &value)
        };

        match result {
            Ok(output) => {
                if tool_name == "edit_file" {
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

                if tool_name == "read_file" {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output) {
                        if let Some(content) = parsed.get("content").and_then(|v| v.as_str()) {
                            let lines = content.lines().count();
                            let path = parsed
                                .get("filePath")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn build_system_prompt() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    Ok(load_system_prompt(
        env::current_dir()?,
        DEFAULT_DATE,
        env::consts::OS,
        "unknown",
    )?)
}

pub(crate) fn permission_policy_from_env() -> PermissionPolicy {
    let mode = env::var("TACHY_PERMISSION_MODE").unwrap_or_else(|_| "workspace-write".to_string());
    match mode.as_str() {
        "read-only" => PermissionPolicy::new(PermissionMode::Deny)
            .with_tool_mode("read_file", PermissionMode::Allow)
            .with_tool_mode("glob_search", PermissionMode::Allow)
            .with_tool_mode("grep_search", PermissionMode::Allow),
        "deny-all" => PermissionPolicy::new(PermissionMode::Deny),
        _ => PermissionPolicy::new(PermissionMode::Allow),
    }
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    match tool_name {
        "bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| format!("`{}`", truncate(c, 80)))
            .unwrap_or_default(),
        "read_file" | "write_file" | "edit_file" => input
            .get("path")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
            .unwrap_or_default(),
        "grep_search" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| format!("/{p}/"))
            .unwrap_or_default(),
        "glob_search" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
            .unwrap_or_default(),
        "list_directory" => input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string(),
        _ => String::new(),
    }
}

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
        "bash" => obj
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| truncate(s, 80))
            .unwrap_or_default(),
        "read_file" | "write_file" | "list_directory" => obj
            .get("path")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| ".".to_string()),
        "edit_file" => {
            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let old = obj.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
            let preview = old.lines().next().unwrap_or("");
            format!("{path}: {}", truncate(preview, 50))
        }
        "grep_search" => obj
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|s| format!("/{s}/"))
            .unwrap_or_default(),
        "glob_search" => obj
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
            .unwrap_or_default(),
        _ => obj
            .values()
            .find_map(|v| v.as_str())
            .map(|s| truncate(s, 60))
            .unwrap_or_default(),
    }
}
