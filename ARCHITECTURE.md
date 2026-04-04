# Tachy Architecture

Tachy is a local-first AI agent operating system. Single Rust binary, zero cloud dependency.

## Crate Structure

```
rust/crates/
├── tachy-cli       CLI binary (REPL, commands, rendering)
├── daemon          HTTP server, agent engine, web UI, MCP server, parallel execution
├── intelligence    Indexer, context, planner, edit-test-fix, LSP, memory, verification
├── platform        Agent templates, YAML agents, scheduler, workspace config
├── backend         Ollama + OpenAI-compatible backends, model registry, discovery
├── audit           Audit trail, license, RBAC, governance, security
├── runtime         Conversation loop, tools, permissions, sessions, diff engine
├── tools           Built-in + custom tool registry and execution
├── commands        Slash command handling
├── api             (legacy, unused)
└── compat-harness  (legacy, unused)
```

## Key Systems

- **Agent Engine**: plan-and-execute loop with model-specific prompt optimization
- **Parallel Execution**: DAG-based task scheduler with worker pool and file-level locking
- **Audit Trail**: SHA-256 hash chain, tamper-proof, survives across sessions
- **Custom Tools**: YAML-defined shell/HTTP tools with parameter substitution
- **MCP Server**: JSON-RPC 2.0 over stdio, compatible with Claude Desktop/Cursor
- **LSP Integration**: Language-aware diagnostics via tsc/pyright/cargo check/go vet
- **Diff Preview**: Unified diff generation before file writes
- **License System**: HMAC-SHA256 signed keys, 7-day trial, offline verification
