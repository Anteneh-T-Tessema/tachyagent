pub mod context;
pub mod edit_test_fix;
pub mod git;
pub mod indexer;
pub mod planner;

use serde::{Deserialize, Serialize};

// Re-exports
pub use context::{ContextConfig, ContextInjection, ContextSelector};
pub use edit_test_fix::{CycleOutcome, CycleResult, EditTestFix, EditTestFixConfig, TestResult};
pub use git::GitTools;
pub use indexer::{CodebaseIndex, CodebaseIndexer, FileEntry, IndexError, IndexerConfig, ProjectMeta};
pub use planner::{Plan, PlanConfig, PlanExecutionResult, PlanExecutor, PlanStep};

/// Top-level configuration for all intelligence features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceConfig {
    /// Enable codebase indexing
    pub indexing_enabled: bool,
    /// Enable smart context selection
    pub context_enabled: bool,
    /// Enable plan-and-execute for complex tasks
    pub planning_enabled: bool,
    /// Enable edit-test-fix cycle
    pub edit_test_fix_enabled: bool,
    /// Enable git integration
    pub git_enabled: bool,
    /// Indexer configuration
    pub indexer: IndexerConfig,
    /// Planner configuration
    pub plan: PlanConfig,
    /// Edit-test-fix configuration
    pub edit_test_fix: EditTestFixConfig,
    /// Context selection configuration
    pub context: ContextConfig,
}

impl Default for IntelligenceConfig {
    fn default() -> Self {
        Self {
            indexing_enabled: true,
            context_enabled: true,
            planning_enabled: true,
            edit_test_fix_enabled: true,
            git_enabled: true,
            indexer: IndexerConfig::default(),
            plan: PlanConfig::default(),
            edit_test_fix: EditTestFixConfig::default(),
            context: ContextConfig::default(),
        }
    }
}
