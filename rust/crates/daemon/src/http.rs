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

#[derive(Debug, Deserialize)]
struct CancelRunRequest {
    task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateTeamRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct InviteRequest {
    email: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct JoinTeamRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
struct UpdateMemberRequest {
    role: String,
}

#[derive(Debug, Deserialize)]
struct PublishRequest {
    template: platform::AgentTemplate,
    description: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct InstallRequest {
    listing_id: String,
    #[serde(default)]
    version: Option<String>,
}

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
        ("GET", "/api/inference/stats") => Response::json(200, &handle_inference_stats(state)),
        ("POST", "/api/models/pull") => handle_pull_model(&body, state),
        ("POST", "/api/complete/stream") => handle_complete_stream(&body, state).await,
        ("POST", "/api/chat/stream") => handle_chat_stream(&body, state).await,
        ("GET", "/api/templates") => Response::json(200, &handle_list_templates(state)),
        ("GET", "/api/agents") => Response::json(200, &handle_list_agents(state)),
        ("GET", "/api/tasks") => Response::json(200, &handle_list_tasks(state)),
        ("GET", "/api/audit") => handle_audit_log(state),
        ("GET", "/api/metrics") => handle_metrics(state),
        ("GET", "/api/conversations") => handle_list_conversations(state),
        ("POST", "/api/conversations") => handle_create_conversation(&body, state),
        ("POST", "/api/conversations/message") => handle_add_message(&body, state),
        ("GET", "/api/auth/sso/login") => handle_sso_login(state),
        ("POST", "/api/auth/sso/callback") => handle_sso_callback(&body, state),
        ("POST", "/api/auth/sso/logout") => handle_sso_logout(&body, state),
        ("GET", "/api/auth/sso/sessions") => handle_sso_sessions(state),
        ("GET", "/api/license/status") => handle_license_status(state),
        ("GET", "/api/billing/status") => handle_billing_status(state),
        ("POST", "/api/teams") => handle_create_team(&body, state),
        ("POST", "/api/teams/join") => handle_join_team(&body, state),
        ("GET", "/api/marketplace") => handle_marketplace_list(&path, state),
        ("POST", "/api/marketplace/install") => handle_install(&body, state),
        ("POST", "/api/parallel/runs") => handle_parallel_run(&body, state),
        ("POST", "/api/agents/run") => handle_run_agent(&body, state),
        ("GET", "/api/pending-approvals") => handle_list_pending_approvals(state),
        ("POST", "/api/approve") => handle_approve_patch(&body, state),
        ("GET", "/api/file-locks") => handle_list_file_locks(state),

        _ => {
            if method == "GET" && path.starts_with("/api/agents/") {
                return handle_get_agent(&path["/api/agents/".len()..], state);
            }
            if method == "GET" && path.starts_with("/api/parallel/runs/") {
                return handle_get_parallel_run(&path["/api/parallel/runs/".len()..], state);
            }
            Response::json(404, &ErrorResponse { error: format!("not found: {method} {path}") })
        }
    }
}

fn handle_inference_stats(state: &Arc<Mutex<DaemonState>>) -> serde_json::Value {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    serde_json::to_value(&s.inference_stats).unwrap_or_default()
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
        let result = {
            let s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
            AgentEngine::run_agent(&bg_agent_id, &config, &prompt, &s.registry, &governance, &s.audit_logger, &s.config.intelligence, &s.workspace_root, Some(s.file_locks.clone()), Some(Arc::clone(&bg_state)))
        };
        let mut s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&bg_agent_id) { if result.success { agent.mark_completed(&result.summary); } else { agent.mark_failed(&result.summary); } }
        s.save();
    });

    Response::json(202, &serde_json::json!({ "agent_id": agent_id, "status": "running" }))
}

async fn handle_complete_stream(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
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

fn handle_parallel_run(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let req: ParallelRunRequest = match serde_json::from_str(body) { Ok(r) => r, Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }) };
    let run_id = format!("run-{}", chrono_now_secs());
    let tasks: Vec<AgentTask> = req.tasks.iter().enumerate().map(|(i, t)| AgentTask { id: format!("{run_id}-t{i}"), run_id: run_id.clone(), template: t.template.clone(), prompt: audit::sanitize_prompt(&t.prompt, 50_000), model: t.model.clone(), deps: t.deps.clone(), priority: t.priority, status: TaskStatus::Pending, result: None, created_at: chrono_now_secs(), started_at: None, completed_at: None, work_dir: None }).collect();
    let run = ParallelRun { id: run_id.clone(), tasks, status: RunStatus::Running, created_at: chrono_now_secs(), max_concurrency: req.max_concurrency.min(8).max(1) };
    let bg_state = Arc::clone(state);
    std::thread::spawn(move || { let _ = parallel::execute_parallel_run(run, &bg_state); });
    Response::json(202, &serde_json::json!({ "run_id": run_id, "status": "running" }))
}

fn handle_get_parallel_run(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let tasks: Vec<_> = s.agents.iter().filter(|(id, _)| id.starts_with(run_id)).map(|(id, agent)| serde_json::json!({ "task_id": id, "status": format!("{:?}", agent.status) })).collect();
    Response::json(200, &serde_json::json!({ "run_id": run_id, "tasks": tasks }))
}

fn handle_team_agents(team_id: &str, _state: &Arc<Mutex<DaemonState>>) -> Response {
    Response::json(200, &serde_json::json!({ "team_id": team_id, "agents": [] }))
}

fn handle_team_audit(team_id: &str, _state: &Arc<Mutex<DaemonState>>) -> Response {
    Response::json(200, &serde_json::json!({ "team_id": team_id, "audit": "enabled" }))
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
        if line.to_lowercase().starts_with("authorization:") {
            let val = line[14..].trim();
            return Some(val.strip_prefix("Bearer ").or(val.strip_prefix("bearer ")).unwrap_or(val).trim().to_string());
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
}
