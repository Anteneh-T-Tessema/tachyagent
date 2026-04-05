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
