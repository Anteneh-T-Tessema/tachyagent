"""Tests for the Tachy Python SDK client."""

import json
import pytest
import responses

from tachy import TachyClient, ParallelTask


BASE = "http://localhost:7777"


@responses.activate
def test_health():
    responses.add(responses.GET, f"{BASE}/health",
                  json={"status": "ok", "models": 5}, status=200)
    client = TachyClient(BASE)
    data = client.health()
    assert data["status"] == "ok"


@responses.activate
def test_list_models():
    responses.add(responses.GET, f"{BASE}/api/models",
                  json=[{"name": "gemma4:26b", "backend": "Ollama", "context_window": 8192}],
                  status=200)
    client = TachyClient(BASE)
    models = client.list_models()
    assert len(models) == 1
    assert models[0].name == "gemma4:26b"
    assert models[0].backend == "Ollama"


@responses.activate
def test_list_templates():
    responses.add(responses.GET, f"{BASE}/api/templates",
                  json=[{"name": "code-reviewer", "model": "gemma4:26b",
                         "description": "Reviews code", "allowed_tools": ["read_file"]}],
                  status=200)
    client = TachyClient(BASE)
    templates = client.list_templates()
    assert len(templates) == 1
    assert templates[0].name == "code-reviewer"


@responses.activate
def test_run_agent():
    responses.add(responses.POST, f"{BASE}/api/agents/run",
                  json={"agent_id": "agent-1", "status": "running",
                        "message": "Agent started."},
                  status=202)
    client = TachyClient(BASE)
    run = client.run_agent("code-reviewer", "review my code")
    assert run.agent_id == "agent-1"
    assert run.status == "running"


@responses.activate
def test_get_agent():
    responses.add(responses.GET, f"{BASE}/api/agents/agent-1",
                  json={"agent_id": "agent-1", "template": "code-reviewer",
                        "status": "Completed", "iterations": 3,
                        "tool_invocations": 5, "summary": "All good"},
                  status=200)
    client = TachyClient(BASE)
    agent = client.get_agent("agent-1")
    assert agent.status == "Completed"
    assert agent.iterations == 3
    assert agent.summary == "All good"


@responses.activate
def test_list_agents():
    responses.add(responses.GET, f"{BASE}/api/agents",
                  json=[{"agent_id": "agent-1", "template": "code-reviewer",
                         "status": "Completed"}],
                  status=200)
    client = TachyClient(BASE)
    agents = client.list_agents()
    assert len(agents) == 1


@responses.activate
def test_wait_for_agent():
    responses.add(responses.GET, f"{BASE}/api/agents/agent-1",
                  json={"agent_id": "agent-1", "template": "t",
                        "status": "Completed", "summary": "done"},
                  status=200)
    client = TachyClient(BASE)
    agent = client.wait_for_agent("agent-1", timeout=5)
    assert agent.status == "Completed"


@responses.activate
def test_run_parallel():
    responses.add(responses.POST, f"{BASE}/api/parallel/run",
                  json={"run_id": "run-123", "status": "running", "task_count": 2},
                  status=202)
    client = TachyClient(BASE)
    tasks = [
        ParallelTask(template="code-reviewer", prompt="review a"),
        ParallelTask(template="test-runner", prompt="run tests", deps=["t0"]),
    ]
    run = client.run_parallel(tasks, max_concurrency=2)
    assert run.run_id == "run-123"
    assert run.task_count == 2


@responses.activate
def test_pending_approvals():
    responses.add(responses.GET, f"{BASE}/api/pending-approvals",
                  json={"pending": [
                      {"type": "patch", "patch_id": "patch-1",
                       "file_path": "src/auth.rs", "reason": "auth path",
                       "agent_id": "agent-1"}
                  ], "count": 1},
                  status=200)
    client = TachyClient(BASE)
    approvals = client.pending_approvals()
    assert len(approvals) == 1
    assert approvals[0].type == "patch"
    assert approvals[0].file_path == "src/auth.rs"


@responses.activate
def test_approve_patch():
    responses.add(responses.POST, f"{BASE}/api/approve",
                  json={"ok": True, "status": "approved", "file_path": "src/auth.rs"},
                  status=200)
    client = TachyClient(BASE)
    result = client.approve(patch_id="patch-1")
    assert result["ok"] is True


@responses.activate
def test_list_file_locks():
    responses.add(responses.GET, f"{BASE}/api/file-locks",
                  json={"locks": [{"file": "src/main.rs", "agent_id": "agent-1"}],
                        "count": 1},
                  status=200)
    client = TachyClient(BASE)
    locks = client.list_file_locks()
    assert len(locks) == 1
    assert locks[0].file == "src/main.rs"


@responses.activate
def test_api_key_header():
    responses.add(responses.GET, f"{BASE}/health", json={"status": "ok"}, status=200)
    client = TachyClient(BASE, api_key="secret-key")
    client.health()
    assert responses.calls[0].request.headers["Authorization"] == "Bearer secret-key"


@responses.activate
def test_error_handling():
    responses.add(responses.GET, f"{BASE}/api/agents/bad",
                  json={"error": "not found"}, status=404)
    client = TachyClient(BASE)
    from tachy.client import TachyError
    with pytest.raises(TachyError) as exc_info:
        client.get_agent("bad")
    assert exc_info.value.status == 404
