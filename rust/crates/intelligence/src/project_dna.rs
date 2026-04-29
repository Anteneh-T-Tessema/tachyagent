//! Project DNA — the persistent architectural truth of the codebase.
//!
//! Manages `.tachy/TACHY.md`, which contains:
//! 1. Project Goal & Tech Stack
//! 2. Core Architecture Rules
//! 3. Recent Critical Changes
//! 4. Active Development Context
//!
//! This file is automatically injected into every agent's system prompt.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDna {
    pub goal: String,
    pub tech_stack: Vec<String>,
    pub rules: Vec<String>,
    pub recent_changes: Vec<String>,
}

pub struct ProjectDnaManager {
    path: PathBuf,
}

impl ProjectDnaManager {
    #[must_use]
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            path: workspace_root.join(".tachy").join("TACHY.md"),
        }
    }

    /// Load the project DNA from disk. Creates a default one if missing.
    #[must_use]
    pub fn load(&self) -> String {
        if self.path.exists() {
            std::fs::read_to_string(&self.path).unwrap_or_default()
        } else {
            let default_dna = concat!(
                "# TACHY PROJECT DNA\n\n",
                "## Project Goal\n",
                "Describe the high-level objective of this project.\n\n",
                "## Tech Stack\n",
                "- Language: \n",
                "- Framework: \n",
                "- Database: \n\n",
                "## Architecture Rules\n",
                "1. Always prefer safety over speed.\n",
                "2. Maintain strict separation of concerns.\n\n",
                "## Recent Changes\n",
                "- Project initialized.\n"
            );
            let _ = std::fs::create_dir_all(self.path.parent().unwrap());
            let _ = std::fs::write(&self.path, default_dna);
            default_dna.to_string()
        }
    }

    /// Update the project DNA.
    pub fn update(&self, new_content: &str) -> Result<(), String> {
        std::fs::write(&self.path, new_content)
            .map_err(|e| format!("failed to update TACHY.md: {e}"))
    }

    /// Formats the DNA for system prompt injection.
    #[must_use]
    pub fn as_system_context(&self) -> String {
        let content = self.load();
        format!(
            "\n## PROJECT DNA (TACHY.md)\n\n{content}\n\nUse the above context to ensure architectural consistency."
        )
    }
}

/// Execute the "`update_project_md`" tool.
pub fn execute_update_project_md(
    input: &serde_json::Value,
    workspace_root: &Path,
) -> Result<String, String> {
    let content = input
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or("'content' parameter required")?;

    let manager = ProjectDnaManager::new(workspace_root);
    manager.update(content)?;
    Ok("TACHY.md updated successfully.".to_string())
}
