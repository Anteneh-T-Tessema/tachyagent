# Requirements Document

## Introduction

Product Hardening V3 covers four product features and three hardening initiatives for the Tachy AI Agent platform. The product features add usage-based billing metering, team workspace sharing, an agent marketplace, and a hosted SaaS deployment mode. The hardening initiatives add end-to-end smoke tests against real Ollama, load testing for the parallel executor, and SSO pen-testing of the SAML parser.

## Glossary

- **Metering_Service**: The subsystem responsible for tracking, aggregating, and reporting usage events (token consumption, tool invocations, agent runs) per user and team.
- **Usage_Event**: A single recorded unit of consumption — one of: token usage, tool invocation, or agent run completion.
- **Stripe_Billing_Connector**: The integration layer that synchronizes metered usage events with Stripe's metered billing API for invoicing.
- **Audit_Logger**: The existing append-only JSONL audit trail system with SHA-256 hash chain integrity.
- **Team_Workspace**: A shared workspace where multiple users collaborate with shared agents, audit trail, and governance policies under team-level RBAC.
- **Workspace_Invitation**: A mechanism for an Admin to invite users to join a Team_Workspace via email or token.
- **RBAC_Engine**: The existing role-based access control system with Viewer, Developer, and Admin roles, extended for team-scoped permissions.
- **Marketplace**: The subsystem for publishing, discovering, installing, and rating agent templates.
- **Marketplace_Listing**: A published agent template with metadata (name, description, version, author, rating).
- **SaaS_Platform**: The multi-tenant hosted cloud deployment of Tachy with user authentication, workspace isolation, and managed infrastructure.
- **Tenant**: An isolated organizational unit in the SaaS_Platform, each with its own workspace, users, and data.
- **E2E_Smoke_Test_Suite**: Integration tests that execute the full agent pipeline against a running Ollama instance.
- **Load_Test_Harness**: A stress-testing framework for the parallel executor, DAG scheduler, and file locking subsystem.
- **SSO_Pen_Test_Suite**: Security audit tests targeting the SAML 2.0 parser, session management, and authentication flow.
- **SAML_Parser**: The existing lightweight XML parser in `audit/src/sso.rs` that extracts assertions from SAML responses.
- **Orchestrator**: The existing parallel execution engine in `daemon/src/parallel.rs` that schedules and executes DAGs of agent tasks.
- **FileLockManager**: The existing cooperative file-level locking system in `runtime/src/filelock.rs`.

---

## Requirements

### Requirement 1: Usage Event Recording

**User Story:** As a platform operator, I want every token consumption, tool invocation, and agent run to be recorded as a usage event, so that I can meter and bill users accurately.

#### Acceptance Criteria

1. WHEN an agent run completes, THE Metering_Service SHALL record a Usage_Event containing the user ID, team ID, agent ID, model name, input token count, output token count, tool invocation count, and a UTC timestamp.
2. WHEN a tool is invoked during an agent run, THE Metering_Service SHALL record a Usage_Event of type "tool_invocation" containing the tool name, agent ID, user ID, and UTC timestamp.
3. THE Metering_Service SHALL persist all Usage_Events to the Audit_Logger as audit events with kind "usage_metering".
4. WHEN a Usage_Event is recorded, THE Metering_Service SHALL increment the cumulative usage counters for the associated user and team in memory.
5. THE Metering_Service SHALL expose cumulative usage data via `GET /api/usage` returning per-user and per-team totals for token consumption, tool invocations, and agent runs within a specified time range.
6. IF a Usage_Event contains a negative token count or missing user ID, THEN THE Metering_Service SHALL reject the event and log a warning to the Audit_Logger.


### Requirement 2: Stripe Metered Billing Integration

**User Story:** As a platform operator, I want metered usage to be reported to Stripe automatically, so that users are invoiced based on actual consumption.

#### Acceptance Criteria

1. WHEN a billing period ends (configurable, default: hourly), THE Stripe_Billing_Connector SHALL aggregate all Usage_Events for each user and report the totals to the Stripe Usage Records API.
2. THE Stripe_Billing_Connector SHALL map each user to a Stripe subscription item ID using a persistent mapping stored in the workspace configuration.
3. IF the Stripe API returns an error during usage reporting, THEN THE Stripe_Billing_Connector SHALL retry the report up to 3 times with exponential backoff and log each failure to the Audit_Logger.
4. THE Stripe_Billing_Connector SHALL support three metered dimensions: total tokens consumed, total tool invocations, and total agent runs.
5. WHEN a new user is provisioned (via SSO or manual creation), THE Stripe_Billing_Connector SHALL create a Stripe customer and subscription if a Stripe API key is configured.
6. THE Stripe_Billing_Connector SHALL expose a `GET /api/billing/status` endpoint returning the current billing period, reported usage, and Stripe sync status.

### Requirement 3: Team Workspace Creation and Membership

**User Story:** As an Admin, I want to create a team workspace and invite members, so that multiple users can collaborate on shared agents and audit trails.

#### Acceptance Criteria

1. WHEN an Admin sends a `POST /api/teams` request with a team name, THE Team_Workspace SHALL create a new team with the requesting user as the team Admin.
2. THE Team_Workspace SHALL store team metadata (ID, name, created_at, member list) in the workspace state file.
3. WHEN an Admin sends a `POST /api/teams/:id/invite` request with an email address and role, THE Team_Workspace SHALL generate a Workspace_Invitation token valid for 72 hours.
4. WHEN a user accepts a Workspace_Invitation via `POST /api/teams/join`, THE Team_Workspace SHALL add the user to the team with the role specified in the invitation.
5. IF a Workspace_Invitation token is expired or already used, THEN THE Team_Workspace SHALL reject the join request with a descriptive error.
6. THE Team_Workspace SHALL enforce that each team has at least one Admin at all times; removing the last Admin SHALL be rejected.

### Requirement 4: Team-Level RBAC

**User Story:** As a team Admin, I want to control what each team member can do, so that governance policies are enforced at the team level.

#### Acceptance Criteria

1. THE RBAC_Engine SHALL scope all permission checks to the team context, so that a Developer in Team A cannot access agents or audit logs belonging to Team B.
2. WHEN a Viewer attempts to run an agent within a Team_Workspace, THE RBAC_Engine SHALL deny the request and return a 403 response with the reason.
3. WHEN an Admin sends a `PUT /api/teams/:id/members/:user_id` request with a new role, THE RBAC_Engine SHALL update the user's role within that team.
4. THE RBAC_Engine SHALL log all role changes to the Audit_Logger with kind "role_change" including the old role, new role, and the Admin who made the change.
5. WHILE a user is a member of multiple teams, THE RBAC_Engine SHALL evaluate permissions independently for each team based on the user's role in that specific team.


### Requirement 5: Team Shared Resources

**User Story:** As a team member, I want to access shared agents, audit trails, and governance policies within my team workspace, so that the team operates with a unified view.

#### Acceptance Criteria

1. WHEN an agent is created within a Team_Workspace, THE Team_Workspace SHALL associate the agent with the team ID and make the agent visible to all team members.
2. THE Team_Workspace SHALL maintain a single shared audit trail per team, containing all agent runs, tool invocations, and governance events from all team members.
3. WHEN a team Admin updates the governance policy, THE Team_Workspace SHALL apply the updated policy to all subsequent agent runs within that team.
4. THE Team_Workspace SHALL expose `GET /api/teams/:id/agents`, `GET /api/teams/:id/audit`, and `GET /api/teams/:id/policy` endpoints scoped to the team.

### Requirement 6: Marketplace Publishing

**User Story:** As an agent author, I want to publish my agent template to the marketplace, so that other users can discover and install it.

#### Acceptance Criteria

1. WHEN a user sends a `POST /api/marketplace/publish` request with an agent template, description, and version string, THE Marketplace SHALL create a Marketplace_Listing with status "published".
2. THE Marketplace SHALL validate that the version string follows semantic versioning (MAJOR.MINOR.PATCH) and reject non-compliant versions.
3. THE Marketplace SHALL store the complete AgentTemplate definition, author ID, description, version history, and publication timestamp for each listing.
4. WHEN a user publishes a new version of an existing listing, THE Marketplace SHALL retain all previous versions and set the new version as the default.
5. IF a listing with the same name and version already exists, THEN THE Marketplace SHALL reject the publish request with a conflict error.

### Requirement 7: Marketplace Discovery and Installation

**User Story:** As a user, I want to browse and install agent templates from the marketplace, so that I can use community-built agents in my workspace.

#### Acceptance Criteria

1. THE Marketplace SHALL expose a `GET /api/marketplace` endpoint returning a paginated list of Marketplace_Listings sorted by rating (descending) with optional search by name or tag.
2. WHEN a user sends a `POST /api/marketplace/install` request with a listing ID and optional version, THE Marketplace SHALL copy the AgentTemplate into the user's workspace configuration.
3. WHEN a user sends a `POST /api/marketplace/:id/rate` request with a rating (1-5 integer), THE Marketplace SHALL update the listing's average rating.
4. THE Marketplace SHALL prevent a user from rating the same listing more than once; subsequent ratings SHALL update the existing rating.
5. WHEN installing a template, THE Marketplace SHALL verify that the template's required tools are available in the target workspace and warn the user of any missing tools.

### Requirement 8: SaaS Multi-Tenant Deployment

**User Story:** As a cloud user, I want to use Tachy as a hosted service with my own isolated workspace, so that I do not need to manage local infrastructure.

#### Acceptance Criteria

1. THE SaaS_Platform SHALL isolate each Tenant's workspace data, agents, audit trail, and configuration so that no Tenant can access another Tenant's data.
2. WHEN a new user signs up, THE SaaS_Platform SHALL create a Tenant with a dedicated workspace directory, default configuration, and a managed Ollama instance endpoint.
3. THE SaaS_Platform SHALL authenticate users via email/password or SSO and issue JWT session tokens with a configurable expiry (default: 24 hours).
4. THE SaaS_Platform SHALL expose a `GET /api/dashboard` endpoint returning the Tenant's usage summary (agent runs, token consumption, active team members, billing status).
5. THE SaaS_Platform SHALL enforce per-Tenant resource limits (configurable: max concurrent agents, max tokens per day, max storage) and return 429 responses when limits are exceeded.
6. IF a managed Ollama instance becomes unreachable, THEN THE SaaS_Platform SHALL return a 503 response with a retry-after header and log the outage to the Tenant's audit trail.


### Requirement 9: E2E Smoke Tests Against Real Ollama

**User Story:** As a developer, I want end-to-end smoke tests that exercise the full agent pipeline against a running Ollama model, so that I can verify tool use and agent execution work correctly before releases.

#### Acceptance Criteria

1. WHEN the E2E_Smoke_Test_Suite is executed with a running Ollama instance, THE E2E_Smoke_Test_Suite SHALL run an agent with the "chat" template, provide a prompt that requires reading a file, and verify the agent's response references the file content.
2. THE E2E_Smoke_Test_Suite SHALL include a test that runs an agent with the "code-reviewer" template against a sample source file and verifies the agent produces a non-empty review summary.
3. THE E2E_Smoke_Test_Suite SHALL include a test that exercises tool use by creating a test file, prompting the agent to read and modify the file, and verifying the file was modified on disk.
4. THE E2E_Smoke_Test_Suite SHALL include a test that runs a parallel execution with two independent tasks and verifies both tasks complete with status "Completed".
5. THE E2E_Smoke_Test_Suite SHALL verify that every agent run produces at least one audit event in the audit trail with a valid hash chain.
6. IF Ollama is not reachable, THEN THE E2E_Smoke_Test_Suite SHALL skip all tests with a descriptive message rather than failing.
7. THE E2E_Smoke_Test_Suite SHALL complete all tests within 300 seconds when using a model with 8B parameters or fewer.

### Requirement 10: Load Testing for the Parallel Executor

**User Story:** As a developer, I want load tests that stress the DAG scheduler, file locking, and concurrent agent execution, so that I can measure throughput, identify bottlenecks, and verify correctness under contention.

#### Acceptance Criteria

1. THE Load_Test_Harness SHALL submit a parallel run with 20 independent tasks at max concurrency 8 and verify all 20 tasks reach a terminal status (Completed or Failed) within 60 seconds using mock agent execution.
2. THE Load_Test_Harness SHALL submit a parallel run with a deep dependency chain (10 sequential tasks where each depends on the previous) and verify tasks execute in strict dependency order.
3. THE Load_Test_Harness SHALL spawn 16 concurrent threads each attempting to acquire file locks on a shared set of 4 files and verify that no two threads hold the same file lock simultaneously.
4. THE Load_Test_Harness SHALL measure and report the throughput (tasks per second) and p50/p95 latency for the Orchestrator's `next_task()` and `complete_task()` operations under a workload of 100 tasks.
5. THE Load_Test_Harness SHALL verify that the FileLockManager's TTL-based expiry correctly releases stale locks after the configured TTL (5 minutes) under concurrent access.
6. THE Load_Test_Harness SHALL verify that `release_all(agent_id)` correctly releases all locks held by a specific agent while other agents' locks remain intact, under concurrent access from 8 agents.

### Requirement 11: SSO Pen-Testing of the SAML Parser

**User Story:** As a security engineer, I want pen-tests that audit the SAML parser for injection attacks, signature bypass, session fixation, token replay, and IdP impersonation, so that I can verify the SSO implementation is secure.

#### Acceptance Criteria

1. THE SSO_Pen_Test_Suite SHALL test that a SAMLResponse containing XML entity expansion (billion laughs attack) is rejected by the SAML_Parser without excessive memory allocation.
2. THE SSO_Pen_Test_Suite SHALL test that a SAMLResponse with a `<script>` tag injected into the NameID field does not propagate the script content into the session or user store unescaped.
3. THE SSO_Pen_Test_Suite SHALL test that a SAMLResponse with a forged Issuer value (not matching the configured `idp_entity_id`) is rejected with an "issuer mismatch" error.
4. THE SSO_Pen_Test_Suite SHALL test that replaying a previously valid session token after invalidation via `invalidate_session()` returns no valid session.
5. THE SSO_Pen_Test_Suite SHALL test that a SAMLResponse with a NameID containing null bytes, control characters, or strings exceeding 1024 characters is handled without panic or memory corruption.
6. THE SSO_Pen_Test_Suite SHALL test that an expired SSO session (created_at + session_duration_secs < now) is rejected by `validate_session()`.
7. THE SSO_Pen_Test_Suite SHALL test that the base64 decoder rejects payloads containing non-base64 characters and does not produce incorrect output for truncated input.
8. THE SSO_Pen_Test_Suite SHALL test that the SAML_Parser correctly handles XML with deeply nested elements (depth > 100) without stack overflow or excessive processing time.
9. FOR ALL valid SamlAssertion objects, encoding the assertion to XML then parsing the XML back SHALL produce an equivalent SamlAssertion (round-trip property for the SAML parser/printer).
10. THE SSO_Pen_Test_Suite SHALL test that a SAMLResponse with CDATA sections wrapping the NameID value is parsed correctly or rejected explicitly.
