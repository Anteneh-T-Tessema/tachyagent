# Tachy Roadmap: Operational Excellence & Sovereign Feedback Loops

This document outlines the strategic technical direction for Tachy following the "Wave 2" architectural hardening. The focus is on transitioning from a single-user runtime to a production-hardened, multi-tenant, and autonomous platform.

## Phase 1: Operational Trust & Hardened Multi-Tenancy

**Goal**: Ensure the platform is safe for enterprise deployment with multiple teams and shared resources.

- [ ] **Team-Bound Persistence**: Update `ParallelRun` and `AgentTask` schemas to include a `team_id`. Partition state storage by team to ensure strict data isolation.
- [ ] **Quota & Budget Enforcement**:
  - Integrate `MeteringService` with the `Orchestrator`.
  - Add `max_cost_usd` and `max_tokens` constraints to parallel runs.
  - Automatically suspend runs that exceed team or run-level budgets.
- [ ] **Audit Alerting & Webhooks**:
  - Add a "Subscription" model to the `AuditLogger`.
  - Trigger external webhooks or Slack alerts for `Critical` severity events or `GovernanceViolation` kinds.
- [ ] **Secrets Hygiene**:
  - Move from environment-only secrets to an encrypted `SecretStore` service.
  - Implement "Secret Masking" in the `AuditLogger` to prevent accidental leakage of keys in logs.

## Phase 2: Sovereign AI Feedback Loops (The "Intelligence" Layer)

**Goal**: Enable the platform to autonomously improve its own performance through session data.

- [ ] **Gold Standard Dataset Extraction**:
  - Implement a "Quality Scorer" that evaluates completed sessions.
  - Automatically export "Gold Standard" turns (high-confidence success, no human correction) to a persistent training dataset.
- [ ] **Expert Adapter Registry**:
  - Create a registry for model adapters (LoRAs/GGUF adapters).
  - Update `AgentTemplate` to allow specifying a required adapter for specific task types (e.g., "Use the 'Tachy-Rust-Expert' adapter for coding tasks").
- [ ] **Closed-Loop Fine-tuning Bridge**:
  - Provide a dedicated worker service that can pull from the candidate dataset and trigger local or remote training jobs.
  - Implement "A/B Testing" for models: run a task twice with different adapters and compare result quality.

## Phase 3: Advanced Orchestration & DAG Maturity

**Goal**: Support complex, enterprise-grade workflows with human oversight.

- [ ] **Conditional DAG Execution**:
  - Extend `AgentTask` with `conditions`: `if_success`, `if_failure`, or `if_match: "pattern"`.
  - Allow the `Orchestrator` to skip or branch the execution graph based on task results.
- [ ] **Human-in-the-Loop (HITL) Integration**:
  - Implement a `Suspended` task status.
  - Add a `ManualApprovalTask` type that waits for a signed POST to `/api/approve/{task_id}` before the DAG proceeds.
- [ ] **Recursive Self-Correction**:
  - Implement "Loop" support in DAGs: allow a task to be re-run up to N times if a "Verifier" agent identifies errors.

## Phase 4: Marketplace & Ecosystem

**Goal**: Enable third-party extensions and cross-platform portability.

- [ ] **Template Schema Formalization**:
  - Move from ad-hoc JSON templates to a versioned, schema-validated DSL.
  - Support "Template Inheritance" to allow teams to build on top of canonical base templates.
- [ ] **Plugin/Tool Verification**:
  - Implement a signing system for tools/MCP servers.
  - Only allow "Verified" tools from the marketplace to run without explicit per-task approval.

---
*Last Updated: 2026-04-22*
