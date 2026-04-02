use std::collections::BTreeMap;
use std::path::PathBuf;

use audit::{AuditEvent, AuditEventKind, AuditLogger, FileAuditSink};
use backend::BackendRegistry;
use platform::{
    AgentConfig, AgentInstance, PlatformConfig, PlatformWorkspace,
    ScheduleRule, ScheduledTask, TaskScheduler,
};
use serde::{Deserialize, Serialize};

/// Persisted state — saved to .tachy/state.json on every mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    agents: BTreeMap<String, AgentInstance>,
    agent_counter: u64,
    task_counter: u64,
}

/// Shared daemon state, wrapped in Arc<Mutex<>> for thread safety.
pub struct DaemonState {
    pub workspace_root: PathBuf,
    pub config: PlatformConfig,
    pub registry: BackendRegistry,
    pub scheduler: TaskScheduler,
    pub agents: BTreeMap<String, AgentInstance>,
    pub audit_logger: AuditLogger,
    pub agent_counter: u64,
    pub task_counter: u64,
    pub api_key: Option<String>,
}

impl DaemonState {
    pub fn init(workspace_root: PathBuf) -> Result<Self, String> {
        let ws = PlatformWorkspace::init(&workspace_root)?;

        let mut audit_logger = AuditLogger::new();
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

        let (agents, agent_counter, task_counter) = match persisted {
            Some(p) => {
                let count = p.agents.len();
                audit_logger.log(&AuditEvent::new(
                    "daemon",
                    AuditEventKind::SessionStart,
                    format!("restored {count} agents from disk"),
                ));
                (p.agents, p.agent_counter, p.task_counter)
            }
            None => (BTreeMap::new(), 0, 0),
        };

        Ok(Self {
            workspace_root,
            config: ws.config,
            registry,
            scheduler: TaskScheduler::new(),
            agents,
            audit_logger,
            agent_counter,
            task_counter,
            api_key,
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
            agent_counter: self.agent_counter,
            task_counter: self.task_counter,
        };
        if let Ok(json) = serde_json::to_string_pretty(&persisted) {
            let _ = std::fs::write(&state_path, json);
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
