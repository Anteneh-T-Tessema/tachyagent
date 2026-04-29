//! Governance: policy, patch approval, teams, conversations, marketplace, cloud jobs, scheduling.

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::state::DaemonState;
use super::{Response, ErrorResponse, chrono_now_secs, chrono_now_str, csv_response};

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

pub(crate) fn handle_get_policy(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let policy_path = s.workspace_root.join("tachy-policy.yaml");
    let pf = audit::PolicyFile::load(&policy_path).unwrap_or_else(|_| audit::PolicyFile::enterprise_default());
    Response::json(200, &pf)
}

pub(crate) fn handle_set_policy(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let pf: audit::PolicyFile = match serde_json::from_str(body) {
        Ok(p) => p,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid policy JSON: {e}") }),
    };
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let policy_path = s.workspace_root.join("tachy-policy.yaml");
    match pf.save(&policy_path) {
        Ok(()) => Response::json(200, serde_json::json!({ "saved": policy_path.display().to_string() })),
        Err(e) => Response::json(500, &ErrorResponse { error: format!("save failed: {e}") }),
    }
}

pub(crate) fn handle_get_mission_feed(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let feed = s.swarm.mission_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, &*feed)
}

// ---------------------------------------------------------------------------
// Patch approvals
// ---------------------------------------------------------------------------

pub(crate) fn handle_list_pending_approvals(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, serde_json::json!({ "pending": s.pending_patches }))
}

pub(crate) fn handle_approve_patch(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { patch_id: String, approved: bool }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Response::json(400, &ErrorResponse { error: "invalid request".to_string() }),
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if req.approved {
        match s.approve_patch(&req.patch_id) {
            Ok(path) => Response::json(200, serde_json::json!({ "status": "approved", "file": path })),
            Err(e) => Response::json(400, &ErrorResponse { error: e }),
        }
    } else {
        match s.reject_patch(&req.patch_id) {
            Ok(path) => Response::json(200, serde_json::json!({ "status": "rejected", "file": path })),
            Err(e) => Response::json(400, &ErrorResponse { error: e }),
        }
    }
}

pub(crate) fn handle_list_file_locks(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, serde_json::json!({ "locks": s.file_locks.list_locks() }))
}

// ---------------------------------------------------------------------------
// Task scheduling
// ---------------------------------------------------------------------------

pub(crate) fn handle_schedule_task(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { template: String, name: String, #[serde(default)] interval_seconds: Option<u64> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    if req.template.trim().is_empty() || req.name.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "template and name are required".to_string() });
    }
    let rule = match req.interval_seconds {
        Some(secs) if secs > 0 => platform::ScheduleRule::Interval { seconds: secs },
        _ => platform::ScheduleRule::Once,
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.schedule_agent(&req.template, rule, &req.name) {
        Ok(task_id) => Response::json(201, serde_json::json!({ "task_id": task_id, "name": req.name })),
        Err(e) => Response::json(400, &ErrorResponse { error: e }),
    }
}

// ---------------------------------------------------------------------------
// Teams
// ---------------------------------------------------------------------------

pub(crate) fn handle_list_teams(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let teams: Vec<&crate::teams::Team> = s.team_manager.teams().values().collect();
    Response::json(200, &teams)
}

pub(crate) fn handle_get_team(team_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.team_manager.teams().get(team_id) {
        Some(team) => Response::json(200, team),
        None => Response::json(404, &ErrorResponse { error: format!("team not found: {team_id}") }),
    }
}

pub(crate) fn handle_team_agents(team_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.team_manager.teams().get(team_id).is_none() {
        return Response::json(404, &ErrorResponse { error: format!("team not found: {team_id}") });
    }
    let agents: Vec<serde_json::Value> = s.agents.iter()
        .filter(|(id, _)| id.contains(team_id))
        .map(|(id, a)| serde_json::json!({
            "id": id, "template": a.config.template.name,
            "status": format!("{:?}", a.status),
            "iterations": a.iterations_completed, "tool_invocations": a.tool_invocations,
        }))
        .collect();
    Response::json(200, serde_json::json!({ "team_id": team_id, "agents": agents }))
}

pub(crate) fn handle_team_audit(team_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.team_manager.teams().get(team_id).is_none() {
        return Response::json(404, &ErrorResponse { error: format!("team not found: {team_id}") });
    }
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");
    let events: Vec<serde_json::Value> = match std::fs::read_to_string(&audit_path) {
        Ok(content) => content.lines()
            .filter(|l| !l.trim().is_empty() && l.contains(team_id))
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    Response::json(200, serde_json::json!({ "team_id": team_id, "events": events }))
}

pub(crate) fn handle_create_team(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { name: String }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { name: String::new() });
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.team_manager.create_team(&req.name, "api-user") {
        Ok(id) => { s.save(); Response::json(200, serde_json::json!({ "team_id": id })) }
        Err(e) => Response::json(400, &ErrorResponse { error: e.to_string() }),
    }
}

pub(crate) fn handle_join_team(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { token: String }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { token: String::new() });
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.team_manager.join(&req.token, "api-user") {
        Ok(_) => { s.save(); Response::json(200, serde_json::json!({ "ok": true })) }
        Err(e) => Response::json(400, &ErrorResponse { error: e.to_string() }),
    }
}

// ---------------------------------------------------------------------------
// Conversations
// ---------------------------------------------------------------------------

pub(crate) fn handle_list_conversations(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let convs: Vec<serde_json::Value> = s.conversations.values().map(|c| serde_json::json!({
        "id": c.id, "title": c.title, "messages": c.messages,
        "message_count": c.messages.len(), "created_at": c.created_at,
        "updated_at": c.updated_at, "workspace": c.workspace,
    })).collect();
    Response::json(200, &convs)
}

pub(crate) fn handle_create_conversation(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { title: Option<String> }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { title: None });
    let title = req.title.unwrap_or_else(|| format!("Conversation {}", chrono_now_secs()));
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = s.create_conversation(&title);
    Response::json(200, serde_json::json!({ "id": id, "title": title }))
}

pub(crate) fn handle_add_message(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        conversation_id: String, role: String, content: String,
        model: Option<String>, iterations: Option<usize>, tool_invocations: Option<u32>,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    let msg = crate::state::ChatMessage {
        role: req.role, content: req.content,
        timestamp: chrono_now_secs().to_string(),
        model: req.model, iterations: req.iterations, tool_invocations: req.tool_invocations,
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.add_message(&req.conversation_id, msg) {
        Response::json(200, serde_json::json!({ "ok": true }))
    } else {
        Response::json(404, &ErrorResponse { error: "conversation not found".to_string() })
    }
}

pub(crate) fn handle_get_conversation(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    if id.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "conversation id required".to_string() });
    }
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.get_conversation(id) {
        Some(conv) => Response::json(200, conv),
        None => Response::json(404, &ErrorResponse { error: format!("conversation not found: {id}") }),
    }
}

pub(crate) fn handle_delete_conversation(id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    if id.trim().is_empty() {
        return Response::json(400, &ErrorResponse { error: "conversation id required".to_string() });
    }
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.delete_conversation(id) {
        Response::json(204, serde_json::json!({}))
    } else {
        Response::json(404, &ErrorResponse { error: format!("conversation not found: {id}") })
    }
}

// ---------------------------------------------------------------------------
// Marketplace
// ---------------------------------------------------------------------------

pub(crate) fn handle_marketplace_list(_path: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, s.commerce.marketplace.search(None, 1, 20))
}

pub(crate) fn handle_install(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { listing_id: String, version: Option<String> }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { listing_id: String::new(), version: None });
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.commerce.marketplace.install(&req.listing_id, req.version.as_deref()) {
        Ok(_) => Response::json(200, serde_json::json!({ "ok": true })),
        Err(e) => Response::json(400, &ErrorResponse { error: e.to_string() }),
    }
}

// ---------------------------------------------------------------------------
// Cloud jobs (AWS Batch bridge)
// ---------------------------------------------------------------------------

pub(crate) fn handle_list_cloud_jobs(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, &s.swarm.cloud_jobs)
}

pub(crate) fn handle_submit_cloud_job(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        name: String,
        #[serde(default)]
        command: Vec<String>,
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
        region: Option<String>,
        queue: Option<String>,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    let client = crate::batch_client::BatchClient::new(
        req.region.as_deref().unwrap_or("us-east-1"),
        req.queue.as_deref().unwrap_or("tachy-default"),
    );
    match client.submit_job(&req.name, req.command, req.env) {
        Ok(job) => {
            let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            s.swarm.cloud_jobs.push(job.clone());
            Response::json(201, &job)
        }
        Err(e) => Response::json(500, &ErrorResponse { error: e }),
    }
}

pub(crate) fn handle_get_cloud_job(job_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let idx = s.swarm.cloud_jobs.iter().position(|j| j.id == job_id);
    match idx {
        None => Response::json(404, &ErrorResponse { error: "job not found".to_string() }),
        Some(i) => {
            let client = crate::batch_client::BatchClient::new("us-east-1", "tachy-default");
            if let Ok(status) = client.get_job_status(job_id) {
                s.swarm.cloud_jobs[i].status = status;
                s.swarm.cloud_jobs[i].updated_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            }
            Response::json(200, &s.swarm.cloud_jobs[i])
        }
    }
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

pub(crate) fn handle_audit_log(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");
    let events: Vec<serde_json::Value> = match std::fs::read_to_string(&audit_path) {
        Ok(content) => content.lines().filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok()).collect(),
        Err(_) => Vec::new(),
    };
    Response::json(200, &events)
}

pub(crate) fn handle_audit_export(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");
    let content = std::fs::read_to_string(&audit_path).unwrap_or_default();
    let mut csv = String::from("sequence,timestamp,session_id,kind,message\n");
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let seq     = v["sequence"].as_u64().unwrap_or(0);
            let ts      = v["timestamp"].as_str().unwrap_or("").replace(',', " ");
            let session = v["session_id"].as_str().unwrap_or("").replace(',', " ");
            let kind    = v["kind"].as_str().unwrap_or("").replace(',', " ");
            let msg     = v["message"].as_str().unwrap_or("").replace([',', '\n'], " ");
            csv.push_str(&format!("{seq},{ts},{session},{kind},{msg}\n"));
        }
    }
    let filename = format!("tachy-audit-{}.csv", chrono_now_str());
    csv_response(&csv, &filename)
}

pub(crate) fn handle_audit_verify(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let audit_path = s.workspace_root.join(".tachy").join("audit.jsonl");
    let content = match std::fs::read_to_string(&audit_path) {
        Ok(c) => c,
        Err(e) => return Response::json(500, &ErrorResponse { error: format!("failed to read audit log: {e}") }),
    };
    let events: Vec<audit::AuditEvent> = content.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    
    match audit::verify_audit_chain(&events) {
        Ok(count) => Response::json(200, serde_json::json!({
            "status": "verified",
            "event_count": count,
            "last_hash": events.last().map(|e| e.hash.clone()).unwrap_or_default()
        })),
        Err(index) => Response::json(400, serde_json::json!({
            "status": "corrupted",
            "error_index": index,
            "corrupted_event": events.get(index)
        })),
    }
}

/// POST /api/audit/flush — flush buffered events in HttpAuditSink and S3AuditSink to
/// their external destinations. Returns counts of sinks flushed and any errors.
pub(crate) fn handle_audit_flush(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut errors: Vec<String> = Vec::new();
    s.audit_logger.flush_with_errors(&mut errors);
    if errors.is_empty() {
        Response::json(200, serde_json::json!({ "status": "flushed" }))
    } else {
        Response::json(207, serde_json::json!({ "status": "partial", "errors": errors }))
    }
}

pub(crate) fn handle_metrics(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, serde_json::json!({
        "total_agents_run": s.agents.len(),
        "completed": s.agents.values().filter(|a| format!("{:?}", a.status) == "Completed").count(),
        "failed": s.agents.values().filter(|a| format!("{:?}", a.status) == "Failed").count(),
        "total_iterations": s.agents.values().map(|a| a.iterations_completed).sum::<usize>(),
        "total_tool_invocations": s.agents.values().map(|a| a.tool_invocations).sum::<u32>(),
        "scheduled_tasks": s.scheduler.list_tasks().len(),
    }))
}

pub(crate) fn handle_dashboard(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let stats = &s.inference_stats;
    let total_cost: f64 = s.commerce.metering.counters().values()
        .map(|agg| agg.total_cost_usd)
        .sum();
    let models: Vec<serde_json::Value> = s.registry.list_models().iter()
        .map(|m| serde_json::json!({ "name": m.name, "tier": format!("{:?}", m.tier) }))
        .collect();
    let cache_hits = s.semantic_cache.hits();
    let active_swarms = s.swarm.orchestrator.lock().unwrap().active_runs();

    Response::json(200, serde_json::json!({
        "total_requests": stats.total_requests,
        "total_tokens": stats.total_tokens,
        "input_tokens": stats.input_tokens,
        "output_tokens": stats.output_tokens,
        "avg_tokens_per_sec": stats.avg_tokens_per_sec,
        "last_tokens_per_sec": stats.last_tokens_per_sec,
        "p50_ttft_ms": stats.p50_ttft_ms,
        "p95_ttft_ms": stats.p95_ttft_ms,
        "estimated_cost_usd": total_cost,
        "models": models,
        "cache_hits": cache_hits,
        "active_swarms": active_swarms,
    }))
}

pub(crate) fn handle_promote(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { template: String, model: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.promote_model_adapter(&req.template, &req.model) {
        Ok(_) => Response::json(200, serde_json::json!({ "status": "promoted", "template": req.template, "model": req.model })),
        Err(e) => Response::json(404, &ErrorResponse { error: e }),
    }
}

pub(crate) fn handle_approve_plan(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { template: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.approve_plan(&req.template) {
        Ok(_) => Response::json(200, serde_json::json!({ "status": "approved", "template": req.template })),
        Err(e) => Response::json(404, &ErrorResponse { error: e }),
    }
}

// ---------------------------------------------------------------------------
// Rewind & Fork (Pillar 1: State Reconstruction)
// ---------------------------------------------------------------------------

pub(crate) fn handle_fork_session(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { session_id: String, event_hash: String }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };

    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.fork_session_at_hash(&req.session_id, &req.event_hash) {
        Ok(new_id) => Response::json(201, serde_json::json!({ "new_session_id": new_id, "forked_from": req.session_id, "at_hash": req.event_hash })),
        Err(e) => Response::json(400, &ErrorResponse { error: e }),
    }
}
