//! Plan-and-execute agent pipeline: generate a plan, run each step with edit-test-fix cycles.

use std::path::Path;
use std::sync::{Arc, Mutex};

use audit::{AuditEvent, AuditEventKind, AuditLogger, AuditSeverity, GovernancePolicy};
use backend::{BackendRegistry, DynBackend};
use intelligence::{
    CodebaseIndex, IntelligenceConfig,
    build_optimized_prompt,
};
use runtime::{ConversationRuntime, Session};

use platform::AgentConfig;

use super::{AgentRunResult, executor::{IntelligentToolExecutor, build_permission_policy}};
use super::simple::{check_governance, extract_text_summary, run_simple};

/// Full intelligence pipeline: plan → execute steps → edit-test-fix → git commit.
pub(super) fn run_with_planning(
    agent_id: &str,
    config: &AgentConfig,
    prompt: &str,
    runtime: &mut ConversationRuntime<DynBackend, IntelligentToolExecutor>,
    intelligence_config: &IntelligenceConfig,
    workspace_root: &Path,
    index: &Option<CodebaseIndex>,
    governance: &GovernancePolicy,
    audit_logger: &AuditLogger,
    registry: &BackendRegistry,
    file_locks: Option<runtime::FileLockManager>,
    daemon_state: Option<Arc<Mutex<crate::state::DaemonState>>>,
    agent_identity: Option<platform::crypto::AgentIdentity>,
) -> AgentRunResult {
    let model = &config.template.model;

    // Step 1: Generate a plan by asking the LLM
    let codebase_summary = index.as_ref().map(|idx| {
        format!("{} files, primary language: {}, test command: {}",
            idx.project.total_files,
            idx.project.primary_language.as_deref().unwrap_or("unknown"),
            idx.project.test_command.as_deref().unwrap_or("none"),
        )
    });

    let planning_prompt = intelligence::PlanExecutor::build_planning_prompt(
        prompt,
        codebase_summary.as_deref(),
    );

    audit_logger.log(
        &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart, "generating plan")
            .with_agent(agent_id),
    );

    let plan_result = runtime.run_turn(&planning_prompt, None);
    let plan_text = match &plan_result {
        Ok(summary) => extract_text_summary(&summary.assistant_messages),
        Err(_) => String::new(),
    };

    let plan = if let Ok(p) = intelligence::PlanExecutor::parse_plan(&plan_text, prompt) { p } else {
        audit_logger.log(
            &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                "plan generation failed, falling back to simple execution")
                .with_agent(agent_id),
        );
        return run_simple(agent_id, config, prompt, runtime, governance, audit_logger);
    };

    audit_logger.log(
        &AuditEvent::new(
            &config.session_id,
            AuditEventKind::SessionStart,
            format!("plan created: {} steps", plan.steps.len()),
        )
        .with_agent(agent_id),
    );

    // Step 2: Create a git branch if enabled
    if intelligence_config.git_enabled && intelligence_config.plan.auto_branch {
        let branch_name = format!("tachy/task-{}", &config.session_id);
        if intelligence::GitTools::is_git_repo() {
            match intelligence::GitTools::branch(&branch_name, true) {
                Ok(result) => {
                    audit_logger.log(
                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                            format!("created branch {}", result.name))
                            .with_agent(agent_id),
                    );
                }
                Err(e) => {
                    audit_logger.log(
                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                            format!("branch creation failed: {e}"))
                            .with_agent(agent_id),
                    );
                }
            }
        }
    }

    // Step 3: Execute each plan step with a FRESH session per step.
    //
    // Why fresh sessions? Each step has a focused context window:
    //   - Full max_iterations budget (not split across all steps)
    //   - No accumulated noise from prior steps' tool outputs
    //   - Step-specific system prompt pre-loading relevant files
    //
    // The previous step's *result text* is passed as the first user message
    // in the next step so the agent knows what was accomplished, without
    // carrying the full conversation history forward.
    let mut total_iterations = 1usize; // count the planning turn
    let mut total_tool_invocations = 0u32;
    let mut all_results = Vec::new();
    let mut steps_completed = 0usize;
    let mut prior_step_result: Option<String> = None;

    for step in &plan.steps {
        audit_logger.log(
            &AuditEvent::new(
                &config.session_id,
                AuditEventKind::SessionStart,
                format!("executing step {}/{}: {}", step.number, plan.steps.len(), step.description),
            )
            .with_agent(agent_id),
        );

        // Snapshot every expected file before the step so we can roll back if
        // the step fails and leaves the workspace in a broken state.
        let step_snapshots: Vec<(std::path::PathBuf, Option<String>)> = step
            .expected_files
            .iter()
            .map(|f| {
                let path = workspace_root.join(f);
                let content = std::fs::read_to_string(&path).ok();
                (path, content)
            })
            .collect();

        let step_system_prompt = build_step_system_prompt(
            config, step, index, workspace_root, intelligence_config, model,
        );

        let enable_tools = !config.template.allowed_tools.is_empty();
        let step_client = match registry.create_client(model, enable_tools) {
            Ok(c) => DynBackend::new(c),
            Err(e) => {
                all_results.push(format!("Step {} FAILED: backend error: {e}", step.number));
                break;
            }
        };

        let mut step_governance = governance.clone();
        if config.template.requires_approval {
            step_governance.enforce_all_approvals = true;
        }

        let step_tool_executor = IntelligentToolExecutor {
            allowed_tools: config.template.allowed_tools.clone(),
            git_enabled: intelligence_config.git_enabled,
            custom_tools: tools::CustomToolRegistry::load(&workspace_root.join(".tachy")),
            workspace_root: workspace_root.to_path_buf(),
            registry: Some(Arc::new(registry.clone())),
            governance: Some(step_governance),
            audit_logger: Some(Arc::new(AuditLogger::new())),
            intelligence_config: Some(intelligence_config.clone()),
            file_locks: file_locks.clone(),
            agent_id: agent_id.to_string(),
            daemon_state: daemon_state.clone(),
            agent_identity: agent_identity.clone(),
            is_simulation: false,
            sentinel: None,
            inspector: None,
        };

        let permission_policy = build_permission_policy(&config.template.allowed_tools);
        let mut step_runtime = ConversationRuntime::new(
            Session::new(),
            step_client,
            step_tool_executor,
            permission_policy,
            step_system_prompt,
        ).with_max_iterations(config.template.max_iterations);

        let step_prompt = if let Some(ref prior) = prior_step_result {
            format!(
                "Context from previous step:\n{prior}\n\n---\n\nYour task for this step:\n{}",
                step.instruction
            )
        } else {
            step.instruction.clone()
        };

        match step_runtime.run_turn(&step_prompt, None) {
            Ok(summary) => {
                total_iterations += summary.iterations;
                total_tool_invocations += summary.tool_results.len() as u32;
                let step_text = extract_text_summary(&summary.assistant_messages);
                all_results.push(format!("Step {}: {}\n{}", step.number, step.description, step_text));
                prior_step_result = Some(if step_text.len() > 800 {
                    format!("{}…", &step_text[..800])
                } else {
                    step_text
                });
                steps_completed += 1;

                // Edit-test-fix cycle
                if intelligence_config.edit_test_fix_enabled && !step.expected_files.is_empty() {
                    if let Some(test_cmd) = intelligence::EditTestFix::detect_test_command(workspace_root, index.as_ref()) {
                        let targeted = intelligence::EditTestFix::targeted_test_command(&test_cmd, &step.expected_files);
                        let lsp_enabled = intelligence_config.edit_test_fix.lsp_diagnostics_enabled;
                        let check = intelligence::EditTestFix::run_diagnostic_then_test(
                            workspace_root, &step.expected_files, &targeted,
                            intelligence_config.edit_test_fix.test_timeout_secs, lsp_enabled,
                        );

                        match check {
                            intelligence::CycleCheckResult::Passed => {
                                // Visual check (Phase 26)
                                if let (Some(url), Some(baseline)) = (&step.url, &step.visual_baseline) {
                                    if let Ok(report) = intelligence::EditTestFix::run_visual_check(url, baseline, workspace_root) {
                                        let diff_summary = report.diff_report.as_deref().unwrap_or("No diff summary available");
                                        if diff_summary.contains("FAILURE") {
                                            audit_logger.log(&AuditEvent::new(&config.session_id, AuditEventKind::VisualSnapshot,
                                                format!("visual failure at {url}: {}", diff_summary))
                                                .with_severity(audit::AuditSeverity::Warning)
                                                .with_agent(agent_id)
                                                .with_visual_anchor(&report.screenshot_path)
                                                .with_visual_metadata(serde_json::json!({
                                                    "url": url,
                                                    "baseline": baseline,
                                                    "diff": diff_summary,
                                                })));
                                            
                                            let fix = intelligence::EditTestFix::build_fix_prompt(&targeted, &intelligence::TestResult {
                                                exit_code: 1,
                                                stdout: String::new(),
                                                stderr: format!("Visual Regression detected: {}", diff_summary),
                                            }, &step.expected_files, Some(&report));
                                            
                                            if let Ok(s) = step_runtime.run_turn(&fix, None) {
                                                total_iterations += s.iterations;
                                                total_tool_invocations += s.tool_results.len() as u32;
                                            }
                                        } else {
                                            audit_logger.log(&AuditEvent::new(&config.session_id, AuditEventKind::VisualSnapshot,
                                                format!("visual verification passed for {url}"))
                                                .with_agent(agent_id)
                                                .with_visual_anchor(&report.screenshot_path)
                                                .with_visual_metadata(serde_json::json!({
                                                    "url": url,
                                                    "baseline": baseline,
                                                    "diff": diff_summary,
                                                })));
                                        }
                                    }
                                }
                            }
                            intelligence::CycleCheckResult::DiagnosticErrors(diag_result) => {
                                audit_logger.log(
                                    &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                                        format!("LSP diagnostics: {} errors, {} warnings — attempting fix",
                                            diag_result.error_count, diag_result.warning_count))
                                        .with_agent(agent_id),
                                );
                                let fix_prompt = intelligence::EditTestFix::build_diagnostic_fix_prompt(&diag_result, &step.expected_files);
                                for retry in 0..intelligence_config.edit_test_fix.max_retries {
                                    if let Ok(fix_summary) = step_runtime.run_turn(&fix_prompt, None) {
                                        total_iterations += fix_summary.iterations;
                                        total_tool_invocations += fix_summary.tool_results.len() as u32;
                                    }
                                    let recheck = intelligence::EditTestFix::run_diagnostic_then_test(
                                        workspace_root, &step.expected_files, &targeted,
                                        intelligence_config.edit_test_fix.test_timeout_secs, lsp_enabled,
                                    );
                                    match recheck {
                                        intelligence::CycleCheckResult::Passed => {
                                            audit_logger.log(&AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd,
                                                format!("fixed after {} retries", retry + 1)).with_agent(agent_id));
                                            break;
                                        }
                                        intelligence::CycleCheckResult::TestFailure(test_result) => {
                                            let fix = intelligence::EditTestFix::build_fix_prompt(&targeted, &test_result, &step.expected_files, None);
                                            if let Ok(s) = step_runtime.run_turn(&fix, None) {
                                                total_iterations += s.iterations;
                                                total_tool_invocations += s.tool_results.len() as u32;
                                            }
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            intelligence::CycleCheckResult::TestFailure(test_result) => {
                                let fix_prompt = intelligence::EditTestFix::build_fix_prompt(&targeted, &test_result, &step.expected_files, None);
                                audit_logger.log(&AuditEvent::new(&config.session_id, AuditEventKind::SelfRepair,
                                    format!("tests failed for step {}, attempting autonomous repair", step.number)).with_agent(agent_id));
                                for retry in 0..intelligence_config.edit_test_fix.max_retries {
                                    if let Ok(s) = step_runtime.run_turn(&fix_prompt, None) {
                                        total_iterations += s.iterations;
                                        total_tool_invocations += s.tool_results.len() as u32;
                                    }
                                    let recheck = intelligence::EditTestFix::run_diagnostic_then_test(
                                        workspace_root, &step.expected_files, &targeted,
                                        intelligence_config.edit_test_fix.test_timeout_secs, lsp_enabled,
                                    );
                                    if matches!(recheck, intelligence::CycleCheckResult::Passed) {
                                        // Phase 27: Swarm Governance Review
                                        let consensus = intelligence::ConsensusEngine::review_repair(
                                            &registry,
                                            "Autonomous Repair Delta",
                                            "Tests Passed",
                                            None,
                                        );

                                        if consensus.is_approved {
                                            audit_logger.log(&AuditEvent::new(&config.session_id, AuditEventKind::ConsensusSeal,
                                                format!("autonomous repair approved (score: {:.2})", consensus.aggregate_score))
                                                .with_agent(agent_id)
                                                .with_consensus_report(serde_json::to_value(&consensus).unwrap_or_default()));
                                            
                                            // Phase 29: Broadcast Consensus Event
                                            if let Some(ref ds) = daemon_state {
                                                let s = ds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                                let event = crate::internal_bus::MissionEvent::ConsensusFormed {
                                                    agent_id: agent_id.to_string(),
                                                    report: consensus.clone(),
                                                };
                                                let _ = s.swarm.mission_control.broadcast(event.clone());
                                                {
                                                    let mut feed = s.swarm.mission_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                                    feed.push_front(event.clone());
                                                    if feed.len() > 100 { feed.pop_back(); }
                                                }
                                                s.publish_event("mission_event", serde_json::to_value(event).unwrap_or_default());
                                            }
                                            break;
                                        } else {
                                            audit_logger.log(&AuditEvent::new(&config.session_id, AuditEventKind::GovernanceViolation,
                                                format!("repair VETOED by consensus (score: {:.2})", consensus.aggregate_score))
                                                .with_severity(audit::AuditSeverity::Warning)
                                                .with_agent(agent_id)
                                                .with_consensus_report(serde_json::to_value(&consensus).unwrap_or_default()));
                                            
                                            // Phase 29: Broadcast Veto
                                            if let Some(ref ds) = daemon_state {
                                                let s = ds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                                let event = crate::internal_bus::MissionEvent::ConsensusFormed {
                                                    agent_id: agent_id.to_string(),
                                                    report: consensus.clone(),
                                                };
                                                let _ = s.swarm.mission_control.broadcast(event.clone());
                                                s.publish_event("mission_event", serde_json::to_value(event).unwrap_or_default());
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Git commit after successful step
                if intelligence_config.git_enabled && intelligence_config.plan.auto_commit
                    && intelligence::GitTools::is_git_repo() {
                        let msg = format!("tachy: step {} — {}", step.number, step.description);
                        if let Ok(result) = intelligence::GitTools::commit(&msg) { audit_logger.log(
                            &AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd,
                                format!("committed: {}", result.hash)).with_agent(agent_id),
                        ); }
                    }
            }
            Err(error) => {
                // Roll back every file this step was expected to touch so the
                // workspace is not left in a broken mid-plan state.
                let mut rolled_back = 0usize;
                for (path, maybe_original) in &step_snapshots {
                    match maybe_original {
                        Some(original) => {
                            if std::fs::write(path, original).is_ok() {
                                rolled_back += 1;
                            }
                        }
                        None => {
                            // File did not exist before this step — remove it.
                            let _ = std::fs::remove_file(path);
                            rolled_back += 1;
                        }
                    }
                }
                audit_logger.log(
                    &AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd,
                        format!("step {} failed: {error} — rolled back {rolled_back} file(s)", step.number))
                        .with_severity(AuditSeverity::Warning)
                        .with_agent(agent_id),
                );
                all_results.push(format!("Step {} FAILED (rolled back {rolled_back} files): {}", step.number, error));
                break;
            }
        }
    }

    check_governance(governance, total_tool_invocations, &config.session_id, agent_id, audit_logger);

    let result_summary = all_results.join("\n\n");
    let result_summary = if result_summary.len() > 4000 {
        format!("{}…", &result_summary[..4000])
    } else {
        result_summary
    };

    let success = steps_completed == plan.steps.len();

    audit_logger.log(
        &AuditEvent::new(
            &config.session_id,
            AuditEventKind::SessionEnd,
            format!("plan {}: {}/{} steps completed, {} iterations, {} tools",
                if success { "completed" } else { "partial" },
                steps_completed, plan.steps.len(), total_iterations, total_tool_invocations),
        )
        .with_agent(agent_id)
        .with_model(model),
    );

    AgentRunResult {
        agent_id: agent_id.to_string(),
        success,
        iterations: total_iterations,
        tool_invocations: total_tool_invocations,
        summary: result_summary,
    }
}

/// Build a focused system prompt for a single plan step.
///
/// Pre-loads the content of `step.expected_files` into the prompt so the
/// model has the relevant file context immediately — no `read_file` tool call
/// needed for files the plan already knows about.
pub(super) fn build_step_system_prompt(
    config: &platform::AgentConfig,
    step: &intelligence::PlanStep,
    index: &Option<CodebaseIndex>,
    workspace_root: &Path,
    intelligence_config: &IntelligenceConfig,
    model: &str,
) -> Vec<String> {
    let mut sections = build_optimized_prompt(model, &config.template.system_prompt, None);

    sections.push(format!(
        "## Current Task\nYou are executing step {} of a plan.\nGoal: {}\n\nFocus only on this step. Do not implement other parts of the plan.",
        step.number, step.description
    ));

    if !step.expected_files.is_empty() {
        let mut file_section = String::from("## Files for this step\n");
        for file_path in &step.expected_files {
            let abs = workspace_root.join(file_path);
            match std::fs::read_to_string(&abs) {
                Ok(content) => {
                    let max_chars = 12_000;
                    let (content, truncated) = if content.len() > max_chars {
                        (&content[..max_chars], true)
                    } else {
                        (content.as_str(), false)
                    };
                    let lang = intelligence::indexer::detect_language(file_path);
                    file_section.push_str(&format!(
                        "\n### {file_path}{}\n```{lang}\n{content}\n```\n",
                        if truncated { " (truncated)" } else { "" }
                    ));
                }
                Err(_) => {
                    file_section.push_str(&format!("\n### {file_path}\n(file not yet created)\n"));
                }
            }
        }
        sections.push(file_section);
    } else if let Some(idx) = index {
        if let Some(lang) = &idx.project.primary_language {
            sections.push(format!(
                "Project: {} files, primary language: {lang}{}",
                idx.project.total_files,
                idx.project.test_command.as_deref()
                    .map(|c| format!(", test command: {c}"))
                    .unwrap_or_default()
            ));
        }
        let _ = intelligence_config;
    }

    sections
}
