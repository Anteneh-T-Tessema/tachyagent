//! `IntelligentToolExecutor` — tool dispatch with git, RAG, custom tools, MCP, and governance.

use std::path::Path;
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
    /// Cryptographic identity for signing audit events.
    pub(super) agent_identity: Option<platform::crypto::AgentIdentity>,
    /// Whether this execution is a simulation (Phase 32).
    pub(super) is_simulation: bool,
    /// Compliance Sentinel for real-time security monitoring (Phase 33).
    pub(super) sentinel: Option<Arc<audit::ComplianceSentinel>>,
    /// Visual Inspector for multi-modal design auditing (Phase 36).
    pub(super) inspector: Option<Arc<intelligence::VisualInspector>>,
}

impl IntelligentToolExecutor {
    fn dispatch(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if !self.allowed_tools.iter().any(|t| t == tool_name) {
            return Err(ToolError::new(format!(
                "tool '{tool_name}' not in agent's allowed tools"
            )));
        }

        // Simulation Mode: Intercept mutation tools (Phase 32)
        if self.is_simulation {
            match tool_name {
                "write_file" | "replace_file_content" | "multi_replace_file_content" | 
                "delete_file" | "git_commit" | "git_push" | "run_command" => {
                    if let Some(logger) = &self.audit_logger {
                        logger.log_signed(
                            &AuditEvent::new(&self.agent_id, AuditEventKind::SelfRepair, format!("SIMULATION: suppressed mutation tool '{}'", tool_name))
                                .with_severity(audit::AuditSeverity::Warning),
                            self.agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                        );
                    }
                    return Ok(format!("SUCCESS (Simulation): '{}' suppressed", tool_name));
                }
                _ => {} // Allow read-only tools to pass through for high-fidelity simulation
            }
        }

        // Sentinel Security Scan: Pre-execution (Phase 33)
        if let Some(sentinel) = &self.sentinel {
            if let Some(violation) = sentinel.scan(input) {
                if let Some(logger) = &self.audit_logger {
                    logger.log_signed(
                        &AuditEvent::new(&self.agent_id, AuditEventKind::GovernanceViolation, format!("SENTINEL: {} ({:?})", violation.detail, violation.action_taken))
                            .with_severity(audit::AuditSeverity::Critical),
                        self.agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                    );
                }

                match violation.action_taken {
                    audit::ViolationAction::Kill => {
                        return Err(ToolError::new(format!("SECURITY TERMINATION: {} - Task killed by Sentinel.", violation.detail)));
                    }
                    audit::ViolationAction::Block => {
                        return Err(ToolError::new(format!("SECURITY BLOCK: {} - Action suppressed by Sentinel.", violation.detail)));
                    }
                    audit::ViolationAction::Warn => {
                        // Just log and continue (logger already called above)
                    }
                }
            }
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
                    team_id: None,
                };

                let result = AgentEngine::run_agent(
                    &sub_agent_id, &agent_config, prompt, reg, gov, Arc::clone(audit), intel,
                    &self.workspace_root, self.file_locks.clone(), self.daemon_state.clone(),
                    self.is_simulation,
                );

                return Ok(format!(
                    "Agent '{}' completed (success={}, {} iterations, {} tool calls):\n\n{}",
                    template, result.success, result.iterations, result.tool_invocations, result.summary
                ));
            }
            return Err(ToolError::new("call_agent not available in this context"));
        }

        // Handle compaction tool
        if tool_name == "compact_context" {
            return Ok("__TACTY_TRIGGER_COMPACTION__".to_string());
        }

        // Handle Project DNA tool
        if tool_name == "update_project_md" {
            let value: serde_json::Value = serde_json::from_str(input)
                .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
            return intelligence::project_dna::execute_update_project_md(&value, &self.workspace_root)
                .map_err(ToolError::new);
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
                    let listeners = s.swarm.mission_control.broadcast(event.clone()).unwrap_or(0);
                    {
                        let mut feed = s.swarm.mission_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    let feed = s.swarm.mission_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            "optimize_brain" => {
                let sessions_dir = self.workspace_root.join(".tachy").join("sessions");
                let output_dir = self.workspace_root.join(".tachy").join("finetune");
                let base_model = "gemma4:26b"; // Default

                if let Some(ref ds) = self.daemon_state {
                    let s = ds.lock().unwrap();
                    let msg = "[BRAIN] Starting autonomous optimization analysis...".to_string();
                    s.publish_event("brain_optimization_started", serde_json::json!({
                        "agent_id": self.agent_id,
                    }));
                    platform::log_info(&msg);
                }

                match intelligence::FinetuneDataset::prepare_training_bundle(&sessions_dir, &output_dir, base_model) {
                    Ok(path) => {
                        let res = format!("✓ Optimization bundle ready at {path}\nNext steps: Run `bash {path}/train.sh` to start local fine-tuning.");
                        if let Some(ref ds) = self.daemon_state {
                            let s = ds.lock().unwrap();
                            s.publish_event("brain_optimization_ready", serde_json::json!({
                                "path": path,
                                "agent_id": self.agent_id,
                            }));
                        }
                        return Ok(res);
                    }
                    Err(e) => return Err(ToolError::new(format!("Optimization failed: {e}"))),
                }
            }
            "capture_screenshot" => {
                let mut input_val: runtime::ScreenshotInput = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;
                
                // Inject workspace root for path resolution
                input_val.workspace_root = Some(self.workspace_root.to_string_lossy().to_string());

                if let Some(ref ds) = self.daemon_state {
                    let s = ds.lock().unwrap();
                    let msg = format!("[VISION] Capturing snapshot of {}", input_val.url);
                    s.publish_event("vision_snapshot_started", serde_json::json!({
                        "url": input_val.url,
                        "agent_id": self.agent_id,
                    }));
                    platform::log_info(&msg);
                }

                let result = runtime::capture_screenshot(input_val.clone())
                    .map_err(|e| ToolError::new(e.to_string()))?;

                if let Some(ref ds) = self.daemon_state {
                    let s = ds.lock().unwrap();
                    s.publish_event("vision_snapshot_completed", serde_json::json!({
                        "url": input_val.url,
                        "agent_id": self.agent_id,
                        "path": result.path,
                    }));
                    
                    // Phase 29: Broadcast to Mission Control for War Room live feed
                    let event = crate::internal_bus::MissionEvent::VisionUpdate {
                        agent_id: self.agent_id.clone(),
                        snapshot_id: format!("snap-{}", std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
                        thumbnail_url: format!("/api/vision/snapshot/{}", 
                            Path::new(&result.path).file_name().unwrap_or_default().to_string_lossy()),
                    };
                    let _ = s.swarm.mission_control.broadcast(event.clone());
                    {
                        let mut feed = s.swarm.mission_feed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        feed.push_front(event.clone());
                        if feed.len() > 100 { feed.pop_back(); }
                    }
                    
                    // Unified publishing to SSE bus
                    s.publish_event("mission_event", serde_json::to_value(event).unwrap_or_default());
                }

                // Log Visual Snapshot to Audit Trail
                if let Some(audit) = &self.audit_logger {
                    audit.log_signed(
                        &AuditEvent::new(&self.agent_id, AuditEventKind::VisualSnapshot,
                            format!("visual snapshot captured for {}", input_val.url))
                            .with_tool(tool_name)
                            .with_visual_anchor(&result.path),
                        self.agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                    );
                }

                // Telemetry
                if let Some(ref ds) = self.daemon_state {
                    let s = ds.lock().unwrap();
                    crate::telemetry::record_vision_snapshot(&s.tracer, &self.agent_id, &input_val.url, &result.path);
                }

                return Ok(serde_json::to_string(&result).unwrap());
            }
            "get_accessibility_tree" => {
                let input_val: runtime::AccessibilityTreeInput = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;

                if let Some(ref ds) = self.daemon_state {
                    let s = ds.lock().unwrap();
                    let msg = format!("[VISION] Extracting structural tree for {}", input_val.url);
                    s.publish_event("vision_analysis_started", serde_json::json!({
                        "url": input_val.url,
                        "agent_id": self.agent_id,
                    }));
                    platform::log_info(&msg);
                }

                let result = runtime::get_accessibility_tree(input_val)
                    .map_err(|e| ToolError::new(e.to_string()))?;

                return Ok(result);
            }
            "visual_diff" => {
                let input_val: runtime::VisualDiffInput = serde_json::from_str(input)
                    .map_err(|e| ToolError::new(format!("invalid input: {e}")))?;

                let result = runtime::compare_snapshots(&input_val.path_a, &input_val.path_b)
                    .map_err(|e| ToolError::new(e.to_string()))?;

                return Ok(result);
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
                return s.connectivity.mcp_client.call_tool(tool_name, &value).map_err(ToolError::new);
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
                        audit.log_signed(
                            &AuditEvent::new("", AuditEventKind::PermissionDenied,
                                format!("write to '{file_path}' requires approval (governance policy)"))
                                .with_tool(tool_name),
                            self.agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
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
                    audit.log_signed(
                        &AuditEvent::new("", AuditEventKind::ToolResult,
                            format!("diff preview: {}", preview.summary))
                            .with_tool(tool_name)
                            .with_redacted_payload(preview.diff_text),
                        self.agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                    );
                }
            }

            if let Some(ref locks) = self.file_locks {
                locks.release(file_path, &self.agent_id);
            }

            // Incremental RAG: update the codebase index for the file that
            // was just written so future context retrieval reflects the change.
            if let Some(ref ds) = self.daemon_state {
                if let Ok(mut s) = ds.try_lock() {
                    if let Some(ref existing_index) = s.codebase_index.clone() {
                        let rel = std::path::Path::new(file_path)
                            .strip_prefix(&self.workspace_root)
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| file_path.to_string());
                        let rel_str = rel.as_str();
                        if let Ok((updated, n)) = intelligence::CodebaseIndexer::reindex_changed_files(
                            &self.workspace_root,
                            existing_index,
                            &[rel_str],
                            &intelligence::IndexerConfig::default(),
                        ) {
                            if n > 0 {
                                s.codebase_index = Some(updated);
                            }
                        }
                    }
                }
            }

            return Ok(output);
        }

        execute_tool(tool_name, &value).map_err(ToolError::new)
    }
}

impl ToolExecutor for IntelligentToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        let start = std::time::Instant::now();
        if let Some(ref ds) = self.daemon_state {
            if let Ok(s) = ds.try_lock() {
                s.publish_event("tool_start", serde_json::json!({
                    "agent_id": self.agent_id,
                    "tool": tool_name,
                    "input_preview": &input[..input.len().min(200)],
                }));
            }
        }
        let result = self.dispatch(tool_name, input);
        let elapsed_ms = start.elapsed().as_millis();
        if let Some(ref ds) = self.daemon_state {
            if let Ok(s) = ds.try_lock() {
                s.publish_event("tool_end", serde_json::json!({
                    "agent_id": self.agent_id,
                    "tool": tool_name,
                    "success": result.is_ok(),
                    "elapsed_ms": elapsed_ms,
                    "error": result.as_ref().err().map(|e| e.to_string()),
                }));
            }
        }
        result
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
            agent_identity: None,
            is_simulation: false,
            sentinel: None,
            inspector: None,
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
