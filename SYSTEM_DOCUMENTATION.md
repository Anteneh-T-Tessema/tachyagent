# Tachy Agent Platform — System Documentation

## Table of Contents

1. [Overview](#1-overview)
2. [Architecture](#2-architecture)
3. [Crate Reference](#3-crate-reference)
4. [CLI Reference](#4-cli-reference)
5. [HTTP API Reference](#5-http-api-reference)
6. [Agent Templates](#6-agent-templates)
7. [Tool System](#7-tool-system)
8. [Intelligence Layer](#8-intelligence-layer)
9. [Governance & Policy Engine](#9-governance--policy-engine)
10. [Parallel Execution](#10-parallel-execution)
11. [File Locking](#11-file-locking)
12. [Diff Preview System](#12-diff-preview-system)
13. [SSO / SAML Integration](#13-sso--saml-integration)
14. [Audit Trail](#14-audit-trail)
15. [Python SDK](#15-python-sdk)
16. [VS Code Extension](#16-vs-code-extension)
17. [CI/CD GitHub Action](#17-cicd-github-action)
18. [Configuration Reference](#18-configuration-reference)
19. [Model Recommendations](#19-model-recommendations)
20. [Development Guide](#20-development-guide)

---

## 1. Overview

Tachy is a local-first AI agent platform. It runs open LLMs (Gemma 4, Qwen3, Llama 3.1) via Ollama, executes tools against your codebase, and logs every action to an immutable audit trail. Single Rust binary. No cloud. No API keys required.

Key properties:
- Zero data leaves your machine
- 11 Rust crates, ~24K lines, 283 tests
- 13 built-in tools + custom YAML tools
- 5 agent templates
- 30+ HTTP API endpoints
- SHA-256 hash-chained audit trail
- Enterprise governance with patch-level policy engine
- SAML 2.0 SSO for enterprise auth
- Parallel agent execution with DAG scheduling
- File-level locking for concurrent agent safety
- Python SDK and VS Code extension

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        tachy-cli (binary)                       │
│  REPL · Commands · Markdown Rendering · Colored Diff Display    │
├─────────────────────────────────────────────────────────────────┤
│                        daemon                                   │
│  HTTP Server · Agent Engine · Web UI · MCP Server               │
│  Parallel Orchestrator · SSO Manager · Policy Engine            │
├──────────────┬──────────────┬───────────────┬───────────────────┤
│ intelligence │   platform   │    backend    │      audit        │
│ Indexer      │ Templates    │ Ollama        │ Events            │
│ Context      │ Scheduler    │ OpenAI-compat │ Logger            │
│ Planner      │ Workspace    │ Registry      │ Governance        │
│ ETF Loop     │ YAML Agents  │ Discovery     │ Policy Engine     │
│ LSP          │              │               │ RBAC              │
│ Memory       │              │               │ SSO/SAML          │
│ Verification │              │               │ License           │
├──────────────┴──────────────┴───────────────┴───────────────────┤
│                        runtime                                  │
│  Conversation Loop · Session · Permissions · Diff Engine        │
│  File Operations · Bash · Config · File Locks · Usage Tracking  │
├─────────────────────────────────────────────────────────────────┤
│                        tools                                    │
│  Built-in Tools · Custom YAML Tools · Web Search/Fetch          │
└─────────────────────────────────────────────────────────────────┘
```

Data flow for a single agent run:

```
User Prompt
  → SystemPromptBuilder (project context, config, memory)
  → ConversationRuntime.run_turn()
    → ApiClient.stream() → Ollama/OpenAI backend
    → ToolExecutor.execute() for each tool call
      → FileLockManager.try_acquire() (if write/edit)
      → PolicyEngine.evaluate() (patch governance)
      → DiffPreview computed before write
      → AuditLogger.log() (diff to audit trail)
      → FileLockManager.release()
    → Loop until no more tool calls or max_iterations
  → TurnSummary returned
```

---

## 3. Crate Reference

### `tachy-cli` (binary)
Entry point. REPL, slash commands, markdown rendering, colored diff display, agent execution, session management.

Key files:
- `main.rs` — CLI parsing, REPL loop, `LiveCli`, `CliToolExecutor`
- `render.rs` — `TerminalRenderer`, syntax highlighting, spinner
- `input.rs` — Line editing, history

### `daemon`
HTTP server, agent execution engine, parallel orchestrator, MCP server, web UI.

Key files:
- `engine.rs` — `AgentEngine::run_agent()`, `IntelligentToolExecutor` (file locks, policy engine, diff audit)
- `http.rs` — 30+ HTTP endpoints, SSO handlers, parallel run handlers, completion endpoint
- `parallel.rs` — `Orchestrator`, `TaskQueue`, `AgentTask`, DAG scheduling, `execute_parallel_run()`
- `state.rs` — `DaemonState` (shared state, patch queue, SSO manager, file locks)
- `mcp.rs` — JSON-RPC 2.0 MCP server over stdio
- `channels.rs` — Channel configuration
- `web.rs` — Embedded web UI HTML

### `intelligence`
AI-powered features that make agents smarter than raw LLM calls.

Key files:
- `indexer.rs` — `CodebaseIndexer`, `CodebaseIndex`, file scanning, language detection, test command detection
- `context.rs` — `ContextSelector`, smart context injection based on prompt relevance
- `planner.rs` — `PlanExecutor`, plan-and-execute for complex multi-step tasks
- `edit_test_fix.rs` — `EditTestFix`, diagnostic-first cycle: LSP diagnostics → tests → fix prompt → retry
- `lsp.rs` — `LspManager`, language-specific diagnostics (tsc, python, cargo check, go vet)
- `memory.rs` — `AgentMemory`, persistent cross-session memory
- `prompts.rs` — Model-specific prompt optimization (Gemma 4, Qwen, Llama families)
- `verification.rs` — Output verification, code block extraction
- `validation.rs` — Code quality validation, artifact cleaning
- `git.rs` — Git tools (status, diff, branch, commit)

### `platform`
Agent configuration, templates, scheduling, workspace management.

Key files:
- `agent.rs` — `AgentTemplate` (5 built-in templates), `AgentConfig`, `AgentInstance`
- `agent_yaml.rs` — YAML agent definition loading
- `scheduler.rs` — `TaskScheduler`, `ScheduledTask`, `ScheduleRule`
- `workspace.rs` — `PlatformWorkspace`, `.tachy/` directory management

### `backend`
LLM backend abstraction. Supports Ollama and OpenAI-compatible APIs.

Key files:
- `ollama.rs` — Ollama client, tool call parsing, streaming
- `openai_compat.rs` — OpenAI-compatible API client
- `registry.rs` — `BackendRegistry`, model-to-backend mapping
- `discovery.rs` — Auto-discovery of available models

### `audit`
Enterprise compliance: audit trail, governance, RBAC, licensing, SSO.

Key files:
- `event.rs` — `AuditEvent`, SHA-256 hash chain, `verify_audit_chain()`
- `logger.rs` — `AuditLogger`, `FileAuditSink`, append-only JSONL
- `policy.rs` — `GovernancePolicy`, tool rules, protected paths, approval-required paths
- `policy_engine.rs` — `PolicyEngine`, `FilePatch`, `PolicyDecision` (AutoApprove/RequiresApproval/Reject)
- `policy_file.rs` — `PolicyFile`, load governance from `.tachy/policy.json`
- `rbac.rs` — `Role` (Viewer/Developer/Admin), `UserStore`, `check_permission()`
- `security.rs` — API key hashing, rate limiting, input sanitization, path safety, secret redaction
- `sso.rs` — `SsoManager`, SAML 2.0 SP, session management, role mapping from IdP groups
- `license.rs` — HMAC-SHA256 license keys, offline verification, tier management

### `runtime`
Core conversation loop and tool execution primitives.

Key files:
- `conversation.rs` — `ConversationRuntime`, `ApiClient` trait, `ToolExecutor` trait, turn loop with error recovery
- `file_ops.rs` — `write_file()`, `edit_file()`, `read_file()`, `preview_write_file()`, `preview_edit_file()`, `DiffPreview`
- `diff.rs` — `UnifiedDiff::compute()`, `render()`, `render_colored()`, LCS-based diff algorithm
- `filelock.rs` — `FileLockManager`, cooperative file locks with TTL, wait-with-timeout
- `permissions.rs` — `PermissionPolicy`, `PermissionPrompter`, Allow/Deny/Prompt modes
- `session.rs` — `Session`, `ConversationMessage`, `ContentBlock`, JSON persistence
- `compact.rs` — Session compaction to prevent context overflow
- `bash.rs` — Shell command execution with timeout
- `config.rs` — `RuntimeConfig`, `ConfigLoader`, settings merge
- `prompt.rs` — `SystemPromptBuilder`, project context discovery, instruction file loading
- `usage.rs` — `UsageTracker`, token counting

### `tools`
Tool definitions and execution.

Key files:
- `lib.rs` — `mvp_tool_specs()` (13 tools), `execute_tool()`, `execute_tool_with_diff()`
- `web.rs` — `web_search()` (DuckDuckGo), `web_fetch()` (curl + HTML stripping)
- `custom.rs` — `CustomToolRegistry`, YAML-defined shell/HTTP tools

---

## 4. CLI Reference

```
tachy                                   Interactive REPL (default model: gemma4:26b)
tachy --model <MODEL>                   REPL with specific model
tachy prompt "TEXT"                      Single prompt, non-interactive
tachy prompt "TEXT" --model <MODEL>      Single prompt with model override

tachy init                              Initialize .tachy/ workspace
tachy setup                             Full setup: install Ollama, pull model, warmup
tachy doctor                            Health check: Ollama, GPU, models, config

tachy models                            List all registered models
tachy models --local                    List locally installed Ollama models
tachy pull <MODEL>                      Pull a model via Ollama
tachy warmup <MODEL>                    Warm up a model (pre-load into memory)

tachy agents                            List agent templates
tachy run-agent <TEMPLATE> "PROMPT"     Run an agent template
tachy run-agent <TEMPLATE> "PROMPT" --model <MODEL>

tachy serve [ADDR]                      Start HTTP daemon (default: 0.0.0.0:7777)
tachy serve --workspace <PATH>          Serve with specific workspace root

tachy license                           Show license status
tachy activate <KEY>                    Activate a license key
tachy verify-audit                      Verify audit trail hash chain integrity

tachy resume                            Resume last saved session
tachy resume --command "TEXT"            Resume and immediately send a command
```

REPL slash commands:
```
/status         Show session stats (model, messages, tokens, tool calls)
/compact        Compact conversation history to free context
/save           Save session to disk for later resumption
/undo           Undo the last file edit
/help           Show available commands
/quit           Exit the REPL
```

---

## 5. HTTP API Reference

All endpoints return JSON. Authentication via `Authorization: Bearer <key>` header (optional, configured via `TACHY_API_KEY` env var).

### Health & Discovery

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Daemon health, model count, agent count |
| `GET` | `/api/models` | List available LLM models |
| `GET` | `/api/templates` | List agent templates |
| `GET` | `/api/metrics` | Prometheus-format metrics |

### Agents

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/agents` | List all agents |
| `GET` | `/api/agents/:id` | Get agent status and results |
| `POST` | `/api/agents/run` | Start an agent (async, returns 202) |

`POST /api/agents/run` body:
```json
{
  "template": "code-reviewer",
  "prompt": "review src/main.rs",
  "model": "gemma4:26b"          // optional
}
```

Response (202):
```json
{
  "agent_id": "agent-1",
  "status": "running",
  "message": "Agent started. Poll GET /api/agents to check status."
}
```

### Inline Completion

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/complete` | Synchronous code completion (for VS Code) |

```json
{
  "prefix": "fn main() {\n    ",
  "suffix": "\n}",
  "language": "rust",
  "model": "gemma4:26b",
  "max_tokens": 128
}
```

### Parallel Execution

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/parallel/run` | Submit a parallel run (DAG of tasks) |
| `GET` | `/api/parallel/runs` | List all parallel runs |
| `GET` | `/api/parallel/runs/:id` | Get run status with per-task details |
| `POST` | `/api/parallel/runs/:id/cancel` | Cancel a run or specific task |

`POST /api/parallel/run` body:
```json
{
  "tasks": [
    {"template": "code-reviewer", "prompt": "review auth", "priority": 10},
    {"template": "security-scanner", "prompt": "scan auth", "deps": ["run-123-t0"]},
    {"template": "test-runner", "prompt": "run tests", "deps": ["run-123-t0", "run-123-t1"]}
  ],
  "max_concurrency": 4
}
```

### Governance & Approvals

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/pending-approvals` | List pending agent + patch approvals |
| `POST` | `/api/approve` | Approve or reject a pending item |
| `GET` | `/api/file-locks` | List active file locks |

`POST /api/approve` body:
```json
{"patch_id": "patch-1", "approved": true}
// or
{"agent_id": "agent-1", "approved": false}
```

### SSO / Authentication

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/auth/sso/login` | Redirect to IdP login page |
| `POST` | `/api/auth/sso/callback` | Process SAML response, create session |
| `POST` | `/api/auth/sso/logout` | Invalidate SSO session |
| `GET` | `/api/auth/sso/sessions` | List active SSO sessions |

### Conversations

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/conversations` | List conversations |
| `POST` | `/api/conversations` | Create a conversation |
| `POST` | `/api/conversations/message` | Add a message |

### Scheduling

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tasks` | List scheduled tasks |
| `POST` | `/api/tasks/schedule` | Schedule an agent |

### Webhooks

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/webhooks` | Register a webhook |
| `GET` | `/api/webhooks` | List webhooks |
| `POST` | `/api/webhook/trigger` | Trigger a webhook-based agent run |

### Export

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/audit` | View audit log |
| `GET` | `/api/export/audit` | Export audit log as CSV |
| `GET` | `/api/export/agents` | Export agents as CSV |

### Streaming

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat/stream` | SSE streaming chat (legacy compat) |

---

## 6. Agent Templates

Five built-in templates, defined in `platform/src/agent.rs`:

| Template | Model | Tools | Planning | Description |
|----------|-------|-------|----------|-------------|
| `code-reviewer` | gemma4:26b | read_file, list_directory, grep_search, glob_search | yes | Reviews code for bugs, style, security |
| `security-scanner` | gemma4:26b | read_file, list_directory, grep_search, glob_search, bash | yes | Scans for vulnerabilities (OWASP Top 10) |
| `doc-writer` | gemma4:26b | read_file, write_file, edit_file, list_directory, grep_search, glob_search | yes | Generates and updates documentation |
| `chat-assistant` | gemma4:26b | all 13 tools (including web_search, web_fetch) | no | General-purpose interactive assistant |
| `test-runner` | gemma4:26b | list_directory, bash, read_file, grep_search | yes | Runs tests, analyzes failures, suggests fixes |

Custom templates can be defined in `.tachy/config.json` or as YAML files in `.tachy/agents/`.

YAML agent example (`.tachy/agents/my-agent.yaml`):
```yaml
name: my-agent
description: Custom agent for my project
model: gemma4:26b
system_prompt: |
  You are a specialist in React components.
  Focus on accessibility and performance.
allowed_tools:
  - read_file
  - write_file
  - edit_file
  - bash
max_iterations: 10
use_planning: true
```

---

## 7. Tool System

### Built-in Tools (13)

| Tool | Description | Category |
|------|-------------|----------|
| `bash` | Execute shell commands | shell |
| `read_file` | Read a file with optional offset/limit | read |
| `write_file` | Write/create a file (with diff preview) | write |
| `edit_file` | Replace text in a file (with diff preview, fuzzy matching) | write |
| `glob_search` | Find files by glob pattern | read |
| `grep_search` | Search file contents with regex | read |
| `list_directory` | List directory contents (skips noise dirs) | read |
| `web_search` | Search the web via DuckDuckGo | web |
| `web_fetch` | Fetch and extract text from a URL | web |
| `remember` | Store persistent cross-session memory | memory |
| `call_agent` | Call another agent (multi-agent orchestration) | agent |
| `git_status` | Git status | git |
| `git_diff` | Git diff | git |
| `git_branch` | Git branch operations | git |
| `git_commit` | Git commit | git |

### Custom Tools

Defined in `.tachy/tools.yaml`:
```yaml
tools:
  - name: deploy
    type: shell
    command: "kubectl apply -f {{file}}"
    description: "Deploy a Kubernetes manifest"
    parameters:
      file:
        type: string
        description: "Path to the manifest file"
        required: true

  - name: check-api
    type: http
    method: GET
    url: "https://api.example.com/health"
    description: "Check API health"
```

### Tool Execution Flow (write/edit)

```
Agent requests write_file/edit_file
  → FileLockManager.acquire_with_wait(file, agent_id, 30s)
  → GovernancePolicy.requires_approval(file_path)
  → preview_write_file() / preview_edit_file() — compute diff without writing
  → PolicyEngine.evaluate(FilePatch) — check rules
    → AutoApprove: proceed to write
    → RequiresApproval: queue patch, return error to agent
    → Reject: return error to agent
  → write_file() / edit_file() — write to disk, return DiffPreview
  → AuditLogger.log(diff_summary, diff_text)
  → FileLockManager.release(file, agent_id)
```

---

## 8. Intelligence Layer

### Codebase Indexing
`CodebaseIndexer` scans the workspace and builds a `CodebaseIndex` with:
- File inventory (path, language, size, line count)
- Project metadata (primary language, test command, build system)
- Function/class summaries for context injection

Index is cached to `.tachy/index.json` and incrementally updated.

### Smart Context Selection
`ContextSelector` analyzes the user's prompt and selects the most relevant files/summaries to inject into the system prompt, respecting the model's context window.

### Plan-and-Execute
For complex tasks, `PlanExecutor` asks the LLM to generate a multi-step plan, then executes each step sequentially with:
- Per-step tool execution
- Edit-test-fix after each step
- Git commit after each step (if enabled)
- Automatic git branch creation

### Edit-Test-Fix Cycle
After each code edit, the system runs a two-phase validation:

1. **LSP Diagnostics** (fast) — `LspManager` runs language-specific checks:
   - TypeScript: `npx tsc --noEmit`
   - Python: `python3 -m py_compile`
   - Rust: `cargo check --message-format=json`
   - Go: `go vet ./...`

2. **Test Execution** (thorough) — runs the project's test command with targeted test selection

If either phase fails, the system builds a fix prompt with error details and retries up to `max_retries` times (default: 3).

### Persistent Memory
`AgentMemory` stores cross-session memories in `.tachy/memory.jsonl`:
```json
{"content": "User prefers tabs over spaces", "category": "preference"}
{"content": "Project uses PostgreSQL 15", "category": "project"}
```

Memories are injected into the system prompt on every session.

### Model-Specific Prompt Optimization
`build_optimized_prompt()` detects the model family and applies tailored prompt formatting:
- Gemma 4: full tool support, native function calling
- Qwen: structured output hints
- Llama: explicit JSON formatting guidance

---

## 9. Governance & Policy Engine

### Two-Layer Governance

**Layer 1: GovernancePolicy** (tool-level, in `audit/src/policy.rs`)
- Block destructive shell commands (`rm -rf /`, `curl | sh`)
- Protected paths — block all writes to `/etc`, `~/.ssh`, `.env`, `secrets/`
- Approval-required paths — require human approval for `auth/`, `security/`, `crypto/`, `migrations/`, `deploy/`, `Dockerfile`, `.github/`
- Per-tool invocation limits (bash: 50, write_file: 200)
- Global invocation limit (500 per session)

**Layer 2: PolicyEngine** (patch-level, in `audit/src/policy_engine.rs`)
- Evaluates every file write/edit before it hits disk
- Rules are declarative and composable:

| Rule Type | Description | Default Action |
|-----------|-------------|----------------|
| `PathMatch` | Block/approve edits to path patterns | RequireApproval for auth/security/crypto |
| `MaxPatchSize` | Limit patch size (additions + deletions) | RequireApproval if > 500 lines |
| `ContentBlock` | Block patches containing regex patterns | Reject for passwords, API keys, private keys |
| `DiffContentBlock` | Check patterns in the diff itself | Configurable |
| `RequireTests` | Require tests to pass before applying | External check |

### Policy Decisions

- **AutoApprove** — patch is safe, write proceeds immediately
- **RequiresApproval** — patch is queued in `DaemonState.pending_patches`, agent gets an error, human reviews via `POST /api/approve`
- **Reject** — patch is blocked, agent gets an error with the reason

### Pending Patch Flow

```
Agent writes to auth/login.rs
  → PolicyEngine: RequiresApproval (path matches **/auth/**)
  → Patch queued as PendingPatch {id, file_path, new_content, reason}
  → Agent receives: "patch queued for approval (id=patch-1)"
  → Human calls GET /api/pending-approvals → sees the patch
  → Human calls POST /api/approve {patch_id: "patch-1", approved: true}
  → File written to disk, audit logged
```

### Custom Policy File

`.tachy/policy.json`:
```json
{
  "version": 1,
  "name": "My Company Policy",
  "rules": {
    "block_destructive_shell": true,
    "max_total_tool_invocations": 300,
    "protected_paths": ["/etc/**", "**/.env"],
    "tool_rules": [
      {
        "tool": "bash",
        "max_invocations": 30,
        "requires_approval": false,
        "blocked_patterns": ["rm\\s+-rf"]
      }
    ]
  }
}
```

---

## 10. Parallel Execution

### Architecture

```
POST /api/parallel/run
  → Build AgentTask DAG from request
  → Orchestrator.submit(ParallelRun)
  → Background thread: execute_parallel_run()
    → Worker loop:
      → Orchestrator.next_task() — respects deps + priority + max_concurrency
      → std::thread::spawn → execute_single_task()
        → AgentEngine::run_agent() with shared FileLockManager
        → FileLockManager.release_all(agent_id) on completion
      → Orchestrator.complete_task(result)
    → Until all tasks done
  → Webhook fired: parallel_run_completed
```

### Task Properties

| Field | Type | Description |
|-------|------|-------------|
| `template` | string | Agent template name |
| `prompt` | string | Task prompt |
| `model` | string? | Optional model override |
| `deps` | string[] | Task IDs this task depends on |
| `priority` | u8 | Higher = runs first (default: 5) |

### Concurrency

- `max_concurrency` capped at 8 parallel agents
- Tasks are priority-sorted within the queue
- Dependency-blocked tasks wait until all deps complete
- Failed tasks mark the run as `PartiallyCompleted`

### Run Status Values

| Status | Meaning |
|--------|---------|
| `Running` | Tasks still executing |
| `Completed` | All tasks succeeded |
| `PartiallyCompleted` | Some tasks succeeded, some failed |
| `Failed` | All tasks failed |
| `Cancelled` | Run was cancelled |

---

## 11. File Locking

`FileLockManager` prevents concurrent agents from corrupting each other's file edits.

- **Cooperative locks** — in-memory, no filesystem locks
- **Per-file granularity** — agents can edit different files simultaneously
- **Reentrant** — same agent can re-acquire its own lock
- **TTL-based expiry** — locks auto-expire after 5 minutes to prevent deadlocks
- **Wait-with-timeout** — `acquire_with_wait(file, agent_id, 30s)` retries every 100ms
- **Cleanup on completion** — `release_all(agent_id)` called when agent finishes

Shared across all parallel agents via `DaemonState.file_locks`.

Viewable via `GET /api/file-locks`:
```json
{
  "locks": [
    {"file": "src/main.rs", "agent_id": "run-123-t0"},
    {"file": "src/auth.rs", "agent_id": "run-123-t1"}
  ],
  "count": 2
}
```

---

## 12. Diff Preview System

Every `write_file` and `edit_file` operation computes a `UnifiedDiff` before writing to disk.

### DiffPreview struct

| Field | Type | Description |
|-------|------|-------------|
| `file_path` | String | Absolute path |
| `diff_text` | String | Unified diff (compatible with `git apply`) |
| `diff_colored` | String | ANSI-colored diff for terminal display |
| `summary` | String | e.g. "src/main.rs: +5 -3" |
| `additions` | usize | Lines added |
| `deletions` | usize | Lines deleted |
| `is_new_file` | bool | True if file didn't exist before |

### Preview-Only Mode

`preview_write_file()` and `preview_edit_file()` compute the diff without writing to disk. Used by the policy engine to evaluate patches before applying.

### CLI Display

In CLI mode, write/edit operations show a bordered colored diff:
```
  ⚡ edit_file src/auth.rs
    ┌─ diff: src/auth.rs: +2 -1
    │ --- a/src/auth.rs
    │ +++ b/src/auth.rs
    │ @@ -10,3 +10,4 @@
    │  fn verify(token: &str) {
    │ -    token == stored
    │ +    constant_time_eq(token, stored)
    │ +    .expect("comparison failed")
    │  }
    └─
```

### Daemon Audit

In daemon mode, the diff summary and full unified diff text are logged to the audit trail via `AuditEvent.with_redacted_payload(diff_text)`.

---

## 13. SSO / SAML Integration

SAML 2.0 Service Provider implementation for enterprise authentication.

### Flow

```
1. User → GET /api/auth/sso/login
2. Server builds SAML AuthnRequest → 302 redirect to IdP SSO URL
3. User authenticates at IdP (Okta, Azure AD, OneLogin, etc.)
4. IdP → POST /api/auth/sso/callback with SAMLResponse
5. Server parses SAML assertion:
   - Extracts NameID (email), Issuer, SessionIndex, Attributes, Groups
   - Validates issuer matches configured IdP
   - Resolves role from groups via role_mapping
   - Provisions user in UserStore
   - Creates SsoSession with token
6. Returns session token to client
7. Client uses token as Bearer auth for subsequent API calls
```

### Configuration

In `.tachy/config.json` or environment:
```json
{
  "sso": {
    "enabled": true,
    "idp_entity_id": "https://idp.yourcompany.com",
    "idp_sso_url": "https://idp.yourcompany.com/saml/sso",
    "idp_certificate": "-----BEGIN CERTIFICATE-----\n...",
    "sp_entity_id": "tachy-agent",
    "sp_acs_url": "https://tachy.yourcompany.com/api/auth/sso/callback",
    "default_role": "developer",
    "role_mapping": {
      "platform-admins": "admin",
      "engineering": "developer",
      "readonly": "viewer"
    },
    "session_duration_secs": 28800
  }
}
```

### Role Mapping

IdP groups are mapped to Tachy roles:

| IdP Group Pattern | Tachy Role | Permissions |
|-------------------|------------|-------------|
| Contains "admin" | Admin | Full access |
| Contains "developer"/"engineer" | Developer | Run agents, view results |
| Contains "viewer"/"readonly" | Viewer | View only |
| No match | `default_role` | Configurable |

### Session Management

- Sessions expire after `session_duration_secs` (default: 8 hours)
- `POST /api/auth/sso/logout` invalidates a session
- `GET /api/auth/sso/sessions` lists active sessions
- Expired sessions are cleaned up automatically

---

## 14. Audit Trail

Every action is logged to an append-only JSONL file at `.tachy/audit.jsonl`.

### Event Structure

```json
{
  "timestamp": "1711900800s",
  "session_id": "sess-agent-1",
  "kind": "tool_invocation",
  "severity": "info",
  "agent_id": "agent-1",
  "tool_name": "write_file",
  "model_name": "gemma4:26b",
  "detail": "diff preview: src/main.rs: +5 -3",
  "redacted_payload": "--- a/src/main.rs\n+++ b/src/main.rs\n...",
  "sequence": 42,
  "hash": "a1b2c3...",
  "prev_hash": "d4e5f6..."
}
```

### Hash Chain

Each event is signed with SHA-256: `hash = SHA256(prev_hash + content)`. If any event is modified or deleted, the chain breaks. Verify with:

```bash
tachy verify-audit
```

### Event Kinds

| Kind | Description |
|------|-------------|
| `SessionStart` | Agent/session started |
| `SessionEnd` | Agent/session completed |
| `UserMessage` | User input received |
| `AssistantMessage` | LLM response |
| `ToolInvocation` | Tool called |
| `ToolResult` | Tool returned (includes diff for write/edit) |
| `PermissionGranted` | Human approved an action |
| `PermissionDenied` | Human rejected or policy blocked |
| `GovernanceViolation` | Policy rule triggered |
| `SessionCompacted` | Session history compacted |
| `ConfigChange` | Configuration modified |
| `ModelSwitch` | Model changed mid-session |

### Severity Levels

| Level | Meaning |
|-------|---------|
| `Info` | Normal operation |
| `Warning` | Potential issue (failed tool, denied permission) |
| `Critical` | Security event (governance violation, destructive command blocked) |

---

## 15. Python SDK

Install: `pip install tachy-agent` (or from source: `pip install -e sdk/python`)

```python
from tachy import TachyClient, ParallelTask

client = TachyClient("http://localhost:7777", api_key="optional-key")

# Health check
client.health()

# Run an agent and wait for results
run = client.run_agent("code-reviewer", "review src/main.rs")
agent = client.wait_for_agent(run.agent_id, timeout=300)
print(agent.summary)

# Parallel execution
tasks = [
    ParallelTask(template="code-reviewer", prompt="review auth"),
    ParallelTask(template="security-scanner", prompt="scan for vulns"),
]
parallel = client.run_parallel(tasks, max_concurrency=2)
result = client.get_parallel_run(parallel.run_id)

# Governance
for approval in client.pending_approvals():
    print(f"{approval.type}: {approval.reason}")
    client.approve(patch_id=approval.id)

# File locks
for lock in client.list_file_locks():
    print(f"{lock.file} held by {lock.agent_id}")

# Webhooks
client.add_webhook("https://slack.com/webhook", ["agent_completed"])

# Audit
print(client.audit_log())
```

### SDK Methods

| Method | Endpoint | Returns |
|--------|----------|---------|
| `health()` | `GET /health` | dict |
| `list_models()` | `GET /api/models` | list[Model] |
| `list_templates()` | `GET /api/templates` | list[Template] |
| `run_agent(template, prompt, model?)` | `POST /api/agents/run` | AgentRun |
| `get_agent(id)` | `GET /api/agents/:id` | Agent |
| `list_agents()` | `GET /api/agents` | list[Agent] |
| `wait_for_agent(id, timeout?)` | Polls GET | Agent |
| `run_parallel(tasks, concurrency?)` | `POST /api/parallel/run` | ParallelRun |
| `get_parallel_run(id)` | `GET /api/parallel/runs/:id` | ParallelRun |
| `cancel_parallel_run(id, task_id?)` | `POST .../cancel` | dict |
| `pending_approvals()` | `GET /api/pending-approvals` | list[PendingApproval] |
| `approve(agent_id?, patch_id?)` | `POST /api/approve` | dict |
| `reject(agent_id?, patch_id?)` | `POST /api/approve` | dict |
| `list_file_locks()` | `GET /api/file-locks` | list[FileLock] |
| `audit_log()` | `GET /api/audit` | str |
| `metrics()` | `GET /api/metrics` | str |
| `add_webhook(url, events)` | `POST /api/webhooks` | dict |
| `list_webhooks()` | `GET /api/webhooks` | list[dict] |

---

## 16. VS Code Extension

Located in `vscode-extension/`. Provides AI-powered inline code completions from the local Tachy daemon.

### How It Works

1. As you type, the extension captures ~60 lines before cursor and ~20 lines after
2. Debounces for 300ms (configurable)
3. Sends context to `POST /api/complete` on the local daemon
4. Daemon runs a single-turn LLM completion via Ollama
5. Extension cleans the output (strips markdown fences, filters explanations)
6. Completion appears as ghost text — accept with Tab

### Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `tachy.endpoint` | `http://localhost:7777` | Daemon URL |
| `tachy.apiKey` | (empty) | API key |
| `tachy.model` | `gemma4:26b` | LLM model |
| `tachy.enabled` | `true` | Enable/disable |
| `tachy.maxTokens` | `128` | Max tokens per completion |
| `tachy.debounceMs` | `300` | Debounce delay |

### Commands

- **Tachy: Toggle Autocomplete** — Enable/disable inline completions
- **Tachy: Check Daemon Health** — Verify daemon is running

### Development

```bash
cd vscode-extension
npm install
npm run compile
# Press F5 in VS Code to launch Extension Development Host
```

---

## 17. CI/CD GitHub Action

### CI Workflow (`.github/workflows/ci.yml`)

Runs on every push/PR to main:
1. **Check & Lint** — `cargo fmt --check` + `cargo clippy` with `-Dwarnings`
2. **Test** — `cargo test --all` on Ubuntu + macOS
3. **Python SDK** — `pytest sdk/python/tests/`
4. **Build** — Release builds for linux-x86_64 and macos-arm64

### Reusable Action (`.github/actions/tachy-agent/`)

Run Tachy agents in any CI pipeline:

```yaml
- name: AI Code Review
  uses: Anteneh-T-Tessema/tachyagent/.github/actions/tachy-agent@main
  with:
    template: code-reviewer
    prompt: "Review the changes in this PR"
    fail-on-error: "true"
```

The action automatically installs Ollama, pulls the model, downloads the Tachy binary, and runs the agent. Outputs `summary`, `success`, `iterations`, and `tool-invocations` for downstream steps.

---

## 18. Configuration Reference

### `.tachy/config.json`

```json
{
  "governance": {
    "block_destructive_shell": true,
    "max_total_tool_invocations": 500,
    "protected_paths": ["/etc/**", "~/.ssh/**", "**/.env"],
    "approval_required_paths": ["**/auth/**", "**/security/**"],
    "tool_rules": [
      {
        "tool_name": "bash",
        "max_invocations_per_session": 50,
        "requires_approval": false,
        "blocked_patterns": ["rm\\s+-rf\\s+/"]
      }
    ]
  },
  "intelligence": {
    "indexing_enabled": true,
    "context_enabled": true,
    "planning_enabled": true,
    "edit_test_fix_enabled": true,
    "git_enabled": true,
    "edit_test_fix": {
      "max_retries": 3,
      "test_timeout_secs": 120,
      "lsp_diagnostics_enabled": true
    }
  },
  "agent_templates": [...]
}
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `TACHY_API_KEY` | API key for HTTP auth |
| `TACHY_PERMISSION_MODE` | `workspace-write` (default), `read-only`, `deny-all` |
| `TACHY_CONFIG_HOME` | Override config directory (default: `~/.tachy`) |
| `OLLAMA_HOST` | Ollama server URL (default: `http://localhost:11434`) |

---

## 19. Model Recommendations

| Model | VRAM/RAM | Best For | Context |
|-------|----------|----------|---------|
| `gemma4:26b` ⭐ | ~16 GB | Default — fast frontier, native tool calling | 256K |
| `gemma4:31b` | ~19 GB | Maximum quality, complex reasoning | 256K |
| `qwen3-coder:30b` | ~18 GB | Coding specialist | 32K |
| `gemma4:e4b` | ~5 GB | Fast simple tasks, quick tool calls | 128K |
| `qwen3:8b` | ~5 GB | Good general purpose on 16GB machines | 32K |
| `llama3.1:8b` | ~5 GB | Solid fallback | 128K |

On Apple M-series (64GB), `gemma4:26b` is the sweet spot — only 4B active parameters (MoE), so it's fast while delivering frontier-level code quality.

---

## 20. Development Guide

### Prerequisites

- Rust 1.75+ (install via `rustup`)
- Ollama (for running models)

### Build

```bash
cd rust
cargo build --release
```

### Test

```bash
# All Rust tests
cargo test --all

# Specific crate
cargo test -p daemon

# Integration tests only
cargo test -p daemon --test integration

# Python SDK tests
pip install pytest responses requests
pytest sdk/python/tests/ -v
```

### Project Structure

```
.
├── rust/                       Rust workspace (11 crates)
│   ├── crates/
│   │   ├── tachy-cli/          CLI binary
│   │   ├── daemon/             HTTP server + engine
│   │   ├── intelligence/       AI features
│   │   ├── platform/           Config + templates
│   │   ├── backend/            LLM backends
│   │   ├── audit/              Compliance + security
│   │   ├── runtime/            Core loop + tools
│   │   ├── tools/              Tool definitions
│   │   ├── commands/           Slash commands
│   │   ├── api/                Legacy API client
│   │   └── compat-harness/     Legacy compat
│   └── Cargo.toml
├── sdk/python/                 Python SDK
├── vscode-extension/           VS Code extension
├── .github/
│   ├── workflows/ci.yml        CI pipeline
│   ├── workflows/release.yml   Release pipeline
│   └── actions/tachy-agent/    Reusable GitHub Action
└── landing/                    Website + install script
```

### Test Counts (as of latest)

| Crate | Tests |
|-------|-------|
| audit | 47 |
| intelligence | 61 |
| runtime | 37 |
| daemon (unit) | 23 |
| daemon (integration) | 15 |
| tools | 17 |
| api | 18 |
| platform | 10 |
| backend | 30 |
| commands | 2 |
| Python SDK | 13 |
| **Total** | **283** |
