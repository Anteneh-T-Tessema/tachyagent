use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::indexer::CodebaseIndex;
use crate::lsp::{Diagnostic, DiagnosticSeverity, LspManager};

/// Configuration for the edit-test-fix cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditTestFixConfig {
    pub max_retries: usize,
    pub test_command: Option<String>,
    pub test_timeout_secs: u64,
    pub targeted_tests: bool,
    /// Run LSP diagnostics before tests for faster feedback on syntax/type errors.
    #[serde(default = "super::default_true")]
    pub lsp_diagnostics_enabled: bool,
}

impl Default for EditTestFixConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            test_command: None,
            test_timeout_secs: 120,
            targeted_tests: true,
            lsp_diagnostics_enabled: true,
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

use crate::vision::VisualReport;

/// Result of running LSP diagnostics on edited files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticResult {
    pub files_checked: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticResult {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }
}

pub struct EditTestFix;

impl EditTestFix {
    /// Detect the project's test command.
    #[must_use]
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
    #[must_use]
    pub fn build_fix_prompt(
        test_command: &str,
        result: &TestResult,
        edited_files: &[String],
        visual: Option<&VisualReport>,
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

        let mut prompt = format!(
            "The following tests failed after your edits:\n\n\
             Command: {test_command}\n\
             Exit code: {}\n\n\
             stderr:\n{stderr}\n\n\
             stdout (tail):\n{stdout_tail}\n\n\
             Files you edited: {}\n\n",
            result.exit_code,
            edited_files.join(", ")
        );

        if let Some(v) = visual {
            if let Some(diff) = &v.diff_report {
                prompt.push_str(&format!(
                    "\n\n## Visual Verification Context\n\
                     A visual regression or failure was detected.\n\
                     Screenshot: {}\n\
                     Diff Report:\n{}\n",
                    v.screenshot_path, diff
                ));
            }
            if let Some(tree) = &v.accessibility_tree {
                prompt.push_str(&format!("\nAccessibility Tree (Simplified):\n{tree}\n"));
            }
        }

        prompt.push_str(
            "\nPlease fix the code to make the tests (and visual state) pass. \
                         Focus on the specific errors shown above. \
                         Do NOT change the test files unless the tests themselves are wrong.",
        );

        prompt
    }

    /// Run LSP diagnostics on the edited files.
    /// Returns errors/warnings without needing to run the full test suite.
    #[must_use]
    pub fn run_diagnostics(workspace_root: &Path, edited_files: &[String]) -> DiagnosticResult {
        let lsp = LspManager::new(workspace_root);
        let mut all_diagnostics = Vec::new();

        for file in edited_files {
            let diags = lsp.get_diagnostics(file);
            all_diagnostics.extend(diags);
        }

        let error_count = all_diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count();
        let warning_count = all_diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count();

        DiagnosticResult {
            files_checked: edited_files.len(),
            error_count,
            warning_count,
            diagnostics: all_diagnostics,
        }
    }

    /// Build a fix prompt from LSP diagnostics (faster than test-based fix prompts).
    #[must_use]
    pub fn build_diagnostic_fix_prompt(
        diag_result: &DiagnosticResult,
        edited_files: &[String],
    ) -> String {
        let mut prompt = format!(
            "LSP diagnostics found {} error(s) and {} warning(s) in your edits:\n\n",
            diag_result.error_count, diag_result.warning_count
        );

        for d in &diag_result.diagnostics {
            let severity = match d.severity {
                DiagnosticSeverity::Error => "ERROR",
                DiagnosticSeverity::Warning => "WARN",
                _ => "INFO",
            };
            prompt.push_str(&format!(
                "  {}:{}:{} [{}] {}\n",
                d.file, d.line, d.column, severity, d.message
            ));
        }

        prompt.push_str(&format!(
            "\nFiles you edited: {}\n\n\
             Please fix the errors above. Focus on the ERROR-level diagnostics first. \
             These are compile/type errors that must be resolved before tests can pass.",
            edited_files.join(", ")
        ));

        prompt
    }

    /// Perform a visual verification check against a baseline.
    pub fn run_visual_check(
        url: &str,
        baseline_path: &str,
        workspace_root: &Path,
    ) -> Result<VisualReport, Box<dyn std::error::Error>> {
        // 1. Capture fresh screenshot
        let input = runtime::ScreenshotInput {
            url: url.to_string(),
            save_path: None,      // Auto-generate in .tachy/vision
            delay_ms: Some(1000), // Give it time to settle
            capture_full_page: Some(false),
            workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            wait_for_selector: None,
        };
        let snapshot = runtime::capture_screenshot(input)?;

        // 2. Extract accessibility tree for semantic comparison
        let tree_input = runtime::AccessibilityTreeInput {
            url: url.to_string(),
            wait_for_selector: None,
        };
        let tree = runtime::get_accessibility_tree(tree_input).ok();

        // 3. Compare with baseline
        let diff_report = runtime::compare_snapshots(baseline_path, &snapshot.path)?;

        Ok(VisualReport {
            screenshot_path: snapshot.path,
            diff_report: Some(diff_report.clone()),
            accessibility_tree: tree,
            issues: vec![],
            passed: !diff_report.contains("FAILURE"),
        })
    }

    /// Combined diagnostic + test cycle. Runs diagnostics first (fast), then tests.
    /// Returns early if diagnostics find errors — no point running tests on broken code.
    #[must_use]
    pub fn run_diagnostic_then_test(
        workspace_root: &Path,
        edited_files: &[String],
        test_command: &str,
        test_timeout_secs: u64,
        lsp_enabled: bool,
    ) -> CycleCheckResult {
        // Phase 1: LSP diagnostics (fast — no compilation needed for most languages)
        if lsp_enabled {
            let diag_result = Self::run_diagnostics(workspace_root, edited_files);
            if diag_result.has_errors() {
                return CycleCheckResult::DiagnosticErrors(diag_result);
            }
        }

        // Phase 2: Run tests (slower but catches logic errors)
        match Self::run_tests(test_command, test_timeout_secs) {
            Ok(test_result) if test_result.exit_code == 0 => CycleCheckResult::Passed,
            Ok(test_result) => CycleCheckResult::TestFailure(test_result),
            Err(_) => CycleCheckResult::TestExecutionError,
        }
    }
}

/// Result of the combined diagnostic + test check.
pub enum CycleCheckResult {
    /// All diagnostics clean and tests pass.
    Passed,
    /// LSP found errors — no tests were run.
    DiagnosticErrors(DiagnosticResult),
    /// Diagnostics clean but tests failed.
    TestFailure(TestResult),
    /// Visual verification failed (Phase 26).
    VisualFailure(VisualReport),
    /// Could not execute the test command.
    TestExecutionError,
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
        let cmd = EditTestFix::targeted_test_command("cargo test", &["src/main.rs".to_string()]);
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
        let prompt =
            EditTestFix::build_fix_prompt("cargo test", &result, &["src/lib.rs".to_string()], None);
        assert!(prompt.len() < 10000);
        assert!(prompt.contains("cargo test"));
        assert!(prompt.contains("src/lib.rs"));
    }

    #[test]
    fn diagnostic_fix_prompt_includes_errors() {
        let diag_result = DiagnosticResult {
            files_checked: 2,
            error_count: 1,
            warning_count: 1,
            diagnostics: vec![
                Diagnostic {
                    file: "src/main.rs".to_string(),
                    line: 10,
                    column: 5,
                    severity: DiagnosticSeverity::Error,
                    message: "cannot find value `foo`".to_string(),
                    source: "cargo".to_string(),
                },
                Diagnostic {
                    file: "src/main.rs".to_string(),
                    line: 20,
                    column: 1,
                    severity: DiagnosticSeverity::Warning,
                    message: "unused variable".to_string(),
                    source: "cargo".to_string(),
                },
            ],
        };
        let prompt =
            EditTestFix::build_diagnostic_fix_prompt(&diag_result, &["src/main.rs".to_string()]);
        assert!(prompt.contains("1 error(s)"));
        assert!(prompt.contains("1 warning(s)"));
        assert!(prompt.contains("cannot find value"));
        assert!(prompt.contains("ERROR"));
        assert!(prompt.contains("src/main.rs"));
    }

    #[test]
    fn diagnostic_result_has_errors() {
        let empty = DiagnosticResult {
            files_checked: 1,
            error_count: 0,
            warning_count: 2,
            diagnostics: Vec::new(),
        };
        assert!(!empty.has_errors());

        let with_errors = DiagnosticResult {
            files_checked: 1,
            error_count: 3,
            warning_count: 0,
            diagnostics: Vec::new(),
        };
        assert!(with_errors.has_errors());
    }

    #[test]
    fn cycle_outcome_values() {
        assert_eq!(CycleOutcome::Passed, CycleOutcome::Passed);
        assert_ne!(CycleOutcome::Passed, CycleOutcome::Failed);
    }
}
