//! `IntelligentToolExecutor` — tool dispatch with git, RAG, custom tools, MCP, and governance.

use std::sync::{Arc, Mutex};

use audit::{AuditEvent, AuditEventKind, AuditLogger, GovernancePolicy};
use backend::BackendRegistry;
use intelligence::IntelligenceConfig;
use runtime::{PermissionMode, PermissionPolicy, ToolError, ToolExecutor};
use tools::execute_tool;

use super::AgentEngine;

/// Tool executor with intelligence features (git tools, custom tools, memory).
pub(super) struct IntelligentToolExecutor {
    pub(super) allowed_tools: Vec<String>,
    pub(super) git_enabled: bool,
    pub(super) custom_tools: tools::CustomToolRegistry,
    pub(super) workspace_root: std::path::PathBuf,
    /// Registry for `call_agent` tool — allows agents to call other agents.
    pub(super) registry: Option<Arc<BackendRegistry>>,
    pub(super) governance: Option<GovernancePolicy>,
    pub(super) audit_logger: Option<Arc<AuditLogger>>,
    pub(super) intelligence_config: Option<IntelligenceConfig>,
    /// File lock manager for parallel agent safety.
    pub(super) file_locks: Option<runtime::FileLockManager>,
    /// Agent ID for lock ownership.
    pub(super) agent_id: String,
    /// Shared daemon state for policy engine patch queuing.
    pub(super) daemon_state: Option<Arc<Mutex<super::super::DaemonState>>>,
}

impl ToolExecutor for IntelligentToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if !self.allowed_tools.iter().any(|t| t == tool_name) {
            return Err(ToolError::new(format!(
                "tool '{tool_name}' not in agent's allowed tools"
            )));
        }

        // Handle remember tool
        if tool_name == "remember" {
            let value = serde_json::from_str(input)
                .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
            let tachy_dir = self.workspace_root.join(".tachy");
            return intelligence::execute_remember(&value, &tachy_dir).map_err(ToolError::new);
        }

        // Handle call_agent tool — agent-to-agent communication
        if tool_name == "call_agent" {
            let value: serde_json::Value = serde_json::from_str(input)
                .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
            let template = value.get("template").and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::new("'template' required"))?;
            let prompt = value.get("prompt").and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::new("'prompt' required"))?;

            if let (Some(reg), Some(gov), Some(audit), Some(intel)) = (
                &self.registry, &self.governance, &self.audit_logger, &self.intelligence_config
            ) {
                let sub_agent_id = format!("sub-{}", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());

                let default_config = platform::PlatformConfig::default();
                let agent_template = default_config.agent_templates.iter()
                    .find(|t| t.name == template)
                    .cloned()
                    .ok_or_else(|| ToolError::new(format!(
                        "unknown agent template: '{}'. Available: {}",
                        template,
                        default_config.agent_templates.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
                    )))?;

                let agent_config = platform::AgentConfig {
                    template: agent_template,
                    session_id: format!("sess-{sub_agent_id}"),
                    working_directory: self.workspace_root.to_string_lossy().to_string(),
                    environment: std::collections::BTreeMap::new(),
                };

                let result = AgentEngine::run_agent(
                    &sub_agent_id, &agent_config, prompt, reg, gov, Arc::clone(audit), intel,
                    &self.workspace_root, self.file_locks.clone(), self.daemon_state.clone(),
                );

                return Ok(format!(
                    "Agent '{}' completed (success={}, {} iterations, {} tool calls):\n\n{}",
                    template, result.success, result.iterations, result.tool_invocations, result.summary
                ));
            }
            return Err(ToolError::new("call_agent not available in this context"));
        }

        // Handle git tools
        if self.git_enabled {
            match tool_name {
                "git_status" => return intelligence::git::execute_git_status().map_err(ToolError::new),
                "git_diff" => {
                    let value = serde_json::from_str(input)
                        .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                    return intelligence::git::execute_git_diff(&value).map_err(ToolError::new);
                }
                "git_branch" => {
                    let value = serde_json::from_str(input)
                        .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                    return intelligence::git::execute_git_branch(&value).map_err(ToolError::new);
                }
                "git_commit" => {
                    let value = serde_json::from_str(input)
                        .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                    return intelligence::git::execute_git_commit(&value).map_err(ToolError::new);
                }
                _ => {}
            }
        }

        // Handle RAG tools
        match tool_name {
            "search_codebase" => {
                let input_val = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                return intelligence::execute_search_codebase(&input_val, &self.workspace_root)
                    .map_err(ToolError::new);
            }
            "expand_context" => {
                let input_val = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                return intelligence::execute_expand_context(&input_val, &self.workspace_root)
                    .map_err(ToolError::new);
            }
            "swarm_refactor" => {
                let input_val = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                return intelligence::execute_swarm_refactor(&input_val, &self.workspace_root)
                    .map_err(ToolError::new);
            }
            "broadcast_mission_status" => {
                let input_val: intelligence::BroadcastStatusInput = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;

                if let Some(ref ds) = self.daemon_state {
                    let s = ds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let event = if let Some(discovery) = &input_val.discovery {
                        crate::internal_bus::MissionEvent::Discovery {
                            agent_id: self.agent_id.clone(),
                            file_path: "all".to_string(),
                            summary: discovery.clone(),
                        }
                    } else {
                        crate::internal_bus::MissionEvent::StatusUpdate {
                            agent_id: self.agent_id.clone(),
                            mission_id: "default".to_string(),
                            status: input_val.status.clone(),
                            percentage: input_val.percentage,
                        }
                    };
                    let listeners = s.mission_control.broadcast(event.clone()).unwrap_or(0);
                    {
                        let mut feed = s.mission_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        feed.push_front(event);
                        if feed.len() > 100 { feed.pop_back(); }
                    }
                    return Ok(serde_json::to_string(&intelligence::BroadcastStatusResult {
                        success: true, listeners,
                    }).unwrap());
                }
                return Err(ToolError::new("Mission Control not available"));
            }
            "get_mission_feed" => {
                let input_val: intelligence::GetMissionFeedInput = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;

                if let Some(ref ds) = self.daemon_state {
                    let s = ds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let feed = s.mission_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let events = feed.iter().take(input_val.limit).map(|e| {
                        intelligence::collaboration::MissionFeedEvent {
                            agent_id: match e {
                                crate::internal_bus::MissionEvent::StatusUpdate { agent_id, .. } => agent_id.clone(),
                                crate::internal_bus::MissionEvent::Discovery { agent_id, .. } => agent_id.clone(),
                                crate::internal_bus::MissionEvent::ConflictAlert { agent_id, .. } => agent_id.clone(),
                                _ => "system".to_string(),
                            },
                            event_type: format!("{e:?}").split('{').next().unwrap_or("unknown").trim().to_string(),
                            payload: serde_json::to_value(e).unwrap(),
                            timestamp: 0,
                        }
                    }).collect();
                    return Ok(serde_json::to_string(&intelligence::GetMissionFeedResult { events }).unwrap());
                }
                return Err(ToolError::new("Mission Control not available"));
            }
            _ => {}
        }

        // Handle custom tools
        if self.custom_tools.find(tool_name).is_some() {
            let value = serde_json::from_str(input)
                .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
            return self.custom_tools.execute(tool_name, &value).map_err(ToolError::new);
        }

        // Handle MCP tools (mcp__<server>__<tool>)
        if tool_name.starts_with("mcp__") {
            let value: serde_json::Value = serde_json::from_str(input)
                .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
            if let Some(ref ds) = self.daemon_state {
                let mut s = ds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                return s.mcp_client.call_tool(tool_name, &value).map_err(ToolError::new);
            }
            return Err(ToolError::new("MCP tools not available in this context"));
        }

        // Standard built-in tools — use diff-aware execution for write/edit
        let value: serde_json::Value = serde_json::from_str(input)
            .map_err(|e| ToolError::new(format!("invalid tool input: {e}")))?;

        if tool_name == "write_file" || tool_name == "edit_file" {
            let file_path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");

            if let Some(ref locks) = self.file_locks {
                if let Err(lock_err) = locks.acquire_with_wait(
                    file_path, &self.agent_id, std::time::Duration::from_secs(30),
                ) {
                    return Err(ToolError::new(format!("file lock: {lock_err}")));
                }
            }

            if let Some(gov) = &self.governance {
                if gov.requires_approval(file_path) {
                    if let Some(ref locks) = self.file_locks {
                        locks.release(file_path, &self.agent_id);
                    }
                    if let Some(audit) = &self.audit_logger {
                        audit.log(
                            &AuditEvent::new("", AuditEventKind::PermissionDenied,
                                format!("write to '{file_path}' requires approval (governance policy)"))
                                .with_tool(tool_name),
                        );
                    }
                    return Err(ToolError::new(format!(
                        "governance: write to '{file_path}' requires human approval (matches approval_required_paths policy). \
                         The change was NOT applied. An administrator must approve this change."
                    )));
                }
            }

            let diff_preview = if tool_name == "write_file" {
                let content = value.get("content").and_then(|v| v.as_str()).unwrap_or("");
                runtime::preview_write_file(file_path, content).ok()
            } else {
                let old_s = value.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                let new_s = value.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                let replace_all = value.get("replace_all").and_then(serde_json::Value::as_bool).unwrap_or(false);
                runtime::preview_edit_file(file_path, old_s, new_s, replace_all).ok()
            };

            if let Some(ref ds) = self.daemon_state {
                if let Some(ref preview) = diff_preview {
                    let new_content = if tool_name == "write_file" {
                        value.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string()
                    } else {
                        std::fs::read_to_string(file_path).unwrap_or_default()
                    };
                    let patch = audit::FilePatch {
                        file_path: file_path.to_string(),
                        original_hash: String::new(),
                        new_content,
                        diff_summary: preview.summary.clone(),
                        additions: preview.additions,
                        deletions: preview.deletions,
                        agent_id: self.agent_id.clone(),
                        task_id: None,
                    };

                    let decision = {
                        let s = ds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        s.policy_engine.evaluate(&patch)
                    };

                    match decision {
                        audit::PolicyDecision::Reject { reason } => {
                            if let Some(ref locks) = self.file_locks {
                                locks.release(file_path, &self.agent_id);
                            }
                            return Err(ToolError::new(format!("policy engine rejected: {reason}")));
                        }
                        audit::PolicyDecision::RequiresApproval { reason } => {
                            let patch_id = {
                                let new_content = if tool_name == "write_file" {
                                    value.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string()
                                } else {
                                    let original = std::fs::read_to_string(file_path).unwrap_or_default();
                                    let old_s = value.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                                    let new_s = value.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                                    let replace_all = value.get("replace_all").and_then(serde_json::Value::as_bool).unwrap_or(false);
                                    if replace_all { original.replace(old_s, new_s) }
                                    else { original.replacen(old_s, new_s, 1) }
                                };
                                let queued_patch = audit::FilePatch {
                                    file_path: file_path.to_string(),
                                    original_hash: String::new(),
                                    new_content,
                                    diff_summary: preview.summary.clone(),
                                    additions: preview.additions,
                                    deletions: preview.deletions,
                                    agent_id: self.agent_id.clone(),
                                    task_id: None,
                                };
                                let mut s = ds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                s.queue_pending_patch(queued_patch, reason.clone())
                            };
                            if let Some(ref locks) = self.file_locks {
                                locks.release(file_path, &self.agent_id);
                            }
                            return Err(ToolError::new(format!(
                                "policy: patch queued for approval (id={patch_id}): {reason}. \
                                 The change was NOT applied. A human must approve via POST /api/approve."
                            )));
                        }
                        audit::PolicyDecision::AutoApprove => {}
                    }
                }
            }

            let result = tools::execute_tool_with_diff(tool_name, &value)
                .map_err(|e| {
                    if let Some(ref locks) = self.file_locks {
                        locks.release(file_path, &self.agent_id);
                    }
                    ToolError::new(e)
                })?;

            let (output, preview) = result;

            if let (Some(audit), Some(preview)) = (&self.audit_logger, preview) {
                if preview.additions > 0 || preview.deletions > 0 {
                    audit.log(
                        &AuditEvent::new("", AuditEventKind::ToolResult,
                            format!("diff preview: {}", preview.summary))
                            .with_tool(tool_name)
                            .with_redacted_payload(preview.diff_text),
                    );
                }
            }

            if let Some(ref locks) = self.file_locks {
                locks.release(file_path, &self.agent_id);
            }

            return Ok(output);
        }

        execute_tool(tool_name, &value).map_err(ToolError::new)
    }
}

pub(super) fn build_permission_policy(allowed_tools: &[String]) -> PermissionPolicy {
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
            custom_tools: tools::CustomToolRegistry::default(),
            workspace_root: std::path::PathBuf::from("/tmp"),
            registry: None,
            governance: None,
            audit_logger: None,
            intelligence_config: None,
            file_locks: None,
            agent_id: "test-agent".to_string(),
            daemon_state: None,
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
