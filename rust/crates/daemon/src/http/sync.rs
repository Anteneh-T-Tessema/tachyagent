use crate::http::types::Response;
use crate::state::DaemonState;
use platform::sync::StateSnapshot;
use std::sync::{Arc, Mutex};

/// POST /api/sync/receive
/// Receive and apply a sovereign state snapshot from a peer.
pub async fn handle_receive_sync(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let snapshot: StateSnapshot = match serde_json::from_str(body) {
        Ok(s) => s,
        Err(e) => {
            return Response::json(
                400,
                serde_json::json!({ "error": "Invalid snapshot", "detail": e.to_string() }),
            )
        }
    };

    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let sync_manager = match &s.connectivity.sync_manager {
        Some(sm) => sm,
        None => {
            return Response::json(
                501,
                serde_json::json!({ "error": "Sync not configured on this node" }),
            )
        }
    };

    match sync_manager.restore_snapshot(&snapshot) {
        Ok(()) => {
            // Note: Full reload requires careful state re-init, handled by daemon restart or dynamic reload
            Response::json(200, serde_json::json!({ "status": "State synchronized" }))
        }
        Err(e) => Response::json(
            500,
            serde_json::json!({ "error": "Sync failed", "detail": e.to_string() }),
        ),
    }
}

/// POST /api/sync/beam
/// Trigger a state beam to a target peer.
pub async fn handle_trigger_beam(query_str: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let target_url = query_str
        .split("target=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .unwrap_or("");
    if target_url.is_empty() {
        return Response::json(
            400,
            serde_json::json!({ "error": "Missing target parameter" }),
        );
    }

    let sync_manager = {
        let s = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &s.connectivity.sync_manager {
            Some(sm) => sm.clone(),
            None => {
                return Response::json(501, serde_json::json!({ "error": "Sync not configured" }))
            }
        }
    };

    match sync_manager.beam_to(target_url).await {
        Ok(()) => Response::json(200, serde_json::json!({ "status": "Beam successful" })),
        Err(e) => Response::json(
            500,
            serde_json::json!({ "error": "Beam failed", "detail": e.to_string() }),
        ),
    }
}

/// GET /api/sync/pulse
pub fn handle_sync_pulse(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let device_id = s
        .connectivity
        .sync_manager
        .as_ref()
        .map(platform::sync::SyncManager::device_id)
        .unwrap_or("unknown");
    Response::json(
        200,
        serde_json::json!({
            "device_id": device_id,
            "status": "active",
        }),
    )
}
