use std::fs;
use std::sync::{Arc, Mutex};

use super::types::{ErrorResponse, Response};
use crate::state::DaemonState;

/// Handle fetching a visual snapshot by its ID (filename).
pub fn handle_get_snapshot(snapshot_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap();
    let vision_dir = s.workspace_root.join(".tachy").join("vision");

    // Safety check: ensure the ID is just a filename, no path traversal
    if snapshot_id.contains('/') || snapshot_id.contains('\\') || snapshot_id.contains("..") {
        return Response::json(
            400,
            &ErrorResponse {
                error: "invalid snapshot id".to_string(),
            },
        );
    }

    let path = vision_dir.join(snapshot_id);
    if !path.exists() {
        return Response::json(
            404,
            &ErrorResponse {
                error: "snapshot not found".to_string(),
            },
        );
    }

    match fs::read(&path) {
        Ok(bytes) => Response::Full {
            status: 200,
            content_type: "image/png".to_string(),
            body: bytes,
            extra_headers: Vec::new(),
        },
        Err(e) => Response::json(
            500,
            &ErrorResponse {
                error: format!("failed to read snapshot: {e}"),
            },
        ),
    }
}
