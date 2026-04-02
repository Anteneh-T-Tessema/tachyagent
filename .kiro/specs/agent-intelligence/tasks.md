# Implementation Plan: Agent Intelligence

## Overview

Build the `intelligence` crate at `rust/crates/intelligence/` with 5 modules (indexer, context, git, edit_test_fix, planner) and integrate into `daemon` and `tachy-cli`. Tasks are ordered by dependency: indexing first (foundation), then context selection, git, edit-test-fix, and finally the planner that orchestrates everything. Each task builds incrementally on the previous ones.

## Tasks

- [x] 1. Create intelligence crate skeleton and shared types
  - [x] 1.1 Create `rust/crates/intelligence/Cargo.toml` with dependencies on `runtime`, `tools`, `backend`, `serde`, `serde_json`
    - Add the crate to the workspace `members` in `rust/Cargo.toml`
    - _Requirements: 13.1, 13.2_

  - [x] 1.2 Create `rust/crates/intelligence/src/lib.rs` with module declarations and `IntelligenceConfig` with `Default` impl
    - Declare modules: `indexer`, `context`, `git`, `edit_test_fix`, `planner`
    - Define `IntelligenceConfig` with all feature flags (`indexing_enabled`, `context_enabled`, `planning_enabled`, `edit_test_fix_enabled`, `git_enabled`) and nested configs (`IndexerConfig`, `PlanConfig`, `EditTestFixConfig`, `ContextConfig`)
    - Implement `Default` with values from design: `max_steps: 10`, `max_revisions: 3`, `max_retries: 3`, `test_timeout_secs: 120`, `max_context_percentage: 0.40`, `max_full_files: 5`, `max_summaries: 20`
    - Create stub module files (`indexer.rs`, `context.rs`, `git.rs`, `edit_test_fix.rs`, `planner.rs`) so the crate compiles
    - _Requirements: 11.1, 11.4, 13.1_

  - [ ]* 1.3 Write unit tests for `IntelligenceConfig` defaults
    - Verify all default values match requirements
    - Verify serde round-trip (serialize → deserialize produces equivalent config)
    - _Requirements: 11.4_

- [x] 2. Implement Codebase Indexing (`indexer.rs`)
  - [x] 2.1 Define data models: `CodebaseIndex`, `FileEntry`, `ProjectMeta`, `IndexError`
    - `CodebaseIndex` with `version: u32`, `workspace_root: String`, `built_at: u64`, `files: BTreeMap<String, FileEntry>`, `project: ProjectMeta`
    - `FileEntry` with `path`, `language`, `size`, `lines`, `exports`, `summary`, `content_hash`
    - `ProjectMeta` with `primary_language`, `test_command`, `build_system`, `total_files`, `total_lines`
    - `IndexError` enum: `Io`, `Json`, `WorkspaceNotFound`
    - All types derive `Serialize`, `Deserialize`, `Debug`, `Clone`
    - _Requirements: 1.1, 14.1, 14.2, 14.3, 14.4_

  - [x] 2.2 Implement `detect_language` and `detect_test_command`
    - `detect_language(path) -> &str` mapping file extensions to language strings (`.rs` → `"rust"`, `.py` → `"python"`, etc.), returning `"unknown"` for unrecognized
    - `detect_test_command(workspace_root) -> Option<String>` checking for `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `Makefile`
    - _Requirements: 1.2, 1.9_

  - [ ]* 2.3 Write property tests for `detect_language` and `detect_test_command`
    - **Property 2: Language detection is deterministic and total**
    - **Property 5: Test command detection is deterministic**
    - **Validates: Requirements 1.2, 1.9**

  - [x] 2.4 Implement `extract_summary` and ignored-path filtering
    - `extract_summary(path, content, language) -> (Vec<String>, String)` extracting up to 20 exports and a summary truncated to 120 chars
    - `is_ignored_path(path) -> bool` for `.git/`, `.tachy/`, `node_modules/`, `target/`, `__pycache__/`, `.venv/`, `vendor/`
    - `is_binary_extension(path) -> bool` for `.png`, `.jpg`, `.wasm`, `.o`, `.so`, `.exe`
    - Language-specific regex patterns for Rust (`pub fn`, `pub struct`), Python (`def `, `class `), TypeScript (`export function`, `export class`), Go (capitalized names)
    - _Requirements: 1.3, 1.4, 1.5, 1.6_

  - [ ]* 2.5 Write property tests for `extract_summary`
    - **Property 3: Extract summary respects output bounds**
    - **Validates: Requirements 1.3, 1.4**

  - [x] 2.6 Implement `build_index`, `save_index`, `load_index`, `update_index`
    - `build_index(workspace_root) -> Result<CodebaseIndex, IndexError>`: walk directory, skip ignored/binary/oversized files, build `FileEntry` for each, compute content hash, assemble `ProjectMeta`
    - `save_index(workspace_root, index) -> Result<(), IndexError>`: write pretty-printed JSON to `.tachy/index.json`
    - `load_index(workspace_root) -> Result<CodebaseIndex, IndexError>`: read and deserialize `.tachy/index.json`
    - `update_index(workspace_root, existing) -> Result<(CodebaseIndex, usize), IndexError>`: re-index only files with changed content hash
    - Return `IndexError::WorkspaceNotFound` if workspace root doesn't exist
    - _Requirements: 1.1, 1.7, 1.8, 2.1, 2.2, 2.4, 2.5, 2.6_

  - [ ]* 2.7 Write property tests for index serialization and file count
    - **Property 1: Index file count invariant**
    - **Property 6: Index serialization round-trip**
    - **Property 23: Index serialization is deterministic and pretty-printed**
    - **Validates: Requirements 1.1, 2.1, 2.2, 2.3, 14.4, 14.5**

  - [x] 2.8 Implement `search(index, query, max_results) -> Vec<&FileEntry>`
    - Match against file path, exports, summary, and language fields
    - Sort results by relevance score
    - Return at most `max_results` entries
    - _Requirements: 3.1, 3.2, 3.3_

  - [ ]* 2.9 Write property tests for search
    - **Property 8: Search respects max_results bound**
    - **Property 9: Search results are sorted by relevance**
    - **Validates: Requirements 3.2, 3.3**

- [x] 3. Checkpoint — Ensure indexer compiles and tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 4. Implement Smart Context Selection (`context.rs`)
  - [x] 4.1 Define data models: `ContextInjection`, `FileSummary`, `FileContent`, `ContextConfig`, `ContextError`
    - `ContextInjection` with `summaries`, `file_contents`, `estimated_tokens`, `token_budget`
    - `ContextConfig` with `max_context_percentage`, `max_full_files`, `max_summaries`, `min_relevance`
    - `ContextError` enum: `NoIndex`, `Io`
    - _Requirements: 10.3, 10.4_

  - [x] 4.2 Implement `extract_keywords`, `score_file`, and `rank_files`
    - `extract_keywords(prompt) -> Vec<String>`: extract file paths, function names, module names, error messages from prompt
    - `score_file(entry, keywords, prompt) -> f32`: score based on direct path mention (1.0), filename match (0.7), export match (0.5), summary overlap (0.2), penalize files >500 lines
    - `rank_files(files, keywords, prompt) -> Vec<(String, f32)>`: sort by score descending
    - _Requirements: 10.1, 10.2_

  - [ ]* 4.3 Write property tests for relevance scoring
    - **Property 21: Relevance scoring is non-negative and deterministic**
    - **Validates: Requirement 10.2**

  - [x] 4.4 Implement `select_context` and `render_injection`
    - `select_context(prompt, index, model_context_window, config) -> Result<ContextInjection, ContextError>`: extract keywords, rank files, read top files within token budget, build injection
    - Token estimation: `text.len() / 4`
    - Token budget: `model_context_window * max_context_percentage`
    - Enforce `max_full_files` and `max_summaries` limits
    - Order file contents by relevance score (highest first)
    - `render_injection(injection) -> String`: format as system prompt section with project metadata, file summaries with exports, and file contents with language annotations
    - _Requirements: 10.3, 10.4, 10.5, 10.6_

  - [ ]* 4.5 Write property tests for context selection
    - **Property 18: Context injection respects token budget**
    - **Property 19: Context injection respects count limits**
    - **Property 20: Context injection files are ordered by relevance**
    - **Validates: Requirements 10.3, 10.4, 10.5**

- [x] 5. Checkpoint — Ensure context module compiles and tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 6. Implement Git Integration (`git.rs`)
  - [x] 6.1 Define data models: `GitStatus`, `FileChange`, `ChangeStatus`, `GitDiff`, `FileDiff`, `DiffStats`, `CommitResult`, `BranchResult`, `GitError`
    - All types derive `Serialize`, `Deserialize`, `Debug`, `Clone`
    - `GitError` enum: `NotARepo`, `CommandFailed`, `NothingToCommit`
    - _Requirements: 7.1, 7.3, 8.1, 8.3_

  - [x] 6.2 Implement `GitTools::status()`, `GitTools::diff()`, `GitTools::current_branch()`, `GitTools::is_git_repo()`
    - `status()`: run `git status --porcelain`, parse into `GitStatus` with branch, staged, unstaged, untracked; set `is_clean` based on all lists being empty
    - `diff(path, staged)`: run `git diff` (or `git diff --staged`), parse into `GitDiff` with per-file diffs and stats
    - `current_branch()`: run `git rev-parse --abbrev-ref HEAD`
    - `is_git_repo()`: run `git rev-parse --is-inside-work-tree`
    - Return `GitError::NotARepo` when not in a git repo
    - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 7.6_

  - [ ]* 6.3 Write property test for GitStatus is_clean consistency
    - **Property 16: GitStatus is_clean consistency**
    - **Validates: Requirement 7.2**

  - [x] 6.4 Implement `GitTools::branch()` and `GitTools::commit()`
    - `branch(name, create)`: run `git checkout -b {name}` or `git checkout {name}`, return `BranchResult` with `created` flag and `previous_branch`
    - `commit(message)`: run `git add -A && git commit -m "{message}"`, parse commit hash from output, return `CommitResult`
    - Return `GitError::NothingToCommit` when no changes to commit
    - _Requirements: 8.1, 8.2, 8.3, 8.5_

  - [x] 6.5 Implement git tool execution functions for tool registration
    - Create `execute_git_status()`, `execute_git_diff(input)`, `execute_git_branch(input)`, `execute_git_commit(input)` functions that deserialize JSON input and delegate to `GitTools`
    - Define `git_tool_specs() -> Vec<ToolSpec>` with JSON schemas for all 4 git tools
    - _Requirements: 9.1, 9.3_

  - [ ]* 6.6 Write unit tests for git output parsing
    - Test `GitStatus` parsing from `git status --porcelain` output
    - Test `GitDiff` parsing from `git diff` output
    - Test `CommitResult` parsing from `git commit` output
    - Test `GitError::NotARepo` when not in a repo
    - _Requirements: 7.1, 7.3, 8.3_

- [x] 7. Checkpoint — Ensure git module compiles and tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 8. Implement Edit-Test-Fix Cycle (`edit_test_fix.rs`)
  - [x] 8.1 Define data models: `CycleResult`, `CycleOutcome`, `EditTestFixConfig`, `EditTestFixError`, `TestResult`
    - `CycleOutcome` enum: `Passed`, `Fixed`, `Failed`, `NoTestCommand`, `Timeout`
    - `CycleResult` with `outcome`, `retries`, `test_command`, `files_modified`, `test_output`
    - `EditTestFixConfig` with `max_retries` (3), `test_command` (Option), `test_timeout_secs` (120), `targeted_tests` (true)
    - `EditTestFixError` enum: `NoTestCommand`, `Timeout`, `Execution`
    - _Requirements: 6.1, 6.2, 6.5, 6.6, 6.7_

  - [x] 8.2 Implement `detect_test_command`, `targeted_test_command`, and `run_tests`
    - `detect_test_command(workspace_root, index)`: use index's `project.test_command` if available, otherwise check build files
    - `targeted_test_command(base_command, edited_files) -> String`: for `cargo test` extract module names from paths, for `pytest` target test files
    - `run_tests(command, timeout_secs) -> Result<TestResult, EditTestFixError>`: execute via `std::process::Command` with timeout, capture stdout/stderr
    - _Requirements: 6.1, 6.7, 6.8_

  - [ ]* 8.3 Write property test for targeted test commands
    - **Property 15: Targeted test command includes edited modules**
    - **Validates: Requirement 6.8**

  - [x] 8.4 Implement `EditTestFix::run_cycle`
    - Detect or use configured test command (return `NoTestCommand` if none)
    - Run tests; if pass, return `CycleResult { outcome: Passed, retries: 0 }`
    - On failure: truncate stderr to 4000 chars, stdout tail to 2000 chars, build fix prompt, call `runtime.run_turn()` with fix prompt
    - Retry up to `max_retries` times; return `Fixed` on success, `Failed` after exhausting retries
    - Handle timeout: return `CycleResult { outcome: Timeout }`
    - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 6.6, 6.7_

  - [ ]* 8.5 Write property test for CycleResult outcome consistency
    - **Property 14: EditTestFix outcome consistency**
    - **Validates: Requirements 6.2, 6.4, 6.5**

- [x] 9. Checkpoint — Ensure edit-test-fix module compiles and tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 10. Implement Plan-and-Execute Loop (`planner.rs`)
  - [x] 10.1 Define data models: `Plan`, `PlanStep`, `PlanStatus`, `StepStatus`, `PlanConfig`, `PlanExecutionResult`, `PlanError`
    - `Plan` with `id`, `prompt`, `steps`, `current_step`, `status`
    - `PlanStep` with `number`, `description`, `instruction`, `expected_files`, `status`, `result`
    - `PlanStatus` enum: `Created`, `InProgress`, `Completed`, `Failed`, `Revised`
    - `StepStatus` enum: `Pending`, `Running`, `Completed`, `Failed`, `Skipped`
    - `PlanConfig` with `max_steps` (10), `max_revisions` (3), `auto_commit` (true), `auto_branch` (true), `auto_test` (true)
    - `PlanError` enum: `PlanGeneration`, `StepFailed`, `MaxRevisions`, `Runtime`
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 5.1, 5.5_

  - [ ]* 10.2 Write property tests for plan structure
    - **Property 10: Created plans have valid structure**
    - **Validates: Requirements 4.2, 4.3**

  - [x] 10.3 Implement `PlanExecutor::create_plan`
    - Build the planning prompt with user task and optional codebase index summary
    - Send to LLM via `runtime.run_turn()`, parse JSON response into `Plan`
    - Validate step count is in `[1, max_steps]`, all steps have `Pending` status
    - Return `PlanError::PlanGeneration` if JSON parsing fails
    - _Requirements: 4.1, 4.2, 4.3, 4.4_

  - [x] 10.4 Implement `PlanExecutor::execute` and `execute_step`
    - Process steps in order, calling `runtime.run_turn(step.instruction)` for each
    - Mark steps `Running` → `Completed` or `Failed`
    - On step success with `auto_commit`: call `GitTools::commit(step.description)`
    - On step success with `auto_test` and code edits: trigger `EditTestFix::run_cycle`
    - On step failure: call `revise_plan` to replace remaining steps
    - _Requirements: 5.1, 5.2, 5.3, 5.7, 5.8, 5.9_

  - [x] 10.5 Implement `PlanExecutor::revise_plan`
    - On step failure, ask LLM to revise remaining unexecuted steps
    - Replace only remaining steps; preserve completed steps and their results
    - Stop execution if `max_revisions` reached, report partial results
    - Return `PlanExecutionResult` with total iterations, tool invocations, revision count, success status
    - _Requirements: 5.3, 5.4, 5.5, 5.6_

  - [ ]* 10.6 Write property tests for plan execution
    - **Property 11: Plan execution processes steps in order and transitions states correctly**
    - **Property 12: Plan revision preserves completed steps**
    - **Property 13: Plan execution respects revision limit and reports results**
    - **Validates: Requirements 5.1, 5.2, 5.4, 5.5, 5.6**

- [x] 11. Checkpoint — Ensure planner module compiles and tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 12. Integrate intelligence into daemon and CLI
  - [x] 12.1 Add `IntelligenceConfig` to `platform::AgentConfig` and wire into `daemon::AgentEngine::run_agent`
    - Add `intelligence: IntelligenceConfig` field to `AgentConfig` in `rust/crates/platform/src/agent.rs`
    - Add `intelligence` dependency to `platform/Cargo.toml`
    - In `daemon/src/engine.rs`, modify `run_agent()` to:
      - Build/load codebase index if `indexing_enabled`
      - Select context and prepend to system prompt if `context_enabled`
      - Use `PlanExecutor` if `planning_enabled`, else fall back to single `run_turn()`
      - Register git tools if `git_enabled`
    - Add `intelligence` and `daemon` dependencies on the `intelligence` crate
    - _Requirements: 10.7, 11.2, 12.1, 12.2, 12.4, 12.5, 12.6, 12.7, 13.4_

  - [x] 12.2 Register git tools in the tool executor and governance pipeline
    - Extend `FilteredToolExecutor` in `daemon/src/engine.rs` to delegate `git_status`, `git_diff`, `git_branch`, `git_commit` to `intelligence::git` functions
    - Ensure governance policy's `allowed_tools` controls access to git tools
    - Log audit events for git commits via `AuditLogger`
    - _Requirements: 9.1, 9.2, 9.3, 8.6_

  - [x] 12.3 Wire `IntelligenceConfig` into `tachy-cli`
    - Add `intelligence` dependency to `tachy-cli/Cargo.toml`
    - Load `IntelligenceConfig` from `.tachy/config.json` (under `"intelligence"` key) or use defaults
    - Pass config through to `AgentEngine::run_agent()` and `LiveCli`
    - Register git tools in `CliToolExecutor`
    - _Requirements: 4.5, 11.3, 13.4_

  - [x] 12.4 Implement graceful degradation in engine integration
    - If indexer fails (`IndexError::Io`): log warning, continue without index
    - If planner fails (`PlanError::PlanGeneration`): fall back to single `run_turn()`
    - If planner fails (`PlanError::StepFailed` after max revisions): report partial results
    - If ETF returns `NoTestCommand`: skip ETF, log info
    - If ETF returns `Timeout`: report timeout, continue to next step
    - If git returns `NotARepo`: skip git operations, log info
    - If context selector fails (`NoIndex`): use base system prompt
    - _Requirements: 12.1, 12.2, 12.3, 12.4, 12.5, 12.6, 12.7_

  - [ ]* 12.5 Write property test for feature flag behavior
    - **Property 22: Feature flags disable corresponding behavior**
    - **Validates: Requirements 11.2, 12.1, 12.2, 12.4, 12.5, 12.6, 12.7**

- [x] 13. Final checkpoint — Ensure all tests pass and crate compiles cleanly
  - Ensure all tests pass, ask the user if questions arise.
  - Verify `cargo build` succeeds for the entire workspace
  - Verify no circular dependencies between `intelligence` and `runtime`/`tools`/`backend`

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation after each major module
- Property tests validate universal correctness properties from the design document
- The `intelligence` crate has one-way dependencies: it depends on `runtime`, `tools`, `backend` but they do not depend on it
- Git tool registration avoids circular deps by wiring at the `daemon`/`tachy-cli` level
