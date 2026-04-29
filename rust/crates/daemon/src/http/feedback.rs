//! Phase 2 feedback-loop HTTP handlers.
//!
//! Endpoints:
//!  GET  /api/gold-standard/status      — Gold Standard dataset stats
//!  POST /api/gold-standard/extract     — Manually rescan all sessions
//!  GET  /api/adapters                  — List all Expert Adapters
//!  POST /api/adapters                  — Register a new adapter
//!  GET  /api/adapters/{id}             — Get a single adapter
//!  POST /api/adapters/{id}/activate    — Activate an adapter (demotes others)
//!  POST /api/adapters/ab-test          — A/B evaluate two adapters
//!  POST /api/finetune/trigger          — Trigger training from gold standard dataset
//!  GET  /api/finetune/jobs             — List training jobs from InferenceStats

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::state::DaemonState;
use super::{Response, ErrorResponse};

// ---------------------------------------------------------------------------
// Gold Standard endpoints
// ---------------------------------------------------------------------------

pub(crate) fn handle_gold_standard_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let gold_path = s.workspace_root
        .join(".tachy").join("gold_standard").join("dataset.jsonl");
    let store = intelligence::GoldStandardStore::new(&gold_path);
    Response::json(200, store.stats())
}

pub(crate) fn handle_gold_standard_extract(state: &Arc<Mutex<DaemonState>>) -> Response {
    let workspace_root = state.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_root.clone();

    let sessions_dir = workspace_root.join(".tachy").join("sessions");
    let gold_path = workspace_root.join(".tachy").join("gold_standard").join("dataset.jsonl");
    let store = intelligence::GoldStandardStore::new(&gold_path);

    let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
        return Response::json(200, serde_json::json!({
            "extracted": 0,
            "total_entries": store.count(),
            "message": "sessions directory not found"
        }));
    };

    let mut extracted = 0usize;
    let mut scanned = 0usize;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(session) = serde_json::from_str::<serde_json::Value>(&content) {
                extracted += store.append_session(&session);
                scanned += 1;
            }
        }
    }

    state.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish_event("gold_standard_extracted", serde_json::json!({
            "source": "manual_extract",
            "scanned": scanned,
            "new_pairs": extracted,
            "total_entries": store.count(),
        }));

    Response::json(200, serde_json::json!({
        "scanned": scanned,
        "extracted": extracted,
        "total_entries": store.count(),
    }))
}

// ---------------------------------------------------------------------------
// Adapter Registry endpoints
// ---------------------------------------------------------------------------

pub(crate) fn handle_list_adapters(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let adapters = s.adapter_registry.list().to_vec();
    Response::json(200, serde_json::json!({ "count": adapters.len(), "adapters": adapters }))
}

#[derive(Debug, Deserialize)]
struct RegisterAdapterRequest {
    base_model: String,
    domain: String,
    #[serde(default)]
    adapter_path: String,
}

pub(crate) fn handle_register_adapter(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let req: RegisterAdapterRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    if req.base_model.is_empty() || req.domain.is_empty() {
        return Response::json(400, &ErrorResponse { error: "base_model and domain are required".into() });
    }

    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = s.adapter_registry.register(&req.base_model, &req.domain, &req.adapter_path);
    let adapter = s.adapter_registry.get(&id).cloned();

    s.audit_logger.log(&audit::AuditEvent::new(
        "feedback", audit::AuditEventKind::UsageMetering,
        format!("adapter registered: id={id} domain={} model={}", req.domain, req.base_model),
    ));

    Response::json(201, serde_json::json!({ "id": id, "adapter": adapter }))
}

pub(crate) fn handle_get_adapter(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.adapter_registry.get(id).cloned() {
        None => Response::json(404, &ErrorResponse { error: format!("adapter not found: {id}") }),
        Some(a) => Response::json(200, &a),
    }
}

pub(crate) fn handle_activate_adapter(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.adapter_registry.activate(id) {
        Err(e) => Response::json(404, &ErrorResponse { error: e }),
        Ok(()) => {
            s.publish_event("adapter_activated", serde_json::json!({ "adapter_id": id }));
            s.audit_logger.log(&audit::AuditEvent::new(
                "feedback", audit::AuditEventKind::UsageMetering,
                format!("adapter activated: {id}"),
            ));
            Response::json(200, serde_json::json!({ "activated": id }))
        }
    }
}

#[derive(Debug, Deserialize)]
struct ABTestRequest {
    adapter_a: String,
    adapter_b: String,
}

pub(crate) fn handle_ab_test(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let req: ABTestRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let a = match s.adapter_registry.get(&req.adapter_a).cloned() {
        Some(a) => a,
        None => return Response::json(404, &ErrorResponse { error: format!("adapter not found: {}", req.adapter_a) }),
    };
    let b = match s.adapter_registry.get(&req.adapter_b).cloned() {
        Some(b) => b,
        None => return Response::json(404, &ErrorResponse { error: format!("adapter not found: {}", req.adapter_b) }),
    };
    drop(s);

    let result = intelligence::FineTuningBridge::ab_test(&a, &b);

    // Auto-update lift score for winner
    {
        let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = s.adapter_registry.update_lift_score(&result.winner, result.lift);
        s.publish_event("ab_test_complete", serde_json::json!(&result));
    }

    Response::json(200, &result)
}

// ---------------------------------------------------------------------------
// Fine-tuning Bridge endpoints
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FineTuneTriggerRequest {
    base_model: String,
    #[serde(default = "default_adapter_name")]
    adapter_name: String,
}

fn default_adapter_name() -> String { "tachy-lora".to_string() }

pub(crate) fn handle_finetune_trigger(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let req: FineTuneTriggerRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    if req.base_model.is_empty() {
        return Response::json(400, &ErrorResponse { error: "base_model is required".into() });
    }

    let workspace_root = state.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_root.clone();

    let dataset_path = workspace_root.join(".tachy").join("gold_standard").join("dataset.jsonl");

    match intelligence::FineTuningBridge::trigger_from_dataset(&dataset_path, &req.base_model, &req.adapter_name) {
        Err(e) => Response::json(422, &ErrorResponse { error: e }),
        Ok(job) => {
            let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

            // Register a Training-state adapter in the registry for this job.
            let adapter_id = s.adapter_registry.register(&req.base_model, &req.adapter_name, "");

            s.inference_stats.active_training_jobs.push(job.clone());
            s.publish_event("finetune_job_queued", serde_json::json!({
                "job_id": &job.id,
                "adapter_id": adapter_id,
                "base_model": &req.base_model,
                "adapter_name": &req.adapter_name,
            }));
            Response::json(202, serde_json::json!({
                "job_id": job.id,
                "adapter_id": adapter_id,
                "status": job.status,
            }))
        }
    }
}

pub(crate) fn handle_list_finetune_jobs(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let jobs = &s.inference_stats.active_training_jobs;
    Response::json(200, serde_json::json!({ "count": jobs.len(), "jobs": jobs }))
}
