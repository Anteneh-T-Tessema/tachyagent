//! Request dispatcher: main routing match and dynamic parameterised routes.

use std::sync::{Arc, Mutex};

use audit::TieredRateLimiter;

use crate::state::DaemonState;
use crate::web;
use super::types::{ErrorResponse, Response};
use super::utils::{extract_auth_header, gate_action, parse_http_request};
use super::server::{
    handle_health, handle_inference_stats, handle_list_agents, handle_list_models,
    handle_list_tasks, handle_list_templates, handle_pull_model,
};
use super::agent::{
    handle_cancel_agent, handle_chat_stream, handle_complete, handle_complete_stream,
    handle_delete_agent, handle_get_agent, handle_prompt_oneshot, handle_run_agent,
};
use super::auth::{
    handle_billing_status, handle_license_activate, handle_license_status, handle_oauth_callback,
    handle_oauth_login, handle_oauth_logout, handle_oauth_sessions, handle_sso_callback,
    handle_sso_config, handle_sso_login, handle_sso_logout, handle_sso_sessions, handle_usage,
};
use super::governance::{
    handle_add_message, handle_approve_patch, handle_approve_plan,
    handle_audit_export, handle_audit_flush, handle_audit_log, handle_audit_verify,
    handle_create_conversation, handle_create_team, handle_dashboard, handle_delete_conversation,
    handle_fork_session, handle_promote,
    handle_get_cloud_job, handle_get_conversation, handle_get_mission_feed, handle_get_policy,
    handle_get_team, handle_install, handle_join_team, handle_list_cloud_jobs,
    handle_list_conversations, handle_list_file_locks, handle_list_pending_approvals,
    handle_list_teams, handle_marketplace_list, handle_metrics, handle_schedule_task,
    handle_set_policy, handle_submit_cloud_job, handle_team_agents, handle_team_audit,
};
use super::intel::{
    handle_dependency_graph, handle_diagnostics, handle_eval_run, handle_finetune_extract,
    handle_finetune_modelfile, handle_harness_start, handle_index_build, handle_index_status, handle_list_harnesses, handle_monorepo,
    handle_search, handle_policy_upload, handle_reweight_guidance, handle_train_start, handle_train_status,
};
use super::runs::{
    handle_approve_task, handle_cancel_parallel_run, handle_delete_run_template, handle_event_stream,
    handle_get_parallel_run, handle_get_run_conflicts, handle_get_run_cost,
    handle_get_run_template, handle_get_swarm_run, handle_list_parallel_runs,
    handle_list_run_templates, handle_list_suspended_tasks, handle_list_swarm_runs,
    handle_parallel_run, handle_replay_run,
    handle_run_history, handle_run_template, handle_save_run_template, handle_start_swarm_run,
};
use super::webhooks::{handle_list_webhooks, handle_register_webhook, handle_verify_webhook_signature};
use super::workers::{
    handle_deregister_worker, handle_list_workers, handle_register_worker,
    handle_telemetry_flush, handle_telemetry_status, handle_worker_heartbeat,
};
use super::yaya::{
    handle_yaya_chat, handle_yaya_get_retrieval_preferences, handle_yaya_list_experts,
    handle_yaya_set_retrieval_preferences, handle_yaya_submit_training_example,
};
use super::feedback::{
    handle_ab_test, handle_activate_adapter, handle_finetune_trigger,
    handle_get_adapter, handle_gold_standard_extract, handle_gold_standard_status,
    handle_list_adapters, handle_list_finetune_jobs, handle_register_adapter,
};
use super::sync;
use super::vision::handle_get_snapshot;

// ── Main request handler ──────────────────────────────────────────────────────

pub async fn handle_request(
    raw: &str,
    state: &Arc<Mutex<DaemonState>>,
    rate_limiter: &Arc<Mutex<TieredRateLimiter>>,
    client_ip: &str,
) -> Response {
    let (method, path_raw, body) = parse_http_request(raw);
    let path_full = path_raw.split('?').next().unwrap_or("/").trim_end_matches('/');
    let path = if path_full.is_empty() { "/" } else { path_full };
    let query_str = path_raw.find('?').map_or("", |i| &path_raw[i + 1..]).to_string();

    if method == "OPTIONS" {
        return Response::Full {
            status: 204,
            content_type: "text/plain".to_string(),
            body: String::new().into_bytes(),
            extra_headers: Vec::new(),
        };
    }

    // Tiered rate limiting: classify path, key by auth token or IP.
    // Authenticated callers get a separate bucket with the same limits,
    // preventing a single bad actor from starving legitimate users.
    let tier = TieredRateLimiter::classify(path);
    {
        let auth_key = extract_auth_header(raw);
        let rate_key = match &auth_key {
            Some(k) => format!("key:{}", &audit::hash_text(k)[..16]),
            None    => format!("ip:{client_ip}"),
        };
        let mut limiter = rate_limiter.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let decision = limiter.check(tier, &rate_key);
        if !decision.allowed {
            return Response::rate_limited(decision.retry_after_secs);
        }
    }

    if !matches!(path, "" | "/" | "/index.html" | "/health") {
        let s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(required_key) = &s.api_key {
            let provided = extract_auth_header(raw);
            match provided {
                Some(key) if key == *required_key => {}
                _ => return Response::json(401, &ErrorResponse { error: "unauthorized".to_string() }),
            }
        }
    }

    match (method.as_str(), path) {
        ("GET", "/" | "/index.html") => Response::html(200, web::INDEX_HTML),
        ("GET", "/health") => Response::json(200, handle_health(state)),
        ("GET", "/api/models") => Response::json(200, handle_list_models(state)),
        ("GET", "/api/inference/stats") => handle_inference_stats(state),
        ("POST", "/api/models/pull") => gate_action(state, raw, audit::Action::ManageModels, |_| handle_pull_model(&body, state)),
        ("POST", "/api/complete/stream") => handle_complete_stream(&body, state).await,
        ("POST", "/api/chat/stream") => handle_chat_stream(&body, state).await,
        ("GET", "/api/templates") => Response::json(200, handle_list_templates(state)),
        ("GET", "/api/agents") => Response::json(200, handle_list_agents(state)),
        ("GET", "/api/tasks") => Response::json(200, handle_list_tasks(state)),
        ("GET", "/api/audit") => gate_action(state, raw, audit::Action::ViewAudit, |_| handle_audit_log(state)),
        ("GET", "/api/audit/export") => gate_action(state, raw, audit::Action::ViewAudit, |_| handle_audit_export(state)),
        ("GET", "/api/audit/verify") => gate_action(state, raw, audit::Action::ViewAudit, |_| handle_audit_verify(state)),
        ("POST", "/api/audit/flush") => gate_action(state, raw, audit::Action::ManageModels, |_| handle_audit_flush(state)),
        ("GET", "/api/metrics") => gate_action(state, raw, audit::Action::ViewAudit, |_| handle_metrics(state)),
        ("GET", "/api/conversations") => handle_list_conversations(state),
        ("POST", "/api/conversations") => handle_create_conversation(&body, state),
        ("POST", "/api/conversations/message") => handle_add_message(&body, state),
        ("GET", "/api/auth/sso/login") => handle_sso_login(state),
        ("POST", "/api/auth/sso/callback") => handle_sso_callback(&body, state),
        ("POST", "/api/auth/sso/logout") => handle_sso_logout(&body, state),
        ("GET", "/api/auth/sso/sessions") => handle_sso_sessions(state),
        ("GET", "/api/license/status") => handle_license_status(state),
        ("GET", "/api/billing/status") => handle_billing_status(state),
        ("GET", "/api/teams") => handle_list_teams(state),
        ("POST", "/api/teams") => handle_create_team(&body, state),
        ("POST", "/api/teams/join") => handle_join_team(&body, state),
        _ if method == "GET" && path.starts_with("/api/teams/") => {
            let rest = path.strip_prefix("/api/teams/").unwrap_or(path);
            let (team_id, suffix) = rest.split_once('/').unwrap_or((rest, ""));
            match suffix {
                "" => handle_get_team(team_id, state),
                "agents" => handle_team_agents(team_id, state),
                "audit" => handle_team_audit(team_id, state),
                _ => Response::json(404, &ErrorResponse { error: "not found".to_string() }),
            }
        }
        ("GET", "/api/marketplace") => handle_marketplace_list(path, state),
        ("POST", "/api/marketplace/install") => gate_action(state, raw, audit::Action::ManageConfig, |_| handle_install(&body, state)),
        ("GET", "/api/parallel/runs") => handle_list_parallel_runs(state, raw),
        ("GET", "/api/runs/history") => handle_run_history(state),
        ("POST", "/api/parallel/runs") => gate_action(state, raw, audit::Action::RunAgent, |_| handle_parallel_run(&body, state, raw)),
        ("GET", "/api/cloud/jobs") => handle_list_cloud_jobs(state),
        ("POST", "/api/cloud/jobs") => gate_action(state, raw, audit::Action::ManageCloudJobs, |_| handle_submit_cloud_job(&body, state)),
        _ if method == "GET" && path.starts_with("/api/cloud/jobs/") => {
            let job_id = path.strip_prefix("/api/cloud/jobs/").unwrap_or(path);
            handle_get_cloud_job(job_id, state)
        }
        ("GET", "/api/swarm/runs") => handle_list_swarm_runs(state),
        ("POST", "/api/swarm/runs") => gate_action(state, raw, audit::Action::RunAgent, |_| handle_start_swarm_run(&body, state)),
        ("POST", "/api/swarm/execute") => super::swarm::handle_swarm_execute(&body, state).await,
        ("POST", "/api/swarm/register") => super::swarm::handle_swarm_register(&body, state).await,
        ("GET", "/api/swarm/nodes") => super::swarm::handle_swarm_list_nodes(state).await,
        _ if method == "GET" && path.starts_with("/api/swarm/runs/") => {
            let run_id = path.strip_prefix("/api/swarm/runs/").unwrap_or(path);
            handle_get_swarm_run(run_id, state)
        }
        ("POST", "/api/agents/run") => gate_action(state, raw, audit::Action::RunAgent, |_| handle_run_agent(&body, state)),
        ("GET", "/api/pending-approvals") => handle_list_pending_approvals(state),
        ("POST", "/api/approve") => gate_action(state, raw, audit::Action::ManageGovernance, |_| handle_approve_patch(&body, state)),
        ("GET", "/api/file-locks") => handle_list_file_locks(state),
        ("GET", "/api/mission/feed") => handle_get_mission_feed(state),
        ("POST", "/api/auth/sso/config") => gate_action(state, raw, audit::Action::ManageEnterpriseSSO, |_| handle_sso_config(&body, state)),
        ("GET", "/api/search") => handle_search(path_full, state),
        ("GET", "/api/yaya/experts") => handle_yaya_list_experts(&query_str, raw, state).await,
        ("GET", "/api/yaya/retrieval-preferences") => handle_yaya_get_retrieval_preferences(&query_str, raw, state).await,
        ("POST", "/api/yaya/retrieval-preferences") => handle_yaya_set_retrieval_preferences(&body, raw, state).await,
        ("POST", "/api/yaya/chat") => handle_yaya_chat(&body, raw, state).await,
        ("POST", "/api/yaya/training/examples") => handle_yaya_submit_training_example(&body, raw, state).await,
        ("GET", "/api/policy") => gate_action(state, raw, audit::Action::ManageGovernance, |_| handle_get_policy(state)),
        ("POST", "/api/policy") => gate_action(state, raw, audit::Action::ManageGovernance, |_| handle_set_policy(&body, state)),

        // --- routes present in OpenAPI spec ---
        ("POST", "/api/complete") => handle_complete(&body, state).await,
        ("POST", "/api/parallel/run") => handle_parallel_run(&body, state, raw), // spec uses singular
        ("GET", "/api/webhooks") => gate_action(state, raw, audit::Action::ManageWebhooks, |_| handle_list_webhooks(state)),
        ("POST", "/api/webhooks") => gate_action(state, raw, audit::Action::ManageWebhooks, |_| handle_register_webhook(&body, state)),
        ("POST", "/api/webhooks/verify") => gate_action(state, raw, audit::Action::ManageWebhooks, |_| handle_verify_webhook_signature(&body, raw, state)),
        ("POST", "/api/tasks/schedule") => gate_action(state, raw, audit::Action::ScheduleTask, |_| handle_schedule_task(&body, state)),
        ("POST", "/api/license/activate") => gate_action(state, raw, audit::Action::ManageGovernance, |_| handle_license_activate(&body, state)),

        ("POST", "/api/prompt") => handle_prompt_oneshot(&body, state),
        ("GET", "/api/usage") => handle_usage(state),

        // OAuth2 endpoints
        _ if method == "GET" && path.starts_with("/api/auth/oauth/") && path.ends_with("/login") => {
            let provider = path.strip_prefix("/api/auth/oauth/").unwrap_or("").trim_end_matches("/login");
            handle_oauth_login(provider, state)
        }
        _ if method == "GET" && path.starts_with("/api/auth/oauth/") && path.contains("/callback") => {
            let provider = path.strip_prefix("/api/auth/oauth/")
                .unwrap_or("")
                .split('/').next().unwrap_or("");
            handle_oauth_callback(provider, &query_str, state)
        }
        ("POST", "/api/auth/oauth/logout") => handle_oauth_logout(&body, state),
        ("GET", "/api/auth/oauth/sessions") => handle_oauth_sessions(state),

        // Telemetry
        ("POST", "/api/telemetry/flush") => handle_telemetry_flush(state),
        ("GET", "/api/telemetry/status") => handle_telemetry_status(state),

        // Distributed swarm worker registry
        ("GET", "/api/workers") => handle_list_workers(state),
        ("POST", "/api/workers/register") => handle_register_worker(&body, state),
        ("POST", "/api/workers/heartbeat") => handle_worker_heartbeat(&body, state),
        ("DELETE", "/api/workers/deregister") => handle_deregister_worker(&body, state),
        ("POST", "/api/finetune/extract") => gate_action(state, raw, audit::Action::ManageIntelligence, |user| handle_finetune_extract(&body, state, user)),
        ("POST", "/api/finetune/modelfile") => handle_finetune_modelfile(&body, state),
        // Phase 2: Gold Standard
        ("GET", "/api/gold-standard/status") => handle_gold_standard_status(state),
        ("POST", "/api/gold-standard/extract") => gate_action(state, raw, audit::Action::ManageIntelligence, |_| handle_gold_standard_extract(state)),
        // Phase 2: Adapter Registry
        ("GET", "/api/adapters") => handle_list_adapters(state),
        ("POST", "/api/adapters") => gate_action(state, raw, audit::Action::ManageModels, |_| handle_register_adapter(&body, state)),
        ("POST", "/api/adapters/ab-test") => gate_action(state, raw, audit::Action::ManageModels, |_| handle_ab_test(&body, state)),
        // Phase 2: Fine-tuning Bridge
        ("GET", "/api/finetune/jobs") => handle_list_finetune_jobs(state),
        ("POST", "/api/finetune/trigger") => gate_action(state, raw, audit::Action::ManageIntelligence, |_| handle_finetune_trigger(&body, state)),
        ("GET", "/api/diagnostics") => handle_diagnostics(&query_str, state),
        ("POST", "/api/index") => handle_index_build(&body, state),
        ("GET", "/api/index") => handle_index_status(state),
        ("POST", "/api/intel/train") => handle_train_start(&body, state),
        ("GET", "/api/intel/train/status") => handle_train_status(state),
        ("GET", "/api/graph") => handle_dependency_graph(&query_str, state),
        ("GET", "/api/monorepo") => handle_monorepo(state),
        ("GET", "/api/dashboard") => handle_dashboard(state),
        ("POST", "/api/eval/run") => handle_eval_run(&body, state),
        ("POST", "/api/harness/start") => handle_harness_start(&body, state),
        ("GET", "/api/harnesses") => handle_list_harnesses(state),
        ("GET", "/api/repo/manifest") => handle_monorepo(state),
        // Wave 2: live events, cost tracking, run replay, DAG templates
        ("GET", "/api/events") => handle_event_stream(state).await,
        ("GET", "/api/run-templates") => handle_list_run_templates(state),
        ("POST", "/api/run-templates") => handle_save_run_template(&body, state),
        ("POST", "/api/governance/promote") => handle_promote(&body, state),
        ("POST", "/api/governance/approve-plan") => handle_approve_plan(&body, state),
        ("POST", "/api/governance/fork") => handle_fork_session(&body, state),
        ("POST", "/api/intel/guidance/reweight") => handle_reweight_guidance(&body, state),
        ("GET", "/api/governance/mission") => handle_get_mission_feed(state),
        ("POST", "/api/governance/policy/upload") => gate_action(state, raw, audit::Action::ManagePolicies, |user| handle_policy_upload(&body, state, user)),
        ("POST", "/api/sync/receive") => sync::handle_receive_sync(&body, state).await,
        ("POST", "/api/sync/beam") => sync::handle_trigger_beam(&query_str, state).await,
        ("GET", "/api/sync/pulse") => sync::handle_sync_pulse(state),
        _ => route_dynamic(method.as_str(), path, &body, state, raw).await,
    }
}

// ── Dynamic parameterised routes ──────────────────────────────────────────────

/// Dynamic route dispatch for parameterised paths (e.g. `/api/agents/{id}`).
///
/// Uses `strip_prefix` / `strip_suffix` instead of manual index arithmetic,
/// eliminating off-by-one risk and making routing intent self-documenting.
async fn route_dynamic(
    method: &str,
    path: &str,
    body: &str,
    state: &Arc<Mutex<DaemonState>>,
    raw: &str,
) -> Response {
    // ── /api/adapters/{id}/* ─────────────────────────────────────────────────
    if let Some(rest) = path.strip_prefix("/api/adapters/") {
        if method == "GET" {
            return handle_get_adapter(rest, state);
        }
        if method == "POST" {
            if let Some(id) = rest.strip_suffix("/activate") {
                return handle_activate_adapter(id, state);
            }
        }
    }

    // ── /api/parallel/runs/{id}/* ────────────────────────────────────────────
    if let Some(rest) = path.strip_prefix("/api/parallel/runs/") {
        if method == "GET" {
            if rest == "suspended" {
                return handle_list_suspended_tasks(state);
            }
            if let Some(run_id) = rest.strip_suffix("/cost") {
                return handle_get_run_cost(run_id, state, raw);
            }
            if let Some(run_id) = rest.strip_suffix("/conflicts") {
                return handle_get_run_conflicts(run_id, state, raw);
            }
            return handle_get_parallel_run(rest, state, raw);
        }
        if method == "POST" {
            if let Some(run_id) = rest.strip_suffix("/replay") {
                return handle_replay_run(run_id, state);
            }
            if let Some(run_id) = rest.strip_suffix("/cancel") {
                return handle_cancel_parallel_run(run_id, body, state);
            }
            // Wave 2D: POST /api/parallel/runs/{run_id}/tasks/{task_id}/approve
            if let Some(task_path) = rest.find("/tasks/").map(|pos| &rest[pos + 7..]) {
                if let Some(task_id) = task_path.strip_suffix("/approve") {
                    return gate_action(state, raw, audit::Action::ManageGovernance, |_| {
                        handle_approve_task(task_id, state)
                    });
                }
            }
        }
    }

    // ── /api/run-templates/{name}/* ─────────────────────────────────────────
    if let Some(rest) = path.strip_prefix("/api/run-templates/") {
        match method {
            "GET" if !rest.ends_with("/run") => return handle_get_run_template(rest, state),
            "DELETE"                          => return handle_delete_run_template(rest, state),
            "POST" => {
                if let Some(name) = rest.strip_suffix("/run") {
                    return handle_run_template(name, body, state, raw);
                }
            }
            _ => {}
        }
    }

    // ── /api/agents/{id}/* ──────────────────────────────────────────────────
    if let Some(rest) = path.strip_prefix("/api/agents/") {
        match method {
            "GET" => return handle_get_agent(rest, state),
            "DELETE" => {
                if let Some(id) = rest.strip_suffix("/cancel") {
                    return handle_cancel_agent(id, state);
                }
                return handle_delete_agent(rest, state);
            }
            "POST" => {
                if let Some(id) = rest.strip_suffix("/cancel") {
                    return handle_cancel_agent(id, state);
                }
            }
            _ => {}
        }
    }

    // ── /api/conversations/{id} ─────────────────────────────────────────────
    if let Some(id) = path.strip_prefix("/api/conversations/") {
        match method {
            "GET"    => return handle_get_conversation(id, state),
            "DELETE" => return handle_delete_conversation(id, state),
            _ => {}
        }
    }

    // ── /api/vision/snapshot/{id} ──────────────────────────────────────────
    if let Some(id) = path.strip_prefix("/api/vision/snapshot/") {
        if method == "GET" {
            return handle_get_snapshot(id, state);
        }
    }

    Response::json(404, &ErrorResponse { error: format!("not found: {method} {path}") })
}
