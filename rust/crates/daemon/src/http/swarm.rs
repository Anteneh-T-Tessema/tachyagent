use crate::http::types::{ErrorResponse, Response};
use crate::state::DaemonState;
use platform::{DelegatedTask, DelegationResult};
use std::sync::{Arc, Mutex};

pub async fn handle_swarm_execute(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let task: DelegatedTask = match serde_json::from_str(body) {
        Ok(t) => t,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid task: {e}"),
                },
            )
        }
    };

    // 1. Verify signature (Security layer)
    let hash = format!("{}:{}:{}", task.task_id, task.instruction, task.session_id);
    if !task.signature.is_empty() {
        if let Err(e) = platform::crypto::AgentIdentity::verify_static(
            &task.signature,
            &task.public_key,
            hash.as_bytes(),
        ) {
            return Response::json(
                401,
                &ErrorResponse {
                    error: format!("invalid task signature: {e}"),
                },
            );
        }
    }

    // 2. Prepare AgentConfig for local execution
    let template = state
        .lock()
        .unwrap()
        .config
        .agent_templates
        .iter()
        .find(|t| t.name == task.template_name)
        .cloned();

    let template = match template {
        Some(t) => t,
        None => {
            return Response::json(
                404,
                &ErrorResponse {
                    error: format!("template {} not found", task.template_name),
                },
            )
        }
    };

    let agent_config = platform::AgentConfig {
        template: template.clone(),
        session_id: task.session_id.clone(),
        working_directory: ".".to_string(), // Context files will be written relative to workspace
        environment: std::collections::BTreeMap::new(),
        team_id: None,
    };

    // 3. Write context files to workspace
    let workspace_root = state.lock().unwrap().workspace_root.clone();
    for file in &task.context {
        let abs_path = workspace_root.join(&file.path);
        if let Some(parent) = abs_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(abs_path, &file.content);
    }

    // 4. Run the agent locally
    let audit_logger = state.lock().unwrap().audit_logger.clone();
    let registry = state.lock().unwrap().registry.clone();
    let governance = state.lock().unwrap().config.governance.clone();
    let intelligence_config = state.lock().unwrap().config.intelligence.clone();

    // Use a temporary identity for signing the result if needed
    // In Phase 23, we use the daemon's own identity if available
    let agent_identity = state
        .lock()
        .unwrap()
        .identity
        .get_or_create_identity("swarm-worker")
        .ok();

    let result = crate::AgentEngine::run_agent(
        &task.task_id,
        &agent_config,
        &task.instruction,
        &registry,
        &governance,
        audit_logger,
        &intelligence_config,
        &workspace_root,
        None,
        Some(state.clone()),
        false,
    );

    // 5. Gather modified files
    let mut modified_files = Vec::new();
    for file in &task.context {
        if let Ok(content) = std::fs::read_to_string(workspace_root.join(&file.path)) {
            modified_files.push(platform::swarm::FileContext {
                path: file.path.clone(),
                content,
            });
        }
    }

    let mut response = DelegationResult {
        task_id: task.task_id,
        success: result.success,
        output: result.summary,
        modified_files,
        signature: String::new(),
        public_key: String::new(),
    };

    // 6. Sign result
    if let Some(ref identity) = agent_identity {
        let res_hash = format!("{}:{}", response.task_id, response.success);
        if let Ok(sig) = identity.asymmetric_sign(res_hash.as_bytes()) {
            response.signature = sig;
            response.public_key = identity.public_key_hex();
        }
    }

    Response::json(200, response)
}

pub async fn handle_swarm_register(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(serde::Deserialize)]
    struct RegisterReq {
        url: String,
    }

    let req: RegisterReq = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::json(
                400,
                &ErrorResponse {
                    error: format!("invalid request: {e}"),
                },
            )
        }
    };

    let peer = platform::PeerInfo {
        id: uuid::Uuid::new_v4().to_string(),
        url: req.url,
        status: platform::PeerStatus::Online,
        last_seen: 0, // SwarmManager will update this
        capabilities: vec!["generic-agent".to_string()],
    };

    state.lock().unwrap().swarm_manager.register_peer(peer);
    Response::json(200, serde_json::json!({ "status": "registered" }))
}

pub async fn handle_swarm_list_nodes(state: &Arc<Mutex<DaemonState>>) -> Response {
    let nodes = state.lock().unwrap().swarm_manager.list_peers();
    Response::json(200, &nodes)
}
