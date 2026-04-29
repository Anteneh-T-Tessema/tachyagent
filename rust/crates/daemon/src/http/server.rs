//! TCP server loop and simple inline handlers that don't warrant a sub-module.

use std::sync::{Arc, Mutex};

use audit::TieredRateLimiter;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::router::handle_request;
use super::types::{
    AgentInfo, ErrorResponse, HealthResponse, ModelInfo, Response, TaskInfo, TemplateInfo,
};
use crate::state::DaemonState;

// ── Server loop ───────────────────────────────────────────────────────────────

pub async fn serve(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(listen_addr).await?;
    let rate_limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
    // TACHY_ALLOWED_ORIGINS: comma-separated list of allowed CORS origins.
    // Defaults to "*" for local dev. Set to "https://app.example.com" in prod.
    let allowed_origin =
        Arc::new(std::env::var("TACHY_ALLOWED_ORIGINS").unwrap_or_else(|_| "*".to_string()));

    eprintln!("Tachy daemon listening on {listen_addr}");

    // Phase 24: Background task to clean up old vision snapshots (every hour)
    let cleanup_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            let s = cleanup_state.lock().unwrap();
            s.clean_vision_cache();
        }
    });

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
                Response::Full {
                    status,
                    content_type,
                    body,
                    extra_headers,
                } => {
                    let extra = extra_headers
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}\r\n"))
                        .collect::<String>();
                    let header = format!(
                        "HTTP/1.1 {status} OK\r\n\
                         Content-Type: {content_type}\r\n\
                         Content-Length: {}\r\n\
                         Access-Control-Allow-Origin: {origin}\r\n\
                         Access-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS\r\n\
                         Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
                         {extra}\
                         Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                }
                Response::Stream {
                    status,
                    content_type,
                    mut rx,
                } => {
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

// ── Simple inline handlers ────────────────────────────────────────────────────

pub fn handle_inference_stats(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, &s.inference_stats)
}

pub fn handle_pull_model(body: &str, _state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct PullRequest {
        model: String,
    }
    let req: PullRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: "invalid request".to_string(),
                },
            )
        }
    };
    let model = req.model.clone();
    tokio::spawn(async move {
        let _ = backend::pull_model(&model);
    });
    Response::json(
        202,
        serde_json::json!({ "message": format!("Pulling {} in background", req.model) }),
    )
}

pub fn handle_health(state: &Arc<Mutex<DaemonState>>) -> HealthResponse {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let active_swarms = s.swarm.orchestrator.lock().unwrap().active_runs();
    let cache_hits = s.semantic_cache.hits();

    HealthResponse {
        status: "ok",
        models: s.registry.list_models().len(),
        agents: s.agents.len(),
        active_swarms,
        tasks: s.scheduler.list_tasks().len(),
        workspace: s.workspace_root.to_string_lossy().to_string(),
        cache_hits,
    }
}

pub fn handle_list_models(state: &Arc<Mutex<DaemonState>>) -> Vec<ModelInfo> {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

pub fn handle_list_templates(state: &Arc<Mutex<DaemonState>>) -> Vec<TemplateInfo> {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

pub fn handle_list_agents(state: &Arc<Mutex<DaemonState>>) -> Vec<AgentInfo> {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

pub fn handle_list_tasks(state: &Arc<Mutex<DaemonState>>) -> Vec<TaskInfo> {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
