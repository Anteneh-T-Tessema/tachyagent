//! Multi-pass verification — send generated code back to the model for review
//! before presenting to the user. Two passes catch most errors that local models make.

use serde::{Deserialize, Serialize};

/// Configuration for multi-pass verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Enable verification pass on code outputs
    pub enabled: bool,
    /// Maximum number of verification passes
    pub max_passes: usize,
    /// Minimum code length (chars) to trigger verification — skip for short responses
    pub min_code_length: usize,
    /// Whether to auto-fix issues found during verification
    pub auto_fix: bool,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_passes: 1,
            min_code_length: 50,
            auto_fix: true,
        }
    }
}

/// Result of a verification pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub passed: bool,
    pub issues: Vec<VerificationIssue>,
    pub fixed_code: Option<String>,
    pub passes_run: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationIssue {
    pub severity: IssueSeverity,
    pub description: String,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

/// Build the verification prompt to send to the model.
#[must_use]
pub fn build_verification_prompt(original_prompt: &str, generated_code: &str) -> String {
    format!(
        "You are a code reviewer. The following code was generated in response to this task:\n\n\
         Task: {original_prompt}\n\n\
         Generated code:\n```\n{generated_code}\n```\n\n\
         Review this code for:\n\
         1. Syntax errors\n\
         2. Logic bugs\n\
         3. Security issues\n\
         4. Missing error handling\n\
         5. Off-by-one errors\n\n\
         If the code is correct, respond with: VERIFIED\n\
         If there are issues, respond with: ISSUES FOUND\n\
         Then list each issue on a new line.\n\
         If you can fix the issues, provide the corrected code in a code block."
    )
}

/// Check if a response contains code blocks (indicates the model generated code).
#[must_use]
pub fn contains_code(text: &str) -> bool {
    text.contains("```")
        || text.contains("fn ")
        || text.contains("def ")
        || text.contains("function ")
        || text.contains("class ")
        || text.contains("pub fn ")
        || text.contains("impl ")
}

/// Extract code blocks from a response.
#[must_use]
pub fn extract_code_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut current = String::new();

    for line in text.lines() {
        if line.trim().starts_with("```") {
            if in_block {
                if !current.trim().is_empty() {
                    blocks.push(current.clone());
                }
                current.clear();
                in_block = false;
            } else {
                in_block = true;
            }
        } else if in_block {
            current.push_str(line);
            current.push('\n');
        }
    }

    blocks
}

/// Parse verification response to determine if code passed review.
#[must_use]
pub fn parse_verification_response(response: &str) -> VerificationResult {
    let upper = response.to_uppercase();

    if upper.contains("VERIFIED") && !upper.contains("ISSUES") {
        return VerificationResult {
            passed: true,
            issues: Vec::new(),
            fixed_code: None,
            passes_run: 1,
        };
    }

    // Extract issues
    let mut issues = Vec::new();
    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Look for numbered issues: "1. ...", "- ..."
        let is_issue = trimmed.starts_with(|c: char| c.is_ascii_digit())
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ");

        if is_issue && trimmed.len() > 3 {
            let severity = if trimmed.to_lowercase().contains("error")
                || trimmed.to_lowercase().contains("bug")
                || trimmed.to_lowercase().contains("security")
            {
                IssueSeverity::Error
            } else if trimmed.to_lowercase().contains("warning")
                || trimmed.to_lowercase().contains("missing")
            {
                IssueSeverity::Warning
            } else {
                IssueSeverity::Info
            };

            issues.push(VerificationIssue {
                severity,
                description: trimmed.to_string(),
                line: None,
            });
        }
    }

    // Extract fixed code if present
    let fixed_code = extract_code_blocks(response).into_iter().next();

    VerificationResult {
        passed: issues.is_empty() && fixed_code.is_none(),
        issues,
        fixed_code,
        passes_run: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_code_in_response() {
        assert!(contains_code("Here's the fix:\n```rust\nfn main() {}\n```"));
        assert!(contains_code("pub fn hello() {}"));
        assert!(!contains_code("The answer is 42."));
    }

    #[test]
    fn extracts_code_blocks() {
        let text = "Here:\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\nDone.";
        let blocks = extract_code_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("fn main()"));
    }

    #[test]
    fn parses_verified_response() {
        let result = parse_verification_response("The code looks correct. VERIFIED.");
        assert!(result.passed);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn parses_issues_response() {
        let response = "ISSUES FOUND:\n1. Missing error handling on line 42\n2. Security: SQL injection vulnerability\n3. Warning: unused variable";
        let result = parse_verification_response(response);
        assert!(!result.passed);
        assert_eq!(result.issues.len(), 3);
        assert_eq!(result.issues[0].severity, IssueSeverity::Error);
        assert_eq!(result.issues[1].severity, IssueSeverity::Error);
        assert_eq!(result.issues[2].severity, IssueSeverity::Warning);
    }

    #[test]
    fn parses_response_with_fixed_code() {
        let response = "ISSUES FOUND:\n1. Bug in loop\n\nFixed:\n```rust\nfor i in 0..n {\n    println!(\"{i}\");\n}\n```";
        let result = parse_verification_response(response);
        assert!(!result.passed);
        assert!(result.fixed_code.is_some());
        assert!(result.fixed_code.unwrap().contains("for i in"));
    }

    #[test]
    fn verification_prompt_includes_context() {
        let prompt = build_verification_prompt("fix the auth bug", "fn auth() { todo!() }");
        assert!(prompt.contains("fix the auth bug"));
        assert!(prompt.contains("fn auth()"));
        assert!(prompt.contains("Syntax errors"));
    }

    #[test]
    fn config_defaults() {
        let config = VerificationConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_passes, 1);
        assert_eq!(config.min_code_length, 50);
        assert!(config.auto_fix);
    }
}
