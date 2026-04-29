mod bash;
mod browser;
mod bootstrap;
mod compact;
mod config;
mod conversation;
pub mod diff;
pub mod filelock;
mod file_ops;
mod json;
mod permissions;
mod prompt;
mod semantic_cache;
mod session;
pub mod transaction;
mod usage;

pub use bash::{execute_bash, BashCommandInput, BashCommandOutput};
pub use browser::{
    capture_screenshot, get_accessibility_tree, compare_snapshots, clean_old_snapshots,
    ScreenshotInput, ScreenshotOutput, AccessibilityTreeInput, VisualDiffInput,
};
pub use bootstrap::{BootstrapPhase, BootstrapPlan};
pub use compact::{
    compact_session, estimate_session_tokens, format_compact_summary,
    get_compact_continuation_message, should_compact, CompactionConfig, CompactionResult,
};
pub use diff::{UnifiedDiff, DiffHunk, DiffLine};
pub use filelock::{FileLockManager, LockError};
pub use transaction::{EditTransaction, PendingEdit, TransactionError};
pub use config::{
    ConfigEntry, ConfigError, ConfigLoader, ConfigSource, RuntimeConfig,
    TACHY_SETTINGS_SCHEMA_NAME,
};
pub use conversation::{
    ApiClient, ApiRequest, AssistantEvent, ConversationRuntime, ResponseFormat, RuntimeError,
    RuntimeEvent, StaticToolExecutor, ToolError, ToolExecutor, TurnSummary,
};
pub use file_ops::{
    edit_file, glob_search, grep_search, list_directory, read_file, write_file,
    preview_write_file, preview_edit_file,
    DiffPreview, DirEntry, EditFileOutput, GlobSearchOutput, GrepSearchInput, GrepSearchOutput,
    ListDirectoryOutput, ReadFileOutput, StructuredPatchHunk, TextFilePayload, WriteFileOutput,
};
pub use permissions::{
    PermissionMode, PermissionOutcome, PermissionPolicy, PermissionPromptDecision,
    PermissionPrompter, PermissionRequest,
};
pub use prompt::{
    load_system_prompt, prepend_bullets, ContextFile, ProjectContext, PromptBuildError,
    SystemPromptBuilder, FRONTIER_MODEL_NAME, SYSTEM_PROMPT_DYNAMIC_BOUNDARY,
};
pub use semantic_cache::{SemanticCache, CachedResult, Embedder};
pub use session::{ContentBlock, ConversationMessage, MessageRole, Session, SessionError};
pub use usage::{TokenUsage, UsageTracker};
