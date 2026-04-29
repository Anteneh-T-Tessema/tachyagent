//! Intelligence handlers: semantic search, fine-tuning, diagnostics, index, dependency graph.

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use super::{urlencoding_decode, ErrorResponse, Response};
use crate::state::DaemonState;

pub(crate) fn handle_search(path_full: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let (query, limit) = {
        let qs = path_full.split_once('?').map_or("", |(_, q)| q);
        let mut q_val = String::new();
        let mut lim_val: usize = 10;
        for pair in qs.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "q" | "query" => q_val = urlencoding_decode(v),
                    "limit" | "n" => lim_val = v.parse().unwrap_or(10).min(50),
                    _ => {}
                }
            }
        }
        (q_val, lim_val)
    };
    if query.is_empty() {
        return Response::json(
            400,
            &ErrorResponse {
                error: "missing query param: ?q=".to_string(),
            },
        );
    }
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let ws = &s.workspace_root;
    let index = if let Ok(idx) = intelligence::CodebaseIndexer::load_index(ws) {
        idx
    } else {
        let cfg = intelligence::IndexerConfig::default();
        match intelligence::CodebaseIndexer::build_index(ws, &cfg) {
            Ok(idx) => {
                let _ = intelligence::CodebaseIndexer::save_index(ws, &idx);
                idx
            }
            Err(e) => {
                return Response::json(
                    503,
                    &ErrorResponse {
                        error: format!("codebase not indexed: {e}"),
                    },
                )
            }
        }
    };
    let results: Vec<serde_json::Value> =
        intelligence::CodebaseIndexer::search(&index, &query, limit)
            .into_iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path, "language": entry.language,
                    "lines": entry.lines, "exports": entry.exports, "summary": entry.summary,
                })
            })
            .collect();
    Response::json(
        200,
        serde_json::json!({ "query": query, "results": results }),
    )
}

pub(crate) fn handle_finetune_extract(
    body: &str,
    state: &Arc<Mutex<DaemonState>>,
    user: &audit::User,
) -> Response {
    #[derive(Deserialize)]
    struct Req {
        sessions_dir: Option<String>,
        gold_standard: Option<bool>,
    }
    let req: Req = serde_json::from_str(body).unwrap_or(Req {
        sessions_dir: None,
        gold_standard: None,
    });
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let workspace_root = s.workspace_root.clone();
    let sessions_dir = req.sessions_dir.map_or_else(
        || workspace_root.join(".tachy").join("sessions"),
        std::path::PathBuf::from,
    );

    // Extract dataset but filter by user's active team_id
    let dataset = intelligence::FinetuneDataset::from_sessions_isolated(
        &sessions_dir,
        req.gold_standard.unwrap_or(false),
        user.active_team_id.as_deref(),
        None,
    );

    Response::json(
        200,
        serde_json::json!({
            "entries": dataset.total_pairs,
            "source_sessions": dataset.source_sessions,
            "jsonl": dataset.to_jsonl(),
            "team_id": user.active_team_id,
        }),
    )
}

pub(crate) fn handle_finetune_modelfile(body: &str, _state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        base_model: String,
        adapter_path: String,
        system_prompt: Option<String>,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid body: {e}"),
                },
            )
        }
    };
    let prompt = req
        .system_prompt
        .as_deref()
        .unwrap_or("You are a helpful AI coding assistant.");
    let mf = intelligence::generate_modelfile(&req.base_model, &req.adapter_path, prompt);
    Response::json(200, serde_json::json!({ "modelfile": mf }))
}

pub(crate) fn handle_diagnostics(query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let file_path = {
        let mut path = String::new();
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "file" || k == "path" {
                    path = urlencoding_decode(v);
                    break;
                }
            }
        }
        path
    };
    if file_path.is_empty() {
        return Response::json(
            400,
            &ErrorResponse {
                error: "missing ?file= param".to_string(),
            },
        );
    }
    let workspace_root = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_root
        .clone();
    let lsp = intelligence::LspManager::new(&workspace_root);
    let diagnostics = lsp.get_diagnostics(&file_path);
    Response::json(
        200,
        serde_json::json!({
            "file": file_path,
            "diagnostics": diagnostics.iter().map(|d| serde_json::json!({
                "file": d.file, "line": d.line, "column": d.column, "message": d.message,
                "severity": match d.severity {
                    intelligence::DiagnosticSeverity::Error => "error",
                    intelligence::DiagnosticSeverity::Warning => "warning",
                    _ => "info",
                },
                "source": d.source,
            })).collect::<Vec<_>>(),
            "count": diagnostics.len(),
        }),
    )
}

pub(crate) fn handle_index_build(_body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_root
        .clone();
    let ws_display = ws.display().to_string();
    std::thread::spawn(move || {
        let cfg = intelligence::IndexerConfig::default();
        match intelligence::CodebaseIndexer::build_index(&ws, &cfg) {
            Ok(idx) => eprintln!("[index] build complete: {} files indexed", idx.files.len()),
            Err(e) => eprintln!("[index] build failed: {e}"),
        }
    });
    Response::json(
        202,
        serde_json::json!({
            "status": "building",
            "workspace": ws_display,
            "message": "Index build started in background — poll GET /api/index for status"
        }),
    )
}

pub(crate) fn handle_index_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_root
        .clone();
    match intelligence::CodebaseIndexer::load_index(&ws) {
        Ok(idx) => Response::json(
            200,
            serde_json::json!({
                "status": "ready", "file_count": idx.files.len(), "workspace": ws.display().to_string()
            }),
        ),
        Err(_) => Response::json(
            200,
            serde_json::json!({
                "status": "not_built", "file_count": 0, "workspace": ws.display().to_string()
            }),
        ),
    }
}

pub(crate) fn handle_dependency_graph(query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_root
        .clone();
    let graph = intelligence::DependencyGraph::build(&ws);
    let file_param = query
        .split('&')
        .find(|p| p.starts_with("file="))
        .map(|p| urlencoding_decode(&p[5..]));
    if let Some(f) = file_param {
        let deps = graph.transitive_dependents(&f);
        let node = graph.nodes.get(&f);
        return Response::json(
            200,
            serde_json::json!({
                "file": f,
                "direct_imports": node.map(|n| &n.imports).cloned().unwrap_or_default(),
                "imported_by": node.map(|n| &n.imported_by).cloned().unwrap_or_default(),
                "transitive_dependents": deps,
            }),
        );
    }
    Response::json(200, &graph)
}

pub(crate) fn handle_monorepo(state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_root
        .clone();
    let manifest = intelligence::MonorepoManifest::detect(&ws);
    Response::json(200, &manifest)
}

pub(crate) fn handle_policy_upload(
    body: &str,
    state: &Arc<Mutex<DaemonState>>,
    user: &audit::User,
) -> Response {
    #[derive(Deserialize)]
    struct Req {
        name: String,
        wasm_base64: String,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid body: {e}"),
                },
            )
        }
    };

    use base64::Engine;
    let wasm_bytes = match base64::engine::general_purpose::STANDARD.decode(&req.wasm_base64) {
        Ok(b) => b,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid base64 wasm: {e}"),
                },
            )
        }
    };

    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let policy_dir = s.workspace_root.join(".tachy").join("policies");
    if !policy_dir.exists() {
        let _ = std::fs::create_dir_all(&policy_dir);
    }

    let policy_path = policy_dir.join(format!("{}.wasm", req.name));
    if let Err(e) = std::fs::write(&policy_path, wasm_bytes) {
        return Response::json(
            500,
            &ErrorResponse {
                error: format!("failed to write policy: {e}"),
            },
        );
    }

    // Register in state if needed, or just let PolicyEngine find it on disk
    Response::json(
        201,
        serde_json::json!({
            "status": "uploaded",
            "name": req.name,
            "path": policy_path.to_string_lossy(),
            "team_id": user.active_team_id,
        }),
    )
}
pub(crate) fn handle_eval_run(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        sessions_dir: Option<String>,
    }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { sessions_dir: None });

    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sessions_dir = req.sessions_dir.map_or_else(
        || s.workspace_root.join(".tachy").join("sessions"),
        std::path::PathBuf::from,
    );

    // Load dataset from Gold Standard sessions
    let dataset =
        intelligence::FinetuneDataset::from_sessions_isolated(&sessions_dir, true, None, None);
    if dataset.entries.is_empty() {
        return Response::json(
            400,
            &ErrorResponse {
                error: "no gold standard sessions found for evaluation".to_string(),
            },
        );
    }

    let evaluator = intelligence::evaluator::ModelEvaluator::new(dataset);
    let report = evaluator.run_benchmark();
    let thresholds = intelligence::evaluator::ShadowThresholds::default();
    let should_promote = evaluator.should_promote(&report, &thresholds);

    Response::json(
        200,
        serde_json::json!({
            "total_cases": report.total_cases,
            "tuned_wins": report.tuned_wins,
            "base_wins": report.base_wins,
            "avg_tuned_similarity": report.avg_tuned_similarity,
            "avg_base_similarity": report.avg_base_similarity,
            "should_promote": should_promote,
            "results": report.results
        }),
    )
}

pub(crate) fn handle_train_start(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        adapter_name: String,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid body: {e}"),
                },
            )
        }
    };

    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let job_id = format!(
        "job-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let job = intelligence::finetune::TrainingJob {
        id: job_id.clone(),
        status: intelligence::finetune::TrainingStatus::Running,
        adapter_name: req.adapter_name.clone(),
        start_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        end_time: None,
        error: None,
    };
    s.inference_stats.active_training_jobs.push(job.clone());
    s.save();

    // Mock background training thread
    let state_clone = Arc::clone(state);
    let job_id_clone = job_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(10)); // Simulated training
        let mut s = state_clone
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(j) = s
            .inference_stats
            .active_training_jobs
            .iter_mut()
            .find(|j| j.id == job_id_clone)
        {
            j.status = intelligence::finetune::TrainingStatus::Completed;
            j.end_time = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            );
        }
        s.save();
    });

    Response::json(202, job)
}

pub(crate) fn handle_train_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, &s.inference_stats.active_training_jobs)
}

// ---------------------------------------------------------------------------
// Guidance Control (Phase 11)
// ---------------------------------------------------------------------------

pub(crate) fn handle_reweight_guidance(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        hash: String,
        reward_score: f32,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid body: {e}"),
                },
            )
        }
    };

    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match s
        .semantic_cache
        .reweight_by_hash(&req.hash, req.reward_score)
    {
        Ok(()) => Response::json(
            200,
            serde_json::json!({ "status": "updated", "hash": req.hash, "new_score": req.reward_score }),
        ),
        Err(e) => Response::json(404, &ErrorResponse { error: e }),
    }
}

pub(crate) fn handle_harness_start(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req {
        conv_id: String,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid body: {e}"),
                },
            )
        }
    };

    let mut s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let harness_id = format!("harness-{}-{}", req.conv_id, timestamp());

    let mut harness = intelligence::AgenticHarness::new(Arc::new(s.registry.clone()));
    harness.transition(intelligence::HarnessStep::GatherContext);

    s.harnesses.insert(harness_id.clone(), harness.clone());
    s.save();

    // Spawn the Main Agentic Loop (Official Step-by-Step)
    let state_clone = Arc::clone(state);
    let harness_id_clone = harness_id.clone();
    tokio::spawn(async move {
        let registry = {
            let s = state_clone.lock().unwrap();
            Arc::new(s.registry.clone())
        };

        // Step 1: Gather Context
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        {
            let mut s = state_clone.lock().unwrap();
            if let Some(h) = s.harnesses.get_mut(&harness_id_clone) {
                h.transition(intelligence::HarnessStep::Plan);
            }
            s.save();
        }

        // Step 2: Plan (Research-only project modeling)
        {
            let mut s = state_clone.lock().unwrap();
            if let Some(h) = s.harnesses.get_mut(&harness_id_clone) {
                let plan_sub = intelligence::SubagentManager::spawn(
                    intelligence::SubagentType::Plan,
                    "Researching multi-file refactor strategy",
                );

                // Real Inference: Generate Implementation Plan
                let dummy_step = intelligence::planner::PlanStep {
                    number: 1,
                    description: "Initial Refactor Plan".to_string(),
                    instruction: "Identify all files requiring synchronization logic.".to_string(),
                    expected_files: vec![],
                    status: intelligence::planner::StepStatus::Pending,
                    result: None,
                    actions: None,
                    worker_node_id: None,
                    url: None,
                    visual_baseline: None,
                };
                let plan_markdown =
                    intelligence::SubagentManager::research_plan(&registry, &dummy_step);

                let mut sub = plan_sub.clone();
                sub.summary = Some(plan_markdown);
                h.active_subagents.insert(sub.id.clone(), sub);
            }
            s.save();
        }
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;

        // Step 3: Act (Authorized Execution)
        {
            // Real Side-Effect: Trigger a sync snapshot to verify logging bridge
            let mut s = state_clone.lock().unwrap();
            if let Some(sync_manager) = s.connectivity.sync_manager.as_ref() {
                // Trigger the snapshot (will call platform::log_info)
                let _ = sync_manager.create_snapshot();
            }

            if let Some(h) = s.harnesses.get_mut(&harness_id_clone) {
                h.transition(intelligence::HarnessStep::Act);
                // Complete plan subagent
                if let Some(sub) = h
                    .active_subagents
                    .values_mut()
                    .find(|a| a.agent_type == intelligence::SubagentType::Plan)
                {
                    sub.status = intelligence::SubagentStatus::Completed;
                }
            }
            s.save();
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Step 4: Verify (Integrated Review)
        {
            let _report = intelligence::ConsensusEngine::review_simulation(
                &registry,
                "Apply synchronization bridge to platform crate",
                "Compilation successful. Tests passed.",
            );

            let mut s = state_clone.lock().unwrap();
            if let Some(h) = s.harnesses.get_mut(&harness_id_clone) {
                h.transition(intelligence::HarnessStep::Verify);
            }
            s.save();
        }
    });

    Response::json(
        202,
        serde_json::json!({ "id": harness_id, "status": "started" }),
    )
}

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub(crate) fn handle_list_harnesses(state: &Arc<Mutex<DaemonState>>) -> Response {
    let s = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Response::json(200, &s.harnesses)
}
