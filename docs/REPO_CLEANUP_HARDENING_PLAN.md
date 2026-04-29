# TachyCode Repo Cleanup and Hardening Plan

## Purpose

This document defines a focused cleanup and hardening plan for TachyCode itself. The goal is not cosmetic tidiness. The goal is to increase trust, reduce contributor confusion, improve release reliability, and harden the platform around its strongest capabilities.

## Current Assessment

The repo contains strong code, especially in the Rust workspace, but it also contains signals that reduce confidence:

- duplicate markdown files such as `ARCHITECTURE 2.md`
- duplicate cargo manifests such as `Cargo 2.toml`
- duplicate-looking source directories such as `src 2`
- uneven documentation quality
- unclear boundary between primary runtime and compatibility layers

The result is that the best parts of the repo are stronger than the repo hygiene suggests.

## Objectives

1. Make the canonical architecture obvious.
2. Remove or quarantine duplicate artifacts.
3. Raise trust in the governance/runtime core.
4. Make release-critical surfaces easier to audit.
5. Document which subsystems are production-grade versus experimental.

## Guiding Rules

1. Prefer archival quarantine over deletion when provenance is unclear.
2. Preserve the Rust workspace as the canonical runtime.
3. Do not let docs imply more maturity than the code supports.
4. Harden trust boundaries before adding more platform breadth.

## Workstreams

## Workstream 1: Repo Canonicalization

### Problems

- duplicated files create ambiguity
- contributors cannot easily tell what is canonical

### Tasks

- inventory all duplicate and shadow files
- identify canonical files for:
  - architecture docs
  - cargo manifests
  - source directories
  - product docs
- move superseded artifacts into an explicit archive area such as `archive/legacy/`
- add a root-level `REPO_MAP.md` describing canonical locations

### Success Criteria

- no ambiguous top-level duplicate files remain in active locations
- every major subsystem has one clearly canonical doc entrypoint

## Workstream 2: Runtime and Daemon Hardening

### Problems

- `DaemonState` is large and central
- route handling mixes too many concerns

### Tasks

- document `DaemonState` ownership boundaries
- split daemon services into narrower domains:
  - auth/governance
  - conversations
  - orchestration
  - integrations
  - intelligence endpoints
- add focused unit tests around service boundaries
- define experimental versus stable HTTP routes

### Success Criteria

- lower coupling in daemon handlers
- clearer service ownership and improved testability

## Workstream 3: Governance Hardening

### Problems

- policy engine is strong but patch-centric
- risk of governance bypass as new features land

### Tasks

- expand policy scope beyond file patch application
- define policy checks for:
  - agent launch
  - model selection overrides
  - marketplace install
  - remote worker registration
  - cloud job submission
- ensure all approval-required operations emit explicit audit events
- add regression tests for policy-sensitive paths

### Success Criteria

- all high-risk operations traverse a common governance path
- audit trail reflects policy decisions, not just outcomes

## Workstream 4: Audit Integrity and Operational Trust

### Problems

- hash-chained audit is strong, but operator workflows around it should be clearer

### Tasks

- document the audit model in a dedicated `docs/AUDIT_MODEL.md`
- add a CLI or admin endpoint for verifying the audit chain on demand
- define retention and export behavior
- classify which payloads are redacted and why

### Success Criteria

- operators can verify chain integrity easily
- audit guarantees are documented and testable

## Workstream 5: Platform and Template Governance

### Problems

- templates are powerful but could drift without schema discipline

### Tasks

- formalize template schema validation
- document template lifecycle:
  - draft
  - approved
  - deprecated
- align tool permissions and approval requirements with template metadata
- create a small set of canonical built-in templates

### Success Criteria

- templates behave like governed contracts rather than ad hoc presets

## Workstream 6: Intelligence Layer Positioning

### Problems

- intelligence modules are broad and unevenly mature
- repo messaging may blur “useful” with “production-hardened”

### Tasks

- classify intelligence modules as:
  - stable
  - beta
  - experimental
- document the scope of:
  - RAG
  - finetune export
  - planner
  - verification
  - swarm tools
- avoid overstating enterprise readiness where implementation is still early

### Success Criteria

- contributors and customers can tell which capabilities are mature

## Workstream 7: API and Surface Stabilization

### Problems

- broad API surface may grow faster than version discipline

### Tasks

- generate a canonical API reference from the OpenAPI spec
- mark routes as stable or experimental
- add compatibility expectations for SDK consumers
- add smoke coverage for the most important API flows

### Success Criteria

- integrations can rely on a stable core contract

## Workstream 8: Security and Secrets Hygiene

### Problems

- local-first systems still need strict operator trust boundaries

### Tasks

- review default localhost and credential assumptions
- document secrets sources and required env vars
- ensure no sample configs encourage insecure defaults
- test policy rules against secret leakage scenarios

### Success Criteria

- install and deployment defaults are safer and more explicit

## Workstream 9: Testing and CI Strategy

### Problems

- the repo contains strong tests in places, but the critical-path trust story should be clearer

### Tasks

- define a tiered test strategy:
  - unit
  - integration
  - smoke
  - e2e with local model
- require critical subsystem coverage for:
  - audit
  - policy
  - daemon routing
  - parallel orchestration
  - backend registry
- add a small release checklist tied to these suites

### Success Criteria

- release confidence maps to critical trust boundaries, not just general code coverage

## Priority Order

## Now

- canonicalize duplicate files
- document the repo map
- harden governance entrypoints
- clarify stable versus experimental surfaces

## Next

- split daemon responsibilities
- strengthen audit verification workflows
- formalize template governance
- tighten API stability discipline

## Later

- deeper multi-tenant and marketplace hardening
- broader distributed worker hardening
- more advanced release automation

## Suggested Immediate Execution Backlog

1. Create `REPO_MAP.md` and identify canonical docs and entrypoints.
2. Move duplicate top-level docs and `Cargo 2.toml` artifacts into an archive folder.
3. Add a “canonical runtime” note to the root README explaining that Rust is the primary runtime and Python is compatibility/porting support.
4. Add stable/experimental markers to the API doc.
5. Add governance tests for non-patch high-risk operations.
6. Add an audit verification operator command or endpoint.

## Exit Criteria for This Cleanup Phase

- active repo surfaces are canonical and unambiguous
- governance and audit claims are easier to validate
- contributor onboarding is materially simpler
- the repo’s perceived trustworthiness better matches the quality of the best underlying code

## Strategic Conclusion

TachyCode does not mainly need more breadth right now. It needs clearer canonical structure and stronger trust signals around what it already does well.

If this cleanup phase is done well, the platform will look and feel much more credible without changing its core strategic direction.
