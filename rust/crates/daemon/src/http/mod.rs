//! HTTP server module.
//!
//! Split into focused sub-modules; this file owns the server loop, request
//! dispatcher, shared types/utilities, and simple inline handlers.

mod agent;
mod auth;
mod governance;
mod intel;
mod runs;
mod webhooks;
mod workers;

// Pull all pub(super) handler functions into scope so handle_request() can
// call them directly, exactly as before the split.
use self::agent::{handle_complete_stream, handle_chat_stream, handle_run_agent, handle_complete, handle_prompt_oneshot, handle_get_agent, handle_cancel_agent, handle_delete_agent};
use self::auth::{handle_sso_login, handle_sso_callback, handle_sso_logout, handle_sso_sessions, handle_license_status, handle_billing_status, handle_sso_config, handle_license_activate, handle_usage, handle_oauth_login, handle_oauth_callback, handle_oauth_logout, handle_oauth_sessions};
use self::governance::{handle_audit_log, handle_audit_export, handle_metrics, handle_list_conversations, handle_create_conversation, handle_add_message, handle_list_teams, handle_create_team, handle_join_team, handle_get_team, handle_team_agents, handle_team_audit, handle_marketplace_list, handle_install, handle_list_cloud_jobs, handle_submit_cloud_job, handle_get_cloud_job, handle_list_pending_approvals, handle_approve_patch, handle_list_file_locks, handle_get_mission_feed, handle_get_policy, handle_set_policy, handle_schedule_task, handle_dashboard, handle_get_conversation, handle_delete_conversation};
use self::intel::{handle_search, handle_finetune_extract, handle_finetune_modelfile, handle_diagnostics, handle_index_build, handle_index_status, handle_dependency_graph, handle_monorepo};
use self::runs::{handle_list_parallel_runs, handle_run_history, handle_parallel_run, handle_list_swarm_runs, handle_start_swarm_run, handle_get_swarm_run, handle_event_stream, handle_list_run_templates, handle_save_run_template, handle_get_run_cost, handle_get_run_conflicts, handle_get_parallel_run, handle_replay_run, handle_cancel_parallel_run, handle_get_run_template, handle_delete_run_template, handle_run_template};
use self::webhooks::{handle_list_webhooks, handle_register_webhook, handle_verify_webhook_signature};
use self::workers::{handle_telemetry_flush, handle_telemetry_status, handle_list_workers, handle_register_worker, handle_worker_heartbeat, handle_deregister_worker};

use std::sync::{Arc, Mutex};

use audit::RateLimiter;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::state::DaemonState;
use crate::web;

// ---------------------------------------------------------------------------
// Shared response / error types
// ---------------------------------------------------------------------------

pub enum Response {
    Full {
        status: u16,
        content_type: String,
        body: String,
    },
    Stream {
        status: u16,
        content_type: String,
        rx: tokio::sync::mpsc::Receiver<String>,
    },
}

impl Response {
    fn json(status: u16, body: impl Serialize) -> Self {
        Self::Full {
            status,
            content_type: "application/json".to_string(),
            body: serde_json::to_string(&body).unwrap_or_default(),
        }
    }

    fn html(status: u16, body: &str) -> Self {
        Self::Full {
            status,
            content_type: "text/html".to_string(),
            body: body.to_string(),
        }
    }

    fn sse() -> (Self, tokio::sync::mpsc::Sender<String>) {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        (
            Self::Stream {
                status: 200,
                content_type: "text/event-stream".to_string(),
                rx,
            },
            tx,
        )
    }

    #[cfg(test)]
    fn contains(&self, s: &str) -> bool {
        match self {
            Self::Full { body, .. } => body.contains(s),
            Self::Stream { .. } => false,
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    models: usize,
    agents: usize,
    tasks: usize,
    workspace: String,
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

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------

pub async fn serve(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(listen_addr).await?;
    let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(120, 60)));
    // TACHY_ALLOWED_ORIGINS: comma-separated list of allowed CORS origins.
    // Defaults to "*" for local dev. Set to "https://app.example.com" in prod.
    let allowed_origin = Arc::new(
        std::env::var("TACHY_ALLOWED_ORIGINS").unwrap_or_else(|_| "*".to_string())
    );

    eprintln!("Tachy daemon listening on {listen_addr}");

    loop {
        let (mut stream, addr) = listener.accept().await?;
        let state = Arc::clone(&state);
        let rate_limiter = Arc::clone(&rate_limiter);
        let client_ip = addr.ip().to_string();
        let origin = Arc::clone(&allowed_origin);

        tokio::spawn(async move {
            let mut buf = vec![0u8; 131_072];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };

            let request_raw = String::from_utf8_lossy(&buf[..n]);
            let response = handle_request(&request_raw, &state, &rate_limiter, &client_ip).await;

            match response {
                Response::Full { status, content_type, body } => {
                    let header = format!(
                        "HTTP/1.1 {status} OK\r\n\
                         Content-Type: {content_type}\r\n\
                         Content-Length: {}\r\n\
                         Access-Control-Allow-Origin: {origin}\r\n\
                         Access-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS\r\n\
                         Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
                         Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(body.as_bytes()).await;
                }
                Response::Stream { status, content_type, mut rx } => {
                    let header = format!(
                        "HTTP/1.1 {status} OK\r\n\
                         Content-Type: {content_type}\r\n\
                         Cache-Control: no-cache\r\n\
                         Connection: keep-alive\r\n\
                         Access-Control-Allow-Origin: {origin}\r\n\
                         Access-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS\r\n\
                         Access-Control-Allow-Headers: Content-Type, Authorization\r\n\r\n"
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    while let Some(chunk) = rx.recv().await {
                        if stream.write_all(chunk.as_bytes()).await.is_err() {
                            break;
                        }
                        let _ = stream.flush().await;
                    }
                }
            }
            let _ = stream.flush().await;
        });
    }
}

async fn handle_request(
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
    rate_limiter: &Arc<Mutex<RateLimiter>>,
    client_ip: &str,
) -> Response {
    let (method, path_raw, body) = parse_http_request(raw);
    let path_full = path_raw.split('?').next().unwrap_or("/").trim_end_matches('/');
    let path = if path_full.is_empty() { "/" } else { path_full };
    let query_str = path_raw.find('?').map_or("", |i| &path_raw[i + 1..]).to_string();

    if method == "OPTIONS" {
        return Response::Full {
            status: 204,
            content_type: "text/plain".to_string(),
            body: String::new(),
        };
    }

    if !path.starts_with("/api/inference/stats") && !matches!(path, "" | "/" | "/index.html" | "/health") {
        let mut limiter = rate_limiter.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let rate_key = if path == "/api/complete" { format!("complete:{client_ip}") } else { client_ip.to_string() };
        if !limiter.check(&rate_key) {
            return Response::json(429, &ErrorResponse { error: "rate limit exceeded".to_string() });
        }
    }

    if !matches!(path, "" | "/" | "/index.html" | "/health") {
        let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(required_key) = &s.api_key {
            let provided = extract_auth_header(raw);
            match provided {
                Some(key) if key == *required_key => {}
                _ => return Response::json(401, &ErrorResponse { error: "unauthorized".to_string() }),
            }
        }
    }

    match (method.as_str(), path) {
        ("GET", "/" | "/index.html") => Response::html(200, web::INDEX_HTML),
        ("GET", "/health") => Response::json(200, handle_health(state)),
        ("GET", "/api/models") => Response::json(200, handle_list_models(state)),
        ("GET", "/api/inference/stats") => handle_inference_stats(state),
        ("POST", "/api/models/pull") => handle_pull_model(&body, state),
        ("POST", "/api/complete/stream") => handle_complete_stream(&body, state).await,
        ("POST", "/api/chat/stream") => handle_chat_stream(&body, state).await,
        ("GET", "/api/templates") => Response::json(200, handle_list_templates(state)),
        ("GET", "/api/agents") => Response::json(200, handle_list_agents(state)),
        ("GET", "/api/tasks") => Response::json(200, handle_list_tasks(state)),
        ("GET", "/api/audit") => gate_action(state, raw, audit::Action::ViewAudit, |_| handle_audit_log(state)),
        ("GET", "/api/audit/export") => gate_action(state, raw, audit::Action::ViewAudit, |_| handle_audit_export(state)),
        ("GET", "/api/metrics") => gate_action(state, raw, audit::Action::ViewAudit, |_| handle_metrics(state)),
        ("GET", "/api/conversations") => handle_list_conversations(state),
        ("POST", "/api/conversations") => handle_create_conversation(&body, state),
        ("POST", "/api/conversations/message") => handle_add_message(&body, state),
        ("GET", "/api/auth/sso/login") => handle_sso_login(state),
        ("POST", "/api/auth/sso/callback") => handle_sso_callback(&body, state),
        ("POST", "/api/auth/sso/logout") => handle_sso_logout(&body, state),
        ("GET", "/api/auth/sso/sessions") => handle_sso_sessions(state),
        ("GET", "/api/license/status") => handle_license_status(state),
        ("GET", "/api/billing/status") => handle_billing_status(state),
        ("GET", "/api/teams") => handle_list_teams(state),
        ("POST", "/api/teams") => handle_create_team(&body, state),
        ("POST", "/api/teams/join") => handle_join_team(&body, state),
        _ if method == "GET" && path.starts_with("/api/teams/") => {
            let rest = path.strip_prefix("/api/teams/").unwrap_or(path);
            let (team_id, suffix) = rest.split_once('/').unwrap_or((rest, ""));
            match suffix {
                "" => handle_get_team(team_id, state),
                "agents" => handle_team_agents(team_id, state),
                "audit" => handle_team_audit(team_id, state),
                _ => Response::json(404, &ErrorResponse { error: "not found".to_string() }),
            }
        }
        ("GET", "/api/marketplace") => handle_marketplace_list(path, state),
        ("POST", "/api/marketplace/install") => handle_install(&body, state),
        ("GET", "/api/parallel/runs") => handle_list_parallel_runs(state),
        ("GET", "/api/runs/history") => handle_run_history(state),
        ("POST", "/api/parallel/runs") => handle_parallel_run(&body, state),
        ("GET", "/api/cloud/jobs") => handle_list_cloud_jobs(state),
        ("POST", "/api/cloud/jobs") => handle_submit_cloud_job(&body, state),
        _ if method == "GET" && path.starts_with("/api/cloud/jobs/") => {
            let job_id = path.strip_prefix("/api/cloud/jobs/").unwrap_or(path);
            handle_get_cloud_job(job_id, state)
        }
        ("GET", "/api/swarm/runs") => handle_list_swarm_runs(state),
        ("POST", "/api/swarm/runs") => handle_start_swarm_run(&body, state),
        _ if method == "GET" && path.starts_with("/api/swarm/runs/") => {
            let run_id = path.strip_prefix("/api/swarm/runs/").unwrap_or(path);
            handle_get_swarm_run(run_id, state)
        }
        ("POST", "/api/agents/run") => gate_action(state, raw, audit::Action::RunAgent, |_| handle_run_agent(&body, state)),
        ("GET", "/api/pending-approvals") => handle_list_pending_approvals(state),
        ("POST", "/api/approve") => gate_action(state, raw, audit::Action::ManageGovernance, |_| handle_approve_patch(&body, state)),
        ("GET", "/api/file-locks") => handle_list_file_locks(state),
        ("GET", "/api/mission/feed") => handle_get_mission_feed(state),
        ("POST", "/api/auth/sso/config") => gate_action(state, raw, audit::Action::ManageEnterpriseSSO, |_| handle_sso_config(&body, state)),
        ("GET", "/api/search") => handle_search(path_full, state),
        ("GET", "/api/policy") => handle_get_policy(state),
        ("POST", "/api/policy") => handle_set_policy(&body, state),

        // --- routes present in OpenAPI spec ---
        ("POST", "/api/complete") => handle_complete(&body, state).await,
        ("POST", "/api/parallel/run") => handle_parallel_run(&body, state), // spec uses singular
        ("GET", "/api/webhooks") => handle_list_webhooks(state),
        ("POST", "/api/webhooks") => handle_register_webhook(&body, state),
        ("POST", "/api/webhooks/verify") => handle_verify_webhook_signature(&body, raw, state),
        ("POST", "/api/tasks/schedule") => handle_schedule_task(&body, state),
        ("POST", "/api/license/activate") => handle_license_activate(&body, state),

        ("POST", "/api/prompt") => handle_prompt_oneshot(&body, state),
        ("GET", "/api/usage") => handle_usage(state),

        // OAuth2 endpoints
        _ if method == "GET" && path.starts_with("/api/auth/oauth/") && path.ends_with("/login") => {
            let provider = path.strip_prefix("/api/auth/oauth/").unwrap_or("").trim_end_matches("/login");
            handle_oauth_login(provider, state)
        }
        _ if method == "GET" && path.starts_with("/api/auth/oauth/") && path.contains("/callback") => {
            let provider = path.strip_prefix("/api/auth/oauth/")
                .unwrap_or("")
                .split('/').next().unwrap_or("");
            handle_oauth_callback(provider, &query_str, state)
        }
        ("POST", "/api/auth/oauth/logout") => handle_oauth_logout(&body, state),
        ("GET", "/api/auth/oauth/sessions") => handle_oauth_sessions(state),

        // Telemetry
        ("POST", "/api/telemetry/flush") => handle_telemetry_flush(state),
        ("GET", "/api/telemetry/status") => handle_telemetry_status(state),

        // Distributed swarm worker registry
        ("GET", "/api/workers") => handle_list_workers(state),
        ("POST", "/api/workers/register") => handle_register_worker(&body, state),
        ("POST", "/api/workers/heartbeat") => handle_worker_heartbeat(&body, state),
        ("DELETE", "/api/workers/deregister") => handle_deregister_worker(&body, state),
        ("POST", "/api/finetune/extract") => handle_finetune_extract(&body, state),
        ("POST", "/api/finetune/modelfile") => handle_finetune_modelfile(&body, state),
        ("GET", "/api/diagnostics") => handle_diagnostics(&query_str, state),
        ("POST", "/api/index") => handle_index_build(&body, state),
        ("GET", "/api/index") => handle_index_status(state),
        ("GET", "/api/graph") => handle_dependency_graph(&query_str, state),
        ("GET", "/api/monorepo") => handle_monorepo(state),
        ("GET", "/api/dashboard") => handle_dashboard(state),
        // Wave 2: live events, cost tracking, run replay, DAG templates
        ("GET", "/api/events") => handle_event_stream(state).await,
        ("GET", "/api/run-templates") => handle_list_run_templates(state),
        ("POST", "/api/run-templates") => handle_save_run_template(&body, state),
        _ => route_dynamic(method.as_str(), path, &body, state).await,
    }
}

/// Dynamic route dispatch for parameterised paths (e.g. `/api/agents/{id}`).
///
/// Uses `strip_prefix` / `strip_suffix` instead of manual index arithmetic,
/// eliminating off-by-one risk and making routing intent self-documenting.
async fn route_dynamic(
    method: &str,
    path: &str,
    body: &str,
    state: &Arc<Mutex<DaemonState>>,
) -> Response {
    // ── /api/parallel/runs/{id}/* ────────────────────────────────────────────
    if let Some(rest) = path.strip_prefix("/api/parallel/runs/") {
        if method == "GET" {
            if let Some(run_id) = rest.strip_suffix("/cost") {
                return handle_get_run_cost(run_id, state);
            }
            if let Some(run_id) = rest.strip_suffix("/conflicts") {
                return handle_get_run_conflicts(run_id, state);
            }
            return handle_get_parallel_run(rest, state);
        }
        if method == "POST" {
            if let Some(run_id) = rest.strip_suffix("/replay") {
                return handle_replay_run(run_id, state);
            }
            if let Some(run_id) = rest.strip_suffix("/cancel") {
                return handle_cancel_parallel_run(run_id, body, state);
            }
        }
    }

    // ── /api/run-templates/{name}/* ─────────────────────────────────────────
    if let Some(rest) = path.strip_prefix("/api/run-templates/") {
        match method {
            "GET" if !rest.ends_with("/run") => return handle_get_run_template(rest, state),
            "DELETE"                          => return handle_delete_run_template(rest, state),
            "POST" => {
                if let Some(name) = rest.strip_suffix("/run") {
                    return handle_run_template(name, body, state);
                }
            }
            _ => {}
        }
    }

    // ── /api/agents/{id}/* ──────────────────────────────────────────────────
    if let Some(rest) = path.strip_prefix("/api/agents/") {
        match method {
            "GET" => return handle_get_agent(rest, state),
            "DELETE" => {
                if let Some(id) = rest.strip_suffix("/cancel") {
                    return handle_cancel_agent(id, state);
                }
                return handle_delete_agent(rest, state);
            }
            "POST" => {
                if let Some(id) = rest.strip_suffix("/cancel") {
                    return handle_cancel_agent(id, state);
                }
            }
            _ => {}
        }
    }

    // ── /api/conversations/{id} ─────────────────────────────────────────────
    if let Some(id) = path.strip_prefix("/api/conversations/") {
        match method {
            "GET"    => return handle_get_conversation(id, state),
            "DELETE" => return handle_delete_conversation(id, state),
            _ => {}
        }
    }

    Response::json(404, &ErrorResponse { error: format!("not found: {method} {path}") })
}

// ---------------------------------------------------------------------------
// Simple inline handlers (not warranting a sub-module)
// ---------------------------------------------------------------------------

fn handle_inference_stats(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, &s.inference_stats)
}

fn handle_pull_model(body: &str, _state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct PullRequest { model: String }
    let req: PullRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Response::json(400, &ErrorResponse { error: "invalid request".to_string() }),
    };
    let model = req.model.clone();
    tokio::spawn(async move { let _ = backend::pull_model(&model); });
    Response::json(202, serde_json::json!({ "message": format!("Pulling {} in background", req.model) }))
}

fn handle_health(state: &Arc<Mutex<DaemonState>>) -> HealthResponse {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    HealthResponse {
        status: "ok",
        models: s.registry.list_models().len(),
        agents: s.agents.len(),
        tasks: s.scheduler.list_tasks().len(),
        workspace: s.workspace_root.to_string_lossy().to_string(),
    }
}

fn handle_list_models(state: &Arc<Mutex<DaemonState>>) -> Vec<ModelInfo> {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.registry.list_models().iter().map(|m| ModelInfo {
        name: m.name.clone(),
        backend: format!("{:?}", m.backend),
        supports_tool_use: m.supports_tool_use,
        context_window: m.context_window,
    }).collect()
}

fn handle_list_templates(state: &Arc<Mutex<DaemonState>>) -> Vec<TemplateInfo> {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.config.agent_templates.iter().map(|t| TemplateInfo {
        name: t.name.clone(),
        description: t.description.clone(),
        model: t.model.clone(),
        tools: t.allowed_tools.clone(),
        max_iterations: t.max_iterations,
        requires_approval: t.requires_approval,
    }).collect()
}

fn handle_list_agents(state: &Arc<Mutex<DaemonState>>) -> Vec<AgentInfo> {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.agents.values().map(|a| AgentInfo {
        id: a.id.clone(),
        template: a.config.template.name.clone(),
        status: format!("{:?}", a.status),
        iterations: a.iterations_completed,
        tool_invocations: a.tool_invocations,
        summary: a.result_summary.clone(),
    }).collect()
}

fn handle_list_tasks(state: &Arc<Mutex<DaemonState>>) -> Vec<TaskInfo> {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.scheduler.list_tasks().iter().map(|t| TaskInfo {
        id: t.id.clone(),
        name: t.name.clone(),
        schedule: format!("{:?}", t.schedule),
        status: format!("{:?}", t.status),
        run_count: t.run_count,
        enabled: t.enabled,
    }).collect()
}

// ---------------------------------------------------------------------------
// Shared utilities (used by sub-modules via `use super::...`)
// ---------------------------------------------------------------------------

 fn chrono_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

 fn chrono_now_str() -> String {
    format!("{}s", chrono_now_secs())
}

 fn urlencoding_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&s[i+1..i+3], 16) {
                out.push(hex as char);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

 fn truncate_completion(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * 4;
    if text.len() <= max_chars { text.to_string() } else { text[..max_chars].to_string() }
}

 fn csv_response(body: &str, _filename: &str) -> Response {
    Response::Full { status: 200, content_type: "text/csv".to_string(), body: body.to_string() }
}

 fn gate_action<F>(state: &Arc<Mutex<DaemonState>>, raw: &str, action: audit::Action, f: F) -> Response
where F: FnOnce(&audit::User) -> Response {
    let user = match extract_user(state, raw) {
        Some(u) => u,
        None => return Response::json(401, &ErrorResponse { error: "unauthorized".to_string() }),
    };
    match audit::check_permission(user.role, action) {
        audit::AccessResult::Allowed => f(&user),
        audit::AccessResult::Denied { reason } => Response::json(403, &ErrorResponse { error: reason }),
    }
}

 fn extract_user(state: &Arc<Mutex<DaemonState>>, raw: &str) -> Option<audit::User> {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let auth = extract_auth_header(raw)?;
    if let Some(user) = s.user_store.authenticate(&audit::hash_api_key(&auth)) {
        return Some(user.clone());
    }
    if let Some(session) = s.sso_manager.validate_session(&auth) {
        return s.user_store.users.get(&session.user_id).cloned();
    }
    None
}

fn parse_http_request(raw: &str) -> (String, String, String) {
    let mut lines = raw.lines();
    let first_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = (*parts.first().unwrap_or(&"GET")).to_string();
    let path = (*parts.get(1).unwrap_or(&"/")).to_string();
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
            let val = line[14..].trim();
            return Some(val.strip_prefix("Bearer ").or(val.strip_prefix("bearer ")).unwrap_or(val).trim().to_string());
        }
        if lower.starts_with("x-sso-token:") {
            return Some(line[12..].trim().to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_endpoint_works() {
        let root = std::env::temp_dir().join(format!("tachy-test-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request("GET /health HTTP/1.1\r\n\r\n", &state, &limiter, "127.0.0.1").await;
        if let Response::Full { body, .. } = res {
            assert!(body.contains("\"status\":\"ok\"") || body.contains("\"status\": \"ok\""));
        } else {
            panic!("not full");
        }
    }

    #[test]
    fn decode_plain_string() {
        assert_eq!(urlencoding_decode("hello"), "hello");
    }

    #[test]
    fn decode_plus_as_space() {
        assert_eq!(urlencoding_decode("hello+world"), "hello world");
    }

    #[test]
    fn decode_percent_encoded() {
        assert_eq!(urlencoding_decode("hello%20world"), "hello world");
        assert_eq!(urlencoding_decode("a%2Fb"), "a/b");
    }

    #[test]
    fn decode_mixed_encoding() {
        assert_eq!(urlencoding_decode("fn+main%28%29"), "fn main()");
    }

    #[test]
    fn decode_incomplete_percent_sequence_kept_as_is() {
        let result = urlencoding_decode("abc%");
        assert!(result.starts_with("abc"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn search_missing_query_returns_400() {
        let root = std::env::temp_dir().join(format!("tachy-search-test-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "GET /api/search HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 400);
            assert!(body.contains("missing query"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_policy_returns_200_with_defaults() {
        let root = std::env::temp_dir().join(format!("tachy-policy-test-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "GET /api/policy HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            assert!(serde_json::from_str::<serde_json::Value>(&body).is_ok());
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_policy_invalid_json_returns_400() {
        let root = std::env::temp_dir().join(format!("tachy-set-policy-test-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let raw = "POST /api/policy HTTP/1.1\r\nContent-Length: 11\r\n\r\nnot-valid{{";
        let res = handle_request(raw, &state, &limiter, "127.0.0.1").await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 400);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_conversation_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-get-conv-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "GET /api/conversations/conv-999 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_conversation_existing_returns_200() {
        let root = std::env::temp_dir().join(format!("tachy-get-conv2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_conversation("test conv");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "GET /api/conversations/conv-1 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            assert!(body.contains("conv-1") || body.contains("test conv"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_conversation_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-del-conv-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "DELETE /api/conversations/conv-999 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_conversation_existing_returns_204() {
        let root = std::env::temp_dir().join(format!("tachy-del-conv2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_conversation("to delete");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "DELETE /api/conversations/conv-1 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 204);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_agent_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-del-ag-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "DELETE /api/agents/agent-999 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_agent_existing_returns_204() {
        let root = std::env::temp_dir().join(format!("tachy-del-ag2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_agent("code-reviewer", "do stuff").expect("create");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "DELETE /api/agents/agent-1 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 204);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_agent_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-cancel-ag-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "POST /api/agents/agent-999/cancel HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_agent_existing_returns_200() {
        let root = std::env::temp_dir().join(format!("tachy-cancel-ag2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_agent("code-reviewer", "cancellable task").expect("create");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "POST /api/agents/agent-1/cancel HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            assert!(body.contains("Failed") || body.contains("agent-1"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn index_status_returns_200() {
        let root = std::env::temp_dir().join(format!("tachy-index-status-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "GET /api/index HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            assert!(body.contains("status"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn event_stream_returns_sse_content_type() {
        let root = std::env::temp_dir().join(format!("tachy-events-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request(
            "GET /api/events HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        match res {
            Response::Stream { status, content_type, .. } => {
                assert_eq!(status, 200);
                assert_eq!(content_type, "text/event-stream");
            }
            _ => panic!("expected Stream response for /api/events"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publish_event_reaches_subscriber() {
        let root = std::env::temp_dir().join(format!("tachy-pub-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let mut rx = state.event_bus.subscribe();
        state.publish_event("test_event", serde_json::json!({"x": 1}));
        let msg = rx.try_recv().expect("should have buffered message");
        assert!(msg.contains("test_event"));
        assert!(msg.contains("\"x\":1") || msg.contains("\"x\": 1"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_template_save_and_list() {
        let root = std::env::temp_dir().join(format!("tachy-tpl-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));

        let body = r#"{"name":"refactor","description":"Standard refactor","tasks":[{"template":"chat","prompt":"refactor src/lib.rs"}],"max_concurrency":2}"#;
        let res = handle_request(
            &format!("POST /api/run-templates HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 201);
            assert!(body.contains("refactor"));
        } else { panic!("expected Full response"); }

        let res = handle_request(
            "GET /api/run-templates HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            assert!(body.contains("refactor"));
            assert!(body.contains("\"count\":1"));
        } else { panic!("expected Full response"); }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_template_get_and_delete() {
        let root = std::env::temp_dir().join(format!("tachy-tpl2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));

        let body = r#"{"name":"myflow","tasks":[{"template":"chat","prompt":"do x"}]}"#;
        handle_request(
            &format!("POST /api/run-templates HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
            &state, &limiter, "127.0.0.1",
        ).await;

        let res = handle_request(
            "GET /api/run-templates/myflow HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            assert!(body.contains("myflow"));
        } else { panic!("expected Full"); }

        let res = handle_request(
            "DELETE /api/run-templates/myflow HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res { assert_eq!(status, 200); }
        else { panic!("expected Full"); }

        let res = handle_request(
            "GET /api/run-templates/myflow HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res { assert_eq!(status, 404); }
        else { panic!("expected Full"); }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_template_missing_name_returns_400() {
        let root = std::env::temp_dir().join(format!("tachy-tpl3-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let body = r#"{"name":"","tasks":[{"template":"chat","prompt":"x"}]}"#;
        let res = handle_request(
            &format!("POST /api/run-templates HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res { assert_eq!(status, 400); }
        else { panic!("expected Full"); }
    }
}
