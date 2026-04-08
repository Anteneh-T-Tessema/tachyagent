use std::sync::{Arc, Mutex};

use audit::{sanitize_prompt, RateLimiter};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::engine::AgentEngine;
use crate::parallel::{self, AgentTask, ParallelRun, RunStatus, TaskStatus};
use crate::state::DaemonState;
use crate::web;

// ---------------------------------------------------------------------------
// Request/response types
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

fn chrono_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn chrono_now_str() -> String {
    format!("{}s", chrono_now_secs())
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct RunAgentRequest {
    template: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ScheduleAgentRequest {
    template: String,
    name: String,
    interval_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct ParallelRunRequest {
    tasks: Vec<ParallelTaskInput>,
    #[serde(default = "default_concurrency")]
    max_concurrency: usize,
}

fn default_concurrency() -> usize { 4 }

#[derive(Debug, Deserialize)]
struct ParallelTaskInput {
    template: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    deps: Vec<String>,
    #[serde(default = "default_priority")]
    priority: u8,
}

fn default_priority() -> u8 { 5 }

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct CancelRunRequest {
    task_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct CreateTeamRequest {
    name: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct InviteRequest {
    email: String,
    role: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct JoinTeamRequest {
    token: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct UpdateMemberRequest {
    role: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct PublishRequest {
    template: platform::AgentTemplate,
    description: String,
    version: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct InstallRequest {
    listing_id: String,
    #[serde(default)]
    version: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct RateRequest {
    rating: u8,
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

    eprintln!("Tachy daemon listening on {listen_addr}");

    loop {
        let (mut stream, addr) = listener.accept().await?;
        let state = Arc::clone(&state);
        let rate_limiter = Arc::clone(&rate_limiter);
        let client_ip = addr.ip().to_string();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 131072];
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
                         Access-Control-Allow-Origin: *\r\n\
                         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
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
                         Access-Control-Allow-Origin: *\r\n\
                         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
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
    let query_str = path_raw.find('?').map(|i| &path_raw[i + 1..]).unwrap_or("").to_string();

    if method == "OPTIONS" {
        return Response::Full {
            status: 204,
            content_type: "text/plain".to_string(),
            body: String::new(),
        };
    }

    if !path.starts_with("/api/inference/stats") && !matches!(path, "" | "/" | "/index.html" | "/health") {
        let mut limiter = rate_limiter.lock().unwrap_or_else(|e| e.into_inner());
        let rate_key = if path == "/api/complete" { format!("complete:{client_ip}") } else { client_ip.to_string() };
        if !limiter.check(&rate_key) {
            return Response::json(429, &ErrorResponse { error: "rate limit exceeded".to_string() });
        }
    }

    if !matches!(path, "" | "/" | "/index.html" | "/health") {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
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
        ("GET", "/health") => Response::json(200, &handle_health(state)),
        ("GET", "/api/models") => Response::json(200, &handle_list_models(state)),
        ("GET", "/api/inference/stats") => handle_inference_stats(state),
        ("POST", "/api/models/pull") => handle_pull_model(&body, state),
        ("POST", "/api/complete/stream") => handle_complete_stream(&body, state).await,
        ("POST", "/api/chat/stream") => handle_chat_stream(&body, state).await,
        ("GET", "/api/templates") => Response::json(200, &handle_list_templates(state)),
        ("GET", "/api/agents") => Response::json(200, &handle_list_agents(state)),
        ("GET", "/api/tasks") => Response::json(200, &handle_list_tasks(state)),
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
            let rest = path.trim_start_matches("/api/teams/");
            let (team_id, suffix) = rest.split_once('/').unwrap_or((rest, ""));
            match suffix {
                "" => handle_get_team(team_id, state),
                "agents" => handle_team_agents(team_id, state),
                "audit" => handle_team_audit(team_id, state),
                _ => Response::json(404, &ErrorResponse { error: "not found".to_string() }),
            }
        }
        ("GET", "/api/marketplace") => handle_marketplace_list(&path, state),
        ("POST", "/api/marketplace/install") => handle_install(&body, state),
        ("GET", "/api/parallel/runs") => handle_list_parallel_runs(state),
        ("GET", "/api/runs/history") => handle_run_history(state),
        ("POST", "/api/parallel/runs") => handle_parallel_run(&body, state),
        ("GET", "/api/cloud/jobs") => handle_list_cloud_jobs(state),
        ("POST", "/api/cloud/jobs") => handle_submit_cloud_job(&body, state),
        _ if method == "GET" && path.starts_with("/api/cloud/jobs/") => {
            let job_id = path.trim_start_matches("/api/cloud/jobs/");
            handle_get_cloud_job(job_id, state)
        }
        ("GET", "/api/swarm/runs") => handle_list_swarm_runs(state),
        ("POST", "/api/swarm/runs") => handle_start_swarm_run(&body, state),
        _ if method == "GET" && path.starts_with("/api/swarm/runs/") => {
            let run_id = path.trim_start_matches("/api/swarm/runs/");
            handle_get_swarm_run(run_id, state)
        }
        ("POST", "/api/agents/run") => gate_action(state, raw, audit::Action::RunAgent, |_| handle_run_agent(&body, state)),
        ("GET", "/api/pending-approvals") => handle_list_pending_approvals(state),
        ("POST", "/api/approve") => gate_action(state, raw, audit::Action::ManageGovernance, |_| handle_approve_patch(&body, state)),
        ("GET", "/api/file-locks") => handle_list_file_locks(state),
        ("GET", "/api/mission/feed") => handle_get_mission_feed(state),
        ("POST", "/api/auth/sso/config") => gate_action(state, raw, audit::Action::ManageEnterpriseSSO, |_| handle_sso_config(&body, state)),
        ("GET", "/api/search") => handle_search(&path_full, state),
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
            let provider = path.trim_start_matches("/api/auth/oauth/").trim_end_matches("/login");
            handle_oauth_login(provider, state)
        }
        _ if method == "GET" && path.starts_with("/api/auth/oauth/") && path.contains("/callback") => {
            let provider = path.trim_start_matches("/api/auth/oauth/")
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
        _ => {
            if method == "GET" && path.starts_with("/api/agents/") {
                return handle_get_agent(&path["/api/agents/".len()..], state);
            }
            if method == "DELETE" && path.starts_with("/api/agents/") && path.ends_with("/cancel") {
                let id = &path["/api/agents/".len()..path.len() - "/cancel".len()];
                return handle_cancel_agent(id, state);
            }
            if method == "POST" && path.starts_with("/api/agents/") && path.ends_with("/cancel") {
                let id = &path["/api/agents/".len()..path.len() - "/cancel".len()];
                return handle_cancel_agent(id, state);
            }
            if method == "DELETE" && path.starts_with("/api/agents/") {
                return handle_delete_agent(&path["/api/agents/".len()..], state);
            }
            if method == "GET" && path.starts_with("/api/conversations/") {
                return handle_get_conversation(&path["/api/conversations/".len()..], state);
            }
            if method == "DELETE" && path.starts_with("/api/conversations/") {
                return handle_delete_conversation(&path["/api/conversations/".len()..], state);
            }
            if method == "GET" && path.starts_with("/api/parallel/runs/") && path.ends_with("/conflicts") {
                let run_id = &path["/api/parallel/runs/".len()..path.len() - "/conflicts".len()];
                return handle_get_run_conflicts(run_id, state);
            }
            if method == "GET" && path.starts_with("/api/parallel/runs/") {
                return handle_get_parallel_run(&path["/api/parallel/runs/".len()..], state);
            }
            if method == "POST" && path.starts_with("/api/parallel/runs/") && path.ends_with("/cancel") {
                let run_id = &path["/api/parallel/runs/".len()..path.len() - "/cancel".len()];
                return handle_cancel_parallel_run(run_id, &body, state);
            }
            Response::json(404, &ErrorResponse { error: format!("not found: {method} {path}") })
        }
    }
}

fn handle_inference_stats(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &s.inference_stats)
}

/// GET /api/search?q=<query>&limit=<n>
/// Semantic search over the codebase index.
fn handle_search(path_full: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    // Parse query string from raw path (e.g. /api/search?q=foo&limit=10)
    let (query, limit) = {
        let qs = path_full.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut q_val = String::new();
        let mut lim_val: usize = 10;
        for pair in qs.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "q" | "query" => q_val = urlencoding_decode(v),
                    "limit" | "n" => lim_val = v.parse().unwrap_or(10).min(50),
                    _ => {}
                }
            }
        }
        (q_val, lim_val)
    };

    if query.is_empty() {
        return Response::json(400, &ErrorResponse { error: "missing query param: ?q=".to_string() });
    }

    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let ws = &s.workspace_root;

    let index = match intelligence::CodebaseIndexer::load_index(ws) {
        Ok(idx) => idx,
        Err(_) => {
            // Index not built yet — build it now (blocking is fine, this is a sync handler)
            let cfg = intelligence::IndexerConfig::default();
            match intelligence::CodebaseIndexer::build_index(ws, &cfg) {
                Ok(idx) => {
                    let _ = intelligence::CodebaseIndexer::save_index(ws, &idx);
                    idx
                }
                Err(e) => {
                    return Response::json(503, &ErrorResponse {
                        error: format!("codebase not indexed: {e}"),
                    });
                }
            }
        }
    };

    let results: Vec<serde_json::Value> = intelligence::CodebaseIndexer::search(&index, &query, limit)
        .into_iter()
        .map(|entry| serde_json::json!({
            "path": entry.path,
            "language": entry.language,
            "lines": entry.lines,
            "exports": entry.exports,
            "summary": entry.summary,
        }))
        .collect();

    Response::json(200, &serde_json::json!({ "query": query, "results": results }))
}

/// Simple percent-decode for URL query values.
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

/// GET /api/policy — return the current tachy-policy.yaml as JSON.
fn handle_get_policy(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let policy_path = s.workspace_root.join("tachy-policy.yaml");
    let pf = audit::PolicyFile::load(&policy_path)
        .unwrap_or_else(|_| audit::PolicyFile::enterprise_default());
    Response::json(200, &pf)
}

/// POST /api/policy — save a new tachy-policy.yaml from JSON body.
fn handle_set_policy(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let pf: audit::PolicyFile = match serde_json::from_str(body) {
        Ok(p) => p,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid policy JSON: {e}") }),
    };
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let policy_path = s.workspace_root.join("tachy-policy.yaml");
    match pf.save(&policy_path) {
        Ok(()) => Response::json(200, &serde_json::json!({ "saved": policy_path.display().to_string() })),
        Err(e) => Response::json(500, &ErrorResponse { error: format!("save failed: {e}") }),
    }
}

fn handle_get_mission_feed(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let feed = s.mission_feed.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &*feed)
}

fn handle_sso_config(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let config: audit::SsoConfig = match serde_json::from_str(body) {
        Ok(c) => c,
        Err(_) => return Response::json(400, &ErrorResponse { error: "invalid config".to_string() }),
    };
    s.sso_manager = audit::SsoManager::new(config);
    Response::json(200, &serde_json::json!({ "status": "updated" }))
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
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let auth = extract_auth_header(raw)?;
    
    // 1. Check if it's an API key
    if let Some(user) = s.user_store.authenticate(&audit::hash_api_key(&auth)) {
        return Some(user.clone());
    }

    // 2. Check if it's an SSO session token
    if let Some(session) = s.sso_manager.validate_session(&auth) {
        return s.user_store.users.get(&session.user_id).cloned();
    }

    None
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
    Response::json(202, &serde_json::json!({ "message": format!("Pulling {} in background", req.model) }))
}

fn handle_health(state: &Arc<Mutex<DaemonState>>) -> HealthResponse {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    HealthResponse { 
        status: "ok", 
        models: s.registry.list_models().len(), 
        agents: s.agents.len(), 
        tasks: s.scheduler.list_tasks().len(),
        workspace: s.workspace_root.to_string_lossy().to_string(),
    }
}

fn handle_list_models(state: &Arc<Mutex<DaemonState>>) -> Vec<ModelInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.registry.list_models().iter().map(|m| ModelInfo { name: m.name.clone(), backend: format!("{:?}", m.backend), supports_tool_use: m.supports_tool_use, context_window: m.context_window }).collect()
}

fn handle_list_templates(state: &Arc<Mutex<DaemonState>>) -> Vec<TemplateInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.config.agent_templates.iter().map(|t| TemplateInfo { name: t.name.clone(), description: t.description.clone(), model: t.model.clone(), tools: t.allowed_tools.clone(), max_iterations: t.max_iterations, requires_approval: t.requires_approval }).collect()
}

fn handle_get_agent(agent_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.agents.get(agent_id) {
        Some(a) => Response::json(200, &AgentInfo { id: a.id.clone(), template: a.config.template.name.clone(), status: format!("{:?}", a.status).to_lowercase(), iterations: a.iterations_completed, tool_invocations: a.tool_invocations, summary: a.result_summary.clone() }),
        None => Response::json(404, &ErrorResponse { error: format!("agent not found: {agent_id}") }),
    }
}

fn handle_list_agents(state: &Arc<Mutex<DaemonState>>) -> Vec<AgentInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.agents.values().map(|a| AgentInfo { id: a.id.clone(), template: a.config.template.name.clone(), status: format!("{:?}", a.status), iterations: a.iterations_completed, tool_invocations: a.tool_invocations, summary: a.result_summary.clone() }).collect()
}

fn handle_list_tasks(state: &Arc<Mutex<DaemonState>>) -> Vec<TaskInfo> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.scheduler.list_tasks().iter().map(|t| TaskInfo { id: t.id.clone(), name: t.name.clone(), schedule: format!("{:?}", t.schedule), status: format!("{:?}", t.status), run_count: t.run_count, enabled: t.enabled }).collect()
}

fn handle_audit_log(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");
    let events: Vec<serde_json::Value> = match std::fs::read_to_string(&audit_path) {
        Ok(content) => content.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect(),
        Err(_) => Vec::new(),
    };
    Response::json(200, &events)
}

fn handle_audit_export(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");
    let content = match std::fs::read_to_string(&audit_path) {
        Ok(c) => c,
        Err(_) => String::new(),
    };

    // Build CSV: sequence,timestamp,session_id,kind,message
    let mut csv = String::from("sequence,timestamp,session_id,kind,message\n");
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let seq     = v["sequence"].as_u64().unwrap_or(0);
            let ts      = v["timestamp"].as_str().unwrap_or("").replace(',', " ");
            let session = v["session_id"].as_str().unwrap_or("").replace(',', " ");
            let kind    = v["kind"].as_str().unwrap_or("").replace(',', " ");
            let msg     = v["message"].as_str().unwrap_or("").replace(',', " ").replace('\n', " ");
            csv.push_str(&format!("{seq},{ts},{session},{kind},{msg}\n"));
        }
    }

    let filename = format!("tachy-audit-{}.csv", chrono_now_str());
    csv_response(&csv, &filename)
}

fn handle_metrics(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let metrics = serde_json::json!({
        "total_agents_run": s.agents.len(),
        "completed": s.agents.values().filter(|a| format!("{:?}", a.status) == "Completed").count(),
        "failed": s.agents.values().filter(|a| format!("{:?}", a.status) == "Failed").count(),
        "total_iterations": s.agents.values().map(|a| a.iterations_completed).sum::<usize>(),
        "total_tool_invocations": s.agents.values().map(|a| a.tool_invocations).sum::<u32>(),
        "scheduled_tasks": s.scheduler.list_tasks().len(),
    });
    Response::json(200, &metrics)
}

fn handle_list_conversations(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let convs: Vec<serde_json::Value> = s.conversations.values().map(|c| {
        serde_json::json!({ "id": c.id, "title": c.title, "messages": c.messages, "message_count": c.messages.len(), "created_at": c.created_at, "updated_at": c.updated_at, "workspace": c.workspace })
    }).collect();
    Response::json(200, &convs)
}

fn handle_list_cloud_jobs(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &s.cloud_jobs)
}

fn handle_submit_cloud_job(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        name: String,
        #[serde(default)]
        command: Vec<String>,
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
        region: Option<String>,
        queue: Option<String>,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let region = req.region.as_deref().unwrap_or("us-east-1");
    let queue = req.queue.as_deref().unwrap_or("tachy-default");
    let client = crate::batch_client::BatchClient::new(region, queue);

    match client.submit_job(&req.name, req.command, req.env) {
        Ok(job) => {
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.cloud_jobs.push(job.clone());
            Response::json(201, &job)
        }
        Err(e) => Response::json(500, &ErrorResponse { error: e }),
    }
}

fn handle_get_cloud_job(job_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let idx = s.cloud_jobs.iter().position(|j| j.id == job_id);
    match idx {
        None => Response::json(404, &ErrorResponse { error: "job not found".to_string() }),
        Some(i) => {
            // Try to refresh status via AWS CLI
            let region = "us-east-1";
            let queue = "tachy-default";
            let client = crate::batch_client::BatchClient::new(region, queue);
            if let Ok(status) = client.get_job_status(job_id) {
                s.cloud_jobs[i].status = status;
                s.cloud_jobs[i].updated_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
            }
            Response::json(200, &s.cloud_jobs[i])
        }
    }
}

fn handle_list_swarm_runs(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let orch = s.orchestrator.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &orch.list_runs())
}

fn handle_get_swarm_run(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let orch = s.orchestrator.lock().unwrap_or_else(|e| e.into_inner());
    match orch.get_run(run_id) {
        Some(run) => Response::json(200, run),
        None => Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") }),
    }
}

fn handle_start_swarm_run(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut input: intelligence::SwarmRefactorInput = match serde_json::from_str(body) {
        Ok(i) => i,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };

    // Inject coordinator config from daemon state so workers stay local
    {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        input.coordinator = Some(s.config.coordinator.clone());
    }

    // Generate the swarm plan synchronously — coordinator may call frontier API here
    let plan = intelligence::plan_swarm_refactor(&input);
    let run_id = format!("swarm-{}", chrono_now_secs());
    eprintln!("[swarm] run={run_id} tasks={} planner={:?}", plan.tasks.len(), plan.planner);

    // Convert swarm plan tasks → AgentTask (the parallel runner's unit of work)
    let now = chrono_now_secs();
    let agent_tasks: Vec<AgentTask> = plan.tasks.iter().map(|t| AgentTask {
        id: format!("{run_id}-{}", t.id),
        run_id: run_id.clone(),
        template: t.template.clone(),
        prompt: audit::sanitize_prompt(&t.prompt, 50_000),
        model: None,
        deps: t.deps.iter().map(|d| format!("{run_id}-{d}")).collect(),
        priority: 128,
        status: TaskStatus::Pending,
        result: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        work_dir: None,
    }).collect();

    let run = ParallelRun {
        id: run_id.clone(),
        tasks: agent_tasks,
        status: RunStatus::Running,
        created_at: now,
        max_concurrency: 4,
        conflicts: Vec::new(),
    };

    let bg_state = Arc::clone(state);
    std::thread::spawn(move || {
        let completed = parallel::execute_parallel_run(run, &bg_state);
        // Persist to durable JSONL log + in-memory orchestrator
        if let Ok(s) = bg_state.lock() {
            parallel::Orchestrator::persist_run(&completed, &s.workspace_root);
            if let Ok(mut orch) = s.orchestrator.lock() {
                orch.register_completed_run(completed);
            }
        }
    });

    Response::json(202, &serde_json::json!({ "run_id": run_id, "status": "running" }))
}

fn handle_create_conversation(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { title: Option<String> }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { title: None });
    let title = req.title.unwrap_or_else(|| format!("Conversation {}", chrono_now_secs()));
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let id = s.create_conversation(&title);
    Response::json(200, &serde_json::json!({ "id": id, "title": title }))
}

fn handle_add_message(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { conversation_id: String, role: String, content: String, model: Option<String>, iterations: Option<usize>, tool_invocations: Option<u32> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    let msg = crate::state::ChatMessage { role: req.role, content: req.content, timestamp: chrono_now_secs().to_string(), model: req.model, iterations: req.iterations, tool_invocations: req.tool_invocations };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if s.add_message(&req.conversation_id, msg) { Response::json(200, &serde_json::json!({ "ok": true })) }
    else { Response::json(404, &ErrorResponse { error: "conversation not found".to_string() }) }
}

fn handle_run_agent(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct WebhookTrigger { source: Option<String>, event: Option<String>, template: Option<String>, prompt: Option<String>, payload: Option<serde_json::Value> }
    let trigger: WebhookTrigger = match serde_json::from_str(body) { Ok(t) => t, Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid webhook body: {e}") }) };
    let _source = trigger.source.as_deref().unwrap_or("unknown");
    let _event = trigger.event.as_deref().unwrap_or("unknown");
    let template = trigger.template.as_deref().unwrap_or("chat");
    let prompt = sanitize_prompt(&trigger.prompt.unwrap_or_else(|| "Analyze this event.".to_string()), 50_000);

    let (agent_id, config, governance) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let agent_id = match s.create_agent(template, &prompt) { Ok(id) => id, Err(e) => return Response::json(400, &ErrorResponse { error: e }) };
        let config = match s.agents.get(&agent_id) { Some(a) => a.config.clone(), None => return Response::json(500, &ErrorResponse { error: "agent not found".to_string() }) };
        if let Some(agent) = s.agents.get_mut(&agent_id) { agent.mark_running(); }
        s.save();
        (agent_id, config, s.config.governance.clone())
    };

    let bg_state = Arc::clone(state);
    let bg_agent_id = agent_id.clone();
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let (result, tracer, model) = {
            let s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
            let model = config.template.model.clone();
            let tracer = s.tracer.clone();
            let r = AgentEngine::run_agent(&bg_agent_id, &config, &prompt, &s.registry, &governance, &s.audit_logger, &s.config.intelligence, &s.workspace_root, Some(s.file_locks.clone()), Some(Arc::clone(&bg_state)));
            (r, tracer, model)
        };
        let duration_ms = t0.elapsed().as_millis() as u64;
        crate::telemetry::record_agent_run(
            &tracer,
            &bg_agent_id,
            &model,
            &config.template.name,
            result.success,
            result.iterations,
            result.tool_invocations,
            duration_ms,
        );
        let mut s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&bg_agent_id) {
            // Truncate summaries to ~4 KB (≈1 000 tokens) before storing to keep state compact.
            let stored_summary = truncate_completion(&result.summary, 1_000);
            if result.success { agent.mark_completed(&stored_summary); } else { agent.mark_failed(&stored_summary); }
        }
        s.save();
    });

    Response::json(202, &serde_json::json!({ "agent_id": agent_id, "status": "running" }))
}

async fn handle_complete_stream(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Req { prefix: String, suffix: Option<String>, model: Option<String>, max_tokens: Option<u32> }
    let req: Req = match serde_json::from_str(body) { Ok(r) => r, Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }) };
    let (response, tx) = Response::sse();
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let (v_backend, err_msg) = {
            let s = state.lock().unwrap_or_else(|e| e.into_inner());
            let model_name = req.model.as_deref().unwrap_or(&s.config.agent_templates[0].model);
            match backend::OllamaBackend::new(model_name.to_string(), "http://localhost:11434".to_string(), false) {
                Ok(b) => (Some(b), None),
                Err(e) => (None, Some(e.to_string())),
            }
        };

        if let Some(mut ollama_backend) = v_backend {
            let (t_tx, mut t_rx) = tokio::sync::mpsc::channel(100);
            ollama_backend.set_token_tx(t_tx);
            
            let tx_inner = tx.clone();
            tokio::spawn(async move {
                while let Some(event) = t_rx.recv().await {
                    match event {
                        backend::BackendEvent::Text(t) => {
                            let _ = tx_inner.send(format!("data: {{\"text\":\"{}\"}}\n\n", t.replace('\"', "\\\""))).await;
                        }
                        backend::BackendEvent::Thinking(t) => {
                            let _ = tx_inner.send(format!("data: {{\"thinking\":\"{}\"}}\n\n", t.replace('\"', "\\\""))).await;
                        }
                    }
                }
            });

            let (pre, suf, mid) = ollama_backend.get_fim_tokens();
            if let Ok((_tokens, metrics)) = ollama_backend.send_streaming_generate(backend::OllamaGenerateRequest {
                model: ollama_backend.model().to_string(),
                prompt: format!("{pre}{}{suf}{}{mid}", req.prefix, req.suffix.as_deref().unwrap_or("")),
                stream: true,
                raw: true,
                options: None,
            }).await {
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.inference_stats.record(metrics.ttft_ms, metrics.tokens_per_sec, metrics.total_tokens);
            }
        }

        if let Some(msg) = err_msg {
            let _ = tx.send(format!("data: {{\"error\":\"Backend fail: {}\"}}\n\n", msg)).await;
        }
        let _ = tx.send("event: done\ndata: {}\n\n".to_string()).await;
    });
    response
}

async fn handle_chat_stream(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { prompt: String, model: Option<String> }
    let req: Req = match serde_json::from_str(body) { Ok(r) => r, Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }) };
    let (response, tx) = Response::sse();
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let (v_backend, err_msg) = {
            let s = state.lock().unwrap_or_else(|e| e.into_inner());
            let model_name = req.model.as_deref().unwrap_or(&s.config.agent_templates[0].model);
            match backend::OllamaBackend::new(model_name.to_string(), "http://localhost:11434".to_string(), false) {
                Ok(b) => (Some(b), None),
                Err(e) => (None, Some(e.to_string())),
            }
        };

        if let Some(mut ollama_backend) = v_backend {
            let (t_tx, mut t_rx) = tokio::sync::mpsc::channel(100);
            ollama_backend.set_token_tx(t_tx);

            let tx_inner = tx.clone();
            tokio::spawn(async move {
                while let Some(event) = t_rx.recv().await {
                    match event {
                        backend::BackendEvent::Text(t) => {
                            let _ = tx_inner.send(format!("data: {{\"text\":\"{}\"}}\n\n", t.replace('\"', "\\\""))).await;
                        }
                        backend::BackendEvent::Thinking(t) => {
                            let _ = tx_inner.send(format!("data: {{\"thinking\":\"{}\"}}\n\n", t.replace('\"', "\\\""))).await;
                        }
                    }
                }
            });

            if let Ok((_tokens, metrics)) = ollama_backend.send_streaming(backend::OllamaChatRequest {
                model: ollama_backend.model().to_string(),
                messages: vec![backend::OllamaMessage { role: "user".to_string(), content: req.prompt, tool_calls: None }],
                stream: true,
                tools: None,
                options: None,
                format: None,
            }).await {
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.inference_stats.record(metrics.ttft_ms, metrics.tokens_per_sec, metrics.total_tokens);
            }
        } 
        
        if let Some(msg) = err_msg {
            let _ = tx.send(format!("data: {{\"text\":\"Error: {} check if model is installed in ollama.\"}}\n\n", msg)).await;
        }

        let _ = tx.send("event: done\ndata: {}\n\n".to_string()).await;
    });
    response
}

fn handle_sso_login(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    if !s.sso_manager.is_enabled() { return Response::json(400, &ErrorResponse { error: "SSO disabled".to_string() }); }
    Response::Full { status: 302, content_type: "text/plain".to_string(), body: format!("Redirecting to {}", s.sso_manager.build_login_url(Some("/"))) }
}

fn handle_sso_callback(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let saml = body.split("SAMLResponse=").nth(1).and_then(|s| s.split('&').next()).unwrap_or("");
    let mut user_store = std::mem::take(&mut s.user_store);
    let res = s.sso_manager.process_callback(saml, &mut user_store);
    s.user_store = user_store;
    match res {
        Ok(sess) => Response::json(200, &serde_json::json!({ "token": sess.token, "email": sess.email })),
        Err(e) => Response::json(401, &ErrorResponse { error: e }),
    }
}

fn handle_sso_logout(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { token: String }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { token: String::new() });
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.sso_manager.invalidate_session(&req.token);
    Response::json(200, &serde_json::json!({ "ok": true }))
}

fn handle_sso_sessions(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &s.sso_manager.active_sessions())
}

fn handle_license_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let license = audit::LicenseFile::load_or_create(&s.workspace_root.join(".tachy"));
    Response::json(200, &serde_json::json!({ "status": license.status().display(), "active": license.status().is_active() }))
}

fn handle_billing_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match &s.billing { Some(b) => Response::json(200, &b.status()), None => Response::json(200, &serde_json::json!({ "enabled": false })) }
}

fn handle_create_team(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { name: String }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { name: String::new() });
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.team_manager.create_team(&req.name, "api-user") { Ok(id) => { s.save(); Response::json(200, &serde_json::json!({ "team_id": id })) }, Err(e) => Response::json(400, &ErrorResponse { error: e.to_string() }) }
}

fn handle_join_team(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { token: String }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { token: String::new() });
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.team_manager.join(&req.token, "api-user") { Ok(_) => { s.save(); Response::json(200, &serde_json::json!({ "ok": true })) }, Err(e) => Response::json(400, &ErrorResponse { error: e.to_string() }) }
}

fn handle_marketplace_list(_path: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &s.marketplace.search(None, 1, 20))
}

fn handle_install(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { listing_id: String, version: Option<String> }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { listing_id: String::new(), version: None });
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.marketplace.install(&req.listing_id, req.version.as_deref()) { Ok(_) => Response::json(200, &serde_json::json!({ "ok": true })), Err(e) => Response::json(400, &ErrorResponse { error: e.to_string() }) }
}

fn handle_list_parallel_runs(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let orch = s.orchestrator.lock().unwrap_or_else(|e| e.into_inner());
    let runs: Vec<serde_json::Value> = orch.list_runs().iter().map(|r| serde_json::json!({
        "run_id": r.id,
        "status": r.status,
        "task_count": r.tasks.len(),
        "created_at": r.created_at,
    })).collect();
    Response::json(200, &serde_json::json!({ "runs": runs }))
}

fn handle_parallel_run(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let req: ParallelRunRequest = match serde_json::from_str(body) { Ok(r) => r, Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }) };
    let run_id = format!("run-{}", chrono_now_secs());
    let tasks: Vec<AgentTask> = req.tasks.iter().enumerate().map(|(i, t)| AgentTask { id: format!("{run_id}-t{i}"), run_id: run_id.clone(), template: t.template.clone(), prompt: audit::sanitize_prompt(&t.prompt, 50_000), model: t.model.clone(), deps: t.deps.clone(), priority: t.priority, status: TaskStatus::Pending, result: None, created_at: chrono_now_secs(), started_at: None, completed_at: None, work_dir: None }).collect();
    let run = ParallelRun { id: run_id.clone(), tasks, status: RunStatus::Running, created_at: chrono_now_secs(), max_concurrency: req.max_concurrency.min(8).max(1), conflicts: Vec::new() };
    let bg_state = Arc::clone(state);
    std::thread::spawn(move || {
        let completed = parallel::execute_parallel_run(run, &bg_state);
        // Persist to durable JSONL log + in-memory orchestrator
        if let Ok(s) = bg_state.lock() {
            parallel::Orchestrator::persist_run(&completed, &s.workspace_root);
            if let Ok(mut orch) = s.orchestrator.lock() {
                orch.register_completed_run(completed);
            }
        }
    });
    Response::json(202, &serde_json::json!({ "run_id": run_id, "status": "running" }))
}

fn handle_get_parallel_run(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    // Check the orchestrator first (populated when the run completes)
    let from_orch = s.orchestrator.lock().ok()
        .and_then(|orch| orch.get_run(run_id).cloned());
    if let Some(run) = from_orch {
        return Response::json(200, &run);
    }
    // Run still in-flight: derive live status from registered agent instances
    let tasks: Vec<_> = s.agents.iter()
        .filter(|(id, _)| id.starts_with(run_id))
        .map(|(id, agent)| serde_json::json!({
            "task_id": id,
            "status": format!("{:?}", agent.status).to_lowercase(),
            "iterations": agent.iterations_completed,
            "tool_invocations": agent.tool_invocations,
        }))
        .collect();
    if tasks.is_empty() {
        return Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") });
    }
    Response::json(200, &serde_json::json!({ "run_id": run_id, "status": "running", "tasks": tasks }))
}

fn handle_list_teams(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let teams: Vec<&crate::teams::Team> = s.team_manager.teams().values().collect();
    Response::json(200, &teams)
}

fn handle_get_team(team_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.team_manager.teams().get(team_id) {
        Some(team) => Response::json(200, team),
        None => Response::json(404, &ErrorResponse { error: format!("team not found: {team_id}") }),
    }
}

fn handle_team_agents(team_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    // Verify team exists
    if s.team_manager.teams().get(team_id).is_none() {
        return Response::json(404, &ErrorResponse { error: format!("team not found: {team_id}") });
    }
    // Return agents whose session_id is prefixed with the team_id (convention used when
    // agents are launched on behalf of a team) or all agents if no naming convention applies.
    let agents: Vec<serde_json::Value> = s.agents.iter()
        .filter(|(id, _)| id.contains(team_id))
        .map(|(id, a)| serde_json::json!({
            "id": id,
            "template": a.config.template.name,
            "status": format!("{:?}", a.status),
            "iterations": a.iterations_completed,
            "tool_invocations": a.tool_invocations,
        }))
        .collect();
    Response::json(200, &serde_json::json!({ "team_id": team_id, "agents": agents }))
}

fn handle_team_audit(team_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    // Verify team exists
    if s.team_manager.teams().get(team_id).is_none() {
        return Response::json(404, &ErrorResponse { error: format!("team not found: {team_id}") });
    }
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");
    let events: Vec<serde_json::Value> = match std::fs::read_to_string(&audit_path) {
        Ok(content) => content.lines()
            .filter(|l| !l.trim().is_empty() && l.contains(team_id))
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    Response::json(200, &serde_json::json!({ "team_id": team_id, "events": events }))
}

fn handle_list_pending_approvals(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &serde_json::json!({ "pending": s.pending_patches }))
}

fn handle_approve_patch(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct ApproveReq { patch_id: String, approved: bool }
    let req: ApproveReq = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Response::json(400, &ErrorResponse { error: "invalid request".to_string() }),
    };
    
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if req.approved {
        match s.approve_patch(&req.patch_id) {
            Ok(path) => Response::json(200, &serde_json::json!({ "status": "approved", "file": path })),
            Err(e) => Response::json(400, &ErrorResponse { error: e }),
        }
    } else {
        match s.reject_patch(&req.patch_id) {
            Ok(path) => Response::json(200, &serde_json::json!({ "status": "rejected", "file": path })),
            Err(e) => Response::json(400, &ErrorResponse { error: e }),
        }
    }
}

fn handle_list_file_locks(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &serde_json::json!({ "locks": s.file_locks.list_locks() }))
}

/// POST /api/complete — non-streaming single-turn completion.
async fn handle_complete(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct CompleteRequest {
        prompt: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default = "default_max_tokens")]
        max_tokens: usize,
    }
    fn default_max_tokens() -> usize { 2048 }

    let req: CompleteRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    if req.prompt.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "prompt must not be empty".to_string() });
    }
    let prompt = audit::sanitize_prompt(&req.prompt, 50_000);
    let model = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        req.model.unwrap_or_else(|| s.config.default_model.clone())
    };
    let model_clone = model.clone();
    let mut ollama = match backend::OllamaBackend::new(model_clone, "http://localhost:11434".to_string(), false) {
        Ok(b) => b,
        Err(e) => return Response::json(502, &ErrorResponse { error: format!("backend unavailable: {e}") }),
    };
    // Collect tokens via channel rather than SSE
    let (t_tx, mut t_rx) = tokio::sync::mpsc::channel(256);
    ollama.set_token_tx(t_tx);
    let _max_toks = req.max_tokens.min(4096) as u32;
    let gen_fut = ollama.send_streaming_generate(backend::OllamaGenerateRequest {
        model: ollama.model().to_string(),
        prompt,
        stream: true,
        raw: false,
        options: None,
    });
    // Run inference and collect tokens concurrently
    let (gen_result, completion) = tokio::join!(
        gen_fut,
        async {
            let mut buf = String::new();
            // Drain up to max_toks*4 chars worth of tokens
            while let Some(ev) = t_rx.recv().await {
                if let backend::BackendEvent::Text(t) = ev { buf.push_str(&t); }
            }
            buf
        }
    );
    // ignore: t_rx closes when ollama drops, join completes
    let _ = gen_result.as_ref().map(|(_, m)| {
        if let Ok(mut s) = state.lock() {
            s.inference_stats.record(m.ttft_ms, m.tokens_per_sec, m.total_tokens);
        }
    });
    let text = if completion.is_empty() {
        gen_result.ok().map(|(events, _)| {
            // fallback: extract text from returned events
            events.into_iter().filter_map(|e| {
                let s = format!("{e:?}");
                if s.starts_with("TextDelta(") {
                    Some(s[10..s.len()-1].to_string())
                } else { None }
            }).collect::<String>()
        }).unwrap_or_default()
    } else {
        completion
    };
    Response::json(200, &serde_json::json!({ "completion": text, "model": model }))
}

/// GET /api/webhooks — list registered webhooks.
fn handle_list_webhooks(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &serde_json::json!({ "webhooks": s.webhooks }))
}

/// POST /api/webhooks — register a new webhook (optionally with an HMAC-SHA256 secret).
fn handle_register_webhook(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct RegisterWebhookRequest {
        url: String,
        #[serde(default)]
        events: Vec<String>,
        #[serde(default = "bool_true")]
        enabled: bool,
        /// Optional signing secret. Outbound payloads will include
        /// `X-Tachy-Signature: sha256=<hmac>` when set.
        secret: Option<String>,
    }
    fn bool_true() -> bool { true }

    let req: RegisterWebhookRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    if req.url.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "url is required".to_string() });
    }
    // Basic URL validation — must start with http
    if !req.url.starts_with("http://") && !req.url.starts_with("https://") {
        return Response::json(400, &ErrorResponse { error: "url must start with http:// or https://".to_string() });
    }
    let events = if req.events.is_empty() { vec!["*".to_string()] } else { req.events };
    let signed = req.secret.is_some();
    let webhook = crate::state::WebhookConfig { url: req.url, events, enabled: req.enabled, secret: req.secret };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.webhooks.push(webhook.clone());
    s.save();
    Response::json(201, &serde_json::json!({
        "ok": true,
        "webhook": webhook,
        "signed": signed,
        "note": if signed { "Outbound payloads will include X-Tachy-Signature header" } else { "No signing secret configured" },
    }))
}

/// POST /api/webhooks/verify — validate an inbound webhook signature.
fn handle_verify_webhook_signature(body: &str, raw: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    // Extract X-Tachy-Signature from raw HTTP request headers
    let sig_header = raw.lines()
        .find(|l| l.to_lowercase().starts_with("x-tachy-signature:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Extract webhook URL from body
    #[derive(serde::Deserialize)]
    struct Req { webhook_url: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Response::json(400, &ErrorResponse { error: "missing webhook_url".to_string() }),
    };

    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.verify_webhook_signature(&req.webhook_url, body.as_bytes(), &sig_header) {
        Ok(()) => Response::json(200, &serde_json::json!({ "valid": true })),
        Err(e) => Response::json(401, &serde_json::json!({ "valid": false, "reason": e })),
    }
}

/// POST /api/tasks/schedule — register a new recurring agent task.
fn handle_schedule_task(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct ScheduleRequest {
        template: String,
        name: String,
        #[serde(default)]
        interval_seconds: Option<u64>,
    }

    let req: ScheduleRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    if req.template.trim().is_empty() || req.name.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "template and name are required".to_string() });
    }
    let rule = match req.interval_seconds {
        Some(secs) if secs > 0 => platform::ScheduleRule::Interval { seconds: secs },
        _ => platform::ScheduleRule::Once,
    };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.schedule_agent(&req.template, rule, &req.name) {
        Ok(task_id) => Response::json(201, &serde_json::json!({ "task_id": task_id, "name": req.name })),
        Err(e) => Response::json(400, &ErrorResponse { error: e }),
    }
}

/// POST /api/license/activate — activate a license key.
fn handle_license_activate(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct ActivateRequest {
        key: String,
        secret: String,
    }

    let req: ActivateRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    if req.key.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "key is required".to_string() });
    }
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let tachy_dir = s.workspace_root.join(".tachy");
    drop(s);

    let mut license = audit::LicenseFile::load_or_create(&tachy_dir);
    match license.activate(&req.key, &req.secret) {
        Ok(data) => {
            if let Err(e) = license.save(&tachy_dir) {
                return Response::json(500, &ErrorResponse { error: format!("activation succeeded but save failed: {e}") });
            }
            Response::json(200, &serde_json::json!({
                "status": "activated",
                "tier": format!("{:?}", data.tier),
                "expires_at": data.expires_at,
            }))
        }
        Err(e) => Response::json(400, &ErrorResponse { error: e }),
    }
}

/// POST /api/parallel/runs/{id}/cancel — cancel a running parallel run.
fn handle_cancel_parallel_run(run_id: &str, _body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let matching: Vec<_> = s.agents.keys().filter(|id| id.starts_with(run_id)).cloned().collect();
    if matching.is_empty() {
        return Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") });
    }
    drop(s);
    // Signal cancellation by marking tasks: in a full implementation this would
    // set a cancellation token; here we record the intent and return accepted.
    Response::json(202, &serde_json::json!({ "run_id": run_id, "status": "cancellation_requested", "tasks": matching.len() }))
}

// ── Telemetry handlers ────────────────────────────────────────────────────────

/// POST /api/telemetry/flush — force-flush buffered spans to OTLP endpoint.
fn handle_telemetry_flush(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.tracer.flush();
    Response::json(200, &serde_json::json!({ "flushed": true }))
}

/// GET /api/telemetry/status — whether OTLP export is configured.
fn handle_telemetry_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let enabled = s.tracer.is_enabled();
    let endpoint = std::env::var("TACHY_OTLP_ENDPOINT").unwrap_or_else(|_| "(not set)".to_string());
    Response::json(200, &serde_json::json!({
        "enabled": enabled,
        "otlp_endpoint": endpoint,
        "service_name": std::env::var("TACHY_SERVICE_NAME").unwrap_or_else(|_| "tachy-daemon".to_string()),
    }))
}

// ── Distributed worker registry handlers ─────────────────────────────────────

/// GET /api/workers — list all registered worker daemons.
fn handle_list_workers(state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.worker_registry.prune_stale();
    let workers = s.worker_registry.list_workers().into_iter().cloned().collect::<Vec<_>>();
    Response::json(200, &serde_json::json!({
        "count": workers.len(),
        "available": s.worker_registry.available_worker_count(),
        "workers": workers,
    }))
}

/// POST /api/workers/register — register a remote worker daemon.
fn handle_register_worker(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let worker: crate::worker_registry::WorkerNode = match serde_json::from_str(body) {
        Ok(w) => w,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid worker registration: {e}") }),
    };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let id = s.worker_registry.register(worker);
    Response::json(200, &serde_json::json!({ "registered": true, "worker_id": id }))
}

/// POST /api/workers/heartbeat — update a worker's liveness + active task count.
fn handle_worker_heartbeat(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Req { worker_id: String, active_tasks: usize }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid heartbeat: {e}") }),
    };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if s.worker_registry.heartbeat(&req.worker_id, req.active_tasks) {
        Response::json(200, &serde_json::json!({ "ok": true }))
    } else {
        Response::json(404, &ErrorResponse { error: format!("worker not found: {}", req.worker_id) })
    }
}

/// DELETE /api/workers/deregister — remove a worker from the registry.
fn handle_deregister_worker(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Req { worker_id: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.worker_registry.deregister(&req.worker_id);
    Response::json(200, &serde_json::json!({ "deregistered": true }))
}

// ── OAuth2 handlers ───────────────────────────────────────────────────────────

/// GET /api/auth/oauth/{provider}/login → redirect to provider authorization URL.
fn handle_oauth_login(provider_str: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    use audit::{OAuthClientConfig, OAuthProvider};

    let provider = match OAuthProvider::from_str(provider_str) {
        Some(p) => p,
        None => return Response::json(400, &ErrorResponse { error: format!("unknown provider: {provider_str}") }),
    };

    let config = match OAuthClientConfig::from_env(&provider) {
        Some(c) => c,
        None => return Response::json(503, &ErrorResponse {
            error: format!("OAuth2 not configured for {provider_str}. Set TACHY_{}_CLIENT_ID and TACHY_{}_CLIENT_SECRET.",
                provider_str.to_uppercase(), provider_str.to_uppercase()),
        }),
    };

    let (url, _state_token) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.oauth_manager.authorization_url(&config)
    };

    // HTTP 302 redirect
    Response::Full {
        status: 302,
        content_type: "text/plain".to_string(),
        body: format!("Location: {url}\r\n"),
    }
}

/// GET /api/auth/oauth/{provider}/callback?code=...&state=...
fn handle_oauth_callback(provider_str: &str, query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    use audit::{OAuthClientConfig, OAuthProvider};

    let provider = match OAuthProvider::from_str(provider_str) {
        Some(p) => p,
        None => return Response::json(400, &ErrorResponse { error: format!("unknown provider: {provider_str}") }),
    };

    let config = match OAuthClientConfig::from_env(&provider) {
        Some(c) => c,
        None => return Response::json(503, &ErrorResponse { error: "OAuth2 not configured".to_string() }),
    };

    // Parse code and state from query string
    let mut code = String::new();
    let mut oauth_state = String::new();
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k { "code" => code = v.to_string(), "state" => oauth_state = v.to_string(), _ => {} }
        }
    }

    if code.is_empty() {
        return Response::json(400, &ErrorResponse { error: "missing code parameter".to_string() });
    }

    let result = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.oauth_manager.handle_callback(&config, &code, &oauth_state)
    };

    match result {
        Ok(session) => Response::json(200, &serde_json::json!({
            "token": session.token,
            "provider": provider_str,
            "user_id": session.user_id,
            "email": session.email,
            "display_name": session.display_name,
            "avatar_url": session.avatar_url,
            "expires_at": session.expires_at,
        })),
        Err(e) => Response::json(401, &ErrorResponse { error: e }),
    }
}

/// POST /api/auth/oauth/logout — revoke an OAuth2 session token.
fn handle_oauth_logout(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Req { token: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Response::json(400, &ErrorResponse { error: "missing token".to_string() }),
    };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.oauth_manager.revoke_session(&req.token);
    Response::json(200, &serde_json::json!({ "revoked": true }))
}

/// GET /api/auth/oauth/sessions — count active OAuth2 sessions.
fn handle_oauth_sessions(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    Response::json(200, &serde_json::json!({
        "active_sessions": s.oauth_manager.active_session_count(),
    }))
}

/// GET /api/runs/history — all completed runs from the durable JSONL log.
fn handle_run_history(state: &Arc<Mutex<DaemonState>>) -> Response {
    let workspace_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let runs = parallel::Orchestrator::load_run_history(&workspace_root);
    Response::json(200, &serde_json::json!({
        "count": runs.len(),
        "runs": runs,
    }))
}

/// GET /api/parallel/runs/{id}/conflicts — semantic merge conflicts for a run.
fn handle_get_run_conflicts(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let run = s.orchestrator.lock().unwrap_or_else(|e| e.into_inner())
        .get_run(run_id).cloned();
    match run {
        None => Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") }),
        Some(r) => {
            let count = r.conflicts.len();
            Response::json(200, &serde_json::json!({
                "run_id": run_id,
                "conflict_count": count,
                "has_conflicts": count > 0,
                "conflicts": r.conflicts,
            }))
        }
    }
}

// Helpers

fn parse_http_request(raw: &str) -> (String, String, String) {
    let mut lines = raw.lines();
    let first_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.first().unwrap_or(&"GET").to_string();
    let path = parts.get(1).unwrap_or(&"/").to_string();
    let body = if let Some(pos) = raw.find("\r\n\r\n") { raw[pos + 4..].to_string() } else if let Some(pos) = raw.find("\n\n") { raw[pos + 2..].to_string() } else { String::new() };
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

fn csv_response(body: &str, _filename: &str) -> Response {
    Response::Full { status: 200, content_type: "text/csv".to_string(), body: body.to_string() }
}

fn truncate_completion(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * 4;
    if text.len() <= max_chars { text.to_string() } else { text[..max_chars].to_string() }
}

/// GET /api/conversations/{id} — return a single conversation by ID.
fn handle_get_conversation(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    if id.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "conversation id required".to_string() });
    }
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.get_conversation(id) {
        Some(conv) => Response::json(200, conv),
        None => Response::json(404, &ErrorResponse { error: format!("conversation not found: {id}") }),
    }
}

/// DELETE /api/conversations/{id} — delete a conversation by ID.
fn handle_delete_conversation(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    if id.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "conversation id required".to_string() });
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if s.delete_conversation(id) {
        Response::json(204, &serde_json::json!({}))
    } else {
        Response::json(404, &ErrorResponse { error: format!("conversation not found: {id}") })
    }
}

/// DELETE /api/agents/{id} — remove an agent from state.
fn handle_delete_agent(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    if id.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "agent id required".to_string() });
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if s.delete_agent(id) {
        Response::json(204, &serde_json::json!({}))
    } else {
        Response::json(404, &ErrorResponse { error: format!("agent not found: {id}") })
    }
}

/// POST /api/agents/{id}/cancel — mark a running agent as cancelled (status=Failed).
fn handle_cancel_agent(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    if id.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "agent id required".to_string() });
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if s.cancel_agent(id) {
        Response::json(200, &serde_json::json!({ "id": id, "status": "Failed" }))
    } else {
        Response::json(404, &ErrorResponse { error: format!("agent not found: {id}") })
    }
}

/// POST /api/index — trigger a codebase index (re)build for the daemon workspace.
///
/// The build runs in a background thread so the HTTP call returns immediately
/// with 202 Accepted.  Poll `GET /api/index` to check when it completes.
/// POST /api/prompt — one-shot synchronous prompt for model comparison UI (E2).
/// Body: { "prompt": "...", "model": "llama3.3", "session_id": "cmp-..." }
/// Returns: { "response": "...", "model": "...", "token_usage": { ... } }
fn handle_prompt_oneshot(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { prompt: String, model: Option<String>, session_id: Option<String> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r, Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    let prompt = sanitize_prompt(&req.prompt, 50_000);

    let (registry, governance, intel_cfg, workspace_root, mut template) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        let tmpl = s.config.agent_templates.first().cloned()
            .unwrap_or_else(platform::AgentTemplate::chat_assistant);
        (s.registry.clone(), s.config.governance.clone(),
         s.config.intelligence.clone(), s.workspace_root.clone(), tmpl)
    };
    let audit_logger = audit::AuditLogger::new();

    if let Some(m) = &req.model { template.model = m.clone(); }
    let session_id = req.session_id.unwrap_or_else(|| format!("cmp-{}", template.model.replace(|c: char| !c.is_alphanumeric(), "_")));
    let config = platform::AgentConfig {
        template,
        session_id: session_id.clone(),
        working_directory: workspace_root.to_string_lossy().to_string(),
        environment: std::collections::BTreeMap::new(),
    };

    let result = AgentEngine::run_agent(
        &session_id, &config, &prompt, &registry, &governance,
        &audit_logger, &intel_cfg, &workspace_root, None, None,
    );

    Response::json(200, &serde_json::json!({
        "model": config.template.model,
        "response": result.summary,
        "iterations": result.iterations,
        "tool_invocations": result.tool_invocations,
        "success": result.success,
    }))
}

/// GET /api/usage — return per-user usage aggregates from MeteringService.
fn handle_usage(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let counters = s.metering.counters();
    let users: Vec<serde_json::Value> = counters.values().map(|a| serde_json::json!({
        "user_id": a.user_id,
        "team_id": a.team_id,
        "total_input_tokens": a.total_input_tokens,
        "total_output_tokens": a.total_output_tokens,
        "total_tool_invocations": a.total_tool_invocations,
        "total_agent_runs": a.total_agent_runs,
        "period_start": a.period_start,
        "period_end": a.period_end,
    })).collect();
    let total_tokens: u64 = counters.values().map(|a| a.total_input_tokens + a.total_output_tokens).sum();
    let total_tools: u64 = counters.values().map(|a| a.total_tool_invocations).sum();
    let total_runs: u64 = counters.values().map(|a| a.total_agent_runs).sum();
    Response::json(200, &serde_json::json!({
        "users": users,
        "totals": {
            "tokens": total_tokens,
            "tool_invocations": total_tools,
            "agent_runs": total_runs,
        }
    }))
}

/// POST /api/finetune/extract — extract Alpaca-format JSONL from session history.
/// Body: { "sessions_dir": ".tachy/sessions" } (optional, defaults to workspace)
/// Returns: { "entries": N, "source_sessions": N, "jsonl": "..." }
fn handle_finetune_extract(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { sessions_dir: Option<String> }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { sessions_dir: None });
    let workspace_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let sessions_dir = req.sessions_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace_root.join(".tachy").join("sessions"));

    let dataset = intelligence::FinetuneDataset::from_sessions(&sessions_dir);
    Response::json(200, &serde_json::json!({
        "entries": dataset.total_pairs,
        "source_sessions": dataset.source_sessions,
        "jsonl": dataset.to_jsonl(),
    }))
}

/// POST /api/finetune/modelfile — generate an Ollama Modelfile for a LoRA adapter.
/// Body: { "base_model": "mistral:7b", "adapter_path": "./adapter.gguf", "system_prompt": "..." }
/// Returns: { "modelfile": "FROM mistral:7b\n..." }
fn handle_finetune_modelfile(body: &str, _state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { base_model: String, adapter_path: String, system_prompt: Option<String> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r, Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    let prompt = req.system_prompt.as_deref().unwrap_or("You are a helpful AI coding assistant.");
    let mf = intelligence::generate_modelfile(&req.base_model, &req.adapter_path, prompt);
    Response::json(200, &serde_json::json!({ "modelfile": mf }))
}

/// GET /api/diagnostics?file=<path> — return LSP-style diagnostics for a file.
/// Falls back to empty list if LSP server not available.
fn handle_diagnostics(query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let file_path = {
        let mut path = String::new();
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "file" || k == "path" { path = urlencoding_decode(v); break; }
            }
        }
        path
    };
    if file_path.is_empty() {
        return Response::json(400, &ErrorResponse { error: "missing ?file= param".to_string() });
    }
    let workspace_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let lsp = intelligence::LspManager::new(&workspace_root);
    let diagnostics = lsp.get_diagnostics(&file_path);
    Response::json(200, &serde_json::json!({
        "file": file_path,
        "diagnostics": diagnostics.iter().map(|d| serde_json::json!({
            "file": d.file,
            "line": d.line,
            "column": d.column,
            "message": d.message,
            "severity": match d.severity {
                intelligence::DiagnosticSeverity::Error => "error",
                intelligence::DiagnosticSeverity::Warning => "warning",
                _ => "info",
            },
            "source": d.source,
        })).collect::<Vec<_>>(),
        "count": diagnostics.len(),
    }))
}

fn handle_index_build(_body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.workspace_root.clone()
    };
    let ws_display = ws.display().to_string();
    std::thread::spawn(move || {
        let cfg = intelligence::IndexerConfig::default();
        match intelligence::CodebaseIndexer::build_index(&ws, &cfg) {
            Ok(idx) => eprintln!("[index] build complete: {} files indexed", idx.files.len()),
            Err(e) => eprintln!("[index] build failed: {e}"),
        }
    });
    Response::json(202, &serde_json::json!({
        "status": "building",
        "workspace": ws_display,
        "message": "Index build started in background — poll GET /api/index for status"
    }))
}

/// GET /api/index — return the current index status (file count + workspace path).
fn handle_index_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.workspace_root.clone()
    };
    match intelligence::CodebaseIndexer::load_index(&ws) {
        Ok(idx) => Response::json(200, &serde_json::json!({
            "status": "ready",
            "file_count": idx.files.len(),
            "workspace": ws.display().to_string()
        })),
        Err(_) => Response::json(200, &serde_json::json!({
            "status": "not_built",
            "file_count": 0,
            "workspace": ws.display().to_string()
        })),
    }
}

/// GET /api/graph?file=<path> — return dependency graph (or per-file if ?file= given).
fn handle_dependency_graph(query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.workspace_root.clone()
    };
    let graph = intelligence::DependencyGraph::build(&ws);
    // Parse optional ?file= query param
    let file_param = query
        .split('&')
        .find(|p| p.starts_with("file="))
        .map(|p| urlencoding_decode(&p[5..]));
    if let Some(f) = file_param {
        let deps = graph.transitive_dependents(&f);
        let node = graph.nodes.get(&f);
        return Response::json(200, &serde_json::json!({
            "file": f,
            "direct_imports": node.map(|n| &n.imports).cloned().unwrap_or_default(),
            "imported_by": node.map(|n| &n.imported_by).cloned().unwrap_or_default(),
            "transitive_dependents": deps,
        }));
    }
    Response::json(200, &graph)
}

/// GET /api/monorepo — detect monorepo structure.
fn handle_monorepo(state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.workspace_root.clone()
    };
    let manifest = intelligence::MonorepoManifest::detect(&ws);
    Response::json(200, &manifest)
}

/// GET /api/dashboard — performance stats + cost estimate.
fn handle_dashboard(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let stats = &s.inference_stats;
    let total_tokens = stats.total_tokens;
    let cost = (total_tokens as f64 / 1_000.0) * 0.002;
    let models: Vec<serde_json::Value> = s
        .registry
        .list_models()
        .iter()
        .map(|m| serde_json::json!({ "name": m.name, "tier": format!("{:?}", m.tier) }))
        .collect();
    Response::json(200, &serde_json::json!({
        "total_requests": stats.total_requests,
        "total_tokens": total_tokens,
        "input_tokens": stats.input_tokens,
        "output_tokens": stats.output_tokens,
        "avg_tokens_per_sec": stats.avg_tokens_per_sec,
        "last_tokens_per_sec": stats.last_tokens_per_sec,
        "p50_ttft_ms": stats.p50_ttft_ms,
        "p95_ttft_ms": stats.p95_ttft_ms,
        "estimated_cost_usd": cost,
        "models": models,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_endpoint_works() {
        let root = std::env::temp_dir().join(format!("tachy-test-{}", chrono_now_secs()));
        // DaemonState::init calls reqwest::blocking which creates a runtime internally.
        // spawn_blocking runs it on a dedicated thread where blocking is allowed.
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(RateLimiter::new(100, 60)));
        let res = handle_request("GET /health HTTP/1.1\r\n\r\n", &state, &limiter, "127.0.0.1").await;
        if let Response::Full { body, .. } = res { assert!(body.contains("\"status\":\"ok\"") || body.contains("\"status\": \"ok\"")); } else { panic!("not full"); }
    }

    // --- urlencoding_decode ---

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
        // A lone % at the end should not panic
        let result = urlencoding_decode("abc%");
        assert!(result.starts_with("abc"));
    }

    // --- handle_search: missing ?q= returns 400 ---

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

    // --- handle_get_policy: returns 200 with policy JSON ---

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
            // Should be valid JSON
            assert!(serde_json::from_str::<serde_json::Value>(&body).is_ok());
        } else {
            panic!("expected Full response");
        }
    }

    // --- handle_set_policy: invalid JSON returns 400 ---

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

    // --- handle_get_conversation ---

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

    // --- handle_delete_conversation ---

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

    // --- handle_delete_agent ---

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

    // --- handle_cancel_agent ---

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

    // --- handle_index_status: returns 200 ---

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
}
