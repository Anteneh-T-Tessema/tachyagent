# Tachy

**The best local AI coding agent. Gemma 4 + Ollama + enterprise-grade security. Zero data leaves your machine.**

Tachy is a local-first AI agent platform. It runs frontier open models (Gemma 4, Qwen3, Llama 3.1) via Ollama, executes tools against your codebase, and logs every action to an immutable audit trail. Single binary. No cloud. No API keys.

## Quick Start

```bash
# One command to install everything
curl -fsSL https://tachy.dev/install.sh | sh
```

That's it. The install script downloads Tachy, which then:
1. Installs Ollama automatically (if not present)
2. Starts the Ollama server
3. Detects your RAM and pulls the right model
4. Initializes the workspace
5. Warms up the model

Or install manually:

```bash
# Install Ollama (if you don't have it)
# macOS: brew install ollama
# Linux: curl -fsSL https://ollama.com/install.sh | sh
# Windows: download from https://ollama.com/download

# Pull a model based on your RAM
ollama pull gemma4:26b    # 32GB+ RAM
ollama pull qwen3:8b      # 16GB RAM
ollama pull gemma4:e4b    # 8GB RAM

# Install and run Tachy
tachy setup
tachy
```

That's it. You're running a frontier-class AI coding agent locally.

`tachy setup` automatically installs Ollama, pulls the right model for your hardware, and warms it up. Zero manual steps.

## What It Does

```bash
# Interactive REPL with tool use
tachy --model gemma4:26b
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

**For anyone who wants frontier AI coding capability without sending code to the cloud:**

- Developers who want full control over their AI tools
- Law firms with attorney-client privilege
- Banks and fintechs under SOX/PCI compliance
- Healthcare companies bound by HIPAA
- Defense contractors with ITAR restrictions
- Any company with a "no external AI" policy

**What you get:**

- Frontier local models — Gemma 4 (256K context, native tool calling, LiveCodeBench 80%)
- Full audit trail — every prompt, tool call, and response logged to append-only JSONL
- Intelligence layer — codebase indexing, smart context, plan-and-execute, edit-test-fix
- Governance policies — block destructive commands, enforce tool rate limits, protect sensitive paths
- Model-agnostic — works with any Ollama model (Gemma 4, Qwen3, Llama 3.1, Mistral, etc.)
- Single binary — no Python, no Node.js, no Docker required
- 10 built-in tools — bash, file read/write/edit, glob search, grep search, git tools
- 4 agent templates — code reviewer, security scanner, doc generator, test runner
- HTTP API — 19 endpoints for programmatic agent management

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
GET  /health                → {"status":"ok","models":19,"agents":0,"tasks":0}
GET  /api/models            → [{name, backend, supports_tool_use, context_window}]
GET  /api/templates         → [{name, description, model, tools}]
GET  /api/agents            → [{id, template, status, iterations, summary}]
GET  /api/agents/:id        → {id, template, status, iterations, tool_invocations, summary}
GET  /api/tasks             → [{id, name, schedule, status, run_count}]
POST /api/agents/run        → 202 {"agent_id":"...","status":"running"} (async)
POST /api/tasks/schedule    → {"template":"...","name":"...","interval_seconds":N}
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

## Recommended Models

| Model | Size (Q4) | Best For | Context |
|---|---|---|---|
| `gemma4:26b` ⭐ | ~16 GB | Default — fast frontier, native tool calling | 256K |
| `gemma4:31b` | ~19 GB | Maximum quality, complex reasoning | 256K |
| `qwen3-coder:30b` | ~18 GB | Coding specialist | 32K |
| `gemma4:e4b` | ~5 GB | Fast simple tasks, quick tool calls | 128K |
| `qwen3:8b` | ~5 GB | Good general purpose | 32K |
| `llama3.1:8b` | ~5 GB | Solid fallback | 128K |

On Apple M-series (64GB), `gemma4:26b` is the sweet spot — only 4B active parameters (MoE), so it's fast while delivering frontier-level code quality. On 16GB machines, `qwen3:8b` is recommended. On 8GB, `gemma4:e4b` works well.

## Pricing

7-day free trial. No credit card required.

| Plan | Price | What you get |
|---|---|---|
| Individual | $29/mo or $249/yr | All features, all models, CLI + Web UI + API |
| Team | $99/mo or $899/yr | 10 seats, RBAC, shared audit trail |
| Enterprise | Custom | Unlimited seats, SSO, custom policies, SLA |

```bash
# Check license status
tachy license

# Activate after purchase
tachy activate TACHY-<your-key>
```

## Architecture

```
tachy-cli          CLI binary (REPL, commands, markdown rendering)
├── daemon         HTTP server + agent execution engine
├── intelligence   Indexer, context, planner, edit-test-fix, verification
├── platform       Agent templates, scheduler, workspace config
├── backend        Multi-model: Ollama (Gemma 4, Qwen3, Llama, Mistral)
├── audit          Compliance: events, logging, governance, RBAC
├── runtime        Core: conversation loop, tools, permissions, sessions
└── tools          Tool definitions + execution (bash, files, search, git)
```

Built in Rust. Single binary. `unsafe_code = "forbid"`. Memory-safe by construction.

## License

Proprietary. Contact hello@tachy.dev for licensing.
