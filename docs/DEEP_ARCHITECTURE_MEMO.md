# TachyCode Deep Architecture Memo

## Executive Summary

TachyCode has evolved into a substantial local-first AI runtime platform whose center of gravity is the Rust workspace under `rust/crates/*`. The Python `src/` tree remains useful, but primarily as a porting, compatibility, and metadata layer rather than the main production runtime.

The clearest architectural truth in the current codebase is this:

- `daemon` is the control plane and API surface
- `runtime` is the execution substrate
- `audit` is the governance and trust layer
- `backend` is the model/provider abstraction layer
- `platform` is the templates/workspace/scheduling layer
- `intelligence` is the higher-order planning, indexing, verification, and dataset export layer
- `tools` is the action surface exposed to agents

This subsystem split is one of the strongest parts of the codebase and should be preserved.

## System Shape

At a high level, TachyCode operates as a local-first agent operating layer:

1. A user or client hits the HTTP daemon.
2. The daemon authenticates, rate-limits, and routes the request.
3. The platform layer selects an agent template and runtime config.
4. The backend registry resolves the best configured model backend.
5. The runtime executes the conversation and tool loop.
6. File mutations are evaluated by the policy engine before apply.
7. Every important action is written into an audit trail.
8. Higher-order intelligence services provide indexing, search, planning, verification, and dataset export.

This is materially different from a simple coding assistant wrapper. It is closer to a governed local agent platform.

## Subsystem Analysis

## 1. Daemon

Primary files:

- `rust/crates/daemon/src/lib.rs`
- `rust/crates/daemon/src/state.rs`
- `rust/crates/daemon/src/http/router.rs`

Responsibilities:

- bootstraps the platform
- holds shared mutable state
- exposes the HTTP API
- coordinates auth, teams, workers, webhooks, runs, governance, and intelligence endpoints

Strengths:

- The route surface is broad and product-like, not toy-like.
- `DaemonState` is the integration nexus for registry, policy engine, audit logger, scheduler, orchestrator, SSO/OAuth, teams, metering, marketplace, and event streams.
- Persistent state exists for agents, conversations, pending patches, and inference stats.

Risks:

- `DaemonState` is very large and is becoming a god object.
- The route layer currently mixes concerns that may become harder to test and evolve independently.

Recommendation:

- Preserve the daemon as the control plane, but gradually split domain services out of `state.rs` and HTTP handlers into narrower service modules.

## 2. Runtime

Primary files:

- `rust/crates/runtime/src/lib.rs`
- underlying modules in `runtime` for conversation, file ops, permissions, prompt building, config, locking, transactions

Responsibilities:

- conversation execution loop
- tool invocation
- file operations
- permission prompting
- session handling
- diff previews and edit transactions

Strengths:

- The runtime surface is concrete and practical.
- File operations, session compaction, usage tracking, and lock management are separated into dedicated modules.
- The public API of the crate is coherent and oriented around core runtime concepts.

Risks:

- Runtime policy and governance handoff boundaries must stay explicit so control logic does not drift into ad hoc checks.

Recommendation:

- Keep runtime focused on execution primitives.
- Avoid pushing platform-specific policy decisions directly into runtime helpers.

## 3. Audit and Governance

Primary files:

- `rust/crates/audit/src/policy_engine.rs`
- `rust/crates/audit/src/event.rs`
- `rust/crates/audit/src/rbac.rs`
- `rust/crates/audit/src/sso.rs`
- `rust/crates/audit/src/oauth.rs`

Responsibilities:

- patch governance
- audit event persistence and verification
- RBAC
- enterprise auth surfaces
- telemetry, metering, billing hooks

Strengths:

- The policy engine is real and useful today.
- The audit trail uses chained hashes and sequence numbers rather than plain text logs.
- RBAC is explicit and readable.
- This subsystem has the clearest enterprise posture in the repo.

Risks:

- Policy coverage still appears patch-centric. Broader policy actions may need the same formalism.
- There is a risk of governance fragmentation if some operations bypass this layer over time.

Recommendation:

- Expand policy evaluation from file patches to include:
  - agent launch policies
  - model change policies
  - marketplace install policies
  - worker registration and remote execution policies
  - cloud job submission policies

## 4. Backend Registry

Primary files:

- `rust/crates/backend/src/registry.rs`
- `rust/crates/backend/src/ollama.rs`
- `rust/crates/backend/src/openai_compat.rs`
- `rust/crates/backend/src/embeddings.rs`

Responsibilities:

- model registration
- backend routing
- model capability metadata
- client creation for inference backends

Strengths:

- The model registry is a strong abstraction point.
- The system is already oriented around local-first backends such as Ollama.
- Model tiers are a good primitive for routing decisions and workload shaping.

Risks:

- Default URLs are still localhost-biased.
- Backend selection logic may need richer health and capability probing as the system grows.

Recommendation:

- Keep this layer focused on capability discovery and client creation.
- Add health-aware routing and backend feature negotiation over time.

## 5. Platform

Primary files:

- `rust/crates/platform/src/agent.rs`
- related platform config, scheduler, and workspace modules

Responsibilities:

- agent templates
- workspace model
- schedules and tasks
- operator-facing reusable platform constructs

Strengths:

- Agent templates are one of the most productized parts of the codebase.
- The template model makes the system configurable without hardcoding per-flow logic everywhere.
- The workspace-first direction is correct.

Risks:

- Template sprawl can become difficult to govern if approval, tool policy, and model policy drift apart.

Recommendation:

- Introduce stronger template schema validation and policy inheritance.
- Keep templates as contracts, not just convenience presets.

## 6. Intelligence

Primary files:

- `rust/crates/intelligence/src/lib.rs`
- `rag.rs`
- `indexer/*`
- `planner.rs`
- `verification.rs`
- `edit_test_fix.rs`
- `finetune.rs`

Responsibilities:

- code indexing and retrieval
- planning
- verification
- edit-test-fix cycles
- monorepo/dependency intelligence
- session export for fine-tuning

Strengths:

- This crate has the right set of capabilities for an advanced local coding agent.
- The fine-tune dataset export is a particularly important bridge to external specialization systems.
- Code indexing and search are grounded in concrete types and APIs rather than just prompt stuffing.

Risks:

- This subsystem is broad and may have uneven maturity across modules.
- Its RAG and fine-tuning helpers are useful, but not yet as operationally mature as the governance/runtime layers.

Recommendation:

- Treat `intelligence` as an augmentation layer, not the control plane.
- Keep deeper managed training and enterprise knowledge lifecycle outside this repo unless they are core to Tachy itself.

## 7. Parallel and Swarm Execution

Primary files:

- `rust/crates/daemon/src/parallel.rs`
- `rust/crates/intelligence/src/swarm.rs`
- `rust/crates/intelligence/src/swarm_tools.rs`

Responsibilities:

- DAG execution
- dependency tracking
- concurrency management
- swarm-style task decomposition

Strengths:

- This is a meaningful differentiator.
- The system is not merely “multi-agent” in marketing language; it has concrete execution structures.

Risks:

- Swarm execution increases operational and governance complexity rapidly.
- Conflict detection, lock semantics, and rollback expectations need continued hardening.

Recommendation:

- Keep swarm execution governed by the same approval and audit model as single-agent work.
- Make semantic conflict handling and cancellation behavior first-class operator concerns.

## 8. Teams, SaaS, Marketplace, and Workers

Primary files:

- `rust/crates/daemon/src/teams.rs`
- `rust/crates/daemon/src/saas.rs`
- `rust/crates/daemon/src/marketplace.rs`
- `rust/crates/daemon/src/worker_registry.rs`

Responsibilities:

- team workspaces
- multi-tenant or SaaS abstractions
- installable marketplace assets
- distributed worker registration

Strengths:

- These modules show that the platform ambition is broader than a local terminal tool.
- Team membership and role handling are already explicit in code.

Risks:

- These surfaces are strategically important but expand the trust boundary significantly.
- They should be held to a higher standard than feature modules because they multiply security and operator complexity.

Recommendation:

- Treat these as controlled expansion zones and prioritize hardening, test coverage, and policy integration over rapid feature growth.

## 9. API, SDK, and VS Code Extension

Primary files:

- `rust/openapi.json`
- `sdk/python/tachy/client.py`
- `vscode-extension/src/extension.ts`

Responsibilities:

- external integration surface
- client SDK usage
- editor integration and operator visibility

Strengths:

- The platform already exposes a real external contract.
- The SDK and extension help prove the API is intended for reuse, not just internal coupling.

Risks:

- API sprawl can outpace internal service boundaries.
- Surface stability will matter more as external integrations deepen.

Recommendation:

- Version the API more deliberately as adoption increases.
- Define stable versus experimental routes.

## Python Layer Assessment

Primary files:

- `src/main.py`
- `src/runtime.py`
- `src/query_engine.py`
- `src/tools.py`

Assessment:

- The Python tree appears to function mainly as a compatibility, porting, and metadata harness.
- It should not be treated as the core architecture when making strategic decisions about the platform.

Recommendation:

- Document this distinction explicitly so new contributors do not mistake the Python layer for the primary runtime.

## Architectural Strengths

- Clear subsystem separation
- Strong governance posture
- Real auditability
- Local-first model routing
- Useful template-driven platform layer
- Meaningful parallel orchestration
- Practical API and client surfaces

## Architectural Risks

- Large central daemon state object
- Duplicate files and artifacts causing trust erosion
- Uneven maturity across subsystems
- Risk of governance bypass as features expand
- Potential overreach into too many platform directions at once

## Strategic Conclusion

TachyCode is best understood as a governed local agent operating layer. Its strongest comparative advantage is not just “coding with local models.” It is the combination of:

- local-first execution
- policy-mediated action
- auditability
- reusable agent templates
- orchestrated task execution

That makes it a strong execution and governance layer for a broader sovereign AI stack.
