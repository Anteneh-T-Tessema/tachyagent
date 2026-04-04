# Tachy Agent SDK (Python)

Python client for the [Tachy AI Agent](https://github.com/Anteneh-T-Tessema/tachyagent) platform.

## Install

```bash
pip install tachy-agent
```

## Quick Start

```python
from tachy import TachyClient, ParallelTask

client = TachyClient("http://localhost:7777", api_key="your-key")

# Run a single agent
run = client.run_agent("code-reviewer", "review src/main.rs")
agent = client.wait_for_agent(run.agent_id)
print(agent.summary)

# Run agents in parallel
tasks = [
    ParallelTask(template="code-reviewer", prompt="review auth module"),
    ParallelTask(template="security-scanner", prompt="scan for vulns"),
]
parallel = client.run_parallel(tasks, max_concurrency=2)

# Check governance approvals
for approval in client.pending_approvals():
    print(f"{approval.type}: {approval.reason}")
    client.approve(patch_id=approval.id)
```

## API Coverage

| Method | Endpoint |
|--------|----------|
| `health()` | `GET /health` |
| `list_models()` | `GET /api/models` |
| `list_templates()` | `GET /api/templates` |
| `run_agent()` | `POST /api/agents/run` |
| `get_agent()` | `GET /api/agents/<id>` |
| `list_agents()` | `GET /api/agents` |
| `wait_for_agent()` | Polls `GET /api/agents/<id>` |
| `run_parallel()` | `POST /api/parallel/run` |
| `get_parallel_run()` | `GET /api/parallel/runs/<id>` |
| `cancel_parallel_run()` | `POST /api/parallel/runs/<id>/cancel` |
| `pending_approvals()` | `GET /api/pending-approvals` |
| `approve()` / `reject()` | `POST /api/approve` |
| `list_file_locks()` | `GET /api/file-locks` |
| `audit_log()` | `GET /api/audit` |
| `metrics()` | `GET /api/metrics` |
| `add_webhook()` | `POST /api/webhooks` |
