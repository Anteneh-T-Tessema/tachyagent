# Golden Standard Roadmap
## How the Sovereign Stack Beats Claude Code and Every Competitor

**Author:** Anteneh Tsegaye Tessema — Yaya Systems LLC  
**Date:** 2026-04-25  
**Stack:** TachyCode (Rust) · SmolAgent (Python) · FinetuningLLMs (Python)

---

## Executive Summary

This document describes the eight builds required to make the sovereign AI stack the
gold standard for enterprise AI-assisted software development — surpassing Claude Code,
GitHub Copilot, Cursor, Devin, and Aider across every axis that matters to regulated
enterprises: data sovereignty, governance, self-improvement, and parallel scale.

---

## What We Have That No Competitor Has

| Capability | This Stack | Claude Code | Cursor | Devin | Copilot |
|---|---|---|---|---|---|
| Local-first / sovereign data | ✓ | ✗ | ✗ | ✗ | ✗ |
| Governance + RBAC + audit chain | ✓ | ✗ | ✗ | ✗ | ✗ |
| Self-improving via feedback loop | ✓ | ✗ | ✗ | ✗ | ✗ |
| Domain-expert grounding (legal/HR/finance/medical) | ✓ | ✗ | ✗ | ✗ | ✗ |
| Parallel DAG execution (8 workers) | ✓ | ✗ | ✗ | partial | ✗ |
| Policy-gated confidence thresholds | ✓ | ✗ | ✗ | ✗ | ✗ |
| Multi-model routing (SLM / LLM / domain expert) | ✓ | ✗ | ✗ | ✗ | ✗ |
| Fine-tune on your own codebase | ✓ | ✗ | ✗ | ✗ | ✗ |
| Human-in-the-loop approval queue | ✓ | ✗ | ✗ | ✗ | ✗ |
| MCP server (Claude Desktop / Cursor compatible) | ✓ | host only | ✗ | ✗ | ✗ |

**The moat is real.** The problem is that the front door — VS Code extension and
dashboard — was not fully wired to the daemon. That is what the builds below fix.

---

## The 5 Killer Markets (Where Competitors Are Legally Blocked)

### 1. Regulated Healthcare Engineering
Hospitals and health-tech companies cannot send code touching PHI to any cloud AI.
Your stack runs on-prem, has a HIPAA-aware audit trail on every tool call, and ships
a medical expert trained on clinical/compliance language. BAA-signable. Unique.

### 2. Legal Technology Firms
Attorney-client privilege and bar association rules prohibit sending client matter
details to third-party clouds. Your stack has a citation-backed legal expert, a
Forensic Logic Layer for risk-scoring, and cryptographic audit trails for every
suggestion made during contract-drafting work.

### 3. Financial Services / Fintech Engineering
Banks and trading firms build under SOX, PCI-DSS, and SEC rules. Source code
containing trading logic or customer account data cannot leave the network. Your
SHA-256 audit chain satisfies SOX Section 404 evidence requirements. The finance
expert answers domain questions during development without sending data offsite.

### 4. Enterprise Codebase Migration at Scale
A Fortune 500 with 200 microservices needs to migrate Java 8 → Java 21 across 50
repos. Parallel DAG execution (8 agents) + human approval gates + per-repo audit
trail makes this a product instead of an 18-month consulting engagement.

### 5. Self-Improving Development Teams
Every other tool is stateless — it forgets everything when the session ends. This
stack learns: every accepted suggestion becomes a training pair; every rejected
suggestion steers the model away. After 6 months, the system knows your codebase
patterns, naming conventions, and business domain. Competitors start from zero.

---

## The Positioning Statement

> **"The first AI development platform that learns your business, governs every
> action, and runs entirely on your infrastructure."**

Three sentences for a CTO:
1. Every other AI coding tool sends your code to someone else's cloud and forgets
   everything when the session ends.
2. Ours runs on your servers, builds institutional knowledge over time through
   fine-tuning, and maintains a cryptographic audit trail of every AI action for
   compliance.
3. After six months, it knows your codebase better than any tool that resets to
   zero every session.

---

## Build 1 — VS Code Extension (CRITICAL)

**Status:** COMPLETE — code fully wired, compiled, deployed.  
**What was fixed:** Default endpoint confirmed at `http://localhost:7777` (daemon default).

### What the Extension Delivers

- **Chat panel** with streaming responses, slash commands (`/fix`, `/explain`,
  `/review`, `/test`, `/commit`), inline code context injection, model picker
- **Inline completions** via `/api/complete/stream` with FIM (fill-in-middle) support,
  debounced 300 ms, latency displayed in status bar
- **Execution DAG panel** — live tree view of parallel/swarm runs, task statuses,
  semantic conflict detection, collapsible task detail
- **Audit Trail panel** — live SHA-256 chained audit events with severity colour-coding,
  expandable event detail, timestamp display
- **Policy diagnostics** — governance violations surface as VS Code squiggles with
  `[Tachy Policy]` source labels
- **Swarm Refactor command** (`Cmd+Shift+S`) — submit a goal over open files,
  watch the DAG panel fill in real time
- **Fix with Tachy** code action — right-click any diagnostic to auto-fix
- **Model picker** (`Cmd+Shift+M`) — live model list from daemon with fallback
  static list when daemon is offline

### How to Install

```bash
cd ~/developer/tachycode/vscode-extension
npm run compile
# In VS Code: Cmd+Shift+P → "Developer: Install Extension from Location"
# Point to: ~/developer/tachycode/vscode-extension
```

### Configuration

```json
// settings.json
{
  "tachy.endpoint": "http://localhost:7777",
  "tachy.model": "gemma4:26b",
  "tachy.enabled": true,
  "tachy.pollIntervalMs": 3000,
  "tachy.showPolicyWarningsInline": true
}
```

---

## Build 2 — Dashboard UI

**Status:** COMPLETE — full sovereign war-room UI at `dashboard/src/app/page.tsx`.

### Tabs Available

| Tab | Content |
|---|---|
| Live Mission | Real-time agent trace, vision feed, consensus reports, mission timeline |
| Audit Scrubber | Cryptographic audit trail with event inspection, visual anchors |
| Evolution Center | Self-optimization proposals with authorize/apply actions |
| Liquidity Hub | Protocol yield monitoring, strategy engine status |
| Governance DAO | Proposal voting, executive veto, sovereign charter rules |
| Hive Mind | Collective intelligence feed, knowledge syndication |
| Agent Roster | Trust index, permission mode, mission count per agent |
| Infrastructure Hub | Node provisioning, budget enforcer, hourly burn rate |
| Swarm Map | Global daemon network, peer connections, latency |
| Crisis Center | Anomaly telemetry, recovery playbooks, red alert |
| Intelligence Lab | Expert adapters, LoRA training pipeline, distillation stats |
| Diplomacy Hub | Allied swarms, trust scores, knowledge trade |

### How to Run

```bash
cd ~/developer/tachycode/dashboard
npm run dev    # http://localhost:3000
```

Daemon must be running on port 7777: `tachy serve`

---

## Build 3 — Browser / Computer Use Agent

**Status:** IN PROGRESS

### What to Build

Add Playwright-based browser tools to SmolAgent so agents can:
- Open URLs, click, type, scrape — wired through `BrowserAgent` in the registry
- Index browser findings into workspace RAG during execution
- TachyCode's existing `capture_screenshot` / `get_accessibility_tree` / `visual_diff`
  tools cover the visual verification side; browser automation covers the interaction side

### Files

| File | Change |
|---|---|
| `smolagent/browser_agent.py` | NEW — Playwright wrapper exposing navigate/click/type/scrape |
| `smolagent/agent_registry.py` | Add `browser_agent` entry with `task_type: web_research` |
| `smolagent/async_orchestrator.py` | Add `_WEB_PATTERNS` routing to `browser_agent` |
| `smolagent/webhook_receiver.py` | No change needed |

### Routing Trigger Patterns

```python
_WEB_PATTERNS = (
    r"\bscrape\b", r"\bweb\s+search\b", r"\bopen\s+(url|link|page)\b",
    r"\bbrowse\s+to\b", r"\bnavigate\s+to\b", r"\bfetch\s+docs?\b",
    r"\bcheck\s+(the\s+)?(website|page|url)\b",
    r"\bresearch\s+(online|the\s+web)\b",
)
```

---

## Build 4 — Multi-Repo Cross-Codebase Awareness

**Status:** IN PROGRESS

### What to Build

Extend `FinetuningLLMs` workspace knowledge base and `TachyCode`'s intelligence
indexer to span multiple repositories.

| Component | Change |
|---|---|
| `FinetuningLLMs/scripts/knowledge_base.py` | Add `index_workspace(repos: list[str])` — iterates repos, builds cross-repo FAISS index |
| `FinetuningLLMs/server.py` | Add `POST /knowledge/index-workspace` — accepts `{repos: [...paths]}` |
| `TachyCode rust/crates/intelligence/src/indexer.rs` | Add `WorkspaceManifest` struct accepting multiple repo paths |
| `TachyCode TACHY.md` convention | Add `[workspace.repos]` section listing additional repo paths |

### Cross-Repo Query Flow

```
Developer: "How does auth work across our services?"
  ↓
TachyCode search_codebase(cross_repo=true)
  ↓
Intelligence indexer queries all repos in TACHY.md [workspace.repos]
  ↓
Returns hits with per-repo citation: [auth-service:src/jwt.rs:42]
```

---

## Build 5 — GitHub / GitLab / Jira Deep Integration

**Status:** IN PROGRESS

### What to Build

| Component | Change |
|---|---|
| `smolagent/github_integration.py` | NEW — watches issues/PRs, opens draft PRs, posts review comments |
| `smolagent/jira_integration.py` | NEW — reads sprint tickets, generates scaffolding tasks |
| `smolagent/webhook_receiver.py` | Add `POST /webhook/github` — receives GitHub webhook events |
| `smolagent/agent_registry.py` | Add `pr_reviewer` agent, `issue_implementer` agent |

### Automation Triggers

```
GitHub issue labeled "tachy-auto"
  → Issue implementer agent reads issue
  → Generates implementation plan
  → Opens draft PR with scaffolding

GitHub PR opened
  → PR reviewer agent reads diff
  → Checks against codebase standards
  → Posts inline review comments via GitHub API
  → Flags governance policy violations

Jira sprint planning
  → For each ticket: generate scaffolding + assign to parallel agents
  → Human approves plan before agents start
```

---

## Build 6 — Security Scanner First-Class Workflow

**Status:** IN PROGRESS

### What to Build

| Component | Change |
|---|---|
| `smolagent/security_workflow.py` | NEW — OWASP Top 10 scan, auto-fix suggestions, severity scoring |
| `FinetuningLLMs/subjects/security/` | NEW — security training data (SQL injection, XSS, hardcoded secrets…) |
| `smolagent/agent_registry.py` | Add `security_scanner` agent |
| `smolagent/async_orchestrator.py` | Add security scan as post-execution hook after any `write_file` |
| `TachyCode` policy engine | Add `security_scan_required` policy rule for auth-touching files |

### OWASP Coverage

| Issue | Detection | Auto-Fix |
|---|---|---|
| SQL Injection | Pattern match + AST analysis | Parameterized query suggestion |
| XSS | Unescaped output detection | Escape function insertion |
| Hardcoded secrets | Entropy + pattern scan | Env var extraction |
| Insecure deserialization | Import graph analysis | Safe deserializer suggestion |
| Broken access control | Route + middleware analysis | Auth middleware insertion |

---

## Build 7 — Real-Time Team Collaboration

**Status:** IN PROGRESS

### What to Build

| Component | Change |
|---|---|
| `smolagent/webhook_receiver.py` | Add `GET /sessions/live` SSE endpoint — broadcasts all active agent runs |
| `smolagent/webhook_receiver.py` | Add `POST /sessions/join` — allows a second developer to follow a session |
| `TachyCode vscode-extension` | Add `tachy.followSession` command — connects to SSE feed from teammate |
| `TachyCode daemon` | `/api/mission/feed` already broadcasts — just need VS Code to subscribe |

### Collaboration Flow

```
Developer A runs: /commit "refactor auth middleware"
  → DAG panel shows 8 parallel tasks
  → Broadcasts mission events via /api/events SSE

Developer B (same team) opens VS Code
  → "Follow Active Mission" in DAG panel
  → Sees Developer A's task graph in real time
  → Can approve governance patches in the shared queue
```

---

## Build 8 — Compliance Profiles

**Status:** COMPLETE

### Profiles Available

Run `tachy init --compliance <profile>` to apply a governance policy template.

| Profile | Regulations | Key Controls |
|---|---|---|
| `hipaa` | HIPAA Privacy/Security Rules | PHI detection scan, BAA audit trail, 90-day retention |
| `soc2` | SOC 2 Type II | Access logging, change management gates, availability monitoring |
| `pci-dss` | PCI-DSS v4.0 | Cardholder data path gating, encryption verification, quarterly scan |
| `gdpr` | GDPR / UK GDPR | Data residency enforcement, PII detection, erasure audit trail |
| `sox` | SOX Section 302/404 | CFO cryptographic sign-off, financial code change approval, audit export |

### Files

| File | Content |
|---|---|
| `tachycode/.tachy/policies/hipaa.toml` | HIPAA governance rules |
| `tachycode/.tachy/policies/soc2.toml` | SOC 2 governance rules |
| `tachycode/.tachy/policies/pci-dss.toml` | PCI-DSS governance rules |
| `tachycode/.tachy/policies/gdpr.toml` | GDPR governance rules |
| `tachycode/.tachy/policies/sox.toml` | SOX governance rules |

---

## Priority Execution Timeline

```
Month 1  → Build 1 (extension) + Build 2 (dashboard) — DONE
           Fix port references, verify daemon connectivity

Month 2  → Build 3 (browser agent) + Build 5 (GitHub/Jira)
           Playwright integration, webhook receivers

Month 3  → Build 4 (multi-repo) + Build 6 (security scanner)
           Cross-repo RAG index, OWASP workflow

Month 4  → Build 7 (team collaboration) + Build 8 (compliance)
           Shared sessions, policy profile templates

Month 5  → Beta with 3 enterprise customers in regulated industries
Month 6  → Self-improving feedback loop has compounding data to demo
```

---

## Self-Improvement Feedback Loop (The Compounding Moat)

```
Developer accepts/rejects a suggestion
          ↓
TachyCode fires agent.completed webhook → SmolAgent port 8100
          ↓
webhook_receiver.py runs TachyExtractor on .tachy/sessions/
          ↓
Approved pairs pushed → FinetuningLLMs /training/examples
          ↓
FinetuningLLMs LoRA fine-tune runs on next scheduled cycle
          ↓
Domain expert models improve
          ↓
Better answers on next domain query → higher acceptance rate
          ↓
More training data → cycle repeats
```

After 6 months of daily use by a 10-person engineering team, the fine-tuned models
will know that team's specific codebase patterns, naming conventions, compliance
requirements, and business domain vocabulary better than any general-purpose tool.

**This compounding advantage is the moat that cannot be copied.**

---

## Service Architecture Reference

```
Ports:
  7777  TachyCode daemon  (execution plane, governance, audit, tools)
  8000  FinetuningLLMs    (knowledge plane, expert inference, fine-tuning)
  8100  SmolAgent         (orchestration gateway, routing, feedback loop)
  3000  Dashboard         (Next.js web UI, connects to 7777)

Routing:
  All domain queries:  → SmolAgent :8100 → FinetuningLLMs :8000
  All tool execution:  → TachyCode :7777
  Training feedback:   → SmolAgent :8100/webhook/tachy → FinetuningLLMs :8000

Shared environment:
  source ~/developer/.env   (single source of truth for all env vars)
  bash ~/developer/start_stack.sh   (starts all 3 services in order)
```
