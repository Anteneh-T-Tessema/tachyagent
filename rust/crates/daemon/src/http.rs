use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::engine::AgentEngine;
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

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------

/// Start the HTTP daemon on the given address.
pub async fn serve(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(listen_addr).await?;
    eprintln!("Tachy daemon listening on {listen_addr}");
    eprintln!("  Web UI:  http://{listen_addr}");
    eprintln!("  API:     http://{listen_addr}/health");

    loop {
        let (mut stream, _addr) = listener.accept().await?;
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };

            let request = String::from_utf8_lossy(&buf[..n]);
            let response = handle_request(&request, &state);

            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        });
    }
}

fn handle_request(raw: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let (method, path, body) = parse_http_request(raw);

    // Auth check — skip for health and web UI
    if !matches!(path.as_str(), "/" | "/index.html" | "/health") {
        if let Some(required_key) = &state.lock().unwrap_or_else(|e| e.into_inner()).api_key {
            let provided = extract_auth_header(raw);
            match provided {
                Some(key) if key == *required_key => {} // authorized
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
        ("POST", "/api/agents/run") => handle_run_agent(&body, state),
        ("POST", "/api/chat/stream") => handle_chat_stream(&body, state),
        ("POST", "/api/tasks/schedule") => handle_schedule_agent(&body, state),
        _ => json_response(404, &ErrorResponse {
            error: format!("not found: {method} {path}"),
        }),
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

fn handle_run_agent(body: &str, state: &Arc<Mutex<DaemonState>>) -> String {
    let req: RunAgentRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return json_response(400, &ErrorResponse {
                error: format!("invalid request body: {e}"),
            });
        }
    };

    // Create the agent
    let (agent_id, config, governance) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let agent_id = match s.create_agent(&req.template, &req.prompt) {
            Ok(id) => id,
            Err(e) => {
                return json_response(400, &ErrorResponse { error: e });
            }
        };
        let mut config = s.agents.get(&agent_id).unwrap().config.clone();
        // Allow model override from request
        if let Some(model) = &req.model {
            if !model.is_empty() {
                config.template.model = model.clone();
            }
        }
        let governance = s.config.governance.clone();
        (agent_id, config, governance)
    };

    // Run the agent (outside the lock so it doesn't block other requests)
    let result = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        let workspace_root = s.workspace_root.clone();
        AgentEngine::run_agent(
            &agent_id,
            &config,
            &req.prompt,
            &s.registry,
            &governance,
            &s.audit_logger,
            &s.config.intelligence,
            &workspace_root,
        )
    };

    // Update agent state
    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&agent_id) {
            agent.iterations_completed = result.iterations;
            agent.tool_invocations = result.tool_invocations;
            if result.success {
                agent.mark_completed(&result.summary);
            } else {
                agent.mark_failed(&result.summary);
            }
        }
        s.save();
    }

    json_response(200, &RunAgentResponse {
        agent_id: result.agent_id,
        success: result.success,
        iterations: result.iterations,
        tool_invocations: result.tool_invocations,
        summary: result.summary,
    })
}

/// Streaming chat endpoint — returns SSE events as the agent runs.
/// The response is chunked: first a "thinking" event, then the full result.
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

    // Send "thinking" event immediately
    let mut events = vec![
        sse_event("status", &format!("{{\"status\":\"thinking\",\"template\":\"{}\"}}", req.template)),
    ];

    // Create and run agent
    let (agent_id, config, governance) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let agent_id = match s.create_agent(&req.template, &req.prompt) {
            Ok(id) => id,
            Err(e) => {
                events.push(sse_event("error", &format!("{{\"error\":\"{e}\"}}")));
                events.push(sse_event("done", "{}"));
                return sse_response(&events);
            }
        };
        let mut config = s.agents.get(&agent_id).unwrap().config.clone();
        if let Some(model) = &req.model {
            if !model.is_empty() {
                config.template.model = model.clone();
            }
        }
        let governance = s.config.governance.clone();
        (agent_id, config, governance)
    };

    let result = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        let workspace_root = s.workspace_root.clone();
        AgentEngine::run_agent(
            &agent_id,
            &config,
            &req.prompt,
            &s.registry,
            &governance,
            &s.audit_logger,
            &s.config.intelligence,
            &workspace_root,
        )
    };

    // Update and persist
    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&agent_id) {
            agent.iterations_completed = result.iterations;
            agent.tool_invocations = result.tool_invocations;
            if result.success {
                agent.mark_completed(&result.summary);
            } else {
                agent.mark_failed(&result.summary);
            }
        }
        s.save();
    }

    // Stream the result in chunks for better UX
    let summary = &result.summary;
    let chunk_size = 80;
    let chunks: Vec<&str> = summary
        .as_bytes()
        .chunks(chunk_size)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();

    for chunk in &chunks {
        if !chunk.is_empty() {
            let escaped = chunk.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
            events.push(sse_event("token", &format!("{{\"text\":\"{escaped}\"}}")));
        }
    }

    let meta = serde_json::json!({
        "agent_id": result.agent_id,
        "success": result.success,
        "iterations": result.iterations,
        "tool_invocations": result.tool_invocations,
    });
    events.push(sse_event("done", &meta.to_string()));

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
// HTTP helpers (minimal, no framework)
// ---------------------------------------------------------------------------

fn json_response<T: Serialize>(status: u16, body: &T) -> String {
    let json = serde_json::to_string_pretty(body).unwrap_or_else(|_| "{}".to_string());
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {json}",
        json.len()
    )
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

        let raw = "GET /health HTTP/1.1\r\n\r\n";
        let response = handle_request(raw, &state);
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

        let raw = "GET /api/models HTTP/1.1\r\n\r\n";
        let response = handle_request(raw, &state);
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

        let raw = "GET /api/templates HTTP/1.1\r\n\r\n";
        let response = handle_request(raw, &state);
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

        let raw = "POST /api/tasks/schedule HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"template\":\"security-scanner\",\"name\":\"hourly scan\",\"interval_seconds\":3600}";
        let response = handle_request(raw, &state);
        assert!(response.contains("task-1"));

        // Verify task was created
        let raw2 = "GET /api/tasks HTTP/1.1\r\n\r\n";
        let response2 = handle_request(raw2, &state);
        assert!(response2.contains("hourly scan"));

        std::fs::remove_dir_all(root).ok();
    }
}
