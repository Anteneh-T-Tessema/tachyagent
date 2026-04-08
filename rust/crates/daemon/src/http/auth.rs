//! Authentication handlers: SSO/SAML, `OAuth2`, license, billing, usage.

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::state::DaemonState;
use super::{Response, ErrorResponse};

// ---------------------------------------------------------------------------
// SSO / SAML
// ---------------------------------------------------------------------------

pub(super) fn handle_sso_login(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !s.sso_manager.is_enabled() {
        return Response::json(400, &ErrorResponse { error: "SSO disabled".to_string() });
    }
    Response::Full {
        status: 302,
        content_type: "text/plain".to_string(),
        body: format!("Redirecting to {}", s.sso_manager.build_login_url(Some("/"))),
    }
}

pub(super) fn handle_sso_callback(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let saml = body.split("SAMLResponse=").nth(1)
        .and_then(|s| s.split('&').next())
        .unwrap_or("");
    let mut user_store = std::mem::take(&mut s.user_store);
    let res = s.sso_manager.process_callback(saml, &mut user_store);
    s.user_store = user_store;
    match res {
        Ok(sess) => Response::json(200, serde_json::json!({ "token": sess.token, "email": sess.email })),
        Err(e) => Response::json(401, &ErrorResponse { error: e }),
    }
}

pub(super) fn handle_sso_logout(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { token: String }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { token: String::new() });
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.sso_manager.invalidate_session(&req.token);
    Response::json(200, serde_json::json!({ "ok": true }))
}

pub(super) fn handle_sso_sessions(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, s.sso_manager.active_sessions())
}

pub(super) fn handle_sso_config(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let config: audit::SsoConfig = match serde_json::from_str(body) {
        Ok(c) => c,
        Err(_) => return Response::json(400, &ErrorResponse { error: "invalid config".to_string() }),
    };
    s.sso_manager = audit::SsoManager::new(config);
    Response::json(200, serde_json::json!({ "status": "updated" }))
}

// ---------------------------------------------------------------------------
// OAuth2 (Google / GitHub)
// ---------------------------------------------------------------------------

/// GET /api/auth/oauth/{provider}/login → redirect to authorization URL.
pub(super) fn handle_oauth_login(provider_str: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    use audit::{OAuthClientConfig, OAuthProvider};
    let provider = match OAuthProvider::from_str(provider_str) {
        Some(p) => p,
        None => return Response::json(400, &ErrorResponse { error: format!("unknown provider: {provider_str}") }),
    };
    let config = match OAuthClientConfig::from_env(&provider) {
        Some(c) => c,
        None => return Response::json(503, &ErrorResponse {
            error: format!(
                "OAuth2 not configured for {provider_str}. Set TACHY_{0}_CLIENT_ID and TACHY_{0}_CLIENT_SECRET.",
                provider_str.to_uppercase()
            ),
        }),
    };
    let (url, _state_token) = {
        let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        s.oauth_manager.authorization_url(&config)
    };
    Response::Full { status: 302, content_type: "text/plain".to_string(), body: format!("Location: {url}\r\n") }
}

/// GET /api/auth/oauth/{provider}/callback?code=...&state=...
pub(super) fn handle_oauth_callback(provider_str: &str, query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    use audit::{OAuthClientConfig, OAuthProvider};
    let provider = match OAuthProvider::from_str(provider_str) {
        Some(p) => p,
        None => return Response::json(400, &ErrorResponse { error: format!("unknown provider: {provider_str}") }),
    };
    let config = match OAuthClientConfig::from_env(&provider) {
        Some(c) => c,
        None => return Response::json(503, &ErrorResponse { error: "OAuth2 not configured".to_string() }),
    };
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
        let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        s.oauth_manager.handle_callback(&config, &code, &oauth_state)
    };
    match result {
        Ok(session) => Response::json(200, serde_json::json!({
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

pub(super) fn handle_oauth_logout(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Req { token: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Response::json(400, &ErrorResponse { error: "missing token".to_string() }),
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.oauth_manager.revoke_session(&req.token);
    Response::json(200, serde_json::json!({ "revoked": true }))
}

pub(super) fn handle_oauth_sessions(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, serde_json::json!({ "active_sessions": s.oauth_manager.active_session_count() }))
}

// ---------------------------------------------------------------------------
// License + billing
// ---------------------------------------------------------------------------

pub(super) fn handle_license_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let license = audit::LicenseFile::load_or_create(&s.workspace_root.join(".tachy"));
    Response::json(200, serde_json::json!({
        "status": license.status().display(),
        "active": license.status().is_active(),
    }))
}

pub(super) fn handle_billing_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match &s.billing {
        Some(b) => Response::json(200, b.status()),
        None => Response::json(200, serde_json::json!({ "enabled": false })),
    }
}

pub(super) fn handle_license_activate(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { key: String, secret: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    if req.key.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "key is required".to_string() });
    }
    let tachy_dir = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).workspace_root.join(".tachy");
    let mut license = audit::LicenseFile::load_or_create(&tachy_dir);
    match license.activate(&req.key, &req.secret) {
        Ok(data) => {
            if let Err(e) = license.save(&tachy_dir) {
                return Response::json(500, &ErrorResponse { error: format!("activation succeeded but save failed: {e}") });
            }
            Response::json(200, serde_json::json!({
                "status": "activated",
                "tier": format!("{:?}", data.tier),
                "expires_at": data.expires_at,
            }))
        }
        Err(e) => Response::json(400, &ErrorResponse { error: e }),
    }
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

pub(super) fn handle_usage(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    Response::json(200, serde_json::json!({
        "users": users,
        "totals": { "tokens": total_tokens, "tool_invocations": total_tools, "agent_runs": total_runs }
    }))
}
