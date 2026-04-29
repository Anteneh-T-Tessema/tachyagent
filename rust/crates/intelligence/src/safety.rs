//! Safety Agent for Command Sanitization.
//!
//! Provides rule-based and model-based verification for shell commands 
//! to prevent destructive or unauthorized actions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyReport {
    pub is_safe: bool,
    pub reason: Option<String>,
    pub suggested_command: Option<String>,
}

pub struct SafetyAgent;

impl SafetyAgent {
    /// Verify a bash command against standard safety rules.
    pub fn verify_command(command: &str) -> SafetyReport {
        let cmd = command.trim().to_lowercase();
        
        // 1. Rule-based checks (Hard blockers)
        if cmd.contains("rm -rf /") || cmd.contains("rm -rf /*") {
            return SafetyReport {
                is_safe: false,
                reason: Some("Destructive root-level deletion blocked.".to_string()),
                suggested_command: None,
            };
        }

        if cmd.contains("curl") && cmd.contains("| bash") {
            return SafetyReport {
                is_safe: false,
                reason: Some("Remote script pipe to bash is a major security risk.".to_string()),
                suggested_command: None,
            };
        }

        if cmd.contains(".env") && (cmd.contains("cat") || cmd.contains("grep") || cmd.contains("read")) {
            return SafetyReport {
                is_safe: false,
                reason: Some("Accessing .env files is restricted to prevent credential leakage.".to_string()),
                suggested_command: None,
            };
        }

        // 2. Environment-specific checks (e.g. git)
        if cmd.contains("git push --force") {
            return SafetyReport {
                is_safe: false,
                reason: Some("Force pushing to a repository is dangerous.".to_string()),
                suggested_command: Some("git push".to_string()),
            };
        }

        SafetyReport {
            is_safe: true,
            reason: None,
            suggested_command: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_destructive_root_deletion() {
        let report = SafetyAgent::verify_command("rm -rf /");
        assert!(!report.is_safe);
        assert!(report.reason.unwrap().contains("root-level"));
    }

    #[test]
    fn blocks_env_access() {
        let report = SafetyAgent::verify_command("cat .env");
        assert!(!report.is_safe);
    }

    #[test]
    fn allows_safe_commands() {
        let report = SafetyAgent::verify_command("ls -la");
        assert!(report.is_safe);
    }
}
