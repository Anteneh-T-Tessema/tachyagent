//! Shared utilities used across HTTP handler sub-modules:
//! timestamps, URL decoding, RBAC gating, auth extraction, request parsing.

use std::sync::{Arc, Mutex};

use super::types::{ErrorResponse, Response};
use crate::state::DaemonState;

// ── Timestamps ────────────────────────────────────────────────────────────────

pub fn chrono_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn chrono_now_str() -> String {
    format!("{}s", chrono_now_secs())
}

// ── String helpers ────────────────────────────────────────────────────────────

pub fn urlencoding_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
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

pub fn truncate_completion(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * 4;
    if text.len() <= max_chars {
        text.to_string()
    } else {
        text[..max_chars].to_string()
    }
}

pub fn csv_response(body: &str, _filename: &str) -> Response {
    Response::Full {
        status: 200,
        content_type: "text/csv".to_string(),
        body: body.to_string().into_bytes(),
        extra_headers: Vec::new(),
    }
}

// ── Auth / RBAC ───────────────────────────────────────────────────────────────

pub fn gate_action<F>(
    state: &Arc<Mutex<DaemonState>>,
    raw: &str,
    action: audit::Action,
    f: F,
) -> Response
where
    F: FnOnce(&audit::User) -> Response,
{
    let user = match extract_user(state, raw) {
        Some(u) => u,
        None => {
            return Response::json(
                401,
                &ErrorResponse {
                    error: "unauthorized".to_string(),
                },
            )
        }
    };
    match audit::check_permission(user.role, action) {
        audit::AccessResult::Allowed => f(&user),
        audit::AccessResult::Denied { reason } => {
            Response::json(403, &ErrorResponse { error: reason })
        }
    }
}

pub fn extract_user(state: &Arc<Mutex<DaemonState>>, raw: &str) -> Option<audit::User> {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let auth = extract_auth_header(raw)?;

    // Check for explicit team header
    let team_header = extract_header(raw, "X-Tachy-Team");

    // 1. Check UserStore (API Key)
    if let Some(user) = s
        .identity
        .user_store
        .authenticate(&audit::hash_api_key(&auth))
    {
        let mut u = user.clone();
        u.active_team_id = team_header.or(u.active_team_id);
        return Some(u);
    }

    // 2. Check SSO
    if let Some(session) = s.identity.sso_manager.validate_session(&auth) {
        if let Some(user) = s.identity.user_store.users.get(&session.user_id) {
            let mut u = user.clone();
            u.active_team_id = team_header.or(u.active_team_id);
            return Some(u);
        }
    }

    // 3. Check SaaS (JWT)
    if let Some(ref commerce) = s.commerce.saas {
        if let Ok(claims) = commerce.validate_jwt(&auth) {
            if let Some(user) = s.identity.user_store.users.get(&claims.email) {
                let mut u = user.clone();
                u.active_team_id = Some(claims.tenant_id);
                return Some(u);
            }
        }
    }

    None
}

pub fn extract_header(raw: &str, name: &str) -> Option<String> {
    let target = format!("{}:", name.to_lowercase());
    for line in raw.lines() {
        if line.to_lowercase().starts_with(&target) {
            return Some(line[target.len()..].trim().to_string());
        }
    }
    None
}

pub fn extract_auth_header(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("authorization:") {
            let val = line[14..].trim();
            return Some(
                val.strip_prefix("Bearer ")
                    .or(val.strip_prefix("bearer "))
                    .unwrap_or(val)
                    .trim()
                    .to_string(),
            );
        }
        if lower.starts_with("x-sso-token:") {
            return Some(line[12..].trim().to_string());
        }
    }
    None
}

// ── HTTP parsing ──────────────────────────────────────────────────────────────

pub fn parse_http_request(raw: &str) -> (String, String, String) {
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
