# Tachy

**Enterprise AI agents that run on your infrastructure. Zero data leaves your network.**

Tachy is an on-premise AI agent platform. It connects to local models via Ollama (or any OpenAI-compatible endpoint), executes tools against your codebase, and logs every action to an immutable audit trail. Single binary. No cloud dependencies.

## Quick Start

```bash
# Install Ollama (if you haven't already)
curl -fsSL https://ollama.com/install.sh | sh
ollama pull llama3.1:8b

# Install Tachy
curl -fsSL https://tachy.dev/install.sh | sh

# Initialize workspace and start
tachy init
tachy --model llama3.1:8b
```

That's it. You're running a tool-using AI agent locally.

## What It Does

```bash
# Interactive REPL with tool use
tachy --model llama3.1:8b
› Read the auth module and check for security issues
⠋ Thinking
### Tool `read_file` → src/auth.rs (247 lines)
### Tool `grep_search` → found 3 matches for "password"
✔ Done
Found 2 issues:
1. Line 42: password compared with == instead of constant-time comparison
2. Line 118: JWT secret loaded from environment without validation

# Run pre-built agent templates
tachy run-agent security-scanner "audit the authentication module"
tachy run-agent code-reviewer "review the latest changes"
tachy run-agent test-runner "run tests and fix failures"

# Start HTTP daemon for programmatic access
tachy serve
curl -X POST localhost:7777/api/agents/run \
  -d '{"template":"code-reviewer","prompt":"review src/api.rs"}'

# Check system health
tachy doctor
```

## Why Tachy

**For enterprises that can't send code to the cloud:**

- Law firms with attorney-client privilege
- Banks and fintechs under SOX/PCI compliance
- Healthcare companies bound by HIPAA
- Defense contractors with ITAR restrictions
- Any company with a "no external AI" policy

**What you get:**

- Full audit trail — every prompt, tool call, and response logged to append-only JSONL
- Governance policies — block destructive commands, enforce tool rate limits, protect sensitive paths
- Model-agnostic — works with Ollama, vLLM, LM Studio, or any OpenAI-compatible endpoint
- Single binary — no Python, no Node.js, no Docker required
- 6 built-in tools — bash, file read/write/edit, glob search, grep search
- 4 agent templates — code reviewer, security scanner, doc generator, test runner
- HTTP API — 7 endpoints for programmatic agent management

## Commands

```
tachy init                              Initialize workspace
tachy doctor                            Check Ollama, GPU, models
tachy pull <model>                      Pull a model via Ollama
tachy models                            List registered models
tachy models --local                    List locally installed models
tachy agents                            List agent templates
tachy [--model MODEL]                   Interactive REPL
tachy [--model MODEL] prompt TEXT       Single prompt
tachy run-agent <template> <prompt>     Run agent template
tachy serve [ADDR]                      Start HTTP daemon
```

## HTTP API

```
GET  /health              → {"status":"ok","models":19,"agents":0,"tasks":0}
GET  /api/models          → [{name, backend, supports_tool_use, context_window}]
GET  /api/templates       → [{name, description, model, tools}]
GET  /api/agents          → [{id, template, status, iterations, summary}]
GET  /api/tasks           → [{id, name, schedule, status, run_count}]
POST /api/agents/run      → {"template":"...","prompt":"..."}
POST /api/tasks/schedule  → {"template":"...","name":"...","interval_seconds":N}
```

## Configuration

`tachy init` creates a `.tachy/` directory with:

```
.tachy/
├── config.json      # Backends, models, governance policy, agent templates
├── audit.jsonl      # Immutable audit log (append-only)
└── sessions/        # Persisted agent sessions
```

Edit `.tachy/config.json` to configure backends, add custom agent templates, or adjust governance rules.

## Governance

Default enterprise policy blocks:
- `rm -rf /` and destructive shell commands
- `curl | sh` and remote code execution patterns
- Writes to `/etc`, `~/.ssh`, `.env`, and `secrets/`
- More than 50 bash calls or 500 total tool calls per session

All violations are logged to the audit trail with severity levels.

## Architecture

```
tachy-cli          CLI binary (REPL, commands, markdown rendering)
├── daemon         HTTP server + agent execution engine
├── platform       Agent templates, scheduler, workspace config
├── backend        Multi-model: Anthropic, Ollama, OpenAI-compatible
├── audit          Compliance: events, logging, governance policy
├── runtime        Core: conversation loop, tools, permissions, sessions
└── tools          Tool definitions + execution (bash, files, search)
```

Built in Rust. Single binary. No unsafe code. Memory-safe by construction.

## License

Proprietary. Contact hello@tachy.dev for licensing.
