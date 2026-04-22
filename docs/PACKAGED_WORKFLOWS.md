# Packaged Workflows

This repo now includes reusable Yaya-assisted workflow paths that run through Tachy's audited execution layer.

## Legal Expert Memo

Purpose:
- Turn grounded Yaya legal guidance into a concise regulated-team memo.

Primary entrypoint:
- `examples/yaya_tachy_workflow.py`

Example:

```bash
PYTHONPATH=sdk/python /path/to/python examples/yaya_tachy_workflow.py \
  --workspace default \
  --subject legal \
  --question "Summarize the purpose of this legal expert workspace." \
  --tachy-url http://localhost:7777 \
  --tachy-api-key local-dev \
  --yaya-url http://localhost:8000 \
  --output-path yaya_grounded_memo.md
```

Output shape:
- `# Legal Expert Memo`
- `## Executive Summary`
- `## Key Source Signals`
- `## Operational Limits`
- `## Supporting Citations`

## Finance Expert Brief

Purpose:
- Turn grounded Yaya finance guidance into a concise control-focused finance brief.

Primary entrypoint:
- `examples/yaya_finance_brief_workflow.py`

Example:

```bash
PYTHONPATH=sdk/python /path/to/python examples/yaya_finance_brief_workflow.py \
  --workspace default \
  --tachy-url http://localhost:7777 \
  --tachy-api-key local-dev \
  --yaya-url http://localhost:8000
```

Default finance question:
- Prepare a finance brief covering close controls, escalation thresholds, cash forecast triggers, and board reporting expectations.

Output shape:
- `# Finance Expert Brief`
- `## Executive Summary`
- `## Control Signals`
- `## Escalations And Limits`
- `## Supporting Citations`

## Architecture Notes

- Yaya provides grounded expert responses plus citations.
- Tachy provides the audited execution layer and writes the final artifact.
- Workflow scripts normalize final output into a canonical markdown shape even if the underlying model drifts.
