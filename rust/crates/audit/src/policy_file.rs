//! Policy-as-code — load governance rules from YAML/JSON files.
//! Allows enterprises to version-control their agent policies.

use crate::policy::{GovernancePolicy, ToolGovernanceRule};
use serde::{Deserialize, Serialize};

/// A policy file that can be loaded from .tachy/policy.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFile {
    /// Schema version
    pub version: u32,
    /// Human-readable policy name
    pub name: String,
    /// Description of what this policy enforces
    pub description: Option<String>,
    /// The governance rules
    pub rules: PolicyRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRules {
    /// Block destructive shell commands
    #[serde(default = "default_true")]
    pub block_destructive_shell: bool,
    /// Maximum total tool invocations per session
    pub max_total_tool_invocations: Option<u32>,
    /// Protected file paths (glob patterns)
    #[serde(default)]
    pub protected_paths: Vec<String>,
    /// Per-tool rules
    #[serde(default)]
    pub tool_rules: Vec<ToolRule>,
    /// Blocked command patterns (regex-like)
    #[serde(default)]
    pub blocked_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRule {
    pub tool: String,
    pub max_invocations: Option<u32>,
    pub requires_approval: bool,
    #[serde(default)]
    pub blocked_patterns: Vec<String>,
}

fn default_true() -> bool { true }

impl PolicyFile {
    /// Load a policy from a JSON file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read policy file: {e}"))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("failed to parse policy file: {e}"))
    }

    /// Convert to a GovernancePolicy.
    pub fn to_governance_policy(&self) -> GovernancePolicy {
        let tool_rules = self.rules.tool_rules.iter().map(|r| {
            ToolGovernanceRule {
                tool_name: r.tool.clone(),
                max_invocations_per_session: r.max_invocations,
                requires_approval: r.requires_approval,
                redact_in_logs: false,
                blocked_patterns: r.blocked_patterns.clone(),
            }
        }).collect();

        GovernancePolicy {
            tool_rules,
            max_total_tool_invocations: self.rules.max_total_tool_invocations,
            block_destructive_shell: self.rules.block_destructive_shell,
            protected_paths: self.rules.protected_paths.clone(),
            approval_required_paths: Vec::new(),
            enforce_all_approvals: false,
        }
    }

    /// Create a default enterprise policy file.
    pub fn enterprise_default() -> Self {
        Self {
            version: 1,
            name: "Enterprise Default".to_string(),
            description: Some("Default security policy for enterprise environments".to_string()),
            rules: PolicyRules {
                block_destructive_shell: true,
                max_total_tool_invocations: Some(500),
                protected_paths: vec![
                    "/etc/**".to_string(),
                    "/usr/**".to_string(),
                    "~/.ssh/**".to_string(),
                    "**/.env".to_string(),
                    "**/secrets/**".to_string(),
                ],
                tool_rules: vec![
                    ToolRule {
                        tool: "bash".to_string(),
                        max_invocations: Some(50),
                        requires_approval: false,
                        blocked_patterns: vec![
                            r"rm\s+-rf\s+/".to_string(),
                            r"curl.*\|.*sh".to_string(),
                        ],
                    },
                    ToolRule {
                        tool: "write_file".to_string(),
                        max_invocations: Some(200),
                        requires_approval: false,
                        blocked_patterns: vec![],
                    },
                ],
                blocked_patterns: vec![],
            },
        }
    }

    /// Save the policy to a JSON file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize policy: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create policy dir: {e}"))?;
        }
        std::fs::write(path, json)
            .map_err(|e| format!("failed to write policy file: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enterprise_default_is_valid() {
        let policy = PolicyFile::enterprise_default();
        assert_eq!(policy.version, 1);
        assert!(policy.rules.block_destructive_shell);
        assert_eq!(policy.rules.max_total_tool_invocations, Some(500));
        assert!(!policy.rules.protected_paths.is_empty());
    }

    #[test]
    fn converts_to_governance_policy() {
        let file = PolicyFile::enterprise_default();
        let governance = file.to_governance_policy();
        assert!(governance.block_destructive_shell);
        assert_eq!(governance.max_total_tool_invocations, Some(500));
        assert!(!governance.tool_rules.is_empty());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "tachy-policy-test-{}",
            std::process::id(),
        ));
        let path = dir.join("policy.json");

        let original = PolicyFile::enterprise_default();
        original.save(&path).expect("should save");

        let loaded = PolicyFile::load(&path).expect("should load");
        assert_eq!(loaded.version, original.version);
        assert_eq!(loaded.name, original.name);
        assert_eq!(loaded.rules.block_destructive_shell, original.rules.block_destructive_shell);

        std::fs::remove_dir_all(dir).ok();
    }
}
