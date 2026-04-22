# Yaya ↔ Tachy Integration Design

## Purpose

This document defines the cleanest integration model between:

- `Tachy / Claw-Code` as the execution and governance plane
- `Yaya / FinetuningLLMs` as the expert intelligence plane

The goal is not to merge the two systems into one monolith. The goal is to compose them into a sovereign AI stack where each product remains strong in its natural domain.

## Integration Principle

Tachy should decide whether and how to act.

Yaya should decide how to answer, specialize, retrieve, and evolve experts.

In short:

- Tachy owns execution.
- Yaya owns expertise.

## System Roles

## Tachy Responsibilities

- agent orchestration
- tool execution
- approval workflows
- patch governance
- audit chain
- RBAC and team policies
- DAG and swarm execution
- editor and runtime operator experience

## Yaya Responsibilities

- enterprise knowledge ingestion
- workspace knowledge indexing
- retrieval with citations
- subject and department expert routing
- scheduled fine-tuning
- model registry and promotion
- evaluation and promotion gating

## Why This Split Is Correct

Tachy is strongest where action, workflow control, and governance matter.

Yaya is strongest where company knowledge, expert specialization, and training lifecycle matter.

If Tachy absorbs Yaya completely, it risks becoming a large enterprise knowledge platform it is not optimized to be.

If Yaya absorbs Tachy completely, it risks becoming a general agent operating system and losing focus.

The API boundary preserves clarity, velocity, and safety.

## Canonical End-to-End Flow

1. A user or operator asks Tachy to perform a business or coding task.
2. Tachy chooses an agent template and gathers local execution context.
3. Tachy calls Yaya for domain-grounded expert reasoning.
4. Yaya retrieves workspace knowledge, selects the expert model, and returns:
   - answer
   - citations
   - expert metadata
   - confidence and grounding metadata
5. Tachy decides whether to:
   - present the answer only
   - ask for approval
   - turn the answer into tool actions
6. Tachy executes actions under policy control.
7. Tachy emits audit events for all material steps.
8. Tachy exports successful sessions back to Yaya as training candidates.

## Integration Modes

## Mode 1: Advisory Expert

Tachy calls Yaya only for expert advice.

Use cases:

- finance policy answer
- legal ops answer
- healthcare admin guidance
- internal SOP explanation

Output:

- answer
- citations
- expert version
- retrieval evidence

No direct tool execution follows automatically.

## Mode 2: Plan Assist

Tachy asks Yaya for expert reasoning to inform a plan.

Use cases:

- “What is the correct compliance workflow before this action?”
- “Which internal policy constrains this migration?”

Output:

- answer
- constraints
- recommended plan
- required approvals

Tachy still owns final execution logic.

## Mode 3: Governed Action

Tachy uses Yaya’s response as one input to a tool-executing workflow.

Use cases:

- generating a regulated report
- updating documentation under internal policy rules
- assembling an internal response packet

Tachy must:

- log that Yaya was consulted
- preserve citations in the audit trail
- mark tool execution as derived from expert guidance

## Required Yaya API Contracts

These are the minimum contracts Tachy should expect.

## 1. Expert Chat

`POST /expert/chat`

Request:

- workspace
- subject or expert id
- user prompt
- optional conversation context
- optional execution context from Tachy

Response:

- answer
- citations
- retrieval mode
- retrieval evidence
- expert model id
- expert version
- evaluation status
- degraded/fallback flag

## 2. Expert Lookup

`GET /experts`

Purpose:

- list available experts by workspace and domain

Response:

- expert ids
- subject/domain
- active version
- last evaluation result
- last training time

## 3. Model Registry Status

`GET /models/registry`

Purpose:

- let Tachy know which promoted expert version is active

## 4. Evaluation Status

`GET /evaluations/latest`

Purpose:

- expose whether an expert is promotion-ready and trustworthy enough for high-stakes use

## 5. Training Export Ingest

`POST /training/examples`

Purpose:

- allow Tachy to send approved session examples back to Yaya

Payload:

- workspace
- subject
- prompt
- answer
- citations if present
- approval metadata
- audit reference
- outcome metadata

## 6. Knowledge Sync

`POST /knowledge/sync`

Purpose:

- allow Tachy to trigger re-indexing or sync refresh when new material is created through action workflows

## What Tachy Should Send to Yaya

For high-quality expert responses, Tachy should send more than raw prompts.

Useful context includes:

- workspace or team id
- acting user id or role
- current task template
- task goal
- local execution context summary
- relevant file paths
- governance sensitivity level

This lets Yaya tailor retrieval and expert behavior without owning execution.

## What Yaya Should Return

Yaya responses should be structured enough for Tachy to govern them.

Required fields:

- `answer`
- `citations`
- `expert_id`
- `expert_version`
- `retrieval_mode`
- `grounded`
- `fallback_used`
- `warnings`

Optional fields:

- `recommended_actions`
- `required_approvals`
- `confidence`
- `evaluation_summary`

## Governance Rules for Integration

The integration should obey four hard rules.

1. Yaya never executes tools directly inside Tachy.
2. Tachy never strips citations when presenting expert-grounded answers.
3. Tachy logs expert consultation as a first-class audit event.
4. High-risk actions cannot be auto-executed solely because Yaya recommended them.

## Audit Mapping

When Tachy consults Yaya, the audit trail should include:

- workspace
- user
- agent template
- expert id
- expert version
- retrieval evidence summary
- citation count
- whether fallback was used
- whether downstream actions were taken

This preserves explainability across the stack.

## Training Flywheel

The strongest joint flywheel is:

1. Yaya provides domain expertise.
2. Tachy executes governed workflows using that expertise.
3. Operators approve or reject outcomes.
4. Approved traces are exported back into Yaya.
5. Yaya retrains experts on approved high-value examples.
6. Tachy consumes improved experts in future tasks.

This is a real sovereign learning loop without relying on public SaaS training providers.

## Deployment Model

Recommended deployment:

- Yaya runs as an internal expert service
- Tachy runs as the local execution/governance service
- both share enterprise identity boundaries and workspace ids
- both remain independently deployable

Recommended communication:

- internal HTTP API
- signed service-to-service auth
- workspace-scoped requests

## Failure Handling

If Yaya is unavailable:

- Tachy may degrade to generic local models for low-risk tasks
- Tachy must log degraded mode
- Tachy must not silently present uncited answers as expert-grounded

If Tachy is unavailable:

- Yaya can still serve expert answers independently
- no execution or policy automation should be assumed

## Phased Integration Plan

## Phase 1

- Tachy calls Yaya for advisory expert chat
- Yaya returns citations and expert metadata
- Tachy logs expert consultations

## Phase 2

- Tachy sends approved conversation outcomes back to Yaya
- Yaya uses them for scheduled fine-tuning and evals

## Phase 3

- Tachy uses Yaya responses inside governed action workflows
- approvals and policy hooks become expert-aware

## Strategic Conclusion

The right integration is service composition, not consolidation.

Tachy should become the sovereign action and governance harness.

Yaya should become the sovereign expert factory.

That stack is more credible and strategically stronger than asking either system to become the whole platform.
