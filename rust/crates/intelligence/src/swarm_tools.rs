//! Agent tools for Multi-Agent Swarm Orchestration (Direction C).
//! 
//! This module provides the high-level "Swarm Refactor" tool that agents use 
//! to delegate large-scale repository-level goals to parallel sub-agents.

use std::path::Path;
use crate::swarm::{SwarmRefactorInput, plan_swarm_refactor};
use crate::indexer::CodebaseIndexer;

/// Execute a swarm-based development run.
/// 
/// This tool takes a high-level goal and a list of target files, then 
/// decomposes the work into a parallel Task DAG for execution.
pub fn execute_swarm_refactor(
    input: &SwarmRefactorInput,
    workspace_root: &Path,
) -> Result<String, String> {
    // 1. Validate the files exist
    for file in &input.files {
        let abs = workspace_root.join(file);
        if !abs.exists() {
            return Err(format!("target file not found: {}", file));
        }
    }

    // 2. Load index for context (optional but good for future refinement)
    let _index = CodebaseIndexer::load_index(workspace_root).ok();

    // 3. Generate the Swarm Plan (DAG)
    let plan = plan_swarm_refactor(input);

    // 4. Return the plan to the daemon for orchestration
    // The daemon will recognize this JSON structure and spin up the Orchestrator.
    serde_json::to_string_pretty(&plan)
        .map_err(|e| e.to_string())
}

/// Returns the tool specifications for Swarm tools.
pub fn swarm_tool_specs() -> Vec<tools::ToolSpec> {
    vec![
        tools::ToolSpec {
            name: "swarm_refactor",
            description: "Decompose a high-level development goal into a parallel DAG of agent sub-tasks. Use this for repository-scale changes (e.g., refactoring all tests, updating a shared library, or adding logging across multiple modules) where parallel execution is preferred.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": { "type": "string", "description": "The high-level objective for the swarm." },
                    "files": { 
                        "type": "array", 
                        "items": { "type": "string" },
                        "description": "The list of workspace-relative file paths to include in the parallel refactor."
                    }
                },
                "required": ["goal", "files"],
                "additionalProperties": false
            }),
        },
    ]
}
