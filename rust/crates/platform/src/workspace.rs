use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::AgentTemplate;
use audit::GovernancePolicy;
use backend::{BackendConfig, BackendKind};
use intelligence::IntelligenceConfig;

/// Top-level platform configuration — loaded from a config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    /// Where audit logs are stored.
    pub audit_log_path: String,
    /// Where agent sessions are persisted.
    pub sessions_dir: String,
    /// Backend configurations (keyed by name).
    pub backends: std::collections::BTreeMap<String, BackendConfig>,
    /// Default model to use when none specified.
    pub default_model: String,
    /// Governance policy for enterprise compliance.
    pub governance: GovernancePolicy,
    /// Pre-configured agent templates.
    pub agent_templates: Vec<AgentTemplate>,
    /// HTTP API listen address (for daemon mode).
    pub api_listen: Option<String>,
    /// Intelligence features configuration.
    #[serde(default)]
    pub intelligence: IntelligenceConfig,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        let mut backends = std::collections::BTreeMap::new();
        backends.insert(
            "ollama".to_string(),
            BackendConfig {
                kind: BackendKind::Ollama,
                base_url: Some("http://localhost:11434".to_string()),
                api_key: None,
                default_model: Some("gemma4:26b".to_string()),
            },
        );

        Self {
            audit_log_path: ".tachy/audit.jsonl".to_string(),
            sessions_dir: ".tachy/sessions".to_string(),
            backends,
            default_model: "gemma4:26b".to_string(),
            governance: GovernancePolicy::enterprise_default(),
            agent_templates: vec![
                AgentTemplate::chat_assistant(),
                AgentTemplate::code_reviewer(),
                AgentTemplate::security_scanner(),
                AgentTemplate::doc_generator(),
                AgentTemplate::test_runner(),
                AgentTemplate::refactor(),
                AgentTemplate::migrator(),
                AgentTemplate::localizer(),
                AgentTemplate::benchmark(),
                AgentTemplate::db_migrator(),
                AgentTemplate::api_compat(),
            ],
            api_listen: Some("127.0.0.1:7777".to_string()),
            intelligence: IntelligenceConfig::default(),
        }
    }
}

impl PlatformConfig {
    /// Load config from a JSON file, falling back to defaults.
    pub fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                eprintln!("warning: failed to parse {}: {e}, using defaults", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Save config to a JSON file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create config dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize config: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("failed to write config: {e}"))
    }
}

/// Represents an initialized platform workspace.
pub struct PlatformWorkspace {
    pub config: PlatformConfig,
    pub root: PathBuf,
}

impl PlatformWorkspace {
    /// Initialize a workspace at the given root directory.
    pub fn init(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        let config_path = root.join(".tachy").join("config.json");
        let config = PlatformConfig::load(&config_path);

        // Ensure directories exist
        let tachy_dir = root.join(".tachy");
        std::fs::create_dir_all(&tachy_dir)
            .map_err(|e| format!("failed to create .tachy dir: {e}"))?;
        std::fs::create_dir_all(root.join(&config.sessions_dir))
            .map_err(|e| format!("failed to create sessions dir: {e}"))?;

        // Save config if it doesn't exist
        if !config_path.exists() {
            config.save(&config_path)?;
        }

        Ok(Self { config, root })
    }

    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.root.join(".tachy").join("config.json")
    }

    #[must_use]
    pub fn audit_log_path(&self) -> PathBuf {
        self.root.join(&self.config.audit_log_path)
    }

    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join(&self.config.sessions_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_backends_and_templates() {
        let config = PlatformConfig::default();
        assert!(config.backends.contains_key("ollama"));
        assert!(!config.agent_templates.is_empty());
        assert!(config.governance.block_destructive_shell);
    }

    #[test]
    fn workspace_init_creates_directories() {
        let dir = std::env::temp_dir().join(format!(
            "tachy-ws-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let ws = PlatformWorkspace::init(&dir).expect("workspace init should succeed");
        assert!(ws.config_path().exists());
        assert!(dir.join(".tachy").exists());

        std::fs::remove_dir_all(dir).ok();
    }
}
