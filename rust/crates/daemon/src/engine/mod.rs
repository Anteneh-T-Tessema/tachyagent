//! Agent execution engine.
//!
//! Split into focused sub-modules:
//!   simple    — single-turn execution + shared helpers
//!   planning  — plan-and-execute pipeline (multi-step + edit-test-fix)
//!   executor  — `IntelligentToolExecutor` (tool dispatch, governance, MCP)

mod executor;
mod optimizer;
mod planning;
mod simple;
mod simulator;

pub use self::optimizer::{EvolutionManager, OptimizationProposal, OptimizationStatus};
pub use self::simulator::{SimulationResult, SimulatedToolCall, SimulationExecutor, SimulationJudge};

use std::path::Path;
use std::sync::{Arc, Mutex};

use audit::{AuditEvent, AuditEventKind, AuditLogger, AuditSeverity, GovernancePolicy};
use backend::BackendRegistry;
use intelligence::{
    CodebaseIndex, ContextSelector, IntelligenceConfig,
    build_optimized_prompt,
};
use runtime::{ConversationRuntime, PermissionPolicy, Session};

use platform::AgentConfig;

use self::executor::{IntelligentToolExecutor, build_permission_policy};
use self::simple::{run_simple, load_or_build_index};
use self::planning::run_with_planning;

fn required_write_file_path(prompt: &str, allowed_tools: &[String]) -> Option<String> {
    if allowed_tools.len() != 1 || allowed_tools.first().map(String::as_str) != Some("write_file") {
        return None;
    }

    prompt
        .lines()
        .find_map(|line| line.strip_prefix("REQUIRED_OUTPUT_PATH:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

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
    pub fn run_agent(
        agent_id: &str,
        config: &AgentConfig,
        prompt: &str,
        registry: &BackendRegistry,
        governance: &GovernancePolicy,
        audit_logger: Arc<AuditLogger>,
        intelligence_config: &IntelligenceConfig,
        workspace_root: &Path,
        file_locks: Option<runtime::FileLockManager>,
        daemon_state: Option<Arc<Mutex<crate::state::DaemonState>>>,
        is_simulation: bool,
    ) -> AgentRunResult {
        let model = &config.template.model;
        let enable_tools = !config.template.allowed_tools.is_empty();

        // Pre-flight: check if Ollama is reachable for local models
        let model_entry = registry.find_model(model);
        if let Some(entry) = model_entry {
            if format!("{:?}", entry.backend) == "Ollama" {
                let (alive, _) = backend::check_ollama("http://localhost:11434");
                if !alive {
                    return AgentRunResult {
                        agent_id: agent_id.to_string(),
                        success: false,
                        iterations: 0,
                        tool_invocations: 0,
                        summary: "Ollama is not running. Start it with: ollama serve".to_string(),
                    };
                }
            }
        }

        // Create backend
        let client = match registry.create_client(model, enable_tools) {
            Ok(c) => c,
            Err(e) => {
                audit_logger.log_signed(
                    &AuditEvent::new(
                        &config.session_id,
                        AuditEventKind::SessionEnd,
                        format!("backend creation failed: {e}"),
                    )
                    .with_severity(AuditSeverity::Critical)
                    .with_agent(agent_id),
                    None,
                );
                return AgentRunResult {
                    agent_id: agent_id.to_string(),
                    success: false,
                    iterations: 0,
                    tool_invocations: 0,
                    summary: format!("backend creation failed: {e}"),
                };
            }
        };

        // Load Agent Identity for signing
        let identity_mgr = platform::IdentityManager::new(&workspace_root.join(".tachy"));
        let agent_identity = identity_mgr.get_or_create_identity(agent_id).ok();

        let backend = backend::DynBackend::new(client);

        // --- Intelligence: Codebase Indexing ---
        let use_workspace_context = config.template.use_workspace_context;

        let index: Option<CodebaseIndex> = if intelligence_config.indexing_enabled && use_workspace_context {
            match load_or_build_index(workspace_root, &intelligence_config.indexer) {
                Ok(idx) => {
                    audit_logger.log_signed(
                        &AuditEvent::new(
                            &config.session_id,
                            AuditEventKind::SessionStart,
                            format!("indexed {} files", idx.project.total_files),
                        )
                        .with_agent(agent_id),
                        agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                    );
                    Some(idx)
                }
                Err(e) => {
                    audit_logger.log_signed(
                        &AuditEvent::new(
                            &config.session_id,
                            AuditEventKind::SessionStart,
                            format!("indexing failed (continuing without): {e}"),
                        )
                        .with_agent(agent_id),
                        agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                    );
                    None
                }
            }
        } else {
            None
        };

        // --- Intelligence: Smart Context Selection ---
        let context_text = if intelligence_config.context_enabled && use_workspace_context {
            if let Some(idx) = &index {
                let ctx_window = registry
                    .find_model(model)
                    .map_or(8192, |m| m.context_window);

                match ContextSelector::select_context(
                    prompt, idx, workspace_root, ctx_window, &intelligence_config.context,
                ) {
                    Ok(injection) => {
                        let rendered = ContextSelector::render_injection(&injection, idx);
                        audit_logger.log_signed(
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
                            agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                        );
                        Some(rendered)
                    }
                    Err(_) => None,
                }
            } else {
                None
            }
        } else {
            None
        };

        // --- Intelligence: Model-specific prompt optimization ---
        let mut system_prompt = build_optimized_prompt(
            model,
            &config.template.system_prompt,
            context_text.as_deref(),
        );

        // --- Intelligence: Inject Project DNA (TACHY.md) ---
        let dna_manager = intelligence::project_dna::ProjectDnaManager::new(workspace_root);
        let dna_context = dna_manager.as_system_context();
        system_prompt.push(dna_context);

        // --- Intelligence: Inject persistent memory ---
        let tachy_dir = workspace_root.join(".tachy");
        let memory = intelligence::AgentMemory::load(&tachy_dir);
        if let Some(memory_context) = memory.as_system_context() {
            system_prompt.push(memory_context);
            audit_logger.log_signed(
                &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                    format!("memory injected: {} entries", memory.entries().len()))
                    .with_agent(agent_id),
                agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
            );
        }

        // --- Intelligence: Inject dependency graph context ---
        if use_workspace_context {
            let dep_graph = intelligence::DependencyGraph::build(workspace_root);
            if !dep_graph.nodes.is_empty() {
                let mentioned: Vec<String> = dep_graph.nodes.keys()
                    .filter(|path| {
                        let stem = std::path::Path::new(path)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        prompt.contains(path.as_str())
                            || (!stem.is_empty() && stem.len() > 3 && prompt.contains(stem))
                    })
                    .take(5).cloned()
                    .collect();

                if !mentioned.is_empty() {
                    let mut dep_ctx = String::from("## Dependency Graph Context\n");
                    for file in &mentioned {
                        let imports = dep_graph.direct_imports(file);
                        let imported_by = dep_graph.nodes.get(file)
                            .map(|n| n.imported_by.clone())
                            .unwrap_or_default();
                        dep_ctx.push_str(&format!("### {file}\n"));
                        if !imports.is_empty() {
                            dep_ctx.push_str(&format!("- imports: {}\n", imports.join(", ")));
                        }
                        if !imported_by.is_empty() {
                            dep_ctx.push_str(&format!("- imported by: {}\n", imported_by.join(", ")));
                        }
                    }
                    system_prompt.push(dep_ctx);
                    audit_logger.log_signed(
                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                            format!("dep graph injected: {} files referenced", mentioned.len()))
                            .with_agent(agent_id),
                        agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                    );
                }
            }
        }

        // --- Load custom tools ---
        let custom_tools = tools::CustomToolRegistry::load(&tachy_dir);
        if !custom_tools.tools().is_empty() {
            audit_logger.log_signed(
                &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                    format!("custom tools loaded: {}", custom_tools.tools().len()))
                    .with_agent(agent_id),
                agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
            );
        }

        // Build permission policy — include git tools and custom tools if enabled
        let mut allowed = config.template.allowed_tools.clone();
        if !allowed.contains(&"remember".to_string()) {
            allowed.push("remember".to_string());
        }
        for tool in custom_tools.tools() {
            if !allowed.contains(&tool.name) {
                allowed.push(tool.name.clone());
            }
        }
        if intelligence_config.git_enabled {
            for name in ["git_status", "git_diff", "git_branch", "git_commit"] {
                if !allowed.contains(&name.to_string()) {
                    allowed.push(name.to_string());
                }
            }
        }
        if intelligence_config.indexing_enabled {
            for name in ["search_codebase", "expand_context", "swarm_refactor"] {
                if !allowed.contains(&name.to_string()) {
                    allowed.push(name.to_string());
                }
            }
        }
        if intelligence_config.vision_enabled {
            if !allowed.contains(&"capture_screenshot".to_string()) {
                allowed.push("capture_screenshot".to_string());
            }
        }
        let permission_policy: PermissionPolicy = build_permission_policy(&allowed);

        let tool_executor = IntelligentToolExecutor {
            allowed_tools: allowed,
            git_enabled: intelligence_config.git_enabled,
            custom_tools,
            workspace_root: workspace_root.to_path_buf(),
            registry: Some(Arc::new(registry.clone())),
            governance: Some(governance.clone()),
            audit_logger: Some(Arc::clone(&audit_logger)),
            intelligence_config: Some(intelligence_config.clone()),
            file_locks: file_locks.clone(),
            agent_id: agent_id.to_string(),
            daemon_state: daemon_state.clone(),
            agent_identity: agent_identity.clone(),
            is_simulation,
            sentinel: Some(Arc::new(audit::ComplianceSentinel::new())),
            inspector: Some(Arc::new(intelligence::VisualInspector::new(&config.template.model))),
        };

        let mut session = Session::new();
        session.team_id = config.team_id.clone();

        let mut runtime = ConversationRuntime::new(
            session,
            backend,
            tool_executor,
            permission_policy,
            system_prompt,
        )
        .with_required_write_file_path(required_write_file_path(prompt, &config.template.allowed_tools))
        .with_max_iterations(config.template.max_iterations);

        if let Some(state_mutex) = &daemon_state {
            let s = state_mutex.lock().unwrap();
            runtime = runtime.with_semantic_cache(s.semantic_cache.clone())
                             .with_embedder(s.embedding_client.clone());
        }

        audit_logger.log_signed(
            &AuditEvent::new(
                &config.session_id,
                AuditEventKind::SessionStart,
                format!("agent {agent_id} running: {prompt}"),
            )
            .with_agent(agent_id)
            .with_model(model),
            agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
        );

        // --- Intelligence: Plan-and-Execute or simple run ---
        let use_planning = intelligence_config.planning_enabled && config.template.use_planning;
        let result = if use_planning {
            run_with_planning(
                agent_id, config, prompt, &mut runtime, intelligence_config,
                workspace_root, &index, governance, audit_logger.as_ref(), registry,
                file_locks, daemon_state.clone(), agent_identity.clone(),
            )
        } else {
            run_simple(agent_id, config, prompt, &mut runtime, governance, audit_logger.as_ref())
        };

        // Visual Verification Audit (Phase 36)
        let mut final_result = result;
        if final_result.success && intelligence_config.vision_enabled {
            let inspector = intelligence::VisualInspector::new(&config.template.model);
            let design_intent = intelligence::IntentMatcher::extract_visual_goals(prompt);
            
            // Capture latest screenshot for audit
            let screenshot_dir = workspace_root.join(".tachy").join("vision");
            let screenshots = std::fs::read_dir(&screenshot_dir).ok().map(|rd| {
                rd.filter_map(|e| e.ok()).collect::<Vec<_>>()
            }).unwrap_or_default();
            
            if let Some(latest) = screenshots.iter().max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok()) {
                if let Ok(content) = std::fs::read_to_string(latest.path()) {
                    let audit = inspector.audit(&content, &design_intent);
                    if audit.status == intelligence::VisualStatus::Fail {
                        final_result.success = false;
                        final_result.summary = format!("VISUAL VETO: {} Audit Score: {:.2}", audit.design_violations.join(", "), audit.similarity_score);
                        
                        audit_logger.log_signed(
                            &AuditEvent::new(&config.session_id, AuditEventKind::GovernanceViolation, format!("Visual Inspector VETO: {}", final_result.summary))
                                .with_severity(audit::AuditSeverity::Warning),
                            agent_identity.as_ref().map(|i| i as &dyn audit::AsymmetricSigner),
                        );
                    }
                }
            }
        }
        let result = final_result;

        // ── Fire webhooks on agent completion ────────────────────────────────
        if let Some(ref ds) = daemon_state {
            if let Ok(state) = ds.lock() {
                let payload = serde_json::json!({
                    "agent_id": result.agent_id,
                    "success": result.success,
                    "iterations": result.iterations,
                    "tool_invocations": result.tool_invocations,
                    "summary": &result.summary,
                });
                let event = if result.success { "agent.completed" } else { "agent.failed" };
                state.fire_webhooks(event, &payload);
            }
        }

        // --- Persist the session for Gold Standard / intelligence extraction ---
        let session_dir = workspace_root.join(".tachy").join("sessions");
        if let Ok(_) = std::fs::create_dir_all(&session_dir) {
            let session_path = session_dir.join(format!("{}.json", config.session_id));
            let _ = runtime.into_session().save_to_path(&session_path);

            // Trigger check for fine-tuning
            if intelligence_config.finetune.auto_collect && intelligence::FinetuneDataset::should_trigger(&session_dir, intelligence_config.finetune.threshold) {
                if let Some(ref ds) = daemon_state {
                    if let Ok(state) = ds.lock() {
                        state.publish_event("finetune_suggested", serde_json::json!({
                            "reason": "Gold Standard threshold reached",
                            "count": intelligence_config.finetune.threshold,
                        }));
                    }
                }
            }
        }

        result
    }
}
