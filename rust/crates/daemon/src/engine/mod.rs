//! Agent execution engine.
//!
//! Split into focused sub-modules:
//!   simple    — single-turn execution + shared helpers
//!   planning  — plan-and-execute pipeline (multi-step + edit-test-fix)
//!   executor  — `IntelligentToolExecutor` (tool dispatch, governance, MCP)

mod executor;
mod planning;
mod simple;

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
        audit_logger: &AuditLogger,
        intelligence_config: &IntelligenceConfig,
        workspace_root: &Path,
        file_locks: Option<runtime::FileLockManager>,
        daemon_state: Option<Arc<Mutex<crate::state::DaemonState>>>,
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

        let backend = backend::DynBackend::new(client);

        // --- Intelligence: Codebase Indexing ---
        let index: Option<CodebaseIndex> = if intelligence_config.indexing_enabled {
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
        let context_text = if intelligence_config.context_enabled {
            if let Some(idx) = &index {
                let ctx_window = registry
                    .find_model(model)
                    .map_or(8192, |m| m.context_window);

                match ContextSelector::select_context(
                    prompt, idx, workspace_root, ctx_window, &intelligence_config.context,
                ) {
                    Ok(injection) => {
                        let rendered = ContextSelector::render_injection(&injection, idx);
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

        // --- Intelligence: Inject persistent memory ---
        let tachy_dir = workspace_root.join(".tachy");
        let memory = intelligence::AgentMemory::load(&tachy_dir);
        if let Some(memory_context) = memory.as_system_context() {
            system_prompt.push(memory_context);
            audit_logger.log(
                &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                    format!("memory injected: {} entries", memory.entries().len()))
                    .with_agent(agent_id),
            );
        }

        // --- Intelligence: Inject dependency graph context ---
        {
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
                    audit_logger.log(
                        &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                            format!("dep graph injected: {} files referenced", mentioned.len()))
                            .with_agent(agent_id),
                    );
                }
            }
        }

        // --- Load custom tools ---
        let custom_tools = tools::CustomToolRegistry::load(&tachy_dir);
        if !custom_tools.tools().is_empty() {
            audit_logger.log(
                &AuditEvent::new(&config.session_id, AuditEventKind::SessionStart,
                    format!("custom tools loaded: {}", custom_tools.tools().len()))
                    .with_agent(agent_id),
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
        let permission_policy: PermissionPolicy = build_permission_policy(&allowed);

        let tool_executor = IntelligentToolExecutor {
            allowed_tools: allowed,
            git_enabled: intelligence_config.git_enabled,
            custom_tools,
            workspace_root: workspace_root.to_path_buf(),
            registry: Some(Arc::new(registry.clone())),
            governance: Some(governance.clone()),
            audit_logger: Some(Arc::new(AuditLogger::new())),
            intelligence_config: Some(intelligence_config.clone()),
            file_locks: file_locks.clone(),
            agent_id: agent_id.to_string(),
            daemon_state: daemon_state.clone(),
        };

        let mut runtime = ConversationRuntime::new(
            Session::new(),
            backend,
            tool_executor,
            permission_policy,
            system_prompt,
        )
        .with_max_iterations(config.template.max_iterations);

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
        let use_planning = intelligence_config.planning_enabled && config.template.use_planning;
        let result = if use_planning {
            run_with_planning(
                agent_id, config, prompt, &mut runtime, intelligence_config,
                workspace_root, &index, governance, audit_logger, registry,
                file_locks, daemon_state.clone(),
            )
        } else {
            run_simple(agent_id, config, prompt, &mut runtime, governance, audit_logger)
        };

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

        result
    }
}
