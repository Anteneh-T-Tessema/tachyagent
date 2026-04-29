//! Compliance Sentinel — real-time policy enforcement for sovereign security.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ViolationAction {
    Warn,
    Block,
    Kill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRule {
    pub id: String,
    pub description: String,
    pub pattern: String, // Regex or keyword
    pub action: ViolationAction,
    pub severity: super::AuditSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityViolation {
    pub rule_id: String,
    pub detail: String,
    pub action_taken: ViolationAction,
}

pub struct ComplianceSentinel {
    rules: Vec<SecurityRule>,
}

impl ComplianceSentinel {
    pub fn new() -> Self {
        Self {
            rules: vec![
                SecurityRule {
                    id: "rule-forbidden-dir".to_string(),
                    description: "Access to system-sensitive directories is forbidden".to_string(),
                    pattern: r"(/etc/|/var/log/|~/\.ssh/|~/\.aws/|~/\.kube/)".to_string(),
                    action: ViolationAction::Kill,
                    severity: super::AuditSeverity::Critical,
                },
                SecurityRule {
                    id: "rule-secret-leak".to_string(),
                    description: "Likely API key or secret detected in output".to_string(),
                    pattern: r"(sk-ant-|sk-[a-zA-Z0-9]{44}|AIza[a-zA-Z0-9_\-]{35})".to_string(),
                    action: ViolationAction::Block,
                    severity: super::AuditSeverity::Warning,
                },
                SecurityRule {
                    id: "rule-env-modification".to_string(),
                    description: "Modification of environment files is restricted".to_string(),
                    pattern: r"(\.env|\.bashrc|\.zshrc)".to_string(),
                    action: ViolationAction::Block,
                    severity: super::AuditSeverity::Warning,
                },
            ],
        }
    }

    /// Scan tool inputs/outputs for security violations.
    pub fn scan(&self, content: &str) -> Option<SecurityViolation> {
        for rule in &self.rules {
            if let Ok(re) = regex::Regex::new(&rule.pattern) {
                if re.is_match(content) {
                    return Some(SecurityViolation {
                        rule_id: rule.id.clone(),
                        detail: format!("Violated: {}", rule.description),
                        action_taken: rule.action.clone(),
                    });
                }
            }
        }
        None
    }
}

impl Default for ComplianceSentinel {
    fn default() -> Self {
        Self::new()
    }
}
