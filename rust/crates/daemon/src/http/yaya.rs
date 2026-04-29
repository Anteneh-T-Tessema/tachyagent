//! Yaya expert-platform integration endpoints.

use std::sync::{Arc, Mutex};

use audit::{AccessResult, Action, AuditEvent, AuditEventKind, AuditSeverity};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::state::DaemonState;
use super::{ErrorResponse, Response};
use super::utils::{extract_user, urlencoding_decode};

fn yaya_base_url() -> String {
    // Default to SmolAgent (port 8100) — the single orchestration gateway.
    // SmolAgent proxies /expert/chat and /experts to FinetuningLLMs internally.
    // Set YAYA_BASE_URL=http://localhost:8000 only to bypass orchestration in dev.
    std::env::var("YAYA_BASE_URL").unwrap_or_else(|_| "http://localhost:8100".to_string())
}

fn yaya_api_key() -> Option<String> {
    std::env::var("YAYA_API_KEY").ok().filter(|value| !value.trim().is_empty())
}

fn auth_user(state: &Arc<Mutex<DaemonState>>, raw: &str, action: Action) -> Result<audit::User, Response> {
    let Some(user) = extract_user(state, raw) else {
        return Err(Response::json(401, &ErrorResponse { error: "unauthorized".to_string() }));
    };
    match audit::check_permission(user.role, action) {
        AccessResult::Allowed => Ok(user),
        AccessResult::Denied { reason } => Err(Response::json(403, &ErrorResponse { error: reason })),
    }
}

fn yaya_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(api_key) = yaya_api_key() {
        if let Ok(value) = HeaderValue::from_str(&api_key) {
            headers.insert("x-api-key", value);
        }
    }
    headers
}

#[derive(Debug, Deserialize, Serialize)]
struct YayaExpertsResponseItem {
    workspace: String,
    subject: String,
    active_version: Option<String>,
    model_path: Option<String>,
    latest_evaluation_passed: Option<bool>,
    latest_evaluation_version: Option<String>,
    latest_trained_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct YayaChatResponse {
    workspace: String,
    subject: String,
    response: String,
    citations: Vec<serde_json::Value>,
    model_type: Option<String>,
    used_fallback: Option<bool>,
    retrieval_mode: Option<String>,
    grounded: Option<bool>,
    expert_version: Option<String>,
    expert_metadata: Option<serde_json::Value>,
    retrieval_preferences: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct YayaChatRequest {
    workspace: String,
    subject: String,
    message: String,
    #[serde(default)]
    execution_context: serde_json::Value,
    #[serde(default)]
    actor: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct YayaTrainingExampleRequest {
    workspace: String,
    subject: String,
    prompt: String,
    answer: String,
    #[serde(default)]
    citations: Vec<serde_json::Value>,
    #[serde(default = "default_true")]
    approved: bool,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default)]
    audit_reference: Option<String>,
    #[serde(default)]
    metadata: serde_json::Value,
}

fn default_true() -> bool { true }
fn default_source() -> String { "tachy".to_string() }

#[derive(Debug, Deserialize, Serialize)]
pub struct YayaRetrievalPreferencesRequest {
    workspace: String,
    subject: String,
    #[serde(default)]
    preferred_sources: Vec<String>,
    #[serde(default)]
    preferred_source_terms: Vec<String>,
}

pub(super) async fn handle_yaya_list_experts(
    query: &str,
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
) -> Response {
    let user = match auth_user(state, raw, Action::RunAgent) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let workspace = query
        .split('&')
        .find_map(|pair| pair.split_once('='))
        .and_then(|(key, value)| (key == "workspace").then(|| urlencoding_decode(value)))
        .unwrap_or_else(|| "default".to_string());

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/experts", yaya_base_url().trim_end_matches('/')))
        .headers(yaya_headers())
        .query(&[("workspace", workspace.clone())])
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => match resp.json::<Vec<YayaExpertsResponseItem>>().await {
            Ok(items) => {
                let state_guard = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state_guard.audit_logger.log(
                    &AuditEvent::new("daemon", AuditEventKind::UserMessage, format!("listed yaya experts for workspace {workspace}"))
                        .with_user(user.id)
                        .with_tool("yaya_expert_catalog")
                        .with_model("yaya")
                );
                Response::json(200, items)
            }
            Err(err) => Response::json(502, &ErrorResponse { error: format!("invalid yaya response: {err}") }),
        },
        Ok(resp) => Response::json(resp.status().as_u16(), &ErrorResponse { error: format!("yaya returned {}", resp.status()) }),
        Err(err) => Response::json(502, &ErrorResponse { error: format!("failed to reach yaya: {err}") }),
    }
}

pub(super) async fn handle_yaya_get_retrieval_preferences(
    query: &str,
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
) -> Response {
    let user = match auth_user(state, raw, Action::ViewAudit) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let mut workspace = "default".to_string();
    let mut subject = None;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            match key {
                "workspace" => workspace = urlencoding_decode(value),
                "subject" => subject = Some(urlencoding_decode(value)),
                _ => {}
            }
        }
    }
    let Some(subject) = subject else {
        return Response::json(400, &ErrorResponse { error: "subject query parameter is required".to_string() });
    };

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/retrieval/preferences", yaya_base_url().trim_end_matches('/')))
        .headers(yaya_headers())
        .query(&[("workspace", workspace.clone()), ("subject", subject.clone())])
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(payload) => {
                let state_guard = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state_guard.audit_logger.log(
                    &AuditEvent::new("daemon", AuditEventKind::UserMessage, format!("inspected yaya retrieval preferences for {workspace}/{subject}"))
                        .with_user(user.id)
                        .with_tool("yaya_retrieval_preferences")
                        .with_model("yaya")
                );
                Response::json(200, payload)
            }
            Err(err) => Response::json(502, &ErrorResponse { error: format!("invalid yaya response: {err}") }),
        },
        Ok(resp) => Response::json(resp.status().as_u16(), &ErrorResponse { error: format!("yaya returned {}", resp.status()) }),
        Err(err) => Response::json(502, &ErrorResponse { error: format!("failed to reach yaya: {err}") }),
    }
}

pub(super) async fn handle_yaya_chat(
    body: &str,
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
) -> Response {
    let user = match auth_user(state, raw, Action::RunAgent) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let req: YayaChatRequest = match serde_json::from_str(body) {
        Ok(req) => req,
        Err(err) => {
            return Response::json(400, &ErrorResponse { error: format!("invalid request: {err}") });
        }
    };

    {
        let state_guard = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state_guard.audit_logger.log(
            &AuditEvent::new("daemon", AuditEventKind::UserMessage, format!("yaya consult requested for {}/{}", req.workspace, req.subject))
                .with_user(user.id.clone())
                .with_tool("yaya_expert_chat")
                .with_model("yaya")
                .with_redacted_payload(req.message.clone())
        );
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/expert/chat", yaya_base_url().trim_end_matches('/')))
        .headers(yaya_headers())
        .json(&req)
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => match resp.json::<YayaChatResponse>().await {
            Ok(payload) => {
                let mut detail = format!(
                    "yaya consult completed for {}/{} grounded={} citations={}",
                    payload.workspace,
                    payload.subject,
                    payload.grounded.unwrap_or(false),
                    payload.citations.len()
                );
                if payload.used_fallback.unwrap_or(false) {
                    detail.push_str(" fallback=true");
                }
                if let Some(preferences) = &payload.retrieval_preferences {
                    if let Some(strategy) = preferences.get("strategy").and_then(|value| value.as_str()) {
                        detail.push_str(&format!(" strategy={strategy}"));
                    }
                    if let Some(sources) = preferences.get("preferred_sources").and_then(|value| value.as_array()) {
                        let sources = sources
                            .iter()
                            .filter_map(|item| item.as_str())
                            .collect::<Vec<_>>();
                        if !sources.is_empty() {
                            detail.push_str(&format!(" sources={}", sources.join("|")));
                        }
                    }
                    if let Some(terms) = preferences.get("preferred_source_terms").and_then(|value| value.as_array()) {
                        let terms = terms
                            .iter()
                            .filter_map(|item| item.as_str())
                            .collect::<Vec<_>>();
                        if !terms.is_empty() {
                            detail.push_str(&format!(" terms={}", terms.join("|")));
                        }
                    }
                }
                let state_guard = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut event = AuditEvent::new("daemon", AuditEventKind::AssistantMessage, detail)
                    .with_user(user.id)
                    .with_tool("yaya_expert_chat")
                    .with_model("yaya");
                if payload.used_fallback.unwrap_or(false) {
                    event = event.with_severity(AuditSeverity::Warning);
                }
                state_guard.audit_logger.log(&event);
                Response::json(200, payload)
            }
            Err(err) => Response::json(502, &ErrorResponse { error: format!("invalid yaya response: {err}") }),
        },
        Ok(resp) => Response::json(resp.status().as_u16(), &ErrorResponse { error: format!("yaya returned {}", resp.status()) }),
        Err(err) => Response::json(502, &ErrorResponse { error: format!("failed to reach yaya: {err}") }),
    }
}

pub(super) async fn handle_yaya_set_retrieval_preferences(
    body: &str,
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
) -> Response {
    let user = match auth_user(state, raw, Action::ManageGovernance) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let req: YayaRetrievalPreferencesRequest = match serde_json::from_str(body) {
        Ok(req) => req,
        Err(err) => {
            return Response::json(400, &ErrorResponse { error: format!("invalid request: {err}") });
        }
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/retrieval/preferences", yaya_base_url().trim_end_matches('/')))
        .headers(yaya_headers())
        .json(&req)
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(payload) => {
                let state_guard = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state_guard.audit_logger.log(
                    &AuditEvent::new(
                        "daemon",
                        AuditEventKind::ToolResult,
                        format!("updated yaya retrieval preferences for {}/{}", req.workspace, req.subject),
                    )
                    .with_user(user.id)
                    .with_tool("yaya_retrieval_preferences")
                    .with_model("yaya")
                );
                Response::json(200, payload)
            }
            Err(err) => Response::json(502, &ErrorResponse { error: format!("invalid yaya response: {err}") }),
        },
        Ok(resp) => Response::json(resp.status().as_u16(), &ErrorResponse { error: format!("yaya returned {}", resp.status()) }),
        Err(err) => Response::json(502, &ErrorResponse { error: format!("failed to reach yaya: {err}") }),
    }
}

pub(super) async fn handle_yaya_submit_training_example(
    body: &str,
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
) -> Response {
    let user = match auth_user(state, raw, Action::ManageGovernance) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let req: YayaTrainingExampleRequest = match serde_json::from_str(body) {
        Ok(req) => req,
        Err(err) => {
            return Response::json(400, &ErrorResponse { error: format!("invalid request: {err}") });
        }
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/training/examples", yaya_base_url().trim_end_matches('/')))
        .headers(yaya_headers())
        .json(&serde_json::json!({
            "workspace": req.workspace,
            "subject": req.subject,
            "examples": [req],
        }))
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(payload) => {
                let state_guard = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state_guard.audit_logger.log(
                    &AuditEvent::new("daemon", AuditEventKind::ToolResult, "submitted approved training example to yaya")
                        .with_user(user.id)
                        .with_tool("yaya_training_feedback")
                        .with_model("yaya")
                );
                Response::json(200, payload)
            }
            Err(err) => Response::json(502, &ErrorResponse { error: format!("invalid yaya response: {err}") }),
        },
        Ok(resp) => Response::json(resp.status().as_u16(), &ErrorResponse { error: format!("yaya returned {}", resp.status()) }),
        Err(err) => Response::json(502, &ErrorResponse { error: format!("failed to reach yaya: {err}") }),
    }
}
