use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::indexer::CodebaseIndex;

/// Configuration for the edit-test-fix cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditTestFixConfig {
    pub max_retries: usize,
    pub test_command: Option<String>,
    pub test_timeout_secs: u64,
    pub targeted_tests: bool,
}

impl Default for EditTestFixConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            test_command: None,
            test_timeout_secs: 120,
            targeted_tests: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleResult {
    pub outcome: CycleOutcome,
    pub retries: usize,
    pub test_command: String,
    pub files_modified: Vec<String>,
    pub test_output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CycleOutcome {
    Passed,
    Fixed,
    Failed,
    NoTestCommand,
    Timeout,
}

#[derive(Debug)]
pub enum EditTestFixError {
    NoTestCommand,
    Timeout { command: String, timeout_secs: u64 },
    Execution(String),
}

pub struct TestResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct EditTestFix;

impl EditTestFix {
    /// Detect the project's test command.
    pub fn detect_test_command(
        workspace_root: &Path,
        index: Option<&CodebaseIndex>,
    ) -> Option<String> {
        // Use index if available
        if let Some(idx) = index {
            if let Some(cmd) = &idx.project.test_command {
                return Some(cmd.clone());
            }
        }
        crate::indexer::detect_test_command(workspace_root)
    }

    /// Build a targeted test command for specific edited files.
    pub fn targeted_test_command(base_command: &str, edited_files: &[String]) -> String {
        match base_command {
            "cargo test" => {
                let modules: Vec<&str> = edited_files
                    .iter()
                    .filter_map(|f| f.strip_prefix("src/"))
                    .filter_map(|f| f.strip_suffix(".rs"))
                    .filter(|m| *m != "main" && *m != "lib")
                    .collect();
                if modules.is_empty() {
                    base_command.to_string()
                } else {
                    format!("cargo test {}", modules.join(" "))
                }
            }
            "pytest" => {
                let test_files: Vec<&str> = edited_files
                    .iter()
                    .filter(|f| f.contains("test"))
                    .map(String::as_str)
                    .collect();
                if test_files.is_empty() {
                    base_command.to_string()
                } else {
                    format!("pytest {}", test_files.join(" "))
                }
            }
            _ => base_command.to_string(),
        }
    }

    /// Run tests and return the result.
    pub fn run_tests(command: &str, _timeout_secs: u64) -> Result<TestResult, EditTestFixError> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| EditTestFixError::Execution(e.to_string()))?;

        Ok(TestResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    /// Build the fix prompt to send to the LLM after test failure.
    pub fn build_fix_prompt(
        test_command: &str,
        result: &TestResult,
        edited_files: &[String],
    ) -> String {
        let stderr = if result.stderr.len() > 4000 {
            &result.stderr[..4000]
        } else {
            &result.stderr
        };

        let stdout_tail = if result.stdout.len() > 2000 {
            &result.stdout[result.stdout.len() - 2000..]
        } else {
            &result.stdout
        };

        format!(
            "The following tests failed after your edits:\n\n\
             Command: {test_command}\n\
             Exit code: {}\n\n\
             stderr:\n{stderr}\n\n\
             stdout (tail):\n{stdout_tail}\n\n\
             Files you edited: {}\n\n\
             Please fix the code to make the tests pass. \
             Focus on the specific errors shown above. \
             Do NOT change the test files unless the tests themselves are wrong.",
            result.exit_code,
            edited_files.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targeted_cargo_test() {
        let cmd = EditTestFix::targeted_test_command(
            "cargo test",
            &["src/indexer.rs".to_string(), "src/context.rs".to_string()],
        );
        assert!(cmd.contains("indexer"));
        assert!(cmd.contains("context"));
    }

    #[test]
    fn targeted_cargo_test_skips_main() {
        let cmd = EditTestFix::targeted_test_command(
            "cargo test",
            &["src/main.rs".to_string()],
        );
        assert_eq!(cmd, "cargo test"); // falls back to full suite
    }

    #[test]
    fn targeted_pytest() {
        let cmd = EditTestFix::targeted_test_command(
            "pytest",
            &["tests/test_auth.py".to_string(), "src/auth.py".to_string()],
        );
        assert!(cmd.contains("test_auth.py"));
        assert!(!cmd.contains("src/auth.py")); // non-test file excluded
    }

    #[test]
    fn fix_prompt_truncates_long_output() {
        let result = TestResult {
            exit_code: 1,
            stdout: "x".repeat(5000),
            stderr: "e".repeat(6000),
        };
        let prompt = EditTestFix::build_fix_prompt("cargo test", &result, &["src/lib.rs".to_string()]);
        assert!(prompt.len() < 10000);
        assert!(prompt.contains("cargo test"));
        assert!(prompt.contains("src/lib.rs"));
    }

    #[test]
    fn cycle_outcome_values() {
        assert_eq!(CycleOutcome::Passed, CycleOutcome::Passed);
        assert_ne!(CycleOutcome::Passed, CycleOutcome::Failed);
    }
}
