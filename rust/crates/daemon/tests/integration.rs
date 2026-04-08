//! Integration tests for build items 1-5.
//! These test the wiring between crates — diff preview, policy engine,
//! file locks, parallel orchestration, and LSP diagnostics.

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// 1. Diff preview flows through write_file and edit_file
// ---------------------------------------------------------------------------

#[test]
fn write_file_produces_diff_preview() {
    let dir = temp_dir("int-diff-write");
    let path = dir.join("hello.txt");

    // New file — diff should show all additions
    let (output, preview) = runtime::write_file(
        path.to_str().unwrap(), "line1\nline2\nline3\n",
    ).expect("write should succeed");
    assert_eq!(output.kind, "create");
    assert!(preview.is_new_file);
    assert_eq!(preview.additions, 3);
    assert_eq!(preview.deletions, 0);
    assert!(preview.diff_text.contains("+line1"));

    // Update — diff should show changes
    let (output2, preview2) = runtime::write_file(
        path.to_str().unwrap(), "line1\nMODIFIED\nline3\n",
    ).expect("update should succeed");
    assert_eq!(output2.kind, "update");
    assert!(!preview2.is_new_file);
    assert!(preview2.additions > 0);
    assert!(preview2.deletions > 0);
    assert!(preview2.diff_colored.contains("\x1b[32m")); // green for additions

    cleanup(&dir);
}

#[test]
fn edit_file_produces_diff_preview() {
    let dir = temp_dir("int-diff-edit");
    let path = dir.join("code.rs");
    std::fs::write(&path, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();

    let (output, preview) = runtime::edit_file(
        path.to_str().unwrap(), "hello", "world", false,
    ).expect("edit should succeed");

    assert_eq!(output.new_string, "world");
    assert!(preview.additions > 0);
    assert!(preview.deletions > 0);
    assert!(preview.diff_text.contains("-"));
    assert!(preview.diff_text.contains("+"));
    assert!(!preview.summary.is_empty());

    cleanup(&dir);
}

// ---------------------------------------------------------------------------
// 2. Preview-only mode (no disk write)
// ---------------------------------------------------------------------------

#[test]
fn preview_write_does_not_touch_disk() {
    let dir = temp_dir("int-preview-write");
    let path = dir.join("ghost.txt");

    let preview = runtime::preview_write_file(
        path.to_str().unwrap(), "should not exist on disk",
    ).expect("preview should succeed");

    assert!(preview.is_new_file);
    assert!(preview.additions > 0);
    // File should NOT exist
    assert!(!path.exists());

    cleanup(&dir);
}

#[test]
fn preview_edit_does_not_touch_disk() {
    let dir = temp_dir("int-preview-edit");
    let path = dir.join("original.txt");
    std::fs::write(&path, "alpha beta gamma").unwrap();

    let preview = runtime::preview_edit_file(
        path.to_str().unwrap(), "beta", "BETA", false,
    ).expect("preview should succeed");

    assert!(preview.additions > 0);
    // File should still have original content
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("beta"));
    assert!(!content.contains("BETA"));

    cleanup(&dir);
}

// ---------------------------------------------------------------------------
// 3. Policy engine evaluates patches correctly
// ---------------------------------------------------------------------------

#[test]
fn policy_engine_auto_approves_safe_patches() {
    let engine = audit::PolicyEngine::enterprise_default();
    let patch = audit::FilePatch {
        file_path: "src/utils.rs".to_string(),
        original_hash: String::new(),
        new_content: "fn helper() { 42 }".to_string(),
        diff_summary: "+fn helper".to_string(),
        additions: 3,
        deletions: 1,
        agent_id: "agent-1".to_string(),
        task_id: None,
    };
    assert_eq!(engine.evaluate(&patch), audit::PolicyDecision::AutoApprove);
}

#[test]
fn policy_engine_blocks_secrets_in_content() {
    let engine = audit::PolicyEngine::enterprise_default();
    let patch = audit::FilePatch {
        file_path: "src/config.rs".to_string(),
        original_hash: String::new(),
        new_content: "let password = \"hunter2\";".to_string(),
        diff_summary: "+password".to_string(),
        additions: 1,
        deletions: 0,
        agent_id: "agent-1".to_string(),
        task_id: None,
    };
    match engine.evaluate(&patch) {
        audit::PolicyDecision::Reject { reason } => {
            assert!(reason.contains("password"));
        }
        other => panic!("expected Reject, got {:?}", other),
    }
}

#[test]
fn policy_engine_requires_approval_for_auth_paths() {
    let engine = audit::PolicyEngine::enterprise_default();
    let patch = audit::FilePatch {
        file_path: "src/auth/login.rs".to_string(),
        original_hash: String::new(),
        new_content: "fn login() {}".to_string(),
        diff_summary: "+fn login".to_string(),
        additions: 5,
        deletions: 2,
        agent_id: "agent-1".to_string(),
        task_id: None,
    };
    match engine.evaluate(&patch) {
        audit::PolicyDecision::RequiresApproval { reason } => {
            assert!(reason.contains("auth"));
        }
        other => panic!("expected RequiresApproval, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 4. File lock manager prevents concurrent access
// ---------------------------------------------------------------------------

#[test]
fn file_locks_prevent_concurrent_writes() {
    let mgr = runtime::FileLockManager::new();

    // Agent 1 acquires lock
    mgr.try_acquire("src/main.rs", "agent-1").expect("should acquire");

    // Agent 2 cannot acquire the same file
    let err = mgr.try_acquire("src/main.rs", "agent-2").expect_err("should be locked");
    assert!(err.to_string().contains("agent-1"));

    // Agent 2 can lock a different file
    mgr.try_acquire("src/lib.rs", "agent-2").expect("different file should work");

    // Agent 1 releases, agent 2 can now acquire
    mgr.release("src/main.rs", "agent-1");
    mgr.try_acquire("src/main.rs", "agent-2").expect("should acquire after release");

    // release_all cleans up
    mgr.release_all("agent-2");
    assert!(mgr.list_locks().is_empty());
}

#[test]
fn file_locks_are_reentrant_for_same_agent() {
    let mgr = runtime::FileLockManager::new();
    mgr.try_acquire("f.rs", "a1").unwrap();
    mgr.try_acquire("f.rs", "a1").unwrap(); // idempotent
    assert_eq!(mgr.list_locks().len(), 1);
}

// ---------------------------------------------------------------------------
// 5. Parallel orchestrator DAG scheduling
// ---------------------------------------------------------------------------

#[test]
fn orchestrator_respects_task_dependencies() {
    use daemon::parallel::*;

    let mut orch = Orchestrator::new(4);
    let run = ParallelRun {
        id: "run-dep".into(),
        tasks: vec![
            AgentTask {
                id: "t1".into(), run_id: "run-dep".into(), template: "chat".into(),
                prompt: "first".into(), model: None, deps: vec![], priority: 5,
                status: TaskStatus::Pending, result: None, created_at: 0,
                started_at: None, completed_at: None, work_dir: None,
            },
            AgentTask {
                id: "t2".into(), run_id: "run-dep".into(), template: "chat".into(),
                prompt: "second".into(), model: None, deps: vec!["t1".into()], priority: 5,
                status: TaskStatus::Pending, result: None, created_at: 0,
                started_at: None, completed_at: None, work_dir: None,
            },
        ],
        status: RunStatus::Running,
        created_at: 0,
        max_concurrency: 4,
        conflicts: vec![],
    };
    orch.submit(run);

    // Only t1 should be available (t2 depends on t1)
    let task = orch.next_task().unwrap();
    assert_eq!(task.id, "t1");
    assert!(orch.next_task().is_none()); // t2 blocked

    // Complete t1 — now t2 should be available
    orch.complete_task("t1", TaskResult {
        success: true, summary: "done".into(), iterations: 1,
        tool_invocations: 0, audit_hash: "h".into(), tokens_in: 0, tokens_out: 0, cost_usd: 0.0,
    });
    let task2 = orch.next_task().unwrap();
    assert_eq!(task2.id, "t2");

    // Complete t2 — run should be completed
    orch.complete_task("t2", TaskResult {
        success: true, summary: "done".into(), iterations: 1,
        tool_invocations: 0, audit_hash: "h".into(), tokens_in: 0, tokens_out: 0, cost_usd: 0.0,
    });
    let run = orch.get_run("run-dep").unwrap();
    assert_eq!(run.status, RunStatus::Completed);
}

#[test]
fn orchestrator_partial_failure() {
    use daemon::parallel::*;

    let mut orch = Orchestrator::new(4);
    let run = ParallelRun {
        id: "run-fail".into(),
        tasks: vec![
            AgentTask {
                id: "ok".into(), run_id: "run-fail".into(), template: "chat".into(),
                prompt: "a".into(), model: None, deps: vec![], priority: 5,
                status: TaskStatus::Pending, result: None, created_at: 0,
                started_at: None, completed_at: None, work_dir: None,
            },
            AgentTask {
                id: "bad".into(), run_id: "run-fail".into(), template: "chat".into(),
                prompt: "b".into(), model: None, deps: vec![], priority: 5,
                status: TaskStatus::Pending, result: None, created_at: 0,
                started_at: None, completed_at: None, work_dir: None,
            },
        ],
        status: RunStatus::Running,
        created_at: 0,
        max_concurrency: 4,
        conflicts: vec![],
    };
    orch.submit(run);

    let t1 = orch.next_task().unwrap();
    let t2 = orch.next_task().unwrap();
    orch.complete_task(&t1.id, TaskResult {
        success: true, summary: "ok".into(), iterations: 1,
        tool_invocations: 0, audit_hash: "h".into(), tokens_in: 0, tokens_out: 0, cost_usd: 0.0,
    });
    orch.complete_task(&t2.id, TaskResult {
        success: false, summary: "failed".into(), iterations: 1,
        tool_invocations: 0, audit_hash: "h".into(), tokens_in: 0, tokens_out: 0, cost_usd: 0.0,
    });

    let run = orch.get_run("run-fail").unwrap();
    assert_eq!(run.status, RunStatus::PartiallyCompleted);
}

// ---------------------------------------------------------------------------
// 6. SSO session lifecycle
// ---------------------------------------------------------------------------

#[test]
fn sso_full_lifecycle() {
    use audit::sso::*;
    use audit::{UserStore, Role};
    use std::collections::BTreeMap;

    let config = SsoConfig {
        enabled: true,
        idp_entity_id: "https://idp.corp.com".to_string(),
        idp_sso_url: "https://idp.corp.com/sso".to_string(),
        idp_certificate: String::new(),
        sp_entity_id: "tachy".to_string(),
        sp_acs_url: "http://localhost:7777/api/auth/sso/callback".to_string(),
        role_mapping: {
            let mut m = BTreeMap::new();
            m.insert("engineers".to_string(), Role::Developer);
            m
        },
        ..SsoConfig::default()
    };

    let mut mgr = SsoManager::new(config);
    let mut users = UserStore::new();

    // Build login URL
    let url = mgr.build_login_url(None);
    assert!(url.contains("SAMLRequest="));

    // Simulate callback
    let xml = r#"<samlp:Response><saml:Issuer>https://idp.corp.com</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>alice@corp.com</saml:NameID></saml:Subject><saml:AuthnStatement SessionIndex="s42"></saml:AuthnStatement><saml:AttributeStatement><saml:Attribute Name="displayName"><saml:AttributeValue>Alice</saml:AttributeValue></saml:Attribute><saml:Attribute Name="groups"><saml:AttributeValue>engineers</saml:AttributeValue></saml:Attribute></saml:AttributeStatement></saml:Assertion></samlp:Response>"#;
    let b64 = base64_encode_test(xml.as_bytes());

    let session = mgr.process_callback(&b64, &mut users).unwrap();
    assert_eq!(session.email, "alice@corp.com");
    assert_eq!(session.role, Role::Developer);
    assert!(session.token.starts_with("sso-"));

    // Validate
    assert!(mgr.validate_session(&session.token).is_some());
    assert_eq!(mgr.active_sessions().len(), 1);

    // Logout
    mgr.invalidate_session(&session.token);
    assert!(mgr.validate_session(&session.token).is_none());
    assert_eq!(mgr.active_sessions().len(), 0);

    // User was provisioned
    assert_eq!(users.list_users().len(), 1);
    assert_eq!(users.list_users()[0].name, "Alice");
}

// ---------------------------------------------------------------------------
// 7. Web tools — unit-level integration
// ---------------------------------------------------------------------------

#[test]
fn web_fetch_rejects_non_http() {
    let input = tools::WebFetchInput {
        url: "ftp://evil.com".to_string(),
        max_length: None,
    };
    assert!(tools::web_fetch(&input).is_err());
}

#[test]
fn web_search_and_fetch_tool_specs_registered() {
    let specs = tools::mvp_tool_specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name).collect();
    assert!(names.contains(&"web_search"));
    assert!(names.contains(&"web_fetch"));
}

// ---------------------------------------------------------------------------
// 8. DaemonState pending patch queue
// ---------------------------------------------------------------------------

#[test]
fn daemon_state_patch_queue_lifecycle() {
    let root = temp_dir("int-patch-queue");
    let mut state = daemon::DaemonState::init(root.clone()).expect("init");

    // Queue a patch
    let patch = audit::FilePatch {
        file_path: root.join("test.rs").to_string_lossy().to_string(),
        original_hash: String::new(),
        new_content: "fn patched() {}".to_string(),
        diff_summary: "+fn patched".to_string(),
        additions: 1,
        deletions: 0,
        agent_id: "agent-1".to_string(),
        task_id: None,
    };
    let patch_id = state.queue_pending_patch(patch, "auth path".to_string());
    assert!(patch_id.starts_with("patch-"));
    assert_eq!(state.pending_patches.len(), 1);

    // Approve — should write to disk
    let file_path = state.approve_patch(&patch_id).expect("approve");
    assert!(std::fs::read_to_string(&file_path).unwrap().contains("patched"));
    assert_eq!(state.pending_patches.len(), 0);

    // Queue another and reject
    let patch2 = audit::FilePatch {
        file_path: root.join("bad.rs").to_string_lossy().to_string(),
        original_hash: String::new(),
        new_content: "bad code".to_string(),
        diff_summary: "+bad".to_string(),
        additions: 1,
        deletions: 0,
        agent_id: "agent-2".to_string(),
        task_id: None,
    };
    let patch_id2 = state.queue_pending_patch(patch2, "suspicious".to_string());
    state.reject_patch(&patch_id2).expect("reject");
    assert!(!root.join("bad.rs").exists()); // should NOT be written
    assert_eq!(state.pending_patches.len(), 0);

    cleanup(&root);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_dir(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("tachy-int-{name}-{unique}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &PathBuf) {
    std::fs::remove_dir_all(dir).ok();
}

/// Simple base64 encoder for test SAML payloads.
fn base64_encode_test(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((triple >> 6) & 0x3F) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(triple & 0x3F) as usize] as char } else { '=' });
    }
    out
}
