//! Policy engine — validates agent patches against governance rules before applying.
//!
//! Every file mutation goes through this engine:
//!   Agent proposes patch → Policy engine evaluates → Auto-approve / HITL / Reject
//!
//! Rules are declarative (loaded from `.tachy/policy.json` or config):
//! - File constraints: "no edits in /auth/ without approval"
//! - Size constraints: "max 500 lines per patch"
//! - Pattern constraints: "block changes containing passwords"
//! - Branch constraints: "only auto-merge to dev"
//! - Test constraints: "tests must pass before apply"

use serde::{Deserialize, Serialize};

/// A proposed file patch awaiting policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePatch {
    pub file_path: String,
    pub original_hash: String,
    pub new_content: String,
    pub diff_summary: String,
    pub additions: usize,
    pub deletions: usize,
    pub agent_id: String,
    pub task_id: Option<String>,
}

/// Result of policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    /// Patch is safe — apply automatically.
    AutoApprove,
    /// Patch requires human review before applying.
    RequiresApproval { reason: String },
    /// Patch is rejected — do not apply.
    Reject { reason: String },
}

/// A single policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub name: String,
    pub rule_type: PolicyRuleType,
    pub action: PolicyAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyRuleType {
    /// Block or require approval for edits to specific paths.
    PathMatch { patterns: Vec<String> },
    /// Limit patch size (additions + deletions).
    MaxPatchSize { max_lines: usize },
    /// Block patches containing specific patterns (e.g., secrets).
    ContentBlock { patterns: Vec<String> },
    /// Block patches whose diff contains specific patterns (checks added lines only).
    DiffContentBlock { patterns: Vec<String> },
    /// Execute a WASM module for custom policy logic.
    Wasm { module_path: String, function_name: String },
    /// Require tests to pass before applying.
    RequireTests,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    RequireApproval,
    Reject,
}

/// The policy engine that evaluates patches against rules.
pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
}

impl PolicyEngine {
    #[must_use] pub fn new(rules: Vec<PolicyRule>) -> Self {
        Self { rules }
    }

    /// Load default enterprise rules.
    #[must_use] pub fn enterprise_default() -> Self {
        Self::new(vec![
            PolicyRule {
                name: "protect_auth".to_string(),
                rule_type: PolicyRuleType::PathMatch {
                    patterns: vec![
                        "**/auth/**".to_string(),
                        "**/security/**".to_string(),
                        "**/crypto/**".to_string(),
                        "**/.env*".to_string(),
                        "**/secrets/**".to_string(),
                        "**/migrations/**".to_string(),
                        "**/database/**".to_string(),
                        "**/db/**".to_string(),
                        "**/routes/**".to_string(),
                        "**/handlers/**".to_string(),
                    ],
                },
                action: PolicyAction::RequireApproval,
            },
            PolicyRule {
                name: "max_patch_size".to_string(),
                rule_type: PolicyRuleType::MaxPatchSize { max_lines: 500 },
                action: PolicyAction::RequireApproval,
            },
            PolicyRule {
                name: "block_secrets".to_string(),
                rule_type: PolicyRuleType::ContentBlock {
                    patterns: vec![
                        "password\\s*=".to_string(),
                        "api_key\\s*=".to_string(),
                        "secret\\s*=".to_string(),
                        "BEGIN RSA PRIVATE KEY".to_string(),
                        "BEGIN OPENSSH PRIVATE KEY".to_string(),
                    ],
                },
                action: PolicyAction::Reject,
            },
            // OWASP Top-10 security scan gate: any diff introducing known injection
            // patterns requires human approval before the patch is applied.
            PolicyRule {
                name: "security_scan_required".to_string(),
                rule_type: PolicyRuleType::DiffContentBlock {
                    patterns: vec![
                        r#"execute\s*\(\s*f["']"#.to_string(),
                        r#"execute\s*\(\s*["'].*%s"#.to_string(),
                        r"shell\s*=\s*True".to_string(),
                        r"pickle\.load".to_string(),
                        r"\beval\s*\(".to_string(),
                        r"yaml\.load\s*\([^,)]+\)".to_string(),
                        r"innerHTML\s*=".to_string(),
                        r"dangerouslySetInnerHTML".to_string(),
                    ],
                },
                action: PolicyAction::RequireApproval,
            },
        ])
    }

    /// Evaluate a patch against all rules.
    #[must_use] pub fn evaluate(&self, patch: &FilePatch) -> PolicyDecision {
        for rule in &self.rules {
            match self.check_rule(rule, patch) {
                PolicyDecision::AutoApprove => {}
                decision => return decision,
            }
        }
        PolicyDecision::AutoApprove
    }

    fn check_rule(&self, rule: &PolicyRule, patch: &FilePatch) -> PolicyDecision {
        match &rule.rule_type {
            PolicyRuleType::PathMatch { patterns } => {
                for pattern in patterns {
                    if path_matches(&patch.file_path, pattern) {
                        return match rule.action {
                            PolicyAction::RequireApproval => PolicyDecision::RequiresApproval {
                                reason: format!("rule '{}': path matches '{}'", rule.name, pattern),
                            },
                            PolicyAction::Reject => PolicyDecision::Reject {
                                reason: format!("rule '{}': path blocked by '{}'", rule.name, pattern),
                            },
                        };
                    }
                }
                PolicyDecision::AutoApprove
            }
            PolicyRuleType::MaxPatchSize { max_lines } => {
                let total = patch.additions + patch.deletions;
                if total > *max_lines {
                    return match rule.action {
                        PolicyAction::RequireApproval => PolicyDecision::RequiresApproval {
                            reason: format!("rule '{}': patch is {} lines (max {})", rule.name, total, max_lines),
                        },
                        PolicyAction::Reject => PolicyDecision::Reject {
                            reason: format!("rule '{}': patch too large ({} lines)", rule.name, total),
                        },
                    };
                }
                PolicyDecision::AutoApprove
            }
            PolicyRuleType::ContentBlock { patterns } => {
                for pattern in patterns {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        if re.is_match(&patch.new_content) {
                            return match rule.action {
                                PolicyAction::RequireApproval => PolicyDecision::RequiresApproval {
                                    reason: format!("rule '{}': content matches '{}'", rule.name, pattern),
                                },
                                PolicyAction::Reject => PolicyDecision::Reject {
                                    reason: format!("rule '{}': blocked content pattern '{}'", rule.name, pattern),
                                },
                            };
                        }
                    }
                }
                PolicyDecision::AutoApprove
            }
            PolicyRuleType::RequireTests => {
                // This rule is checked externally (after test execution)
                PolicyDecision::AutoApprove
            }
            PolicyRuleType::DiffContentBlock { patterns } => {
                // Check patterns against the diff summary (added lines)
                for pattern in patterns {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        if re.is_match(&patch.diff_summary) {
                            return match rule.action {
                                PolicyAction::RequireApproval => PolicyDecision::RequiresApproval {
                                    reason: format!("rule '{}': diff matches '{}'", rule.name, pattern),
                                },
                                PolicyAction::Reject => PolicyDecision::Reject {
                                    reason: format!("rule '{}': blocked diff pattern '{}'", rule.name, pattern),
                                },
                            };
                        }
                    }
                }
                PolicyDecision::AutoApprove
            }
            PolicyRuleType::Wasm { module_path, function_name } => {
                self.evaluate_wasm(module_path, function_name, patch)
            }
        }
    }

    fn evaluate_wasm(&self, module_path: &str, function_name: &str, patch: &FilePatch) -> PolicyDecision {
        use wasmtime::*;

        let engine = Engine::default();
        let module = match Module::from_file(&engine, module_path) {
            Ok(m) => m,
            Err(e) => return PolicyDecision::Reject { reason: format!("failed to load WASM module: {e}") },
        };

        let mut store = Store::new(&engine, ());
        let linker = Linker::new(&engine);
        let instance = match linker.instantiate(&mut store, &module) {
            Ok(i) => i,
            Err(e) => return PolicyDecision::Reject { reason: format!("failed to instantiate WASM: {e}") },
        };

        let func = match instance.get_typed_func::<(u32, u32, u32, u32), i32>(&mut store, function_name) {
            Ok(f) => f,
            Err(_) => return PolicyDecision::Reject { reason: format!("function '{function_name}' not found in WASM") },
        };

        // Simplified interface: pass additions, deletions, path length, content length
        // In a real production system, we'd share memory for full patch access.
        let path_len = patch.file_path.len() as u32;
        let content_len = patch.new_content.len() as u32;
        
        match func.call(&mut store, (patch.additions as u32, patch.deletions as u32, path_len, content_len)) {
            Ok(0) => PolicyDecision::AutoApprove,
            Ok(1) => PolicyDecision::RequiresApproval { reason: "WASM policy flagged for review".to_string() },
            Ok(2) => PolicyDecision::Reject { reason: "WASM policy rejected patch".to_string() },
            Ok(n) => PolicyDecision::Reject { reason: format!("WASM policy returned unknown code: {n}") },
            Err(e) => PolicyDecision::Reject { reason: format!("WASM execution failed: {e}") },
        }
    }
}

/// Simple glob-like path matching.
fn path_matches(path: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("**/") {
        if let Some(middle) = suffix.strip_suffix("/**") {
            return path.contains(&format!("/{middle}/")) || path.contains(&format!("{middle}/"));
        }
        // Handle patterns like **/.env* — match any path containing the suffix
        if suffix.contains('*') {
            let prefix = suffix.split('*').next().unwrap_or("");
            return path.contains(prefix);
        }
        return path.contains(suffix) || path.ends_with(suffix);
    }
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0;
        for part in &parts {
            if part.is_empty() { continue; }
            if let Some(found) = path[pos..].find(part) {
                pos += found + part.len();
            } else {
                return false;
            }
        }
        return true;
    }
    path == pattern || path.ends_with(pattern)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn test_patch(file: &str, content: &str, adds: usize, dels: usize) -> FilePatch {
        FilePatch {
            file_path: file.to_string(),
            original_hash: "abc".to_string(),
            new_content: content.to_string(),
            diff_summary: "test".to_string(),
            additions: adds,
            deletions: dels,
            agent_id: "agent-1".to_string(),
            task_id: None,
        }
    }

    #[test]
    fn auto_approves_safe_patch() {
        let engine = PolicyEngine::enterprise_default();
        let patch = test_patch("src/utils.rs", "fn helper() {}", 5, 2);
        assert_eq!(engine.evaluate(&patch), PolicyDecision::AutoApprove);
    }

    #[test]
    fn requires_approval_for_auth_path() {
        let engine = PolicyEngine::enterprise_default();
        let patch = test_patch("src/auth/login.rs", "fn login() {}", 5, 2);
        match engine.evaluate(&patch) {
            PolicyDecision::RequiresApproval { reason } => {
                assert!(reason.contains("auth"));
            }
            other => panic!("expected RequiresApproval, got {:?}", other),
        }
    }

    #[test]
    fn requires_approval_for_large_patch() {
        let engine = PolicyEngine::enterprise_default();
        let patch = test_patch("src/big.rs", "lots of code", 400, 200);
        match engine.evaluate(&patch) {
            PolicyDecision::RequiresApproval { reason } => {
                assert!(reason.contains("600 lines"));
            }
            other => panic!("expected RequiresApproval, got {:?}", other),
        }
    }

    #[test]
    fn rejects_secrets_in_content() {
        let engine = PolicyEngine::enterprise_default();
        let patch = test_patch("src/config.rs", "let password = \"hunter2\";", 1, 0);
        match engine.evaluate(&patch) {
            PolicyDecision::Reject { reason } => {
                assert!(reason.contains("password"));
            }
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    #[test]
    fn path_matching_works() {
        assert!(path_matches("src/auth/login.rs", "**/auth/**"));
        assert!(path_matches("config/.env.local", "**/.env*"));
        assert!(!path_matches("src/utils.rs", "**/auth/**"));
    }

    #[test]
    fn diff_content_block_checks_diff_summary() {
        let engine = PolicyEngine::new(vec![
            PolicyRule {
                name: "block_unsafe".to_string(),
                rule_type: PolicyRuleType::DiffContentBlock {
                    patterns: vec!["unsafe".to_string()],
                },
                action: PolicyAction::RequireApproval,
            },
        ]);
        let patch = test_patch("src/lib.rs", "safe code", 5, 2);
        assert_eq!(engine.evaluate(&patch), PolicyDecision::AutoApprove);

        let unsafe_patch = FilePatch {
            diff_summary: "+unsafe fn do_thing()".to_string(),
            ..test_patch("src/lib.rs", "unsafe fn do_thing() {}", 1, 0)
        };
        match engine.evaluate(&unsafe_patch) {
            PolicyDecision::RequiresApproval { reason } => {
                assert!(reason.contains("unsafe"));
            }
            other => panic!("expected RequiresApproval, got {:?}", other),
        }
    }
}
