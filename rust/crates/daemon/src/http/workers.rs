//! Distributed worker registry and OpenTelemetry telemetry handlers.

use std::sync::{Arc, Mutex};

use super::{ErrorResponse, Response};
use crate::state::DaemonState;

// ---------------------------------------------------------------------------
// Worker registry
// ---------------------------------------------------------------------------

pub(crate) fn handle_list_workers(state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    s.swarm.worker_registry.prune_stale();
    let available = s.swarm.worker_registry.available_worker_count();
    let workers = s
        .swarm
        .worker_registry
        .list_workers()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    Response::json(
        200,
        serde_json::json!({
            "count": workers.len(),
            "available": available,
            "workers": workers,
        }),
    )
}

pub(crate) fn handle_register_worker(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let worker: crate::worker_registry::WorkerNode = match serde_json::from_str(body) {
        Ok(w) => w,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: e.to_string(),
                },
            )
        }
    };
    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = s.swarm.worker_registry.register(worker);
    Response::json(201, serde_json::json!({ "id": id }))
}

pub(crate) fn handle_worker_heartbeat(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    struct Req {
        worker_id: String,
        active_tasks: usize,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: e.to_string(),
                },
            )
        }
    };
    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.swarm
        .worker_registry
        .heartbeat(&req.worker_id, req.active_tasks)
    {
        Response::json(200, serde_json::json!({ "ok": true }))
    } else {
        Response::json(
            404,
            &ErrorResponse {
                error: "worker not found".to_string(),
            },
        )
    }
}

pub(crate) fn handle_deregister_worker(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    struct Req {
        worker_id: String,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: e.to_string(),
                },
            )
        }
    };
    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    s.swarm.worker_registry.deregister(&req.worker_id);
    Response::json(200, serde_json::json!({ "deregistered": true }))
}

// ---------------------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------------------

pub(crate) fn handle_telemetry_flush(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    s.tracer.flush();
    Response::json(200, serde_json::json!({ "flushed": true }))
}

pub(crate) fn handle_telemetry_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(
        200,
        serde_json::json!({
            "enabled": s.tracer.is_enabled(),
            "otlp_endpoint": std::env::var("TACHY_OTLP_ENDPOINT").unwrap_or_else(|_| "(not set)".to_string()),
            "service_name": std::env::var("TACHY_SERVICE_NAME").unwrap_or_else(|_| "tachy-daemon".to_string()),
        }),
    )
}
