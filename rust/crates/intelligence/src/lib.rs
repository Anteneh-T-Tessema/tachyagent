pub mod context;
pub mod edit_test_fix;
pub mod git;
pub mod indexer;
pub mod lsp;
pub mod memory;
pub mod planner;
pub mod prompts;
pub mod validation;
pub mod verification;

use serde::{Deserialize, Serialize};

// Re-exports
pub use context::{ContextConfig, ContextInjection, ContextSelector};
pub use edit_test_fix::{CycleOutcome, CycleResult, EditTestFix, EditTestFixConfig, TestResult};
pub use git::GitTools;
pub use indexer::{CodebaseIndex, CodebaseIndexer, FileEntry, IndexError, IndexerConfig, ProjectMeta};
pub use lsp::{LspManager, Diagnostic, DiagnosticSeverity, Location, HoverInfo, execute_get_diagnostics, execute_find_references};
pub use memory::{AgentMemory, MemoryCategory, MemoryEntry, execute_remember};
pub use planner::{Plan, PlanConfig, PlanExecutionResult, PlanExecutor, PlanStep};
pub use prompts::{build_optimized_prompt, detect_family, template_for_model, ModelFamily, PromptTemplate};
pub use validation::{clean_code_output, validate_code, ValidationResult};
pub use verification::{
    build_verification_prompt, contains_code, extract_code_blocks,
    parse_verification_response, VerificationConfig, VerificationResult,
};

/// Top-level configuration for all intelligence features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceConfig {
    /// Enable codebase indexing
    #[serde(default = "default_true")]
    pub indexing_enabled: bool,
    /// Enable smart context selection
    #[serde(default = "default_true")]
    pub context_enabled: bool,
    /// Enable plan-and-execute for complex tasks
    #[serde(default = "default_true")]
    pub planning_enabled: bool,
    /// Enable edit-test-fix cycle
    #[serde(default = "default_true")]
    pub edit_test_fix_enabled: bool,
    /// Enable git integration
    #[serde(default = "default_true")]
    pub git_enabled: bool,
    /// Indexer configuration
    #[serde(default)]
    pub indexer: IndexerConfig,
    /// Planner configuration
    #[serde(default)]
    pub plan: PlanConfig,
    /// Edit-test-fix configuration
    #[serde(default)]
    pub edit_test_fix: EditTestFixConfig,
    /// Context selection configuration
    #[serde(default)]
    pub context: ContextConfig,
    /// Verification configuration
    #[serde(default)]
    pub verification: VerificationConfig,
}

fn default_true() -> bool { true }

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
            verification: VerificationConfig::default(),
        }
    }
}
