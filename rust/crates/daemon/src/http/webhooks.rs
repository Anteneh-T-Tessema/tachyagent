//! Webhook registration, listing, and HMAC-SHA256 signature verification.

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use super::{ErrorResponse, Response};
use crate::state::DaemonState;

pub(crate) fn handle_list_webhooks(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(
        200,
        serde_json::json!({ "webhooks": s.connectivity.webhooks }),
    )
}

pub(crate) fn handle_register_webhook(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        url: String,
        #[serde(default)]
        events: Vec<String>,
        #[serde(default = "bool_true")]
        enabled: bool,
        secret: Option<String>,
    }
    fn bool_true() -> bool {
        true
    }

    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid request: {e}"),
                },
            )
        }
    };
    if req.url.trim().is_empty() {
        return Response::json(
            400,
            &ErrorResponse {
                error: "url is required".to_string(),
            },
        );
    }
    if !req.url.starts_with("http://") && !req.url.starts_with("https://") {
        return Response::json(
            400,
            &ErrorResponse {
                error: "url must start with http:// or https://".to_string(),
            },
        );
    }
    let events = if req.events.is_empty() {
        vec!["*".to_string()]
    } else {
        req.events
    };
    let signed = req.secret.is_some();
    let webhook = crate::state::WebhookConfig {
        url: req.url,
        events,
        enabled: req.enabled,
        secret: req.secret,
    };
    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    s.connectivity.webhooks.push(webhook.clone());
    s.save();
    Response::json(
        201,
        serde_json::json!({
            "ok": true,
            "webhook": webhook,
            "signed": signed,
            "note": if signed { "Outbound payloads will include X-Tachy-Signature header" } else { "No signing secret configured" },
        }),
    )
}

pub(crate) fn handle_verify_webhook_signature(
    body: &str,
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
) -> Response {
    let sig_header = raw
        .lines()
        .find(|l| l.to_lowercase().starts_with("x-tachy-signature:"))
        .and_then(|l| l.split_once(':').map(|x| x.1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    #[derive(Deserialize)]
    struct Req {
        webhook_url: String,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: "missing webhook_url".to_string(),
                },
            )
        }
    };
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.verify_webhook_signature(&req.webhook_url, body.as_bytes(), &sig_header) {
        Ok(()) => Response::json(200, serde_json::json!({ "valid": true })),
        Err(e) => Response::json(401, serde_json::json!({ "valid": false, "reason": e })),
    }
}
