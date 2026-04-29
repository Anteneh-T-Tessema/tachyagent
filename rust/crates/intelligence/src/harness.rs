use crate::planner::Plan;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The main agentic loop lifecycle stages as documented in Claude Code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HarnessStep {
    /// Initial phase: Gathering project files, terminal state, and git context.
    GatherContext,
    /// Research phase: Proposing a plan without modifying the filesystem.
    Plan,
    /// Execution phase: Applying tool calls (Read, Write, Bash, etc.).
    Act,
    /// Validation phase: Running tests, checking LSP, and Specialist Review.
    Verify,
}

impl HarnessStep {
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        match self {
            HarnessStep::GatherContext | HarnessStep::Plan => true,
            HarnessStep::Act | HarnessStep::Verify => false,
        }
    }
}

/// Documented built-in subagent types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubagentType {
    /// Read-only subagent optimized for codebase exploration and search.
    Explore,
    /// Research subagent for plan mode; denied Write and Edit tools.
    Plan,
    /// Full-power worker subagent for complex multi-step implementation tasks.
    General,
}

/// Status of an active subagent task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed(String),
}

/// A handle to a spawned subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInstance {
    pub id: String,
    pub agent_type: SubagentType,
    pub status: SubagentStatus,
    pub summary: Option<String>,
}

use backend::BackendRegistry;
use std::sync::Arc;

/// The Agentic Harness orchestrates the core Gather -> Act -> Verify loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticHarness {
    pub current_step: HarnessStep,
    pub active_subagents: BTreeMap<String, SubagentInstance>,
    pub current_plan: Option<Plan>,
    #[serde(skip)]
    pub registry: Option<Arc<BackendRegistry>>,
}

impl AgenticHarness {
    #[must_use]
    pub fn new(registry: Arc<BackendRegistry>) -> Self {
        Self {
            current_step: HarnessStep::GatherContext,
            active_subagents: BTreeMap::new(),
            current_plan: None,
            registry: Some(registry),
        }
    }

    /// Progress the harness to the next step in the loop.
    pub fn transition(&mut self, next: HarnessStep) {
        self.current_step = next;
    }
}
