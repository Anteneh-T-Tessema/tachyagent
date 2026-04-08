//! Agent CRUD, streaming completion, and one-shot prompt handlers.

use std::sync::{Arc, Mutex};

use audit::sanitize_prompt;
use serde::Deserialize;

use crate::engine::AgentEngine;
use crate::state::DaemonState;
use super::{Response, ErrorResponse, AgentInfo, truncate_completion};

pub(super) fn handle_get_agent(agent_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match s.agents.get(agent_id) {
        Some(a) => Response::json(200, &AgentInfo {
            id: a.id.clone(),
            template: a.config.template.name.clone(),
            status: format!("{:?}", a.status).to_lowercase(),
            iterations: a.iterations_completed,
            tool_invocations: a.tool_invocations,
            summary: a.result_summary.clone(),
        }),
        None => Response::json(404, &ErrorResponse { error: format!("agent not found: {agent_id}") }),
    }
}

pub(super) fn handle_delete_agent(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
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

pub(super) fn handle_cancel_agent(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
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

pub(super) fn handle_run_agent(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct WebhookTrigger {
        source: Option<String>,
        event: Option<String>,
        template: Option<String>,
        prompt: Option<String>,
        payload: Option<serde_json::Value>,
    }
    let trigger: WebhookTrigger = match serde_json::from_str(body) {
        Ok(t) => t,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid webhook body: {e}") }),
    };
    let template = trigger.template.as_deref().unwrap_or("chat");
    let prompt = sanitize_prompt(
        &trigger.prompt.unwrap_or_else(|| "Analyze this event.".to_string()),
        50_000,
    );

    let (agent_id, config, governance) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let agent_id = match s.create_agent(template, &prompt) {
            Ok(id) => id,
            Err(e) => return Response::json(400, &ErrorResponse { error: e }),
        };
        let config = match s.agents.get(&agent_id) {
            Some(a) => a.config.clone(),
            None => return Response::json(500, &ErrorResponse { error: "agent not found".to_string() }),
        };
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
            let r = AgentEngine::run_agent(
                &bg_agent_id, &config, &prompt, &s.registry, &governance,
                &s.audit_logger, &s.config.intelligence, &s.workspace_root,
                Some(s.file_locks.clone()), Some(Arc::clone(&bg_state)),
            );
            (r, tracer, model)
        };
        let duration_ms = t0.elapsed().as_millis() as u64;
        crate::telemetry::record_agent_run(
            &tracer, &bg_agent_id, &model, &config.template.name,
            result.success, result.iterations, result.tool_invocations, duration_ms,
        );
        let mut s = bg_state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent) = s.agents.get_mut(&bg_agent_id) {
            let stored_summary = truncate_completion(&result.summary, 1_000);
            if result.success { agent.mark_completed(&stored_summary); } else { agent.mark_failed(&stored_summary); }
        }
        s.publish_event("agent_run_complete", serde_json::json!({
            "agent_id": bg_agent_id,
            "success": result.success,
            "iterations": result.iterations,
            "tool_invocations": result.tool_invocations,
            "duration_ms": duration_ms,
        }));
        s.save();
    });

    Response::json(202, &serde_json::json!({ "agent_id": agent_id, "status": "running" }))
}

pub(super) async fn handle_complete_stream(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Req { prefix: String, suffix: Option<String>, model: Option<String>, max_tokens: Option<u32> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
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
        if let Some(err) = err_msg {
            let _ = tx.send(format!("data: {{\"error\":\"{err}\"}}\n\n")).await;
            return;
        }
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
                        _ => {}
                    }
                }
            });
            let _ = ollama_backend.send_streaming_generate(backend::OllamaGenerateRequest {
                model: ollama_backend.model().to_string(),
                prompt: req.prefix.clone(),
                stream: true,
                raw: false,
                options: None,
            }).await;
        }
        let _ = tx.send("data: [DONE]\n\n".to_string()).await;
    });
    response
}

pub(super) async fn handle_chat_stream(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Req { messages: Vec<serde_json::Value>, model: Option<String>, max_tokens: Option<u32> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
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
        if let Some(err) = err_msg {
            let _ = tx.send(format!("data: {{\"error\":\"{err}\"}}\n\n")).await;
            return;
        }
        if let Some(mut ollama_backend) = v_backend {
            let (t_tx, mut t_rx) = tokio::sync::mpsc::channel(100);
            ollama_backend.set_token_tx(t_tx);
            let tx_inner = tx.clone();
            tokio::spawn(async move {
                while let Some(event) = t_rx.recv().await {
                    if let backend::BackendEvent::Text(t) = event {
                        let _ = tx_inner.send(format!("data: {{\"text\":\"{}\"}}\n\n", t.replace('\"', "\\\""))).await;
                    }
                }
            });
            let prompt = req.messages.iter()
                .filter_map(|m| m["content"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join("\n");
            let _ = ollama_backend.send_streaming_generate(backend::OllamaGenerateRequest {
                model: ollama_backend.model().to_string(),
                prompt,
                stream: true,
                raw: false,
                options: None,
            }).await;
        }
        let _ = tx.send("data: [DONE]\n\n".to_string()).await;
    });
    response
}

pub(super) async fn handle_complete(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        prompt: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default = "default_max_tokens")]
        max_tokens: usize,
    }
    fn default_max_tokens() -> usize { 2048 }

    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    if req.prompt.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "prompt must not be empty".to_string() });
    }
    let prompt = sanitize_prompt(&req.prompt, 50_000);
    let model = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        req.model.unwrap_or_else(|| s.config.default_model.clone())
    };
    let model_clone = model.clone();
    let mut ollama = match backend::OllamaBackend::new(model_clone, "http://localhost:11434".to_string(), false) {
        Ok(b) => b,
        Err(e) => return Response::json(502, &ErrorResponse { error: format!("backend unavailable: {e}") }),
    };
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
    let (gen_result, completion) = tokio::join!(
        gen_fut,
        async {
            let mut buf = String::new();
            while let Some(ev) = t_rx.recv().await {
                if let backend::BackendEvent::Text(t) = ev { buf.push_str(&t); }
            }
            buf
        }
    );
    let _ = gen_result.as_ref().map(|(_, m)| {
        if let Ok(mut s) = state.lock() {
            s.inference_stats.record(m.ttft_ms, m.tokens_per_sec, m.total_tokens);
        }
    });
    let text = if completion.is_empty() {
        gen_result.ok().map(|(events, _)| {
            events.into_iter().filter_map(|e| {
                let s = format!("{e:?}");
                if s.starts_with("TextDelta(") { Some(s[10..s.len()-1].to_string()) } else { None }
            }).collect::<String>()
        }).unwrap_or_default()
    } else {
        completion
    };
    Response::json(200, &serde_json::json!({ "completion": text, "model": model }))
}

pub(super) fn handle_prompt_oneshot(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { prompt: String, model: Option<String>, session_id: Option<String> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    let prompt = sanitize_prompt(&req.prompt, 50_000);
    let (registry, governance, intel_cfg, workspace_root, mut template) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        let tmpl = s.config.agent_templates.first().cloned()
            .unwrap_or_else(platform::AgentTemplate::chat_assistant);
        (s.registry.clone(), s.config.governance.clone(), s.config.intelligence.clone(), s.workspace_root.clone(), tmpl)
    };
    let audit_logger = audit::AuditLogger::new();
    if let Some(m) = &req.model { template.model = m.clone(); }
    let session_id = req.session_id.unwrap_or_else(|| {
        format!("cmp-{}", template.model.replace(|c: char| !c.is_alphanumeric(), "_"))
    });
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
