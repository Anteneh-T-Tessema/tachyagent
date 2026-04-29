mod bash;
mod bootstrap;
mod browser;
mod compact;
mod config;
mod conversation;
pub mod diff;
mod file_ops;
pub mod filelock;
mod json;
mod permissions;
mod prompt;
mod semantic_cache;
mod session;
pub mod transaction;
mod usage;

pub use bash::{execute_bash, BashCommandInput, BashCommandOutput};
pub use bootstrap::{BootstrapPhase, BootstrapPlan};
pub use browser::{
    capture_screenshot, clean_old_snapshots, compare_snapshots, get_accessibility_tree,
    AccessibilityTreeInput, ScreenshotInput, ScreenshotOutput, VisualDiffInput,
};
pub use compact::{
    compact_session, estimate_session_tokens, format_compact_summary,
    get_compact_continuation_message, should_compact, CompactionConfig, CompactionResult,
};
pub use config::{
    ConfigEntry, ConfigError, ConfigLoader, ConfigSource, RuntimeConfig, TACHY_SETTINGS_SCHEMA_NAME,
};
pub use conversation::{
    ApiClient, ApiRequest, AssistantEvent, ConversationRuntime, ResponseFormat, RuntimeError,
    RuntimeEvent, StaticToolExecutor, ToolError, ToolExecutor, TurnSummary,
};
pub use diff::{DiffHunk, DiffLine, UnifiedDiff};
pub use file_ops::{
    edit_file, glob_search, grep_search, list_directory, preview_edit_file, preview_write_file,
    read_file, write_file, DiffPreview, DirEntry, EditFileOutput, GlobSearchOutput,
    GrepSearchInput, GrepSearchOutput, ListDirectoryOutput, ReadFileOutput, StructuredPatchHunk,
    TextFilePayload, WriteFileOutput,
};
pub use filelock::{FileLockManager, LockError};
pub use permissions::{
    PermissionMode, PermissionOutcome, PermissionPolicy, PermissionPromptDecision,
    PermissionPrompter, PermissionRequest,
};
pub use prompt::{
    load_system_prompt, prepend_bullets, ContextFile, ProjectContext, PromptBuildError,
    SystemPromptBuilder, FRONTIER_MODEL_NAME, SYSTEM_PROMPT_DYNAMIC_BOUNDARY,
};
pub use semantic_cache::{CachedResult, Embedder, SemanticCache};
pub use session::{ContentBlock, ConversationMessage, MessageRole, Session, SessionError};
pub use transaction::{EditTransaction, PendingEdit, TransactionError};
pub use usage::{TokenUsage, UsageTracker};
