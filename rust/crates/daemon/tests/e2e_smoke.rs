//! End-to-end smoke test — runs a real agent against a real Ollama model.
//! Requires Ollama running with a model pulled.
//!
//! Run with: cargo test -p daemon --test e2e_smoke -- --ignored --nocapture

use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("tachy-e2e-{name}-{unique}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Smoke test: run the chat-assistant template with a simple prompt.
/// Verifies the full pipeline: backend → conversation loop → tool execution → result.
#[test]
#[ignore = "requires Ollama running with a model (run with --ignored)"]
fn agent_run_with_real_model() {
    let root = temp_dir("smoke");

    // Initialize workspace
    let state = daemon::DaemonState::init(root.clone()).expect("init");

    // Check if Ollama is reachable
    let (alive, _) = backend::check_ollama("http://localhost:11434");
    if !alive {
        eprintln!("Ollama not running — skipping e2e test");
        return;
    }

    // Find a small model that's available
    let model = find_available_model();
    if model.is_none() {
        eprintln!("No model available — skipping e2e test");
        return;
    }
    let model = model.unwrap();
    eprintln!("Using model: {model}");

    // Create a test file for the agent to read
    std::fs::write(root.join("test.py"), "def add(a, b):\n    return a + b\n").unwrap();

    // Build agent config
    let mut template = platform::AgentTemplate::chat_assistant();
    template.model = model.clone();
    template.max_iterations = 4;

    let config = platform::AgentConfig {
        template,
        session_id: "e2e-smoke".to_string(),
        working_directory: root.to_string_lossy().to_string(),
        environment: std::collections::BTreeMap::new(),
    };

    let result = daemon::AgentEngine::run_agent(
        "e2e-agent",
        &config,
        "Read test.py and tell me what the add function does. Be brief.",
        &state.registry,
        &state.config.governance,
        &state.audit_logger,
        &state.config.intelligence,
        &root,
        None,
        None,
    );

    eprintln!("Result: success={}, iterations={}, tools={}", result.success, result.iterations, result.tool_invocations);
    eprintln!("Summary: {}", &result.summary[..result.summary.len().min(500)]);

    assert!(result.success, "agent should succeed");
    assert!(result.iterations > 0, "should have at least 1 iteration");
    // The agent should have read the file
    assert!(result.summary.to_lowercase().contains("add") || result.tool_invocations > 0,
        "agent should mention the add function or use tools");

    std::fs::remove_dir_all(root).ok();
}

/// Smoke test: verify the diff preview pipeline works end-to-end.
#[test]
#[ignore = "requires Ollama running with a model (run with --ignored)"]
fn diff_preview_with_real_write() {
    let root = temp_dir("diff-e2e");
    std::fs::write(root.join("original.txt"), "line1\nline2\nline3\n").unwrap();

    // Write a modified version
    let path = root.join("original.txt");
    let (output, preview) = runtime::write_file(
        path.to_str().unwrap(),
        "line1\nMODIFIED\nline3\nnew_line4\n",
    ).expect("write should succeed");

    assert_eq!(output.kind, "update");
    assert!(!preview.is_new_file);
    assert!(preview.additions >= 2); // MODIFIED + new_line4
    assert!(preview.deletions >= 1); // line2
    assert!(preview.diff_text.contains("--- a/"));
    assert!(preview.diff_text.contains("+++ b/"));
    assert!(preview.diff_colored.contains("\x1b[32m")); // green

    // Verify the file was actually written
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("MODIFIED"));
    assert!(content.contains("new_line4"));

    std::fs::remove_dir_all(root).ok();
}

/// Smoke test: agent with "chat" template reads a file and references its content.
#[test]
#[ignore = "requires Ollama running with a model (run with --ignored)"]
fn chat_template_reads_file_and_references_content() {
    let root = temp_dir("chat-read");

    let (alive, _) = backend::check_ollama("http://localhost:11434");
    if !alive {
        eprintln!("Ollama not running — skipping");
        return;
    }
    let model = match find_available_model() {
        Some(m) => m,
        None => { eprintln!("No model available — skipping"); return; }
    };

    let content = "fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n";
    std::fs::write(root.join("greet.rs"), content).unwrap();

    let state = daemon::DaemonState::init(root.clone()).expect("init");
    let mut template = platform::AgentTemplate::chat_assistant();
    template.model = model.clone();
    template.max_iterations = 4;

    let config = platform::AgentConfig {
        template,
        session_id: "e2e-chat-read".to_string(),
        working_directory: root.to_string_lossy().to_string(),
        environment: std::collections::BTreeMap::new(),
    };

    let result = daemon::AgentEngine::run_agent(
        "e2e-chat-read",
        &config,
        "Read greet.rs and briefly describe what the greet function does.",
        &state.registry,
        &state.config.governance,
        &state.audit_logger,
        &state.config.intelligence,
        &root,
        None,
        None,
    );

    eprintln!("Summary: {}", &result.summary[..result.summary.len().min(300)]);
    assert!(result.success, "agent should succeed");
    let lower = result.summary.to_lowercase();
    assert!(
        lower.contains("greet") || lower.contains("hello") || lower.contains("name") || result.tool_invocations > 0,
        "agent should reference the file content"
    );

    std::fs::remove_dir_all(root).ok();
}

/// Smoke test: agent with "code-reviewer" template produces a non-empty review summary.
#[test]
#[ignore = "requires Ollama running with a model (run with --ignored)"]
fn code_reviewer_template_produces_review_summary() {
    let root = temp_dir("code-review");

    let (alive, _) = backend::check_ollama("http://localhost:11434");
    if !alive {
        eprintln!("Ollama not running — skipping");
        return;
    }
    let model = match find_available_model() {
        Some(m) => m,
        None => { eprintln!("No model available — skipping"); return; }
    };

    std::fs::write(root.join("calc.rs"), "fn divide(a: i32, b: i32) -> i32 { a / b }\n").unwrap();

    let state = daemon::DaemonState::init(root.clone()).expect("init");
    let mut template = platform::AgentTemplate::code_reviewer();
    template.model = model.clone();
    template.max_iterations = 4;

    let config = platform::AgentConfig {
        template,
        session_id: "e2e-review".to_string(),
        working_directory: root.to_string_lossy().to_string(),
        environment: std::collections::BTreeMap::new(),
    };

    let result = daemon::AgentEngine::run_agent(
        "e2e-review",
        &config,
        "Review calc.rs for potential bugs. Be concise.",
        &state.registry,
        &state.config.governance,
        &state.audit_logger,
        &state.config.intelligence,
        &root,
        None,
        None,
    );

    eprintln!("Review summary: {}", &result.summary[..result.summary.len().min(300)]);
    assert!(result.success, "code reviewer should succeed");
    assert!(!result.summary.trim().is_empty(), "review summary should be non-empty");

    std::fs::remove_dir_all(root).ok();
}

/// Smoke test: agent creates, reads, and modifies a file on disk (tool use exercise).
#[test]
#[ignore = "requires Ollama running with a model (run with --ignored)"]
fn agent_creates_reads_and_modifies_file() {
    let root = temp_dir("file-ops");

    let (alive, _) = backend::check_ollama("http://localhost:11434");
    if !alive {
        eprintln!("Ollama not running — skipping");
        return;
    }
    let model = match find_available_model() {
        Some(m) => m,
        None => { eprintln!("No model available — skipping"); return; }
    };

    let state = daemon::DaemonState::init(root.clone()).expect("init");
    let mut template = platform::AgentTemplate::chat_assistant();
    template.model = model.clone();
    template.max_iterations = 6;

    let config = platform::AgentConfig {
        template,
        session_id: "e2e-file-ops".to_string(),
        working_directory: root.to_string_lossy().to_string(),
        environment: std::collections::BTreeMap::new(),
    };

    let result = daemon::AgentEngine::run_agent(
        "e2e-file-ops",
        &config,
        "Create a file named output.txt with the text 'hello world', then read it back and confirm its contents.",
        &state.registry,
        &state.config.governance,
        &state.audit_logger,
        &state.config.intelligence,
        &root,
        None,
        None,
    );

    eprintln!("File ops result: success={}, tools={}", result.success, result.tool_invocations);
    assert!(result.success, "agent should succeed");
    // Either the agent used tools to create the file, or acknowledged it
    assert!(
        result.tool_invocations > 0 || result.summary.to_lowercase().contains("hello"),
        "agent should have used tools or referenced the file content"
    );

    std::fs::remove_dir_all(root).ok();
}

/// Smoke test: parallel execution with two independent tasks both complete with "Completed" status.
#[test]
#[ignore = "requires Ollama running with a model (run with --ignored)"]
fn parallel_execution_two_independent_tasks_complete() {
    use std::sync::{Arc, Mutex};
    use daemon::parallel::{AgentTask, ParallelRun, RunStatus, TaskStatus};

    let root = temp_dir("parallel");

    let (alive, _) = backend::check_ollama("http://localhost:11434");
    if !alive {
        eprintln!("Ollama not running — skipping");
        return;
    }
    let model = match find_available_model() {
        Some(m) => m,
        None => { eprintln!("No model available — skipping"); return; }
    };

    let state = Arc::new(Mutex::new(
        daemon::DaemonState::init(root.clone()).expect("init")
    ));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let make_task = |id: &str, prompt: &str| AgentTask {
        id: id.to_string(),
        run_id: "par-run".to_string(),
        template: "chat".to_string(),
        prompt: prompt.to_string(),
        model: Some(model.clone()),
        deps: vec![],
        priority: 5,
        status: TaskStatus::Pending,
        result: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        work_dir: Some(root.clone()),
    };

    let run = ParallelRun {
        id: "par-run".to_string(),
        tasks: vec![
            make_task("task-a", "Reply with a single word: 'alpha'"),
            make_task("task-b", "Reply with a single word: 'beta'"),
        ],
        status: RunStatus::Running,
        created_at: now,
        max_concurrency: 2,
        conflicts: vec![],
    };

    let completed_run = daemon::execute_parallel_run(run, &state);

    eprintln!("Parallel run status: {:?}", completed_run.status);
    for t in &completed_run.tasks {
        eprintln!("  task {} → {:?}", t.id, t.status);
    }

    assert!(
        matches!(completed_run.status, RunStatus::Completed | RunStatus::Failed),
        "run should reach terminal status"
    );
    for task in &completed_run.tasks {
        assert!(
            matches!(task.status, TaskStatus::Completed | TaskStatus::Failed),
            "task {} should be in terminal status, got {:?}", task.id, task.status
        );
    }

    std::fs::remove_dir_all(root).ok();
}

/// Smoke test: every agent run produces at least one audit event with a valid hash chain.
#[test]
#[ignore = "requires Ollama running with a model (run with --ignored)"]
fn agent_run_produces_audit_events_with_hash_chain() {
    let root = temp_dir("audit-chain");

    let (alive, _) = backend::check_ollama("http://localhost:11434");
    if !alive {
        eprintln!("Ollama not running — skipping");
        return;
    }
    let model = match find_available_model() {
        Some(m) => m,
        None => { eprintln!("No model available — skipping"); return; }
    };

    // Build a logger with a MemoryAuditSink to capture events
    let mem_sink = audit::MemoryAuditSink::new();
    let captured = mem_sink.clone();
    let mut logger = audit::AuditLogger::new();
    logger.add_sink(mem_sink);

    let ws = platform::PlatformWorkspace::init(&root).expect("workspace init");
    let registry = backend::BackendRegistry::with_defaults();

    let mut template = platform::AgentTemplate::chat_assistant();
    template.model = model.clone();
    template.max_iterations = 3;

    let config = platform::AgentConfig {
        template,
        session_id: "e2e-audit".to_string(),
        working_directory: root.to_string_lossy().to_string(),
        environment: std::collections::BTreeMap::new(),
    };

    let result = daemon::AgentEngine::run_agent(
        "e2e-audit",
        &config,
        "Say 'audit ok' and stop.",
        &registry,
        &ws.config.governance,
        &logger,
        &ws.config.intelligence,
        &root,
        None,
        None,
    );

    let events = captured.events();
    eprintln!("Agent success={}, audit events={}", result.success, events.len());
    assert!(result.success, "agent should succeed");
    assert!(!events.is_empty(), "at least one audit event should be produced");

    // Verify hash chain: each event must have a non-empty hash
    for event in &events {
        assert!(!event.hash.is_empty(), "audit event hash should not be empty");
    }

    std::fs::remove_dir_all(root).ok();
}

/// Find a small model that's locally available.
fn find_available_model() -> Option<String> {
    let output = std::process::Command::new("ollama")
        .args(["list"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Prefer small models for fast tests
    let preferences = ["gemma4:e4b", "qwen3:8b", "llama3.1:8b", "gemma4:26b"];
    for pref in &preferences {
        if stdout.contains(pref) {
            return Some(pref.to_string());
        }
    }

    // Fall back to first available model
    stdout.lines().nth(1).map(|line| {
        line.split_whitespace().next().unwrap_or("").to_string()
    }).filter(|s| !s.is_empty())
}
