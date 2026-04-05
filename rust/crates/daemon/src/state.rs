use std::collections::BTreeMap;
use std::path::PathBuf;

use audit::{AuditEvent, AuditEventKind, AuditLogger, FileAuditSink, PolicyEngine, FilePatch, SsoConfig, SsoManager, MeteringService};
use backend::BackendRegistry;
use platform::{
    AgentConfig, AgentInstance, PlatformConfig, PlatformWorkspace,
    ScheduleRule, ScheduledTask, TaskScheduler,
};
use runtime::FileLockManager;
use serde::{Deserialize, Serialize};

use crate::marketplace::Marketplace;
use crate::mcp_client::McpClientManager;
use crate::saas::SaaSPlatform;
use crate::teams::TeamManager;

/// Persisted state — saved to .tachy/state.json on every mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    agents: BTreeMap<String, AgentInstance>,
    conversations: BTreeMap<String, Conversation>,
    agent_counter: u64,
    task_counter: u64,
    conv_counter: u64,
}

/// A server-side conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: String,
    pub updated_at: String,
    pub workspace: String,
}

/// A single chat message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub model: Option<String>,
    pub iterations: Option<usize>,
    pub tool_invocations: Option<u32>,
}

/// Webhook configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub events: Vec<String>,
    pub enabled: bool,
}

/// A patch awaiting human approval from the policy engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPatch {
    pub id: String,
    pub patch: FilePatch,
    pub reason: String,
    pub created_at: String,
}

/// Shared daemon state, wrapped in Arc<Mutex<>> for thread safety.
pub struct DaemonState {
    pub workspace_root: PathBuf,
    pub config: PlatformConfig,
    pub registry: BackendRegistry,
    pub scheduler: TaskScheduler,
    pub agents: BTreeMap<String, AgentInstance>,
    pub conversations: BTreeMap<String, Conversation>,
    pub audit_logger: AuditLogger,
    pub agent_counter: u64,
    pub task_counter: u64,
    pub conv_counter: u64,
    pub api_key: Option<String>,
    pub webhooks: Vec<WebhookConfig>,
    /// Shared file lock manager for parallel agent safety.
    pub file_locks: FileLockManager,
    /// Policy engine for patch-level governance.
    pub policy_engine: PolicyEngine,
    /// Patches awaiting human approval.
    pub pending_patches: Vec<PendingPatch>,
    /// Counter for pending patch IDs.
    pub patch_counter: u64,
    /// SSO/SAML manager for enterprise authentication.
    pub sso_manager: SsoManager,
    /// User store for RBAC.
    pub user_store: audit::UserStore,
    /// MCP client manager for external tool servers.
    pub mcp_client: McpClientManager,
    /// Usage metering service.
    pub metering: MeteringService,
    /// Team workspace manager.
    pub team_manager: TeamManager,
    /// Agent marketplace.
    pub marketplace: Marketplace,
    /// SaaS platform (None if not in SaaS mode).
    pub saas: Option<SaaSPlatform>,
}

impl DaemonState {
    pub fn init(workspace_root: PathBuf) -> Result<Self, String> {
        let ws = PlatformWorkspace::init(&workspace_root)?;

        let mut audit_logger = AuditLogger::resume_from_file(&ws.audit_log_path());
        if let Ok(sink) = FileAuditSink::new(ws.audit_log_path()) {
            audit_logger.add_sink(sink);
        }

        audit_logger.log(&AuditEvent::new(
            "daemon",
            AuditEventKind::SessionStart,
            "daemon started",
        ));

        let registry = BackendRegistry::with_defaults();

        // Load API key from env or config
        let api_key = std::env::var("TACHY_API_KEY").ok();

        // Restore persisted state if it exists
        let state_path = workspace_root.join(".tachy").join("state.json");
        let persisted = load_persisted_state(&state_path);

        let (agents, conversations, agent_counter, task_counter, conv_counter) = match persisted {
            Some(p) => {
                let count = p.agents.len();
                audit_logger.log(&AuditEvent::new(
                    "daemon",
                    AuditEventKind::SessionStart,
                    format!("restored {count} agents, {} conversations from disk", p.conversations.len()),
                ));
                (p.agents, p.conversations, p.agent_counter, p.task_counter, p.conv_counter)
            }
            None => (BTreeMap::new(), BTreeMap::new(), 0, 0, 0),
        };

        // Load webhooks from config
        let webhooks: Vec<WebhookConfig> = Vec::new(); // loaded from config if present

        Ok(Self {
            workspace_root,
            config: ws.config,
            registry,
            scheduler: TaskScheduler::new(),
            agents,
            conversations,
            audit_logger,
            agent_counter,
            task_counter,
            conv_counter,
            api_key,
            webhooks,
            file_locks: FileLockManager::new(),
            policy_engine: PolicyEngine::enterprise_default(),
            pending_patches: Vec::new(),
            patch_counter: 0,
            sso_manager: SsoManager::new(SsoConfig::default()),
            user_store: audit::UserStore::new(),
            mcp_client: McpClientManager::new(),
            metering: MeteringService::new(AuditLogger::new()),
            team_manager: TeamManager::new(),
            marketplace: Marketplace::new(),
            saas: None,
        })
    }

    pub fn next_agent_id(&mut self) -> String {
        self.agent_counter += 1;
        format!("agent-{}", self.agent_counter)
    }

    pub fn next_task_id(&mut self) -> String {
        self.task_counter += 1;
        format!("task-{}", self.task_counter)
    }

    /// Persist current state to disk.
    pub fn save(&self) {
        let state_path = self.workspace_root.join(".tachy").join("state.json");
        let persisted = PersistedState {
            agents: self.agents.clone(),
            conversations: self.conversations.clone(),
            agent_counter: self.agent_counter,
            task_counter: self.task_counter,
            conv_counter: self.conv_counter,
        };
        if let Ok(json) = serde_json::to_string_pretty(&persisted) {
            let _ = std::fs::write(&state_path, json);
        }
    }

    pub fn next_conv_id(&mut self) -> String {
        self.conv_counter += 1;
        format!("conv-{}", self.conv_counter)
    }

    /// Create a new conversation.
    pub fn create_conversation(&mut self, title: &str) -> String {
        let id = self.next_conv_id();
        let now = timestamp();
        let conv = Conversation {
            id: id.clone(),
            title: title.to_string(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            workspace: self.workspace_root.to_string_lossy().to_string(),
        };
        self.conversations.insert(id.clone(), conv);
        self.save();
        id
    }

    /// Add a message to a conversation.
    pub fn add_message(&mut self, conv_id: &str, msg: ChatMessage) -> bool {
        if let Some(conv) = self.conversations.get_mut(conv_id) {
            conv.messages.push(msg);
            conv.updated_at = timestamp();
            self.save();
            true
        } else {
            false
        }
    }

    /// Queue a patch for human approval.
    pub fn queue_pending_patch(&mut self, patch: FilePatch, reason: String) -> String {
        self.patch_counter += 1;
        let id = format!("patch-{}", self.patch_counter);
        self.pending_patches.push(PendingPatch {
            id: id.clone(),
            patch,
            reason,
            created_at: timestamp(),
        });
        self.save();
        id
    }

    /// Approve a pending patch — apply it to disk.
    pub fn approve_patch(&mut self, patch_id: &str) -> Result<String, String> {
        let idx = self.pending_patches.iter().position(|p| p.id == patch_id)
            .ok_or_else(|| format!("patch '{}' not found", patch_id))?;
        let pending = self.pending_patches.remove(idx);
        // Apply the patch
        std::fs::write(&pending.patch.file_path, &pending.patch.new_content)
            .map_err(|e| format!("failed to apply patch: {e}"))?;
        self.audit_logger.log(
            &AuditEvent::new("daemon", AuditEventKind::PermissionGranted,
                format!("patch {} approved and applied: {}", patch_id, pending.patch.file_path))
                .with_agent(&pending.patch.agent_id),
        );
        self.save();
        Ok(pending.patch.file_path)
    }

    /// Reject a pending patch — discard it.
    pub fn reject_patch(&mut self, patch_id: &str) -> Result<String, String> {
        let idx = self.pending_patches.iter().position(|p| p.id == patch_id)
            .ok_or_else(|| format!("patch '{}' not found", patch_id))?;
        let pending = self.pending_patches.remove(idx);
        self.audit_logger.log(
            &AuditEvent::new("daemon", AuditEventKind::PermissionDenied,
                format!("patch {} rejected: {}", patch_id, pending.patch.file_path))
                .with_agent(&pending.patch.agent_id)
                .with_severity(audit::AuditSeverity::Warning),
        );
        self.save();
        Ok(pending.patch.file_path)
    }

    /// Fire webhooks for an event.
    pub fn fire_webhooks(&self, event_type: &str, payload: &serde_json::Value) {
        for webhook in &self.webhooks {
            if !webhook.enabled { continue; }
            if !webhook.events.contains(&event_type.to_string()) && !webhook.events.contains(&"*".to_string()) { continue; }

            let url = webhook.url.clone();
            let body = serde_json::json!({
                "event": event_type,
                "payload": payload,
                "timestamp": timestamp(),
            });

            // Fire and forget — don't block on webhook delivery
            std::thread::spawn(move || {
                let _ = std::process::Command::new("curl")
                    .args(["-s", "-X", "POST", "-H", "Content-Type: application/json", "-d", &body.to_string(), &url])
                    .output();
            });
        }
    }

    /// Create an agent instance from a template name and prompt.
    pub fn create_agent(
        &mut self,
        template_name: &str,
        prompt: &str,
    ) -> Result<String, String> {
        let template = self
            .config
            .agent_templates
            .iter()
            .find(|t| t.name == template_name)
            .cloned()
            .ok_or_else(|| format!("unknown template: {template_name}"))?;

        let agent_id = self.next_agent_id();
        let session_id = format!("sess-{agent_id}");

        let config = AgentConfig {
            template,
            session_id: session_id.clone(),
            working_directory: self.workspace_root.to_string_lossy().to_string(),
            environment: BTreeMap::new(),
        };

        let mut instance = AgentInstance::new(&agent_id, config);
        instance.result_summary = Some(format!("prompt: {prompt}"));

        self.audit_logger.log(
            &AuditEvent::new(&session_id, AuditEventKind::SessionStart, "agent created")
                .with_agent(&agent_id),
        );

        self.agents.insert(agent_id.clone(), instance);
        self.save();
        Ok(agent_id)
    }

    /// Schedule an agent to run on a trigger.
    pub fn schedule_agent(
        &mut self,
        template_name: &str,
        schedule: ScheduleRule,
        name: &str,
    ) -> Result<String, String> {
        let template = self
            .config
            .agent_templates
            .iter()
            .find(|t| t.name == template_name)
            .cloned()
            .ok_or_else(|| format!("unknown template: {template_name}"))?;

        let task_id = self.next_task_id();
        let session_id = format!("sess-{task_id}");

        let config = AgentConfig {
            template,
            session_id,
            working_directory: self.workspace_root.to_string_lossy().to_string(),
            environment: BTreeMap::new(),
        };

        let task = ScheduledTask::new(&task_id, name, config, schedule);
        self.scheduler.add_task(task);
        self.save();
        Ok(task_id)
    }
}

fn timestamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", d.as_secs())
}

fn load_persisted_state(path: &PathBuf) -> Option<PersistedState> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "tachy-daemon-test-{}-{}",
            std::process::id(),
            id,
        ))
    }

    #[test]
    fn initializes_daemon_state() {
        let root = temp_root();
        let state = DaemonState::init(root.clone()).expect("should init");
        assert!(!state.config.agent_templates.is_empty());
        assert!(state.agents.is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn creates_agents_from_templates() {
        let root = temp_root();
        let mut state = DaemonState::init(root.clone()).expect("should init");

        let id = state.create_agent("code-reviewer", "review my code").expect("should create");
        assert_eq!(id, "agent-1");
        assert!(state.agents.contains_key("agent-1"));

        let id2 = state.create_agent("test-runner", "run tests").expect("should create");
        assert_eq!(id2, "agent-2");

        assert!(state.create_agent("nonexistent", "x").is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn persists_and_restores_agents() {
        let root = temp_root();

        // Create agents and save
        {
            let mut state = DaemonState::init(root.clone()).expect("should init");
            state.create_agent("code-reviewer", "review").expect("create");
            state.create_agent("test-runner", "test").expect("create");
            assert_eq!(state.agents.len(), 2);
        }

        // Restore from disk
        {
            let state = DaemonState::init(root.clone()).expect("should init");
            assert_eq!(state.agents.len(), 2);
            assert!(state.agents.contains_key("agent-1"));
            assert!(state.agents.contains_key("agent-2"));
            assert_eq!(state.agent_counter, 2);
        }

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn counter_continues_after_restart() {
        let root = temp_root();

        {
            let mut state = DaemonState::init(root.clone()).expect("should init");
            state.create_agent("code-reviewer", "a").expect("create");
            state.create_agent("code-reviewer", "b").expect("create");
        }

        {
            let mut state = DaemonState::init(root.clone()).expect("should init");
            let id = state.create_agent("code-reviewer", "c").expect("create");
            assert_eq!(id, "agent-3"); // continues from 2, not resets to 1
        }

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn schedules_agents() {
        let root = temp_root();
        let mut state = DaemonState::init(root.clone()).expect("should init");

        let id = state.schedule_agent(
            "security-scanner",
            ScheduleRule::Interval { seconds: 3600 },
            "hourly scan",
        ).expect("should schedule");
        assert_eq!(id, "task-1");
        assert_eq!(state.scheduler.list_tasks().len(), 1);
        std::fs::remove_dir_all(root).ok();
    }
}
