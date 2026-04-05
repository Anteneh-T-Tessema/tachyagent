# Implementation Plan: Product Hardening V3

## Overview

Implements usage-based billing metering, team workspaces, agent marketplace, hosted SaaS mode, and three hardening initiatives (E2E smoke tests, load tests, SSO pen-tests) for the Tachy AI Agent platform. Tasks are ordered by dependency: foundational modules first, then features that build on them, then HTTP wiring, then tests.

## Tasks

- [x] 1. Implement MeteringService (`audit/src/metering.rs`)
  - [x] 1.1 Create `UsageEvent`, `UsageEventType`, `UsageAggregate`, `MeteringError` types and `MeteringService` struct with `record_event`, `get_usage`, `get_team_usage`, and `drain_period` methods
    - Validate events: reject negative token counts and empty `user_id`
    - Persist events to `AuditLogger` with kind `"usage_metering"`
    - Maintain in-memory `BTreeMap<String, UsageAggregate>` counters keyed by user_id
    - Export the new module from `audit/src/lib.rs`
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.6_

  - [ ]* 1.2 Write property test for usage event field preservation
    - **Property 1: Usage event recording preserves all fields and produces audit entry**
    - **Validates: Requirements 1.1, 1.2, 1.3**
    - Test file: `rust/crates/audit/tests/metering_test.rs`

  - [ ]* 1.3 Write property test for usage counter consistency
    - **Property 2: Counter consistency**
    - **Validates: Requirements 1.4**
    - Test file: `rust/crates/audit/tests/metering_test.rs`

  - [ ]* 1.4 Write property test for invalid event rejection
    - **Property 3: Invalid usage events are rejected**
    - **Validates: Requirements 1.6**
    - Test file: `rust/crates/audit/tests/metering_test.rs`

- [x] 2. Implement StripeBillingConnector (`audit/src/billing.rs`)
  - [x] 2.1 Create `BillingBackend` trait, `StripeBillingConnector` struct, `BillingError`, `BillingReport`, `BillingStatus`, and `SubscriptionInfo` types
    - Implement `flush_period` that aggregates usage from `MeteringService` and reports to `BillingBackend` with retry (3x exponential backoff)
    - Implement `provision_user` to create Stripe customer and subscription via `BillingBackend`
    - Implement `status` returning current billing period, reported usage, and sync status
    - Support three metered dimensions: `tokens_consumed`, `tool_invocations`, `agent_runs`
    - Log failures to `AuditLogger`
    - Export from `audit/src/lib.rs`
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6_

  - [ ]* 2.2 Write property test for billing aggregation
    - **Property 4: Billing aggregation reports correct totals per user across all three dimensions**
    - **Validates: Requirements 2.1, 2.4**
    - Test file: `rust/crates/audit/tests/billing_test.rs`

- [x] 3. Checkpoint — Metering and billing core
  - Ensure all tests pass, ask the user if questions arise.

- [x] 4. Implement TeamManager (`daemon/src/teams.rs`)
  - [x] 4.1 Create `Team`, `TeamMember`, `WorkspaceInvitation`, `TeamError` types and `TeamManager` struct
    - Implement `create_team` (creator becomes Admin), `invite` (72h expiry token), `join` (validate token, reject expired/used), `update_member_role`, `remove_member`
    - Enforce last-admin invariant: reject removing or demoting the last Admin
    - Store team metadata (ID, name, created_at, member list) with serde Serialize/Deserialize
    - Export the new module from `daemon/src/lib.rs`
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6_

  - [ ]* 4.2 Write property test for team creation round-trip
    - **Property 5: Team creation and persistence round-trip**
    - **Validates: Requirements 3.1, 3.2**
    - Test file: `rust/crates/daemon/tests/teams_test.rs`

  - [ ]* 4.3 Write property test for invitation-join round-trip
    - **Property 6: Invitation-join round-trip preserves role**
    - **Validates: Requirements 3.3, 3.4**
    - Test file: `rust/crates/daemon/tests/teams_test.rs`

  - [ ]* 4.4 Write property test for expired/used invitation rejection
    - **Property 7: Expired or used invitations are rejected**
    - **Validates: Requirements 3.5**
    - Test file: `rust/crates/daemon/tests/teams_test.rs`

  - [ ]* 4.5 Write property test for last-admin invariant
    - **Property 8: Last-admin invariant**
    - **Validates: Requirements 3.6**
    - Test file: `rust/crates/daemon/tests/teams_test.rs`

- [ ] 5. Implement team-scoped RBAC extension (`audit/src/rbac.rs`)
  - [ ] 5.1 Add `check_team_permission(user_id, team_id, action, team_manager)` function to `audit/src/rbac.rs`
    - Look up user's role in the specific team via `TeamManager`
    - Deny with 403 reason if Viewer attempts to run an agent
    - Log role changes to `AuditLogger` with kind `"role_change"` including old role, new role, and admin ID
    - Export the new function from `audit/src/lib.rs`
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

  - [ ]* 5.2 Write property test for team-scoped permission isolation
    - **Property 9: Team-scoped permission isolation**
    - **Validates: Requirements 4.1, 4.5**
    - Test file: `rust/crates/daemon/tests/teams_test.rs`

  - [ ]* 5.3 Write property test for role changes auditing
    - **Property 10: Role changes are applied and audited**
    - **Validates: Requirements 4.3, 4.4**
    - Test file: `rust/crates/daemon/tests/teams_test.rs`

  - [ ]* 5.4 Write property test for agent-team association
    - **Property 11: Agent-team association**
    - **Validates: Requirements 5.1**
    - Test file: `rust/crates/daemon/tests/teams_test.rs`

- [ ] 6. Checkpoint — Teams and RBAC
  - Ensure all tests pass, ask the user if questions arise.

- [x] 7. Implement Marketplace (`daemon/src/marketplace.rs`)
  - [x] 7.1 Create `MarketplaceListing`, `MarketplaceVersion`, `MarketplaceError` types and `Marketplace` struct
    - Implement `publish` with semver validation (`^\d+\.\d+\.\d+$`), conflict detection (same name+version), version history (append-only, latest as default)
    - Implement `search` with optional query filter, pagination, sorted by `average_rating` descending
    - Implement `install` returning the `AgentTemplate` for the requested or default version, with missing tools detection
    - Implement `rate` with 1-5 integer validation, one rating per user (subsequent calls update), correct average calculation
    - Export from `daemon/src/lib.rs`
    - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 7.1, 7.2, 7.3, 7.4, 7.5_

  - [ ]* 7.2 Write property test for publish field preservation
    - **Property 12: Marketplace publish preserves all listing fields**
    - **Validates: Requirements 6.1, 6.3**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

  - [ ]* 7.3 Write property test for semver validation
    - **Property 13: Semver validation**
    - **Validates: Requirements 6.2**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

  - [ ]* 7.4 Write property test for version history append-only
    - **Property 14: Version history is append-only with latest as default**
    - **Validates: Requirements 6.4**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

  - [ ]* 7.5 Write property test for duplicate conflict detection
    - **Property 15: Duplicate name+version conflict detection**
    - **Validates: Requirements 6.5**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

  - [ ]* 7.6 Write property test for search sort order
    - **Property 16: Marketplace search results are sorted by rating descending**
    - **Validates: Requirements 7.1**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

  - [ ]* 7.7 Write property test for install round-trip
    - **Property 17: Marketplace install round-trip**
    - **Validates: Requirements 7.2**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

  - [ ]* 7.8 Write property test for rating average correctness
    - **Property 18: Rating average correctness with idempotent per-user updates**
    - **Validates: Requirements 7.3, 7.4**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

  - [ ]* 7.9 Write property test for missing tools detection
    - **Property 19: Missing tools detection on install**
    - **Validates: Requirements 7.5**
    - Test file: `rust/crates/daemon/tests/marketplace_test.rs`

- [x] 8. Implement SaaSPlatform (`daemon/src/saas.rs`)
  - [x] 8.1 Create `Tenant`, `ResourceLimits`, `SaaSPlatform`, `SaaSError`, `TenantClaims`, `DashboardSummary` types
    - Implement `signup` creating tenant with dedicated workspace dir, default config, managed Ollama endpoint
    - Implement `authenticate` with email/password returning JWT (configurable expiry, default 24h)
    - Implement `validate_jwt` verifying token and returning `TenantClaims`
    - Implement `check_limits` enforcing per-tenant resource limits (max concurrent agents, max tokens/day, max storage), returning 429 when exceeded
    - Implement `dashboard` returning tenant usage summary
    - Handle Ollama unreachable with 503 + Retry-After
    - Export from `daemon/src/lib.rs`
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6_

  - [ ]* 8.2 Write property test for tenant data isolation
    - **Property 20: Tenant data isolation**
    - **Validates: Requirements 8.1**
    - Test file: `rust/crates/daemon/tests/saas_test.rs`

  - [ ]* 8.3 Write property test for tenant signup resource creation
    - **Property 21: Tenant signup creates all required resources**
    - **Validates: Requirements 8.2**
    - Test file: `rust/crates/daemon/tests/saas_test.rs`

  - [ ]* 8.4 Write property test for JWT authentication round-trip
    - **Property 22: JWT authentication round-trip**
    - **Validates: Requirements 8.3**
    - Test file: `rust/crates/daemon/tests/saas_test.rs`

  - [ ]* 8.5 Write property test for resource limit enforcement
    - **Property 23: Resource limit enforcement**
    - **Validates: Requirements 8.5**
    - Test file: `rust/crates/daemon/tests/saas_test.rs`

- [x] 9. Checkpoint — Marketplace and SaaS core
  - Ensure all tests pass, ask the user if questions arise.

- [ ] 10. Extend DaemonState (`daemon/src/state.rs`)
  - [ ] 10.1 Add `metering`, `billing`, `team_manager`, `marketplace`, and `saas` fields to `DaemonState`
    - `metering: MeteringService` — always present
    - `billing: Option<StripeBillingConnector>` — initialized only when Stripe API key is configured
    - `team_manager: TeamManager` — always present
    - `marketplace: Marketplace` — always present
    - `saas: Option<SaaSPlatform>` — initialized only when SaaS mode is configured
    - Update `DaemonState::init` to construct and wire the new fields
    - Update `DaemonState::save` to persist team and marketplace state
    - Add `proptest` dev-dependency to `daemon/Cargo.toml` and `audit/Cargo.toml`
    - _Requirements: 1.1, 2.1, 3.2, 6.3, 8.1_

- [ ] 11. Implement HTTP API extensions (`daemon/src/http.rs`)
  - [ ] 11.1 Add usage and billing endpoints
    - `GET /api/usage` → `handle_usage` returning per-user and per-team totals for a time range
    - `GET /api/billing/status` → `handle_billing_status` returning billing period, reported usage, Stripe sync status
    - _Requirements: 1.5, 2.6_

  - [ ] 11.2 Add team management endpoints
    - `POST /api/teams` → `handle_create_team`
    - `POST /api/teams/:id/invite` → `handle_invite`
    - `POST /api/teams/join` → `handle_join_team`
    - `PUT /api/teams/:id/members/:uid` → `handle_update_member`
    - _Requirements: 3.1, 3.3, 3.4, 4.3_

  - [ ] 11.3 Add team shared resource endpoints
    - `GET /api/teams/:id/agents` → `handle_team_agents`
    - `GET /api/teams/:id/audit` → `handle_team_audit`
    - `GET /api/teams/:id/policy` → `handle_team_policy`
    - All endpoints scoped to team via `check_team_permission`
    - _Requirements: 5.1, 5.2, 5.3, 5.4_

  - [ ] 11.4 Add marketplace endpoints
    - `POST /api/marketplace/publish` → `handle_publish`
    - `GET /api/marketplace` → `handle_marketplace_list` with pagination and search
    - `POST /api/marketplace/install` → `handle_install`
    - `POST /api/marketplace/:id/rate` → `handle_rate`
    - _Requirements: 6.1, 7.1, 7.2, 7.3_

  - [ ] 11.5 Add SaaS dashboard endpoint
    - `GET /api/dashboard` → `handle_dashboard` returning tenant usage summary
    - JWT validation middleware for SaaS endpoints
    - 503 with Retry-After when Ollama is unreachable
    - _Requirements: 8.4, 8.5, 8.6_

  - [ ] 11.6 Wire route matching in `handle_request` for all 14 new endpoints
    - Add path matching and method dispatch for each new route
    - Return proper HTTP status codes and error responses per the design error handling table
    - _Requirements: 1.5, 2.6, 3.1, 3.3, 3.4, 4.3, 5.4, 6.1, 7.1, 7.2, 7.3, 8.4_

- [ ] 12. Checkpoint — HTTP API wiring
  - Ensure all tests pass, ask the user if questions arise.

- [ ] 13. Implement SSO pen tests (`audit/tests/sso_pen_test.rs`)
  - [ ] 13.1 Create SSO pen test file with security audit tests
    - Test XML entity expansion (billion laughs) rejection without excessive memory allocation
    - Test `<script>` tag in NameID does not propagate unescaped
    - Test forged Issuer rejection with "issuer mismatch" error
    - Test session token replay after `invalidate_session` returns no valid session
    - Test NameID with null bytes, control characters, strings > 1024 chars handled without panic
    - Test expired session rejection by `validate_session`
    - Test base64 decoder rejects non-base64 characters and handles truncated input
    - Test deeply nested XML (depth > 100) without stack overflow
    - Test CDATA sections wrapping NameID
    - _Requirements: 11.1, 11.2, 11.3, 11.4, 11.5, 11.6, 11.7, 11.8, 11.10_

  - [ ]* 13.2 Write property test for malicious NameID handling
    - **Property 28: Malicious NameID content is handled safely**
    - **Validates: Requirements 11.2, 11.5**
    - Test file: `rust/crates/audit/tests/sso_pen_test.rs`

  - [ ]* 13.3 Write property test for forged issuer rejection
    - **Property 29: Forged issuer rejection**
    - **Validates: Requirements 11.3**
    - Test file: `rust/crates/audit/tests/sso_pen_test.rs`

  - [ ]* 13.4 Write property test for session token replay
    - **Property 30: Session token replay after invalidation**
    - **Validates: Requirements 11.4**
    - Test file: `rust/crates/audit/tests/sso_pen_test.rs`

  - [ ]* 13.5 Write property test for expired session rejection
    - **Property 31: Expired session rejection**
    - **Validates: Requirements 11.6**
    - Test file: `rust/crates/audit/tests/sso_pen_test.rs`

  - [ ]* 13.6 Write property test for base64 decoder robustness
    - **Property 32: Base64 decoder rejects invalid input**
    - **Validates: Requirements 11.7**
    - Test file: `rust/crates/audit/tests/sso_pen_test.rs`

  - [ ]* 13.7 Write property test for SAML assertion round-trip
    - **Property 33: SAML assertion round-trip**
    - **Validates: Requirements 11.9**
    - Test file: `rust/crates/audit/tests/sso_pen_test.rs`

- [ ] 14. Implement file lock property tests (`runtime/tests/filelock_prop_test.rs`)
  - [ ]* 14.1 Write property test for file lock mutual exclusion
    - **Property 25: File lock mutual exclusion under contention**
    - **Validates: Requirements 10.3**
    - Add `proptest` dev-dependency to `runtime/Cargo.toml`
    - Test file: `rust/crates/runtime/tests/filelock_prop_test.rs`

  - [ ]* 14.2 Write property test for file lock TTL expiry
    - **Property 26: File lock TTL expiry**
    - **Validates: Requirements 10.5**
    - Test file: `rust/crates/runtime/tests/filelock_prop_test.rs`

  - [ ]* 14.3 Write property test for release_all selectivity
    - **Property 27: release_all is selective**
    - **Validates: Requirements 10.6**
    - Test file: `rust/crates/runtime/tests/filelock_prop_test.rs`

- [ ] 15. Checkpoint — Pen tests and property tests
  - Ensure all tests pass, ask the user if questions arise.

- [ ] 16. Extend E2E smoke tests (`daemon/tests/e2e_smoke.rs`)
  - [ ] 16.1 Add E2E smoke tests for full agent pipeline
    - Add test: agent with "chat" template reads a file and references its content
    - Add test: agent with "code-reviewer" template produces non-empty review summary
    - Add test: agent creates, reads, and modifies a file on disk (tool use exercise)
    - Add test: parallel execution with two independent tasks both complete with "Completed" status
    - Add test: every agent run produces at least one audit event with valid hash chain
    - Skip all tests with descriptive message if Ollama is not reachable
    - All tests must complete within 300 seconds with 8B model or smaller
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5, 9.6, 9.7_

  - [ ]* 16.2 Write property test for DAG dependency order
    - **Property 24: DAG execution respects dependency order**
    - **Validates: Requirements 10.2**
    - Test file: `rust/crates/daemon/tests/load_test.rs`

- [ ] 17. Implement load tests (`daemon/tests/load_test.rs`)
  - [ ] 17.1 Create load test file with parallel executor stress tests
    - Test: 20 independent tasks at max concurrency 8, all reach terminal status within 60s (mock agent execution)
    - Test: deep dependency chain (10 sequential tasks), verify strict dependency order
    - Test: 16 concurrent threads acquiring locks on 4 shared files, verify mutual exclusion
    - Test: throughput measurement (tasks/sec) and p50/p95 latency for `next_task`/`complete_task` under 100-task workload
    - Test: FileLockManager TTL-based expiry releases stale locks after configured TTL under concurrent access
    - Test: `release_all(agent_id)` releases only that agent's locks while other agents' locks remain intact, under 8 concurrent agents
    - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6_

- [ ] 18. Final checkpoint — All tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests use `proptest` with minimum 100 cases per property
- All property tests include comment: `// Feature: product-hardening-v3, Property N: <title>`
- E2E smoke tests are `#[ignore]` and require a running Ollama instance
- Load tests use mock agent execution for deterministic timing
