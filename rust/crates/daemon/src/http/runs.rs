//! Parallel run, swarm, event stream, cost tracking, replay, and DAG template handlers.

use std::sync::{Arc, Mutex};

use audit::sanitize_prompt;
use serde::Deserialize;

use crate::parallel::{self, AgentTask, ParallelRun, RunStatus, TaskStatus};
use crate::state::DaemonState;
use super::{Response, ErrorResponse, chrono_now_secs};

// ---------------------------------------------------------------------------
// Request types (only used by handlers in this module)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct ParallelRunRequest {
    pub tasks: Vec<ParallelTaskInput>,
    #[serde(default = "default_concurrency")]
    pub max_concurrency: usize,
}

fn default_concurrency() -> usize { 4 }

#[derive(Debug, Deserialize)]
pub(super) struct ParallelTaskInput {
    pub template: String,
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
}

fn default_priority() -> u8 { 5 }

// ---------------------------------------------------------------------------
// Parallel run handlers
// ---------------------------------------------------------------------------

pub(super) fn handle_list_parallel_runs(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let orch = s.orchestrator.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let runs: Vec<serde_json::Value> = orch.list_runs().iter().map(|r| serde_json::json!({
        "run_id": r.id,
        "status": r.status,
        "task_count": r.tasks.len(),
        "created_at": r.created_at,
    })).collect();
    Response::json(200, serde_json::json!({ "runs": runs }))
}

pub(super) fn handle_parallel_run(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let req: ParallelRunRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    let run_id = format!("run-{}", chrono_now_secs());
    let tasks: Vec<AgentTask> = req.tasks.iter().enumerate().map(|(i, t)| AgentTask {
        id: format!("{run_id}-t{i}"),
        run_id: run_id.clone(),
        template: t.template.clone(),
        prompt: sanitize_prompt(&t.prompt, 50_000),
        model: t.model.clone(),
        deps: t.deps.clone(),
        priority: t.priority,
        status: TaskStatus::Pending,
        result: None,
        created_at: chrono_now_secs(),
        started_at: None,
        completed_at: None,
        work_dir: None,
    }).collect();
    let run = ParallelRun {
        id: run_id.clone(),
        tasks,
        status: RunStatus::Running,
        created_at: chrono_now_secs(),
        max_concurrency: req.max_concurrency.clamp(1, 8),
        conflicts: Vec::new(),
    };
    let bg_state = Arc::clone(state);
    std::thread::spawn(move || {
        let completed = parallel::execute_parallel_run(run, &bg_state);
        if let Ok(s) = bg_state.lock() {
            parallel::Orchestrator::persist_run(&completed, &s.workspace_root);
            if let Ok(mut orch) = s.orchestrator.lock() {
                orch.register_completed_run(completed);
            }
        }
    });
    Response::json(202, serde_json::json!({ "run_id": run_id, "status": "running" }))
}

pub(super) fn handle_get_parallel_run(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let from_orch = s.orchestrator.lock().ok()
        .and_then(|orch| orch.get_run(run_id).cloned());
    if let Some(run) = from_orch {
        return Response::json(200, &run);
    }
    let tasks: Vec<_> = s.agents.iter()
        .filter(|(id, _)| id.starts_with(run_id))
        .map(|(id, agent)| serde_json::json!({
            "task_id": id,
            "status": format!("{:?}", agent.status).to_lowercase(),
            "iterations": agent.iterations_completed,
            "tool_invocations": agent.tool_invocations,
        }))
        .collect();
    if tasks.is_empty() {
        return Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") });
    }
    Response::json(200, serde_json::json!({ "run_id": run_id, "status": "running", "tasks": tasks }))
}

pub(super) fn handle_cancel_parallel_run(run_id: &str, _body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let matching: Vec<_> = s.agents.keys().filter(|id| id.starts_with(run_id)).cloned().collect();
    if matching.is_empty() {
        return Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") });
    }
    drop(s);
    Response::json(202, serde_json::json!({
        "run_id": run_id,
        "status": "cancellation_requested",
        "tasks": matching.len()
    }))
}

pub(super) fn handle_run_history(state: &Arc<Mutex<DaemonState>>) -> Response {
    let workspace_root = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).workspace_root.clone();
    let runs = parallel::Orchestrator::load_run_history(&workspace_root);
    Response::json(200, serde_json::json!({ "count": runs.len(), "runs": runs }))
}

pub(super) fn handle_get_run_conflicts(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let run = s.orchestrator.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_run(run_id).cloned();
    match run {
        None => Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") }),
        Some(r) => {
            let count = r.conflicts.len();
            Response::json(200, serde_json::json!({
                "run_id": run_id,
                "conflict_count": count,
                "has_conflicts": count > 0,
                "conflicts": r.conflicts,
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Swarm handlers
// ---------------------------------------------------------------------------

pub(super) fn handle_list_swarm_runs(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let orch = s.orchestrator.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, orch.list_runs())
}

pub(super) fn handle_get_swarm_run(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let orch = s.orchestrator.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match orch.get_run(run_id) {
        Some(run) => Response::json(200, run),
        None => Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") }),
    }
}

pub(super) fn handle_start_swarm_run(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut input: intelligence::SwarmRefactorInput = match serde_json::from_str(body) {
        Ok(i) => i,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid request: {e}") }),
    };
    {
        let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        input.coordinator = Some(s.config.coordinator.clone());
    }
    let plan = intelligence::plan_swarm_refactor(&input);
    let run_id = format!("swarm-{}", chrono_now_secs());
    eprintln!("[swarm] run={run_id} tasks={} planner={:?}", plan.tasks.len(), plan.planner);
    let now = chrono_now_secs();
    let agent_tasks: Vec<AgentTask> = plan.tasks.iter().map(|t| AgentTask {
        id: format!("{run_id}-{}", t.id),
        run_id: run_id.clone(),
        template: t.template.clone(),
        prompt: sanitize_prompt(&t.prompt, 50_000),
        model: None,
        deps: t.deps.iter().map(|d| format!("{run_id}-{d}")).collect(),
        priority: 128,
        status: TaskStatus::Pending,
        result: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        work_dir: None,
    }).collect();
    let run = ParallelRun {
        id: run_id.clone(),
        tasks: agent_tasks,
        status: RunStatus::Running,
        created_at: now,
        max_concurrency: 4,
        conflicts: Vec::new(),
    };
    let bg_state = Arc::clone(state);
    std::thread::spawn(move || {
        let completed = parallel::execute_parallel_run(run, &bg_state);
        if let Ok(s) = bg_state.lock() {
            parallel::Orchestrator::persist_run(&completed, &s.workspace_root);
            if let Ok(mut orch) = s.orchestrator.lock() {
                orch.register_completed_run(completed);
            }
        }
    });
    Response::json(202, serde_json::json!({ "run_id": run_id, "status": "running" }))
}

// ---------------------------------------------------------------------------
// Wave 2A: Live SSE event stream
// ---------------------------------------------------------------------------

/// GET /api/events — Server-Sent Events stream of all daemon activity.
pub(super) async fn handle_event_stream(state: &Arc<Mutex<DaemonState>>) -> Response {
    let rx = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).event_bus.subscribe();
    let (resp, tx) = Response::sse();
    tokio::spawn(async move {
        let mut rx = rx;
        loop {
            match rx.recv().await {
                Ok(msg) => { if tx.send(msg).await.is_err() { break; } }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = tx.send("event: lag\ndata: {\"dropped\":true}\n\n".to_string()).await;
                }
            }
        }
    });
    resp
}

// ---------------------------------------------------------------------------
// Wave 2B: Cost/token tracking
// ---------------------------------------------------------------------------

pub(super) fn handle_get_run_cost(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let run = s.orchestrator.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_run(run_id).cloned();
    match run {
        None => {
            let workspace_root = s.workspace_root.clone();
            drop(s);
            let history = parallel::Orchestrator::load_run_history(&workspace_root);
            match history.iter().find(|r| r.id == run_id) {
                None => Response::json(404, &ErrorResponse { error: format!("run not found: {run_id}") }),
                Some(r) => Response::json(200, parallel::RunCost::from_run(r)),
            }
        }
        Some(r) => Response::json(200, parallel::RunCost::from_run(&r)),
    }
}

// ---------------------------------------------------------------------------
// Wave 2C: Run replay
// ---------------------------------------------------------------------------

pub(super) fn handle_replay_run(run_id: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let (workspace_root, run_opt) = {
        let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let run = s.orchestrator.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_run(run_id).cloned();
        (s.workspace_root.clone(), run)
    };
    let original = if let Some(r) = run_opt { r } else {
        let history = parallel::Orchestrator::load_run_history(&workspace_root);
        match history.into_iter().find(|r| r.id == run_id) {
            None => return Response::json(404, &ErrorResponse {
                error: format!("run not found: {run_id}"),
            }),
            Some(r) => r,
        }
    };
    let new_run_id = format!("replay-{}-{}", run_id, now_epoch());
    let tasks: Vec<AgentTask> = original.tasks.iter().map(|t| AgentTask {
        id: format!("{}-r", t.id),
        run_id: new_run_id.clone(),
        template: t.template.clone(),
        prompt: t.prompt.clone(),
        model: t.model.clone(),
        deps: t.deps.iter().map(|d| format!("{d}-r")).collect(),
        priority: t.priority,
        status: TaskStatus::Pending,
        result: None,
        created_at: now_epoch(),
        started_at: None,
        completed_at: None,
        work_dir: None,
    }).collect();
    let replay_run = ParallelRun {
        id: new_run_id.clone(),
        tasks,
        status: RunStatus::Running,
        created_at: now_epoch(),
        max_concurrency: original.max_concurrency,
        conflicts: vec![],
    };
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish_event("run_replay_started", serde_json::json!({
            "original_run_id": run_id, "replay_run_id": new_run_id,
        }));
    let completed = parallel::execute_parallel_run(replay_run, state);
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish_event("run_replay_complete", serde_json::json!({
            "run_id": completed.id, "status": format!("{:?}", completed.status),
        }));
    Response::json(200, &completed)
}

// ---------------------------------------------------------------------------
// Wave 2D: Named DAG templates
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SaveTemplateRequest {
    name: String,
    #[serde(default)]
    description: String,
    tasks: Vec<TemplateTaskInput>,
    #[serde(default = "default_conc")]
    max_concurrency: usize,
}

#[derive(Debug, Deserialize)]
struct TemplateTaskInput {
    template: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    deps: Vec<String>,
    #[serde(default = "default_pri")]
    priority: u8,
}

fn default_conc() -> usize { 4 }
fn default_pri() -> u8 { 5 }

pub(super) fn handle_list_run_templates(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let templates: Vec<&crate::state::RunTemplate> = s.run_templates.values().collect();
    Response::json(200, serde_json::json!({ "count": templates.len(), "templates": templates }))
}

pub(super) fn handle_save_run_template(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let req: SaveTemplateRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    if req.name.is_empty() {
        return Response::json(400, &ErrorResponse { error: "name is required".into() });
    }
    if req.tasks.is_empty() {
        return Response::json(400, &ErrorResponse { error: "tasks array must not be empty".into() });
    }
    let template = crate::state::RunTemplate {
        name: req.name.clone(),
        description: req.description,
        tasks: req.tasks.into_iter().map(|t| crate::state::TemplateTask {
            template: t.template,
            prompt: t.prompt,
            model: t.model,
            deps: t.deps,
            priority: t.priority,
        }).collect(),
        max_concurrency: req.max_concurrency,
        created_at: now_epoch(),
    };
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.run_templates.insert(req.name.clone(), template.clone());
    drop(s);
    Response::json(201, &template)
}

pub(super) fn handle_get_run_template(name: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.run_templates.get(name) {
        None => Response::json(404, &ErrorResponse { error: format!("template not found: {name}") }),
        Some(t) => Response::json(200, t),
    }
}

pub(super) fn handle_delete_run_template(name: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match s.run_templates.remove(name) {
        None => Response::json(404, &ErrorResponse { error: format!("template not found: {name}") }),
        Some(_) => Response::json(200, serde_json::json!({ "deleted": name })),
    }
}

pub(super) fn handle_run_template(name: &str, _body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let template = {
        let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match s.run_templates.get(name).cloned() {
            None => return Response::json(404, &ErrorResponse { error: format!("template not found: {name}") }),
            Some(t) => t,
        }
    };
    let run_id = format!("tpl-{name}-{}", now_epoch());
    let tasks: Vec<AgentTask> = template.tasks.iter().enumerate().map(|(i, t)| {
        let task_id = format!("{run_id}-t{i}");
        let deps = t.deps.iter().filter_map(|dep_name| {
            template.tasks.iter().position(|x| x.template == *dep_name || x.prompt.starts_with(dep_name.as_str()))
                .map(|j| format!("{run_id}-t{j}"))
        }).collect();
        AgentTask {
            id: task_id,
            run_id: run_id.clone(),
            template: t.template.clone(),
            prompt: t.prompt.clone(),
            model: t.model.clone(),
            deps,
            priority: t.priority,
            status: TaskStatus::Pending,
            result: None,
            created_at: now_epoch(),
            started_at: None,
            completed_at: None,
            work_dir: None,
        }
    }).collect();
    let run = ParallelRun {
        id: run_id.clone(),
        tasks,
        status: RunStatus::Running,
        created_at: now_epoch(),
        max_concurrency: template.max_concurrency,
        conflicts: vec![],
    };
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish_event("template_run_started", serde_json::json!({ "template": name, "run_id": run_id }));
    let completed = parallel::execute_parallel_run(run, state);
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish_event("template_run_complete", serde_json::json!({
            "template": name, "run_id": completed.id, "status": format!("{:?}", completed.status),
        }));
    Response::json(200, &completed)
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
