//! Simple (single-turn) agent execution and shared helpers used by the planning pipeline.

use std::path::Path;

use audit::{AuditEvent, AuditEventKind, AuditLogger, AuditSeverity, GovernancePolicy};
use backend::DynBackend;
use intelligence::{
    clean_code_output, contains_code, validate_code, CodebaseIndex, CodebaseIndexer, IndexerConfig,
};
use runtime::ConversationRuntime;

use platform::AgentConfig;

use super::{executor::IntelligentToolExecutor, AgentRunResult};

/// Simple execution — single `run_turn` with output validation.
pub(super) fn run_simple(
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
            check_governance(
                governance,
                tool_count,
                &config.session_id,
                agent_id,
                audit_logger,
            );
            let mut result_summary = extract_text_summary(&summary.assistant_messages);

            // --- Output validation: clean model artifacts ---
            result_summary = clean_code_output(&result_summary);

            // --- Output validation: check code quality ---
            if contains_code(&result_summary) {
                let lang = detect_language_from_content(&result_summary);
                let validation = validate_code(&result_summary, &lang);
                if !validation.valid {
                    let issues: Vec<String> = validation
                        .errors
                        .iter()
                        .map(|e| e.message.clone())
                        .collect();
                    audit_logger.log(
                        &AuditEvent::new(
                            &config.session_id,
                            AuditEventKind::SessionEnd,
                            format!("output validation warnings: {}", issues.join("; ")),
                        )
                        .with_agent(agent_id),
                    );
                }
            }

            audit_logger.log(
                &AuditEvent::new(
                    &config.session_id,
                    AuditEventKind::SessionEnd,
                    format!(
                        "agent {agent_id} completed: iterations={} tools={}",
                        summary.iterations, tool_count
                    ),
                )
                .with_agent(agent_id)
                .with_model(model),
            );

            runtime.session_mut().success = true;
            runtime.session_mut().human_override = false;

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
                &AuditEvent::new(
                    &config.session_id,
                    AuditEventKind::SessionEnd,
                    format!("agent {agent_id} failed: {error}"),
                )
                .with_severity(AuditSeverity::Warning)
                .with_agent(agent_id)
                .with_model(model),
            );
            runtime.session_mut().success = false;
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

pub(super) fn check_governance(
    governance: &GovernancePolicy,
    tool_count: u32,
    session_id: &str,
    agent_id: &str,
    audit_logger: &AuditLogger,
) {
    if let Some(max) = governance.max_total_tool_invocations {
        if tool_count > max {
            audit_logger.log(
                &AuditEvent::new(
                    session_id,
                    AuditEventKind::GovernanceViolation,
                    format!("agent exceeded {max} tool invocations"),
                )
                .with_severity(AuditSeverity::Warning)
                .with_agent(agent_id),
            );
        }
    }
}

pub(super) fn extract_text_summary(messages: &[runtime::ConversationMessage]) -> String {
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

/// Detect programming language from code content (for validation).
pub(super) fn detect_language_from_content(code: &str) -> String {
    if code.contains("fn ") && (code.contains("let ") || code.contains("pub ")) {
        "rust".to_string()
    } else if code.contains("def ") && code.contains(':') {
        "python".to_string()
    } else if code.contains("function ") || code.contains("const ") || code.contains("=>") {
        "javascript".to_string()
    } else if code.contains("func ") && code.contains("package ") {
        "go".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Load existing index or build a new one.
pub(super) fn load_or_build_index(
    workspace_root: &Path,
    config: &IndexerConfig,
) -> Result<CodebaseIndex, String> {
    if let Ok(existing) = CodebaseIndexer::load_index(workspace_root) {
        if !config.auto_rebuild {
            return Ok(existing);
        }
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

    let index = CodebaseIndexer::build_index(workspace_root, config).map_err(|e| e.to_string())?;
    let _ = CodebaseIndexer::save_index(workspace_root, &index);
    Ok(index)
}
