//! Distributed worker registry and OpenTelemetry telemetry handlers.

use std::sync::{Arc, Mutex};

use crate::state::DaemonState;
use super::{Response, ErrorResponse};

// ---------------------------------------------------------------------------
// Worker registry
// ---------------------------------------------------------------------------

pub(super) fn handle_list_workers(state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.worker_registry.prune_stale();
    let available = s.worker_registry.available_worker_count();
    let workers = s.worker_registry.list_workers().into_iter().cloned().collect::<Vec<_>>();
    Response::json(200, serde_json::json!({
        "count": workers.len(),
        "available": available,
        "workers": workers,
    }))
}

pub(super) fn handle_register_worker(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let worker: crate::worker_registry::WorkerNode = match serde_json::from_str(body) {
        Ok(w) => w,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid worker registration: {e}") }),
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = s.worker_registry.register(worker);
    Response::json(200, serde_json::json!({ "registered": true, "worker_id": id }))
}

pub(super) fn handle_worker_heartbeat(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    struct Req { worker_id: String, active_tasks: usize }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid heartbeat: {e}") }),
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.worker_registry.heartbeat(&req.worker_id, req.active_tasks) {
        Response::json(200, serde_json::json!({ "ok": true }))
    } else {
        Response::json(404, &ErrorResponse { error: format!("worker not found: {}", req.worker_id) })
    }
}

pub(super) fn handle_deregister_worker(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    struct Req { worker_id: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.worker_registry.deregister(&req.worker_id);
    Response::json(200, serde_json::json!({ "deregistered": true }))
}

// ---------------------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------------------

pub(super) fn handle_telemetry_flush(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.tracer.flush();
    Response::json(200, serde_json::json!({ "flushed": true }))
}

pub(super) fn handle_telemetry_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, serde_json::json!({
        "enabled": s.tracer.is_enabled(),
        "otlp_endpoint": std::env::var("TACHY_OTLP_ENDPOINT").unwrap_or_else(|_| "(not set)".to_string()),
        "service_name": std::env::var("TACHY_SERVICE_NAME").unwrap_or_else(|_| "tachy-daemon".to_string()),
    }))
}
