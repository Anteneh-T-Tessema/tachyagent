//! Swarm Simulation Engine — Digital Twin for mission validation.

use runtime::{ToolError, ToolExecutor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub success: bool,
    pub iterations: usize,
    pub tool_calls: Vec<SimulatedToolCall>,
    pub risk_score: f32,
    pub risk_report: String,
    pub cost_estimate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedToolCall {
    pub tool: String,
    pub input: String,
    pub output: String,
    pub side_effects: Vec<String>,
}

/// A non-destructive executor that mocks mutations.
#[allow(dead_code)]
pub struct SimulationExecutor {
    workspace_root: PathBuf,
    vfs_overlay: HashMap<PathBuf, String>,
    history: Vec<SimulatedToolCall>,
}

#[allow(dead_code)]
impl SimulationExecutor {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            vfs_overlay: HashMap::new(),
            history: Vec::new(),
        }
    }

    pub fn history(&self) -> &[SimulatedToolCall] {
        &self.history
    }
}

impl ToolExecutor for SimulationExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        let mut side_effects = Vec::new();

        // Simulation logic: mock high-risk tools
        let output = match tool_name {
            "write_file" | "replace_file_content" | "multi_replace_file_content" => {
                side_effects.push(format!("MUTATION: {tool_name} update suppressed"));
                "SUCCESS (Simulated)".to_string()
            }
            "run_command" => {
                side_effects.push("EXECUTION: shell command mocked".to_string());
                "Command simulation complete (no side effects)".to_string()
            }
            "delete_file" => {
                side_effects.push("DELETION: file removal suppressed".to_string());
                "SUCCESS (Simulated)".to_string()
            }
            _ => {
                // For read-only tools, we can actually run them
                // This allows the simulator to be high-fidelity
                "SUCCESS (Read-only passthrough simulated)".to_string()
            }
        };

        self.history.push(SimulatedToolCall {
            tool: tool_name.to_string(),
            input: input.to_string(),
            output: output.clone(),
            side_effects,
        });

        Ok(output)
    }
}

#[allow(dead_code)]
pub struct SimulationJudge {
    pub min_score: f32,
}

#[allow(dead_code)]
impl SimulationJudge {
    pub fn analyze(&self, history: &[SimulatedToolCall]) -> (f32, String) {
        let mut score = 1.0;
        let mut report = Vec::new();

        let mutation_count = history
            .iter()
            .filter(|c| c.side_effects.iter().any(|s| s.contains("MUTATION")))
            .count();
        let deletion_count = history
            .iter()
            .filter(|c| c.side_effects.iter().any(|s| s.contains("DELETION")))
            .count();

        if deletion_count > 0 {
            score -= 0.3 * (deletion_count as f32);
            report.push(format!(
                "WARNING: Detected {deletion_count} file deletions."
            ));
        }

        if mutation_count > 10 {
            score -= 0.1;
            report.push(
                "CAUTION: Mission involves high volume of mutations (>10 files).".to_string(),
            );
        }

        if score < 0.0 {
            score = 0.0;
        }
        if score < self.min_score {
            report.push(format!(
                "Risk score {score:.2} is below minimum {:.2}.",
                self.min_score
            ));
        }

        if report.is_empty() {
            report.push("Simulation complete. Low risk detected.".to_string());
        }

        (score, report.join("\n"))
    }
}
