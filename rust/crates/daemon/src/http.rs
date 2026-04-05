use std::sync::{Arc, Mutex};

use audit::{sanitize_prompt, RateLimiter};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::engine::AgentEngine;
use crate::parallel::{self, AgentTask, ParallelRun, RunStatus, TaskStatus};
use crate::state::DaemonState;
use crate::web;
use platform::ScheduleRule;

// ---------------------------------------------------------------------------
// Request/response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    models: usize,
    agents: usize,
    tasks: usize,
}

#[derive(Debug, Serialize)]
struct ModelInfo {
    name: String,
    backend: String,
    supports_tool_use: bool,
    context_window: usize,
}

#[derive(Debug, Serialize)]
struct AgentInfo {
    id: String,
    template: String,
    status: String,
    iterations: usize,
    tool_invocations: u32,
    summary: Option<String>,
}

#[derive(Debug, Serialize)]
struct TaskInfo {
    id: String,
    name: String,
    schedule: String,
    status: String,
    run_count: u32,
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct TemplateInfo {
    name: String,
    description: String,
    model: String,
    tools: Vec<String>,
    max_iterations: usize,
    requires_approval: bool,
}

#[derive(Debug, Deserialize)]
struct RunAgentRequest {
    template: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScheduleAgentRequest {
    template: String,
    name: String,
    interval_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct RunAgentResponse {
    agent_id: String,
    success: bool,
    iterations: usize,
    tool_invocations: u32,
    summary: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

/// Request to submit a parallel run.
#[derive(Debug, Deserialize)]
struct ParallelRunRequest {
    /// Tasks to execute in parallel (with optional dependencies).
    tasks: Vec<ParallelTaskInput>,
    /// Maximum concurrent agents (default: 4, capped at 8).
    #[serde(default = "default_concurrency")]
    max_concurrency: usize,
}

fn default_concurrency() -> usize { 4 }

#[derive(Debug, Deserialize)]
struct ParallelTaskInput {
    /// Agent template name.
    template: String,
    /// Prompt for the agent.
    prompt: String,
    /// Optional model override.
    #[serde(default)]
    model: Option<String>,
    /// Task IDs this task depends on (must complete first).
    #[serde(default)]
    deps: Vec<String>,
    /// Priority (higher = runs first). Default: 5.
    #[serde(default = "default_priority")]
    priority: u8,
}

fn default_priority() -> u8 { 5 }

/// Cancel request for a parallel run.
#[derive(Debug, Deserialize)]
struct CancelRunRequest {
    /// Optional: cancel a specific task. If omitted, cancels all pending tasks.
    task_id: Option<String>,
}

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------

/// Start the HTTP daemon on the given address.
pub async fn serve(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(listen_addr).await?;
    let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(120, 60))); // 120 requests per minute

    eprintln!("Tachy daemon listening on {listen_addr}");
    eprintln!("  Web UI:  http://{listen_addr}");
    eprintln!("  API:     http://{listen_addr}/health");

    loop {
        let (mut stream, addr) = listener.accept().await?;
        let state = Arc::clone(&state);
        let rate_limiter = Arc::clone(&rate_limiter);
        let client_ip = addr.ip().to_string();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 131072]; // 128KB buffer for large requests
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };

            let request = String::from_utf8_lossy(&buf[..n]);
            let response = handle_request(&request, &state, &rate_limiter, &client_ip);

            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        });
    }
}

fn handle_request(
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
    rate_limiter: &Arc<Mutex<RateLimiter>>,
    client_ip: &str,
) -> String {
    let (method, path, body) = parse_http_request(raw);

    // CORS preflight
    if method == "OPTIONS" {
        return cors_preflight_response();
    }

    // Rate limiting — skip for health and web UI
    if !matches!(path.as_str(), "/" | "/index.html" | "/health") {
        let mut limiter = rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
        // Stricter rate limit for completion endpoint (120/min vs 60/min for others)
        let rate_key = if path == "/api/complete" {
            format!("complete:{client_ip}")
        } else {
            client_ip.to_string()
        };
        if !limiter.check(&rate_key) {
            return json_response(429, &ErrorResponse {
                error: "rate limit exceeded — try again later".to_string(),
            });
        }
    }

    // Auth + RBAC check — skip for health and web UI
    if !matches!(path.as_str(), "/" | "/index.html" | "/health") {
        if let Some(required_key) = &state.lock().unwrap_or_else(|e| e.into_inner()).api_key {
            let provided = extract_auth_header(raw);
            match provided {
                Some(key) if key == *required_key => {
                    // Key matches — check RBAC for write operations
                    let action = match (method.as_str(), path.as_str()) {
                        ("GET", _) => audit::Action::ListAgents, // read access
                        ("POST", "/api/agents/run") | ("POST", "/api/chat/stream") => audit::Action::RunAgent,
                        ("POST", "/api/tasks/schedule") => audit::Action::ScheduleTask,
                        ("POST", "/api/webhooks") => audit::Action::ManageConfig,
                        _ => audit::Action::ListAgents,
                    };
                    // Default role is Admin for single-key auth
                    // Multi-user RBAC uses the UserStore
                    let role = audit::Role::Admin;
                    if let audit::AccessResult::Denied { reason } = audit::check_permission(role, action) {
                        return json_response(403, &ErrorResponse { error: reason });
                    }
                }
                Some(_) => {
                    return json_response(401, &ErrorResponse {
                        error: "invalid API key".to_string(),
                    });
                }
                None => {
                    return json_response(401, &ErrorResponse {
                        error: "API key required — set Authorization: Bearer <key> header".to_string(),
                    });
                }
            }
        }
    }

    match (method.as_str(), path.as_str()) {
        ("GET", "/" | "/index.html") => html_response(200, web::INDEX_HTML),
        ("GET", "/health") => json_response(200, &handle_health(state)),
        ("GET", "/api/models") => json_response(200, &handle_list_models(state)),
        ("GET", "/api/templates") => json_response(200, &handle_list_templates(state)),
        ("GET", "/api/agents") => json_response(200, &handle_list_agents(state)),
        ("GET", "/api/tasks") => json_response(200, &handle_list_tasks(state)),
        ("GET", "/api/audit") => handle_audit_log(state),
        ("GET", "/api/metrics") => handle_metrics(state),
        ("GET", "/api/conversations") => handle_list_conversations(state),
        ("POST", "/api/conversations") => handle_create_conversation(&body, state),
        ("POST", "/api/conversations/message") => handle_add_message(&body, state),
        ("GET", "/api/export/audit") => handle_export_audit(state),
        ("GET", "/api/export/agents") => handle_export_agents(state),
        ("POST", "/api/webhooks") => handle_add_webhook(&body, state),
        ("GET", "/api/webhooks") => handle_list_webhooks(state),
        ("POST", "/api/webhook/trigger") => handle_webhook_trigger(&body, state),
        ("GET", "/api/pending-approvals") => handle_pending_approvals(state),
        ("GET", "/api/file-locks") => handle_list_file_locks(state),
        ("POST", "/api/approve") => handle_approve(&body, state),
        ("GET", "/api/auth/sso/login") => handle_sso_login(state),
        ("POST", "/api/auth/sso/callback") => handle_sso_callback(&body, state),
        ("POST", "/api/auth/sso/logout") => handle_sso_logout(&body, state),
        ("GET", "/api/auth/sso/sessions") => handle_sso_sessions(state),
        ("POST", "/api/license/activate") => handle_license_activate(&body, state),
        ("GET", "/api/license/status") => handle_license_status(state),
        ("POST", "/api/agents/run") => {
            // License check for agent execution
            let ws_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
            let license = audit::LicenseFile::load_or_create(&ws_root.join(".tachy"));
            if !license.status().is_active() {
                return json_response(402, &ErrorResponse {
                    error: format!("{}. Purchase at https://tachy.dev/pricing", license.status().display()),
                });
            }
            handle_run_agent(&body, state)
        }
        ("POST", "/api/complete") => handle_complete(&body, state),
        ("POST", "/api/complete/stream") => handle_complete_stream(&body, state),
        ("POST", "/api/chat/stream") => {
            let ws_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
            let license = audit::LicenseFile::load_or_create(&ws_root.join(".tachy"));
            if !license.status().is_active() {
                return sse_response(&[
                    sse_event("error", &format!("{{\"error\":\"{}\"}}", license.status().display())),
                    sse_event("done", "{}"),
                ]);
            }
            handle_chat_stream(&body, state)
        }
        ("POST", "/api/tasks/schedule") => handle_schedule_agent(&body, state),
        ("POST", "/api/parallel/run") => {
            let ws_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
            let license = audit::LicenseFile::load_or_create(&ws_root.join(".tachy"));
            if !license.status().is_active() {
                return json_response(402, &ErrorResponse {
                    error: format!("{}. Purchase at https://tachy.dev/pricing", license.status().display()),
                });
            }
            handle_parallel_run(&body, state)
        }
        ("GET", "/api/parallel/runs") => handle_list_parallel_runs(state),
        _ => {
            // Dynamic routes: GET /api/agents/<id>
            if method == "GET" && path.starts_with("/api/agents/") {
                let agent_id = &path["/api/agents/".len()..];
                return handle_get_agent(agent_id, state);
            }
            // Dynamic routes: GET /api/parallel/runs/<id>
            if method == "GET" && path.starts_with("/api/parallel/runs/") {
                let run_id = &path["/api/parallel/runs/".len()..];
                return handle_get_parallel_run(run_id, state);
            }
            // Dynamic routes: POST /api/parallel/runs/<id>/cancel
            if method == "POST" && path.starts_with("/api/parallel/runs/") && path.ends_with("/cancel") {
                let run_id = path
                    .strip_prefix("/api/parallel/runs/")
                    .and_then(|s| s.strip_suffix("/cancel"))
                    .unwrap_or("");
                return handle_cancel_parallel_run(run_id, &body, state);
            }
            json_response(404, &ErrorResponse {
                error: format!("not found: {method} {path}"),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

fn handle_health(state: &Arc<Mutex<DaemonState>>) -> HealthResponse {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    HealthResponse {
        status: "ok",
        models: s.registry.list_models().len(),
        agents: s.agents.len(),
        tasks: s.scheduler.list_tasks().len(),
    }
}

fn handle_list_models(state: &Arc<Mutex<DaemonState>>) -> Vec<ModelInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.registry
        .list_models()
        .iter()
        .map(|m| ModelInfo {
            name: m.name.clone(),
            backend: format!("{:?}", m.backend),
            supports_tool_use: m.supports_tool_use,
            context_window: m.context_window,
        })
        .collect()
}

fn handle_list_templates(state: &Arc<Mutex<DaemonState>>) -> Vec<TemplateInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.config
        .agent_templates
        .iter()
        .map(|t| TemplateInfo {
            name: t.name.clone(),
            description: t.description.clone(),
            model: t.model.clone(),
            tools: t.allowed_tools.clone(),
            max_iterations: t.max_iterations,
            requires_approval: t.requires_approval,
        })
        .collect()
}

fn handle_get_agent(agent_id: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.agents.get(agent_id) {
        Some(a) => json_response(200, &AgentInfo {
            id: a.id.clone(),
            template: a.config.template.name.clone(),
            status: format!("{:?}", a.status).to_lowercase(),
            iterations: a.iterations_completed,
            tool_invocations: a.tool_invocations,
            summary: a.result_summary.clone(),
        }),
        None => json_response(404, &ErrorResponse {
            error: format!("agent not found: {agent_id}"),
        }),
    }
}

fn handle_list_agents(state: &Arc<Mutex<DaemonState>>) -> Vec<AgentInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.agents
        .values()
        .map(|a| AgentInfo {
            id: a.id.clone(),
            template: a.config.template.name.clone(),
            status: format!("{:?}", a.status),
            iterations: a.iterations_completed,
            tool_invocations: a.tool_invocations,
            summary: a.result_summary.clone(),
        })
        .collect()
}

fn handle_list_tasks(state: &Arc<Mutex<DaemonState>>) -> Vec<TaskInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.scheduler
        .list_tasks()
        .iter()
        .map(|t| TaskInfo {
            id: t.id.clone(),
            name: t.name.clone(),
            schedule: format!("{:?}", t.schedule),
            status: format!("{:?}", t.status),
            run_count: t.run_count,
            enabled: t.enabled,
        })
        .collect()
}

fn handle_audit_log(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");

    let events: Vec<serde_json::Value> = match std::fs::read_to_string(&audit_path) {
        Ok(content) => content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
        Err(_) => Vec::new(),
    };

    json_response(200, &events)
}

fn handle_metrics(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());

    let total_agents = s.agents.len();
    let completed = s.agents.values().filter(|a| format!("{:?}", a.status) == "Completed").count();
    let failed = s.agents.values().filter(|a| format!("{:?}", a.status) == "Failed").count();
    let total_iterations: usize = s.agents.values().map(|a| a.iterations_completed).sum();
    let total_tools: u32 = s.agents.values().map(|a| a.tool_invocations).sum();

    // Count by template
    let mut template_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for agent in s.agents.values() {
        *template_counts.entry(agent.config.template.name.clone()).or_insert(0) += 1;
    }

    let metrics = serde_json::json!({
        "total_agents_run": total_agents,
        "completed": completed,
        "failed": failed,
        "total_iterations": total_iterations,
        "total_tool_invocations": total_tools,
        "agents_by_template": template_counts,
        "scheduled_tasks": s.scheduler.list_tasks().len(),
        "models_available": s.registry.list_models().len(),
    });

    json_response(200, &metrics)
}

// --- Conversation API ---

fn handle_list_conversations(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let convs: Vec<serde_json::Value> = s.conversations.values().map(|c| {
        serde_json::json!({
            "id": c.id,
            "title": c.title,
            "messages": c.messages,
            "message_count": c.messages.len(),
            "created_at": c.created_at,
            "updated_at": c.updated_at,
            "workspace": c.workspace,
        })
    }).collect();
    json_response(200, &convs)
}

fn handle_create_conversation(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct Req { title: Option<String> }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { title: None });
    let title = req.title.unwrap_or_else(|| format!("Conversation {}", chrono_now()));

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let id = s.create_conversation(&title);
    json_response(200, &serde_json::json!({ "id": id, "title": title }))
}

fn handle_add_message(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct Req {
        conversation_id: String,
        role: String,
        content: String,
        model: Option<String>,
        iterations: Option<usize>,
        tool_invocations: Option<u32>,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return json_response(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let msg = crate::state::ChatMessage {
        role: req.role,
        content: req.content,
        timestamp: chrono_now(),
        model: req.model,
        iterations: req.iterations,
        tool_invocations: req.tool_invocations,
    };

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if s.add_message(&req.conversation_id, msg) {
        json_response(200, &serde_json::json!({ "ok": true }))
    } else {
        json_response(404, &ErrorResponse { error: "conversation not found".to_string() })
    }
}

// --- Export API ---

fn handle_export_audit(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");

    let csv = match std::fs::read_to_string(&audit_path) {
        Ok(content) => {
            let mut lines = vec!["timestamp,kind,severity,session_id,agent_id,tool_name,detail".to_string()];
            for line in content.lines() {
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                    lines.push(format!(
                        "{},{},{},{},{},{},\"{}\"",
                        event.get("timestamp").and_then(|v| v.as_str()).unwrap_or(""),
                        event.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                        event.get("severity").and_then(|v| v.as_str()).unwrap_or(""),
                        event.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                        event.get("agent_id").and_then(|v| v.as_str()).unwrap_or(""),
                        event.get("tool_name").and_then(|v| v.as_str()).unwrap_or(""),
                        event.get("detail").and_then(|v| v.as_str()).unwrap_or("").replace('"', "\"\""),
                    ));
                }
            }
            lines.join("\n")
        }
        Err(_) => "No audit data".to_string(),
    };

    csv_response(&csv, "tachy-audit-export.csv")
}

fn handle_export_agents(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let mut lines = vec!["id,template,status,iterations,tool_invocations,created_at,summary".to_string()];
    for agent in s.agents.values() {
        lines.push(format!(
            "{},{},{:?},{},{},{},\"{}\"",
            agent.id,
            agent.config.template.name,
            agent.status,
            agent.iterations_completed,
            agent.tool_invocations,
            agent.created_at,
            agent.result_summary.as_deref().unwrap_or("").replace('"', "\"\""),
        ));
    }
    csv_response(&lines.join("\n"), "tachy-agents-export.csv")
}

// --- Webhook API ---

fn handle_add_webhook(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct Req { url: String, events: Vec<String> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return json_response(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.webhooks.push(crate::state::WebhookConfig {
        url: req.url.clone(),
        events: req.events.clone(),
        enabled: true,
    });
    s.save();
    json_response(200, &serde_json::json!({ "ok": true, "url": req.url, "events": req.events }))
}

fn handle_list_webhooks(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    json_response(200, &s.webhooks)
}

/// Webhook trigger endpoint — external systems (GitHub, Jira, PagerDuty) can
/// trigger Tachy agents via HTTP POST.
///
/// POST /api/webhook/trigger
/// {
///   "source": "github",
///   "event": "push",
///   "template": "code-reviewer",
///   "prompt": "Review the latest push to main",
///   "payload": { ... }  // raw webhook payload from the source
/// }
fn handle_webhook_trigger(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct WebhookTrigger {
        source: Option<String>,
        event: Option<String>,
        template: Option<String>,
        prompt: Option<String>,
        payload: Option<serde_json::Value>,
    }

    let trigger: WebhookTrigger = match serde_json::from_str(body) {
        Ok(t) => t,
        Err(e) => {
            return json_response(400, &ErrorResponse { error: format!("invalid webhook body: {e}") });
        }
    };

    let source = trigger.source.as_deref().unwrap_or("unknown");
    let event = trigger.event.as_deref().unwrap_or("unknown");
    let template = trigger.template.as_deref().unwrap_or("chat");

    // Build prompt from webhook data
    let prompt = if let Some(p) = &trigger.prompt {
        p.clone()
    } else {
        // Auto-generate prompt from source and payload
        let payload_summary = trigger.payload
            .as_ref()
            .map(|p| serde_json::to_string_pretty(p).unwrap_or_default())
            .unwrap_or_default();
        let truncated = if payload_summary.len() > 2000 {
            format!("{}…", &payload_summary[..2000])
        } else {
            payload_summary
        };
        format!("Webhook from {source} ({event}):\n\n```json\n{truncated}\n```\n\nAnalyze this event and take appropriate action.")
    };

    let prompt = sanitize_prompt(&prompt, 50_000);

    // Create and run agent asynchronously
    let (agent_id, config, governance) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let agent_id = match s.create_agent(template, &prompt) {
            Ok(id) => id,
            Err(e) => {
                return json_response(400, &ErrorResponse { error: e });
            }
        };
        let config = match s.agents.get(&agent_id) {
            Some(a) => a.config.clone(),
            None => {
                return json_response(500, &ErrorResponse { error: "agent not found".to_string() });
            }
        };
        if let Some(agent) = s.agents.get_mut(&agent_id) {
            agent.mark_running();
        }
        s.save();

        // Log the webhook trigger
        s.audit_logger.log(
            &audit::AuditEvent::new(&config.session_id, audit::AuditEventKind::SessionStart,
                format!("webhook trigger: source={source} event={event} template={template}"))
                .with_agent(&agent_id),
        );

        let governance = s.config.governance.clone();
        (agent_id, config, governance)
    };

    // Run in background
    let bg_state = Arc::clone(state);
    let bg_agent_id = agent_id.clone();
    let bg_prompt = prompt.clone();
    std::thread::spawn(move || {
        let result = {
            let s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
            let workspace_root = s.workspace_root.clone();
            let file_locks = s.file_locks.clone();
            AgentEngine::run_agent(
                &bg_agent_id, &config, &bg_prompt, &s.registry, &governance,
                &s.audit_logger, &s.config.intelligence, &workspace_root,
                Some(file_locks),
            )
        };
        let mut s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&bg_agent_id) {
            agent.iterations_completed = result.iterations;
            agent.tool_invocations = result.tool_invocations;
            if result.success { agent.mark_completed(&result.summary); }
            else { agent.mark_failed(&result.summary); }
        }
        s.save();
        s.fire_webhooks("agent_completed", &serde_json::json!({
            "agent_id": result.agent_id,
            "source": "webhook_trigger",
            "success": result.success,
        }));
    });

    json_response(202, &serde_json::json!({
        "agent_id": agent_id,
        "status": "running",
        "source": source,
        "event": event,
        "template": template,
    }))
}

// --- Human-in-the-Loop Approval ---

fn handle_pending_approvals(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());

    // Agents in suspended state
    let pending_agents: Vec<serde_json::Value> = s.agents.values()
        .filter(|a| format!("{:?}", a.status) == "Suspended")
        .map(|a| serde_json::json!({
            "type": "agent",
            "agent_id": a.id,
            "template": a.config.template.name,
            "prompt": a.result_summary.as_deref().unwrap_or(""),
            "created_at": a.created_at,
        }))
        .collect();

    // Patches awaiting approval from policy engine
    let pending_patches: Vec<serde_json::Value> = s.pending_patches.iter()
        .map(|p| serde_json::json!({
            "type": "patch",
            "patch_id": p.id,
            "file_path": p.patch.file_path,
            "agent_id": p.patch.agent_id,
            "reason": p.reason,
            "diff_summary": p.patch.diff_summary,
            "additions": p.patch.additions,
            "deletions": p.patch.deletions,
            "created_at": p.created_at,
        }))
        .collect();

    let mut all = pending_agents;
    all.extend(pending_patches);

    json_response(200, &serde_json::json!({
        "pending": all,
        "count": all.len(),
    }))
}

fn handle_approve(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct Req {
        /// Agent ID (for agent approvals) or patch ID (for patch approvals).
        #[serde(default)]
        agent_id: Option<String>,
        #[serde(default)]
        patch_id: Option<String>,
        approved: bool,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return json_response(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    // Handle patch approval
    if let Some(patch_id) = &req.patch_id {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        if req.approved {
            match s.approve_patch(patch_id) {
                Ok(file_path) => {
                    return json_response(200, &serde_json::json!({
                        "ok": true, "status": "approved", "file_path": file_path
                    }));
                }
                Err(e) => return json_response(404, &ErrorResponse { error: e }),
            }
        } else {
            match s.reject_patch(patch_id) {
                Ok(file_path) => {
                    return json_response(200, &serde_json::json!({
                        "ok": true, "status": "rejected", "file_path": file_path
                    }));
                }
                Err(e) => return json_response(404, &ErrorResponse { error: e }),
            }
        }
    }

    // Handle agent approval (legacy)
    let agent_id = match &req.agent_id {
        Some(id) => id.clone(),
        None => return json_response(400, &ErrorResponse {
            error: "either 'agent_id' or 'patch_id' is required".to_string(),
        }),
    };

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(agent) = s.agents.get_mut(&agent_id) {
        let session_id = agent.config.session_id.clone();
        if req.approved {
            agent.mark_running();
            s.audit_logger.log(
                &audit::AuditEvent::new(
                    &session_id,
                    audit::AuditEventKind::PermissionGranted,
                    format!("human approved agent {agent_id}"),
                )
                .with_agent(&agent_id),
            );
            json_response(200, &serde_json::json!({ "ok": true, "status": "approved" }))
        } else {
            agent.mark_failed("rejected by human reviewer");
            s.audit_logger.log(
                &audit::AuditEvent::new(
                    &session_id,
                    audit::AuditEventKind::PermissionDenied,
                    format!("human rejected agent {agent_id}"),
                )
                .with_agent(&agent_id)
                .with_severity(audit::AuditSeverity::Warning),
            );
            json_response(200, &serde_json::json!({ "ok": true, "status": "rejected" }))
        }
    } else {
        json_response(404, &ErrorResponse { error: "agent not found".to_string() })
    }
}

// --- Helpers ---

fn chrono_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", d.as_secs())
}

fn csv_response(body: &str, filename: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/csv\r\n\
         Content-Disposition: attachment; filename=\"{filename}\"\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    )
}

fn handle_run_agent(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let req: RunAgentRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return json_response(400, &ErrorResponse {
                error: format!("invalid request body: {e}"),
            });
        }
    };

    // Sanitize input
    let prompt = sanitize_prompt(&req.prompt, 50_000);
    if prompt.is_empty() {
        return json_response(400, &ErrorResponse {
            error: "prompt cannot be empty".to_string(),
        });
    }

    // Create the agent
    let (agent_id, config, governance) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let agent_id = match s.create_agent(&req.template, &prompt) {
            Ok(id) => id,
            Err(e) => {
                return json_response(400, &ErrorResponse { error: e });
            }
        };
        let mut config = match s.agents.get(&agent_id) {
            Some(a) => a.config.clone(),
            None => {
                return json_response(500, &ErrorResponse { error: "agent not found after creation".to_string() });
            }
        };
        // Allow model override from request
        if let Some(model) = &req.model {
            if !model.is_empty() {
                config.template.model = model.clone();
            }
        }
        // Mark agent as running
        if let Some(agent) = s.agents.get_mut(&agent_id) {
            agent.mark_running();
        }
        s.save();
        let governance = s.config.governance.clone();
        (agent_id, config, governance)
    };

    // Spawn agent execution in a background thread so the HTTP response returns immediately.
    // The client can poll GET /api/agents to check status.
    let bg_state = Arc::clone(state);
    let bg_agent_id = agent_id.clone();
    let bg_prompt = prompt.clone();
    std::thread::spawn(move || {
        let result = {
            let s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
            let workspace_root = s.workspace_root.clone();
            let file_locks = s.file_locks.clone();
            AgentEngine::run_agent(
                &bg_agent_id,
                &config,
                &bg_prompt,
                &s.registry,
                &governance,
                &s.audit_logger,
                &s.config.intelligence,
                &workspace_root,
                Some(file_locks),
            )
        };

        // Update agent state with results
        let mut s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&bg_agent_id) {
            agent.iterations_completed = result.iterations;
            agent.tool_invocations = result.tool_invocations;
            if result.success {
                agent.mark_completed(&result.summary);
            } else {
                agent.mark_failed(&result.summary);
            }
        }
        s.save();

        // Fire webhooks
        s.fire_webhooks("agent_completed", &serde_json::json!({
            "agent_id": result.agent_id,
            "success": result.success,
            "iterations": result.iterations,
            "tool_invocations": result.tool_invocations,
        }));
    });

    // Return immediately with agent ID — client polls GET /api/agents for status
    json_response(202, &serde_json::json!({
        "agent_id": agent_id,
        "status": "running",
        "message": "Agent started. Poll GET /api/agents to check status.",
    }))
}

/// Streaming chat endpoint — now delegates to the async agent pattern.
/// The web UI uses POST /api/agents/run + polling instead.
/// This endpoint is kept for backward compatibility with direct API users.
fn handle_chat_stream(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let req: RunAgentRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return sse_response(&[
                sse_event("error", &format!("{{\"error\":\"{e}\"}}")),
                sse_event("done", "{}"),
            ]);
        }
    };

    let prompt = sanitize_prompt(&req.prompt, 50_000);

    // Create agent
    let (agent_id, config, governance) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let agent_id = match s.create_agent(&req.template, &prompt) {
            Ok(id) => id,
            Err(e) => {
                return sse_response(&[
                    sse_event("error", &format!("{{\"error\":\"{e}\"}}")),
                    sse_event("done", "{}"),
                ]);
            }
        };
        let mut config = match s.agents.get(&agent_id) {
            Some(a) => a.config.clone(),
            None => {
                return sse_response(&[
                    sse_event("error", "{\"error\":\"agent not found\"}"),
                    sse_event("done", "{}"),
                ]);
            }
        };
        if let Some(model) = &req.model {
            if !model.is_empty() {
                config.template.model = model.clone();
            }
        }
        if let Some(agent) = s.agents.get_mut(&agent_id) {
            agent.mark_running();
        }
        s.save();
        let governance = s.config.governance.clone();
        (agent_id, config, governance)
    };

    // Run synchronously for SSE (the response streams all at once after completion)
    let result = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        let workspace_root = s.workspace_root.clone();
        let file_locks = s.file_locks.clone();
        AgentEngine::run_agent(
            &agent_id, &config, &prompt, &s.registry, &governance,
            &s.audit_logger, &s.config.intelligence, &workspace_root,
            Some(file_locks),
        )
    };

    // Update state
    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&agent_id) {
            agent.iterations_completed = result.iterations;
            agent.tool_invocations = result.tool_invocations;
            if result.success { agent.mark_completed(&result.summary); }
            else { agent.mark_failed(&result.summary); }
        }
        s.save();
    }

    // Send result as SSE events
    let mut events = vec![
        sse_event("status", &format!("{{\"status\":\"thinking\",\"template\":\"{}\"}}", req.template)),
    ];

    // Stream text in chunks
    let escaped = result.summary.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    for chunk in escaped.as_bytes().chunks(200) {
        if let Ok(s) = std::str::from_utf8(chunk) {
            events.push(sse_event("token", &format!("{{\"text\":\"{s}\"}}")));
        }
    }

    events.push(sse_event("done", &serde_json::json!({
        "agent_id": result.agent_id,
        "success": result.success,
        "iterations": result.iterations,
        "tool_invocations": result.tool_invocations,
    }).to_string()));

    sse_response(&events)
}

fn sse_event(event_type: &str, data: &str) -> String {
    format!("event: {event_type}\ndata: {data}\n\n")
}

fn sse_response(events: &[String]) -> String {
    let body = events.join("");
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {body}",
        body.len()
    )
}

fn handle_schedule_agent(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let req: ScheduleAgentRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return json_response(400, &ErrorResponse {
                error: format!("invalid request body: {e}"),
            });
        }
    };

    let schedule = match req.interval_seconds {
        Some(secs) => ScheduleRule::Interval { seconds: secs },
        None => ScheduleRule::Once,
    };

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.schedule_agent(&req.template, schedule, &req.name) {
        Ok(task_id) => json_response(200, &serde_json::json!({ "task_id": task_id })),
        Err(e) => json_response(400, &ErrorResponse { error: e }),
    }
}

// ---------------------------------------------------------------------------
// Inline completion handler (for VS Code extension)
// ---------------------------------------------------------------------------

fn handle_complete(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct CompleteRequest {
        prefix: String,
        suffix: Option<String>,
        language: Option<String>,
        #[serde(default = "default_complete_model")]
        model: String,
        #[serde(default = "default_max_tokens")]
        max_tokens: usize,
    }
    fn default_complete_model() -> String { "gemma4:26b".to_string() }
    fn default_max_tokens() -> usize { 128 }

    let req: CompleteRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return json_response(400, &ErrorResponse {
                error: format!("invalid request: {e}"),
            });
        }
    };

    let lang = req.language.as_deref().unwrap_or("code");
    let max_tokens = req.max_tokens;
    let prompt = format!(
        "Complete the following {lang} code. Return ONLY the completion, no explanation, no markdown fences. Maximum {max_tokens} tokens.\n\n{}{}\n",
        req.prefix,
        req.suffix.as_deref().unwrap_or("")
    );

    // Run synchronously with a minimal agent config
    let (config, governance) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        let mut template = platform::AgentTemplate::chat_assistant();
        template.model = req.model.clone();
        template.max_iterations = 1; // Single turn for completion
        let config = platform::AgentConfig {
            template,
            session_id: format!("complete-{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()),
            working_directory: s.workspace_root.to_string_lossy().to_string(),
            environment: std::collections::BTreeMap::new(),
        };
        let governance = s.config.governance.clone();
        (config, governance)
    };

    let result = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        AgentEngine::run_agent(
            "completer", &config, &prompt, &s.registry, &governance,
            &s.audit_logger, &s.config.intelligence, &s.workspace_root,
            None,
        )
    };

    json_response(200, &serde_json::json!({
        "completion": truncate_completion(&result.summary, max_tokens),
        "model": req.model,
        "success": result.success,
    }))
}

/// Streaming completion endpoint — returns SSE events as tokens arrive.
fn handle_complete_stream(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct Req {
        prefix: String,
        suffix: Option<String>,
        language: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        max_tokens: Option<usize>,
    }

    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return sse_response(&[
                sse_event("error", &format!("{{\"error\":\"{e}\"}}")),
                sse_event("done", "{}"),
            ]);
        }
    };

    let lang = req.language.as_deref().unwrap_or("code");
    let max_tokens = req.max_tokens.unwrap_or(128);
    let model = req.model.unwrap_or_else(|| "gemma4:26b".to_string());
    let prompt = format!(
        "Complete the following {lang} code. Return ONLY the completion, no explanation, no markdown fences. Maximum {max_tokens} tokens.\n\n{}{}\n",
        req.prefix,
        req.suffix.as_deref().unwrap_or("")
    );

    // Run the agent synchronously (single turn)
    let (config, governance) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        let mut template = platform::AgentTemplate::chat_assistant();
        template.model = model.clone();
        template.max_iterations = 1;
        // Disable tools for pure completion
        template.allowed_tools = Vec::new();
        let config = platform::AgentConfig {
            template,
            session_id: format!("stream-{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()),
            working_directory: s.workspace_root.to_string_lossy().to_string(),
            environment: std::collections::BTreeMap::new(),
        };
        (config, s.config.governance.clone())
    };

    let result = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        AgentEngine::run_agent(
            "completer-stream", &config, &prompt, &s.registry, &governance,
            &s.audit_logger, &s.config.intelligence, &s.workspace_root, None,
        )
    };

    // Stream the result as SSE token events (chunked)
    let completion = truncate_completion(&result.summary, max_tokens);
    let mut events = Vec::new();
    events.push(sse_event("start", &serde_json::json!({"model": model}).to_string()));

    // Chunk the completion into ~20 char pieces to simulate streaming
    for chunk in completion.as_bytes().chunks(20) {
        if let Ok(s) = std::str::from_utf8(chunk) {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
            events.push(sse_event("token", &format!("{{\"text\":\"{escaped}\"}}")));
        }
    }

    events.push(sse_event("done", &serde_json::json!({
        "success": result.success,
        "total_tokens": completion.len() / 4,
    }).to_string()));

    sse_response(&events)
}

// ---------------------------------------------------------------------------
// SSO/SAML handlers
// ---------------------------------------------------------------------------

fn handle_sso_login(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    if !s.sso_manager.is_enabled() {
        return json_response(400, &ErrorResponse {
            error: "SSO is not configured. Set sso.enabled=true in config.".to_string(),
        });
    }
    let login_url = s.sso_manager.build_login_url(Some("/"));

    // Return a redirect response
    format!(
        "HTTP/1.1 302 Found\r\n\
         Location: {login_url}\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\
         \r\n"
    )
}

fn handle_sso_callback(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if !s.sso_manager.is_enabled() {
        return json_response(400, &ErrorResponse {
            error: "SSO is not configured".to_string(),
        });
    }

    // Extract SAMLResponse from form-encoded body
    let saml_response = extract_form_value(body, "SAMLResponse");
    let saml_response = match saml_response {
        Some(r) => r,
        None => {
            // Try JSON body
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) {
                match parsed.get("SAMLResponse").and_then(|v| v.as_str()) {
                    Some(r) => r.to_string(),
                    None => return json_response(400, &ErrorResponse {
                        error: "SAMLResponse parameter required".to_string(),
                    }),
                }
            } else {
                return json_response(400, &ErrorResponse {
                    error: "SAMLResponse parameter required".to_string(),
                });
            }
        }
    };

    let mut user_store = std::mem::take(&mut s.user_store);
    let result = s.sso_manager.process_callback(&saml_response, &mut user_store);
    s.user_store = user_store;

    match result {
        Ok(session) => {
            s.audit_logger.log(
                &audit::AuditEvent::new("sso", audit::AuditEventKind::PermissionGranted,
                    format!("SSO login: {} (role={:?})", session.email, session.role))
                    .with_user(&session.user_id),
            );
            json_response(200, &serde_json::json!({
                "token": session.token,
                "user_id": session.user_id,
                "email": session.email,
                "role": format!("{:?}", session.role),
                "expires_at": session.expires_at,
            }))
        }
        Err(e) => {
            s.audit_logger.log(
                &audit::AuditEvent::new("sso", audit::AuditEventKind::PermissionDenied,
                    format!("SSO login failed: {e}"))
                    .with_severity(audit::AuditSeverity::Warning),
            );
            json_response(401, &ErrorResponse { error: e })
        }
    }
}

fn handle_sso_logout(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct Req { token: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return json_response(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.sso_manager.invalidate_session(&req.token);
    s.audit_logger.log(
        &audit::AuditEvent::new("sso", audit::AuditEventKind::SessionEnd, "SSO logout"),
    );
    json_response(200, &serde_json::json!({ "ok": true }))
}

fn handle_sso_sessions(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let sessions: Vec<_> = s.sso_manager.active_sessions().iter().map(|sess| {
        serde_json::json!({
            "user_id": sess.user_id,
            "email": sess.email,
            "role": format!("{:?}", sess.role),
            "created_at": sess.created_at,
            "expires_at": sess.expires_at,
        })
    }).collect();
    json_response(200, &serde_json::json!({
        "sessions": sessions,
        "count": sessions.len(),
    }))
}

/// Truncate a completion to approximately max_tokens (4 chars ≈ 1 token).
fn truncate_completion(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * 4;
    if text.len() <= max_chars {
        text.to_string()
    } else {
        text[..max_chars].to_string()
    }
}

/// Extract a value from URL-encoded form data.
fn extract_form_value(body: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    for part in body.split('&') {
        if let Some(value) = part.strip_prefix(&prefix) {
            // URL-decode
            return Some(value.replace('+', " ").replace("%3D", "=").replace("%2B", "+"));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// License handlers
// ---------------------------------------------------------------------------

fn handle_license_activate(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    #[derive(Deserialize)]
    struct Req { key: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return json_response(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let tachy_dir = s.workspace_root.join(".tachy");
    let mut license = audit::LicenseFile::load_or_create(&tachy_dir);

    // Use the default license secret (same as the license server)
    let secret = std::env::var("TACHY_LICENSE_SECRET")
        .unwrap_or_else(|_| "tachy-license-secret-v1".to_string());

    match license.activate(&req.key, &secret) {
        Ok(data) => {
            s.audit_logger.log(
                &audit::AuditEvent::new("daemon", audit::AuditEventKind::ConfigChange,
                    format!("license activated: {:?}", data.tier)),
            );
            json_response(200, &serde_json::json!({
                "ok": true,
                "status": license.status().display(),
                "tier": format!("{:?}", data.tier),
            }))
        }
        Err(e) => json_response(400, &ErrorResponse { error: e }),
    }
}

fn handle_license_status(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let tachy_dir = s.workspace_root.join(".tachy");
    let license = audit::LicenseFile::load_or_create(&tachy_dir);
    json_response(200, &serde_json::json!({
        "status": license.status().display(),
        "active": license.status().is_active(),
    }))
}

// ---------------------------------------------------------------------------
// File lock handler
// ---------------------------------------------------------------------------

fn handle_list_file_locks(state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let locks = s.file_locks.list_locks();
    let entries: Vec<_> = locks.iter().map(|(file, agent)| {
        serde_json::json!({ "file": file, "agent_id": agent })
    }).collect();
    json_response(200, &serde_json::json!({
        "locks": entries,
        "count": entries.len(),
    }))
}

// ---------------------------------------------------------------------------
// Parallel execution handlers
// ---------------------------------------------------------------------------

fn handle_parallel_run(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let req: ParallelRunRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return json_response(400, &ErrorResponse {
                error: format!("invalid request body: {e}"),
            });
        }
    };

    if req.tasks.is_empty() {
        return json_response(400, &ErrorResponse {
            error: "at least one task is required".to_string(),
        });
    }

    // Validate templates exist
    {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        for task_input in &req.tasks {
            if !s.config.agent_templates.iter().any(|t| t.name == task_input.template) {
                return json_response(400, &ErrorResponse {
                    error: format!("unknown template: {}", task_input.template),
                });
            }
        }
    }

    // Build the run ID and tasks
    let run_id = format!("run-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let tasks: Vec<AgentTask> = req.tasks.iter().enumerate().map(|(i, t)| {
        AgentTask {
            id: format!("{run_id}-t{i}"),
            run_id: run_id.clone(),
            template: t.template.clone(),
            prompt: audit::sanitize_prompt(&t.prompt, 50_000),
            model: t.model.clone(),
            deps: t.deps.clone(),
            priority: t.priority,
            status: TaskStatus::Pending,
            result: None,
            created_at: now,
            started_at: None,
            completed_at: None,
            work_dir: None,
        }
    }).collect();

    let max_concurrency = req.max_concurrency.min(8).max(1);

    let run = ParallelRun {
        id: run_id.clone(),
        tasks,
        status: RunStatus::Running,
        created_at: now,
        max_concurrency,
    };

    // Log the parallel run start
    {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.audit_logger.log(
            &audit::AuditEvent::new("daemon", audit::AuditEventKind::SessionStart,
                format!("parallel run {run_id}: {} tasks, max_concurrency={max_concurrency}",
                    req.tasks.len()))
        );
    }

    // Execute in a background thread
    let bg_state = Arc::clone(state);
    let bg_run_id = run_id.clone();
    let task_count = run.tasks.len();
    std::thread::spawn(move || {
        let completed_run = parallel::execute_parallel_run(run, &bg_state);

        // Log completion
        let s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
        let completed = completed_run.tasks.iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();
        s.audit_logger.log(
            &audit::AuditEvent::new("daemon", audit::AuditEventKind::SessionEnd,
                format!("parallel run {bg_run_id}: {completed}/{task_count} tasks completed, status={:?}",
                    completed_run.status))
        );

        // Fire webhooks
        s.fire_webhooks("parallel_run_completed", &serde_json::json!({
            "run_id": bg_run_id,
            "status": completed_run.status,
            "tasks_completed": completed,
            "tasks_total": task_count,
        }));
    });

    json_response(202, &serde_json::json!({
        "run_id": run_id,
        "status": "running",
        "task_count": req.tasks.len(),
        "max_concurrency": max_concurrency,
        "message": "Parallel run started. Poll GET /api/parallel/runs/<id> for status.",
    }))
}

fn handle_list_parallel_runs(state: &Arc<Mutex<DaemonState>>) -> String {
    // The orchestrator lives inside execute_parallel_run threads, so we track runs
    // via the audit log. For now, return a summary from audit events.
    let s = state.lock().unwrap_or_else(|e| e.into_inner());

    // Scan agents for parallel run tasks
    let parallel_agents: Vec<_> = s.agents.iter()
        .filter(|(id, _)| id.starts_with("run-"))
        .map(|(id, agent)| {
            serde_json::json!({
                "agent_id": id,
                "status": format!("{:?}", agent.status),
                "template": agent.config.template.name,
            })
        })
        .collect();

    json_response(200, &serde_json::json!({
        "runs": parallel_agents,
        "count": parallel_agents.len(),
    }))
}

fn handle_get_parallel_run(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());

    // Find all tasks belonging to this run
    let tasks: Vec<_> = s.agents.iter()
        .filter(|(id, _)| id.starts_with(run_id))
        .map(|(id, agent)| {
            serde_json::json!({
                "task_id": id,
                "template": agent.config.template.name,
                "status": format!("{:?}", agent.status),
                "iterations": agent.iterations_completed,
                "tool_invocations": agent.tool_invocations,
                "summary": agent.result_summary,
            })
        })
        .collect();

    if tasks.is_empty() {
        return json_response(404, &ErrorResponse {
            error: format!("parallel run '{run_id}' not found"),
        });
    }

    let all_done = tasks.iter().all(|t| {
        let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
        status.contains("Completed") || status.contains("Failed")
    });
    let any_success = tasks.iter().any(|t| {
        let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
        status.contains("Completed")
    });

    let run_status = if !all_done {
        "running"
    } else if any_success && tasks.iter().all(|t| {
        t.get("status").and_then(|v| v.as_str()).unwrap_or("").contains("Completed")
    }) {
        "completed"
    } else if any_success {
        "partially_completed"
    } else {
        "failed"
    };

    json_response(200, &serde_json::json!({
        "run_id": run_id,
        "status": run_status,
        "tasks": tasks,
        "task_count": tasks.len(),
    }))
}

fn handle_cancel_parallel_run(run_id: &str, body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let cancel_req: Option<CancelRunRequest> = serde_json::from_str(body).ok();

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());

    let mut cancelled = 0usize;
    let task_ids: Vec<String> = s.agents.keys()
        .filter(|id| id.starts_with(run_id))
        .cloned()
        .collect();

    if task_ids.is_empty() {
        return json_response(404, &ErrorResponse {
            error: format!("parallel run '{run_id}' not found"),
        });
    }

    for task_id in &task_ids {
        // If a specific task_id was requested, only cancel that one
        if let Some(ref req) = cancel_req {
            if let Some(ref specific_id) = req.task_id {
                if task_id != specific_id {
                    continue;
                }
            }
        }

        if let Some(agent) = s.agents.get_mut(task_id) {
            // Only cancel if not already completed
            if !agent.result_summary.as_deref().map(|s| s.contains("completed")).unwrap_or(false) {
                agent.mark_failed("cancelled by user");
                cancelled += 1;
            }
        }
    }

    s.audit_logger.log(
        &audit::AuditEvent::new("daemon", audit::AuditEventKind::SessionEnd,
            format!("parallel run {run_id}: {cancelled} tasks cancelled"))
    );
    s.save();

    json_response(200, &serde_json::json!({
        "run_id": run_id,
        "cancelled": cancelled,
        "message": format!("{cancelled} task(s) cancelled"),
    }))
}

// ---------------------------------------------------------------------------
// HTTP helpers (minimal, no framework)
// ---------------------------------------------------------------------------

fn json_response<T: Serialize>(status: u16, body: &T) -> String {
    let json = serde_json::to_string_pretty(body).unwrap_or_else(|_| "{}".to_string());
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: http://localhost:7777\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Connection: close\r\n\
         \r\n\
         {json}",
        json.len()
    )
}

fn cors_preflight_response() -> String {
    "HTTP/1.1 204 No Content\r\n\
     Access-Control-Allow-Origin: http://localhost:7777\r\n\
     Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
     Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
     Access-Control-Max-Age: 86400\r\n\
     Content-Length: 0\r\n\
     Connection: close\r\n\
     \r\n".to_string()
}

fn html_response(status: u16, body: &str) -> String {
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Unknown",
    };
    format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    )
}

fn parse_http_request(raw: &str) -> (String, String, String) {
    let mut lines = raw.lines();
    let first_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.first().unwrap_or(&"GET").to_string();
    let path = parts.get(1).unwrap_or(&"/").to_string();

    // Find body after empty line
    let body = if let Some(pos) = raw.find("\r\n\r\n") {
        raw[pos + 4..].to_string()
    } else if let Some(pos) = raw.find("\n\n") {
        raw[pos + 2..].to_string()
    } else {
        String::new()
    };

    (method, path, body)
}

fn extract_auth_header(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("authorization:") {
            let value = line[14..].trim();
            if let Some(token) = value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")) {
                return Some(token.trim().to_string());
            }
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_request() {
        let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let raw = "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (method, path, body) = parse_http_request(raw);
        assert_eq!(method, "GET");
        assert_eq!(path, "/health");
        assert!(body.is_empty());
    }

    #[test]
    fn parses_post_with_body() {
        let raw = "POST /api/agents/run HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"template\":\"code-reviewer\",\"prompt\":\"review\"}";
        let (method, path, body) = parse_http_request(raw);
        assert_eq!(method, "POST");
        assert_eq!(path, "/api/agents/run");
        assert!(body.contains("code-reviewer"));
    }

    #[test]
    fn json_response_formats_correctly() {
        let resp = json_response(200, &serde_json::json!({"ok": true}));
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("application/json"));
        assert!(resp.contains(r#""ok": true"#));
    }

    #[test]
    fn health_endpoint_works() {
        let root = std::env::temp_dir().join(format!(
            "tachy-http-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = Arc::new(Mutex::new(
            DaemonState::init(root.clone()).expect("should init"),
        ));

        let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let raw = "GET /health HTTP/1.1\r\n\r\n";
        let response = handle_request(raw, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("\"status\": \"ok\""));
        assert!(response.contains("\"models\":"));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn models_endpoint_returns_list() {
        let root = std::env::temp_dir().join(format!(
            "tachy-http-models-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = Arc::new(Mutex::new(
            DaemonState::init(root.clone()).expect("should init"),
        ));

        let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let raw = "GET /api/models HTTP/1.1\r\n\r\n";
        let response = handle_request(raw, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("qwen2.5-coder"));
        assert!(response.contains("Ollama"));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn templates_endpoint_returns_list() {
        let root = std::env::temp_dir().join(format!(
            "tachy-http-templates-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = Arc::new(Mutex::new(
            DaemonState::init(root.clone()).expect("should init"),
        ));

        let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let raw = "GET /api/templates HTTP/1.1\r\n\r\n";
        let response = handle_request(raw, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("code-reviewer"));
        assert!(response.contains("security-scanner"));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn schedule_endpoint_creates_task() {
        let root = std::env::temp_dir().join(format!(
            "tachy-http-schedule-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = Arc::new(Mutex::new(
            DaemonState::init(root.clone()).expect("should init"),
        ));

        let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let raw = "POST /api/tasks/schedule HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"template\":\"security-scanner\",\"name\":\"hourly scan\",\"interval_seconds\":3600}";
        let response = handle_request(raw, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("task-1"));

        // Verify task was created
        let raw2 = "GET /api/tasks HTTP/1.1\r\n\r\n";
        let response2 = handle_request(raw2, &state, &rate_limiter, "127.0.0.1");
        assert!(response2.contains("hourly scan"));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn parallel_run_endpoint_validates_and_accepts() {
        let root = std::env::temp_dir().join(format!(
            "tachy-http-parallel-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = Arc::new(Mutex::new(
            DaemonState::init(root.clone()).expect("should init"),
        ));

        let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));

        // Empty tasks should fail
        let raw_empty = "POST /api/parallel/run HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"tasks\":[]}";
        let response = handle_request(raw_empty, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("400"));
        assert!(response.contains("at least one task"));

        // Unknown template should fail
        let raw_bad = "POST /api/parallel/run HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"tasks\":[{\"template\":\"nonexistent\",\"prompt\":\"test\"}]}";
        let response = handle_request(raw_bad, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("400"));
        assert!(response.contains("unknown template"));

        // List runs should return empty initially
        let raw_list = "GET /api/parallel/runs HTTP/1.1\r\n\r\n";
        let response = handle_request(raw_list, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("200"));
        assert!(response.contains("\"count\": 0"));

        // Non-existent run should 404
        let raw_get = "GET /api/parallel/runs/run-nonexistent HTTP/1.1\r\n\r\n";
        let response = handle_request(raw_get, &state, &rate_limiter, "127.0.0.1");
        assert!(response.contains("404"));

        std::fs::remove_dir_all(root).ok();
    }
}
