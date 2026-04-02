use std::path::Path;

use audit::{AuditEvent, AuditEventKind, AuditLogger, AuditSeverity, GovernancePolicy};
use backend::{BackendRegistry, DynBackend};
use intelligence::{
    CodebaseIndex, CodebaseIndexer, ContextSelector, IntelligenceConfig,
    git, IndexerConfig,
};
use runtime::{
    ConversationRuntime, PermissionMode, PermissionPolicy, Session, ToolError, ToolExecutor,
};
use tools::execute_tool;

use platform::AgentConfig;

/// Result of running an agent to completion.
#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub agent_id: String,
    pub success: bool,
    pub iterations: usize,
    pub tool_invocations: u32,
    pub summary: String,
}

/// The engine that executes agents against the conversation runtime.
pub struct AgentEngine;

impl AgentEngine {
    /// Run an agent to completion with intelligence features.
    pub fn run_agent(
        agent_id: &str,
        config: &AgentConfig,
        prompt: &str,
        registry: &BackendRegistry,
        governance: &GovernancePolicy,
        audit_logger: &AuditLogger,
        intelligence_config: &IntelligenceConfig,
        workspace_root: &Path,
    ) -> AgentRunResult {
        let model = &config.template.model;
        let enable_tools = !config.template.allowed_tools.is_empty();

        // Create backend
        let client = match registry.create_client(model, enable_tools) {
            Ok(c) => c,
            Err(e) => {
                audit_logger.log(
                    &AuditEvent::new(
                        &config.session_id,
                        AuditEventKind::SessionEnd,
                        format!("backend creation failed: {e}"),
                    )
                    .with_severity(AuditSeverity::Critical)
                    .with_agent(agent_id),
                );
                return AgentRunResult {
                    agent_id: agent_id.to_string(),
                    success: false,
                    iterations: 0,
                    tool_invocations: 0,
                    summary: format!("failed to create backend for model {model}: {e}"),
                };
            }
        };

        let backend = DynBackend::new(client);

        // --- Intelligence: Codebase Indexing ---
        let index = if intelligence_config.indexing_enabled {
            match load_or_build_index(workspace_root, &intelligence_config.indexer) {
                Ok(idx) => {
                    audit_logger.log(
                        &AuditEvent::new(
                            &config.session_id,
                            AuditEventKind::SessionStart,
                            format!("indexed {} files", idx.project.total_files),
                        )
                        .with_agent(agent_id),
                    );
                    Some(idx)
                }
                Err(e) => {
                    // Graceful degradation: continue without index
                    audit_logger.log(
                        &AuditEvent::new(
                            &config.session_id,
                            AuditEventKind::SessionStart,
                            format!("indexing failed (continuing without): {e}"),
                        )
                        .with_agent(agent_id),
                    );
                    None
                }
            }
        } else {
            None
        };

        // --- Intelligence: Smart Context Selection ---
        let mut system_prompt = vec![config.template.system_prompt.clone()];

        if intelligence_config.context_enabled {
            if let Some(idx) = &index {
                // Determine model context window from registry
                let ctx_window = registry
                    .find_model(model)
                    .map(|m| m.context_window)
                    .unwrap_or(8192);

                match ContextSelector::select_context(
                    prompt,
                    idx,
                    workspace_root,
                    ctx_window,
                    &intelligence_config.context,
                ) {
                    Ok(injection) => {
                        let rendered = ContextSelector::render_injection(&injection, idx);
                        system_prompt.push(rendered);
                        audit_logger.log(
                            &AuditEvent::new(
                                &config.session_id,
                                AuditEventKind::SessionStart,
                                format!(
                                    "context injected: {} summaries, {} files, ~{} tokens",
                                    injection.summaries.len(),
                                    injection.file_contents.len(),
                                    injection.estimated_tokens,
                                ),
                            )
                            .with_agent(agent_id),
                        );
                    }
                    Err(_) => {
                        // Graceful degradation: continue without context
                    }
                }
            }
        }

        // Build permission policy — include git tools if enabled
        let mut allowed = config.template.allowed_tools.clone();
        if intelligence_config.git_enabled {
            for name in ["git_status", "git_diff", "git_branch", "git_commit"] {
                if !allowed.contains(&name.to_string()) {
                    allowed.push(name.to_string());
                }
            }
        }
        let permission_policy = build_permission_policy(&allowed);

        // Build tool executor with git tools
        let tool_executor = IntelligentToolExecutor {
            allowed_tools: allowed,
            git_enabled: intelligence_config.git_enabled,
        };

        let mut runtime = ConversationRuntime::new(
            Session::new(),
            backend,
            tool_executor,
            permission_policy,
            system_prompt,
        )
        .with_max_iterations(config.template.max_iterations);

        // Log agent start
        audit_logger.log(
            &AuditEvent::new(
                &config.session_id,
                AuditEventKind::SessionStart,
                format!("agent {agent_id} running: {prompt}"),
            )
            .with_agent(agent_id)
            .with_model(model),
        );

        // --- Intelligence: Plan-and-Execute or simple run ---
        if intelligence_config.planning_enabled {
            Self::run_with_planning(
                agent_id, config, prompt, &mut runtime, intelligence_config,
                workspace_root, &index, governance, audit_logger,
            )
        } else {
            Self::run_simple(agent_id, config, prompt, &mut runtime, governance, audit_logger)
        }
    }

    /// Simple execution — single run_turn, no planning.
    fn run_simple(
        agent_id: &str,
        config: &AgentConfig,
        prompt: &str,
        runtime: &mut ConversationRuntime<DynBackend, IntelligentToolExecutor>,
        governance: &GovernancePolicy,
        audit_logger: &AuditLogger,
    ) -> AgentRunResult {
        let model = &config.template.model;
        match runtime.run_turn(prompt, None) {
            Ok(summary) => {
                let tool_count = summary.tool_results.len() as u32;
                check_governance(governance, tool_count, &config.session_id, agent_id, audit_logger);
                let result_summary = extract_text_summary(&summary.assistant_messages);

                audit_logger.log(
                    &AuditEvent::new(
                        &config.session_id,
                        AuditEventKind::SessionEnd,
                        format!("agent {agent_id} completed: iterations={} tools={}", summary.iterations, tool_count),
                    )
                    .with_agent(agent_id)
                    .with_model(model),
                );

                AgentRunResult {
                    agent_id: agent_id.to_string(),
                    success: true,
                    iterations: summary.iterations,
                    tool_invocations: tool_count,
                    summary: result_summary,
                }
            }
            Err(error) => {
                audit_logger.log(
                    &AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd, format!("agent {agent_id} failed: {error}"))
                        .with_severity(AuditSeverity::Warning)
                        .with_agent(agent_id)
                        .with_model(model),
                );
                AgentRunResult {
                    agent_id: agent_id.to_string(),
                    success: false,
                    iterations: 0,
                    tool_invocations: 0,
                    summary: format!("runtime error: {error}"),
                }
            }
        }
    }

    /// Full intelligence pipeline: plan → execute steps → edit-test-fix → git commit.
    fn run_with_planning(
        agent_id: &str,
        config: &AgentConfig,
        prompt: &str,
        runtime: &mut ConversationRuntime<DynBackend, IntelligentToolExecutor>,
        intelligence_config: &IntelligenceConfig,
        workspace_root: &Path,
        index: &Option<CodebaseIndex>,
        governance: &GovernancePolicy,
        audit_logger: &AuditLogger,
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

        // Ask the model to create a plan
        let plan_result = runtime.run_turn(&planning_prompt, None);
        let plan_text = match &plan_result {
            Ok(summary) => extract_text_summary(&summary.assistant_messages),
            Err(_) => String::new(),
        };

        // Try to parse the plan
        let plan = match intelligence::PlanExecutor::parse_plan(&plan_text, prompt) {
            Ok(p) => p,
            Err(_) => {
                // Graceful degradation: fall back to simple execution
                audit_logger.log(
                    &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart, "plan generation failed, falling back to simple execution")
                        .with_agent(agent_id),
                );
                // Re-run with the original prompt (not the planning prompt)
                return Self::run_simple(agent_id, config, prompt, runtime, governance, audit_logger);
            }
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
                            &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart, format!("created branch {}", result.name))
                                .with_agent(agent_id),
                        );
                    }
                    Err(e) => {
                        audit_logger.log(
                            &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart, format!("branch creation failed: {e}"))
                                .with_agent(agent_id),
                        );
                    }
                }
            }
        }

        // Step 3: Execute each plan step
        let mut total_iterations = 1usize; // count the planning turn
        let mut total_tool_invocations = 0u32;
        let mut all_results = Vec::new();
        let mut steps_completed = 0usize;

        for step in &plan.steps {
            audit_logger.log(
                &AuditEvent::new(
                    &config.session_id,
                    AuditEventKind::SessionStart,
                    format!("executing step {}: {}", step.number, step.description),
                )
                .with_agent(agent_id),
            );

            match runtime.run_turn(&step.instruction, None) {
                Ok(summary) => {
                    total_iterations += summary.iterations;
                    total_tool_invocations += summary.tool_results.len() as u32;
                    let step_text = extract_text_summary(&summary.assistant_messages);
                    all_results.push(format!("Step {}: {}\n{}", step.number, step.description, step_text));
                    steps_completed += 1;

                    // Edit-test-fix: if the step edited files and ETF is enabled
                    if intelligence_config.edit_test_fix_enabled && !step.expected_files.is_empty() {
                        if let Some(test_cmd) = intelligence::EditTestFix::detect_test_command(workspace_root, index.as_ref()) {
                            let targeted = intelligence::EditTestFix::targeted_test_command(&test_cmd, &step.expected_files);
                            audit_logger.log(
                                &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart, format!("running tests: {targeted}"))
                                    .with_agent(agent_id),
                            );

                            match intelligence::EditTestFix::run_tests(&targeted, intelligence_config.edit_test_fix.test_timeout_secs) {
                                Ok(test_result) if test_result.exit_code == 0 => {
                                    audit_logger.log(
                                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd, "tests passed")
                                            .with_agent(agent_id),
                                    );
                                }
                                Ok(test_result) => {
                                    // Tests failed — try to fix
                                    let fix_prompt = intelligence::EditTestFix::build_fix_prompt(&targeted, &test_result, &step.expected_files);
                                    audit_logger.log(
                                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart, "tests failed, attempting fix")
                                            .with_agent(agent_id),
                                    );

                                    for retry in 0..intelligence_config.edit_test_fix.max_retries {
                                        if let Ok(fix_summary) = runtime.run_turn(&fix_prompt, None) {
                                            total_iterations += fix_summary.iterations;
                                            total_tool_invocations += fix_summary.tool_results.len() as u32;
                                        }
                                        // Re-run tests
                                        if let Ok(retest) = intelligence::EditTestFix::run_tests(&targeted, intelligence_config.edit_test_fix.test_timeout_secs) {
                                            if retest.exit_code == 0 {
                                                audit_logger.log(
                                                    &AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd, format!("tests fixed after {} retries", retry + 1))
                                                        .with_agent(agent_id),
                                                );
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(_) => {
                                    // Test execution failed, continue
                                }
                            }
                        }
                    }

                    // Git commit after successful step
                    if intelligence_config.git_enabled && intelligence_config.plan.auto_commit {
                        if intelligence::GitTools::is_git_repo() {
                            let msg = format!("tachy: step {} — {}", step.number, step.description);
                            match intelligence::GitTools::commit(&msg) {
                                Ok(result) => {
                                    audit_logger.log(
                                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd, format!("committed: {}", result.hash))
                                            .with_agent(agent_id),
                                    );
                                }
                                Err(_) => {} // nothing to commit is fine
                            }
                        }
                    }
                }
                Err(error) => {
                    audit_logger.log(
                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionEnd, format!("step {} failed: {error}", step.number))
                            .with_severity(AuditSeverity::Warning)
                            .with_agent(agent_id),
                    );
                    all_results.push(format!("Step {} FAILED: {}", step.number, error));
                    break; // stop on failure
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
}

fn check_governance(governance: &GovernancePolicy, tool_count: u32, session_id: &str, agent_id: &str, audit_logger: &AuditLogger) {
    if let Some(max) = governance.max_total_tool_invocations {
        if tool_count > max {
            audit_logger.log(
                &AuditEvent::new(session_id, AuditEventKind::GovernanceViolation, format!("agent exceeded {max} tool invocations"))
                    .with_severity(AuditSeverity::Warning)
                    .with_agent(agent_id),
            );
        }
    }
}

fn extract_text_summary(messages: &[runtime::ConversationMessage]) -> String {
    let text: String = messages
        .iter()
        .flat_map(|msg| msg.blocks.iter())
        .filter_map(|block| match block {
            runtime::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if text.len() > 4000 {
        format!("{}…", &text[..4000])
    } else {
        text
    }
}

/// Load existing index or build a new one.
fn load_or_build_index(
    workspace_root: &Path,
    config: &IndexerConfig,
) -> Result<CodebaseIndex, String> {
    // Try loading existing index first
    if let Ok(existing) = CodebaseIndexer::load_index(workspace_root) {
        if !config.auto_rebuild {
            return Ok(existing);
        }
        // Incremental update
        match CodebaseIndexer::update_index(workspace_root, &existing, config) {
            Ok((updated, count)) => {
                if count > 0 {
                    let _ = CodebaseIndexer::save_index(workspace_root, &updated);
                }
                return Ok(updated);
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    // Build fresh index
    let index = CodebaseIndexer::build_index(workspace_root, config)
        .map_err(|e| e.to_string())?;
    let _ = CodebaseIndexer::save_index(workspace_root, &index);
    Ok(index)
}

/// Tool executor with intelligence features (git tools).
struct IntelligentToolExecutor {
    allowed_tools: Vec<String>,
    git_enabled: bool,
}

impl ToolExecutor for IntelligentToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if !self.allowed_tools.iter().any(|t| t == tool_name) {
            return Err(ToolError::new(format!(
                "tool '{tool_name}' not in agent's allowed tools"
            )));
        }

        // Handle git tools
        if self.git_enabled {
            match tool_name {
                "git_status" => return git::execute_git_status().map_err(ToolError::new),
                "git_diff" => {
                    let value = serde_json::from_str(input)
                        .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                    return git::execute_git_diff(&value).map_err(ToolError::new);
                }
                "git_branch" => {
                    let value = serde_json::from_str(input)
                        .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                    return git::execute_git_branch(&value).map_err(ToolError::new);
                }
                "git_commit" => {
                    let value = serde_json::from_str(input)
                        .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                    return git::execute_git_commit(&value).map_err(ToolError::new);
                }
                _ => {}
            }
        }

        // Standard tools
        let value = serde_json::from_str(input)
            .map_err(|e| ToolError::new(format!("invalid tool input: {e}")))?;
        execute_tool(tool_name, &value).map_err(ToolError::new)
    }
}

fn build_permission_policy(allowed_tools: &[String]) -> PermissionPolicy {
    let mut policy = PermissionPolicy::new(PermissionMode::Deny);
    for tool in allowed_tools {
        policy = policy.with_tool_mode(tool, PermissionMode::Allow);
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intelligent_executor_blocks_disallowed_tools() {
        let mut executor = IntelligentToolExecutor {
            allowed_tools: vec!["read_file".to_string()],
            git_enabled: false,
        };
        let result = executor.execute("bash", r#"{"command":"ls"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn permission_policy_from_allowed_tools() {
        let policy = build_permission_policy(&[
            "read_file".to_string(),
            "git_status".to_string(),
        ]);
        assert_eq!(policy.mode_for("read_file"), PermissionMode::Allow);
        assert_eq!(policy.mode_for("git_status"), PermissionMode::Allow);
        assert_eq!(policy.mode_for("bash"), PermissionMode::Deny);
    }
}
