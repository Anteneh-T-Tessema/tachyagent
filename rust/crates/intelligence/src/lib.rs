pub mod collaboration;
pub mod context;
pub mod compaction;
pub mod dep_graph;
pub mod edit_test_fix;
pub mod evaluator;
pub mod finetune;
pub mod git;
pub mod indexer;
pub mod lsp;
pub mod memory;
pub mod monorepo;
pub mod planner;
pub mod prompts;
pub mod rag;
pub mod rag_tools;
pub mod swarm;
pub mod swarm_tools;
pub mod reward;
pub mod safety;
pub mod dream;
pub mod consensus;
pub mod summary;
pub mod validation;
pub mod project_dna;
pub mod vision;
pub mod verification;
pub mod harness;
pub mod inspector;
pub mod crisis;
pub mod training;

use serde::{Deserialize, Serialize};

// Re-exports
pub use harness::*;
pub use inspector::*;
pub use crisis::*;
pub use training::*;

// Re-exports
pub use context::{ContextConfig, ContextInjection, ContextSelector};
pub use dep_graph::{DependencyGraph, GraphNode};
pub use edit_test_fix::{CycleCheckResult, CycleOutcome, CycleResult, DiagnosticResult, EditTestFix, EditTestFixConfig, TestResult};
pub use finetune::{FinetuneDataset, FinetuneEntry, GoldStandardStats, GoldStandardStore, QualityScorer, generate_modelfile, generate_training_script};
pub use git::GitTools;
pub use indexer::{CodebaseIndex, CodebaseIndexer, FileEntry, IndexError, IndexerConfig, ProjectMeta, WorkspaceManifest};
pub use lsp::{LspManager, Diagnostic, DiagnosticSeverity, Location, HoverInfo, execute_get_diagnostics, execute_find_references};
pub use memory::{AgentMemory, HiveMind, MemoryCategory, MemoryEntry, MemorySyndicator, execute_remember};
pub use monorepo::{MonorepoKind, MonorepoManifest, WorkspaceMember};
pub use planner::{Plan, PlanConfig, PlanExecutionResult, PlanExecutor, PlanStep, StepStatus};
pub use rag_tools::{execute_search_codebase, SearchCodebaseInput, SearchCodebaseResult, SearchResultEntry, execute_expand_context, ExpandContextInput, rag_tool_specs};
pub use prompts::{build_optimized_prompt, detect_family, template_for_model, ModelFamily, PromptTemplate};
pub use rag::{VectorStore, CodeChunk, Chunker};
pub use swarm::{SwarmPlan, SwarmTask, SwarmRefactorInput, plan_swarm_refactor};
pub use swarm_tools::{execute_swarm_refactor, swarm_tool_specs};
pub use summary::{ConversationSummary, SummaryManager};
pub use safety::{SafetyAgent, SafetyReport};
pub use dream::{SubagentManager, DreamCandidate};
pub use harness::*;
pub use consensus::{ConsensusEngine, ConsensusReport, JudgeReview};
pub use vision::{VisionAgent, VisualReport, VisualIssue};
pub use validation::{clean_code_output, validate_code, ValidationResult};
pub use collaboration::{collaboration_tool_specs, BroadcastStatusInput, BroadcastStatusResult, GetMissionFeedInput, GetMissionFeedResult};
pub use reward::{RewardConfig, RewardEngine, RewardScore};
pub use verification::{
    build_verification_prompt, contains_code, extract_code_blocks,
    parse_verification_response, VerificationConfig, VerificationResult,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinetuneConfig {
    /// Enable automated fine-tuning dataset generation.
    #[serde(default = "default_true")]
    pub auto_collect: bool,
    /// Minimum number of successful "Gold Standard" sessions before triggering.
    #[serde(default = "default_finetune_threshold")]
    pub threshold: usize,
    /// Whether to only include "Gold Standard" (success: true, override: false).
    #[serde(default = "default_true")]
    pub gold_standard_only: bool,
}

impl Default for FinetuneConfig {
    fn default() -> Self {
        Self {
            auto_collect: true,
            threshold: 10,
            gold_standard_only: true,
        }
    }
}

fn default_finetune_threshold() -> usize { 10 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationConfig {
    /// Enable autonomous self-evolution (Phase 31).
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// Number of recent audit events to analyze for optimization.
    #[serde(default = "default_log_window")]
    pub log_window: usize,
    /// Score threshold below which optimization is triggered (0-1).
    #[serde(default = "default_opt_threshold")]
    pub score_threshold: f32,
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            log_window: 100,
            score_threshold: 0.7,
        }
    }
}

fn default_log_window() -> usize { 100 }
fn default_opt_threshold() -> f32 { 0.7 }

/// Top-level configuration for all intelligence features.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
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
    /// Fine-tuning configuration
    #[serde(default)]
    pub finetune: FinetuneConfig,
    /// Reward engine configuration
    #[serde(default)]
    pub reward: RewardConfig,
    #[serde(default = "default_true")]
    pub vision_enabled: bool,
    /// Autonomous self-evolution and prompt optimization (Phase 31).
    #[serde(default)]
    pub optimization: OptimizationConfig,
    /// Enable mission simulation and risk analysis (Phase 32).
    #[serde(default = "default_false")]
    pub simulation_enabled: bool,
}

fn default_true() -> bool { true }
fn default_false() -> bool { false }

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
            finetune: FinetuneConfig::default(),
            reward: RewardConfig::default(),
            vision_enabled: true,
            optimization: OptimizationConfig::default(),
            simulation_enabled: true,
        }
    }
}
