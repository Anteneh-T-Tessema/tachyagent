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


# ── Streaming completion tests ─────────────────────────────────────────────

SSE_COMPLETE = (
    b'data: {"text":"def "}\n\n'
    b'data: {"text":"add"}\n\n'
    b'data: {"text":"(a, b):"}\n\n'
    b'event: done\ndata: {}\n\n'
)

SSE_CHAT = (
    b'data: {"text":"A closure"}\n\n'
    b'data: {"text":" captures"}\n\n'
    b'data: {"text":" its environment."}\n\n'
    b'event: done\ndata: {}\n\n'
)

SSE_ERROR = b'data: {"error":"model not loaded"}\n\n'


@responses.activate
def test_stream_complete():
    responses.add(
        responses.POST,
        f"{BASE}/api/complete/stream",
        body=SSE_COMPLETE,
        status=200,
        content_type="text/event-stream",
        stream=True,
    )
    client = TachyClient(BASE)
    tokens = list(client.stream_complete("def ", suffix="\n    return a + b"))
    assert tokens == ["def ", "add", "(a, b):"]


@responses.activate
def test_complete_joins_tokens():
    responses.add(
        responses.POST,
        f"{BASE}/api/complete/stream",
        body=SSE_COMPLETE,
        status=200,
        content_type="text/event-stream",
        stream=True,
    )
    client = TachyClient(BASE)
    result = client.complete("def ", model="gemma4:26b")
    assert result == "def add(a, b):"


@responses.activate
def test_chat_stream():
    responses.add(
        responses.POST,
        f"{BASE}/api/chat/stream",
        body=SSE_CHAT,
        status=200,
        content_type="text/event-stream",
        stream=True,
    )
    client = TachyClient(BASE)
    tokens = list(client.chat_stream("Explain a closure", model="gemma4:26b"))
    assert tokens == ["A closure", " captures", " its environment."]


@responses.activate
def test_chat_joins_tokens():
    responses.add(
        responses.POST,
        f"{BASE}/api/chat/stream",
        body=SSE_CHAT,
        status=200,
        content_type="text/event-stream",
        stream=True,
    )
    client = TachyClient(BASE)
    result = client.chat("Explain a closure")
    assert result == "A closure captures its environment."


@responses.activate
def test_stream_complete_error():
    responses.add(
        responses.POST,
        f"{BASE}/api/complete/stream",
        body=SSE_ERROR,
        status=200,
        content_type="text/event-stream",
        stream=True,
    )
    client = TachyClient(BASE)
    from tachy.client import TachyError
    with pytest.raises(TachyError) as exc_info:
        list(client.stream_complete("x = "))
    assert "model not loaded" in str(exc_info.value)


@responses.activate
def test_stream_complete_http_error():
    responses.add(
        responses.POST,
        f"{BASE}/api/complete/stream",
        json={"error": "rate limited"},
        status=429,
    )
    client = TachyClient(BASE)
    from tachy.client import TachyError
    with pytest.raises(TachyError) as exc_info:
        list(client.stream_complete("x = "))
    assert exc_info.value.status == 429



@responses.activate
def test_search():
    results = [
        {"path": "src/main.rs", "language": "rust", "lines": 200, "exports": ["main"], "summary": "Entry point"},
        {"path": "src/lib.rs", "language": "rust", "lines": 100, "exports": ["TachyClient"], "summary": "Library root"},
    ]
    responses.add(responses.GET, f"{BASE}/api/search",
                  json={"results": results}, status=200)
    client = TachyClient(BASE)
    found = client.search("main")
    assert len(found) == 2
    assert found[0]["path"] == "src/main.rs"
    assert found[0]["exports"] == ["main"]


@responses.activate
def test_search_empty():
    responses.add(responses.GET, f"{BASE}/api/search",
                  json={"results": []}, status=200)
    client = TachyClient(BASE)
    found = client.search("nonexistent_symbol_xyz")
    assert found == []


@responses.activate
def test_get_policy():
    policy = {"version": 1, "rules": [{"action": "deny", "resource": "/etc/passwd"}]}
    responses.add(responses.GET, f"{BASE}/api/policy",
                  json=policy, status=200)
    client = TachyClient(BASE)
    p = client.get_policy()
    assert p["version"] == 1
    assert len(p["rules"]) == 1


@responses.activate
def test_set_policy():
    policy = {"version": 2, "rules": []}
    responses.add(responses.POST, f"{BASE}/api/policy",
                  json={"ok": True}, status=200)
    client = TachyClient(BASE)
    resp = client.set_policy(policy)
    assert resp.get("ok") is True
    # Verify the body was sent correctly
    assert json.loads(responses.calls[0].request.body) == policy


@responses.activate
def test_schedule_task_interval():
    responses.add(responses.POST, f"{BASE}/api/tasks/schedule",
                  json={"task_id": "t-1", "name": "nightly-scan"}, status=201)
    client = TachyClient(BASE)
    result = client.schedule_task("security-scanner", "nightly-scan", interval_seconds=86400)
    assert result["task_id"] == "t-1"
    body = json.loads(responses.calls[0].request.body)
    assert body["template"] == "security-scanner"
    assert body["interval_seconds"] == 86400


@responses.activate
def test_schedule_task_oneshot():
    responses.add(responses.POST, f"{BASE}/api/tasks/schedule",
                  json={"task_id": "t-2", "name": "one-off"}, status=201)
    client = TachyClient(BASE)
    result = client.schedule_task("reviewer", "one-off")
    assert result["task_id"] == "t-2"
    body = json.loads(responses.calls[0].request.body)
    assert "interval_seconds" not in body


@responses.activate
def test_activate_license():
    responses.add(responses.POST, f"{BASE}/api/license/activate",
                  json={"status": "activated", "tier": "Enterprise", "expires_at": 9999999999},
                  status=200)
    client = TachyClient(BASE)
    result = client.activate_license("TACHY-abc-xyz", "my-secret")
    assert result["status"] == "activated"
    assert result["tier"] == "Enterprise"
    body = json.loads(responses.calls[0].request.body)
    assert body["key"] == "TACHY-abc-xyz"
    assert body["secret"] == "my-secret"


@responses.activate
def test_activate_license_invalid_key():
    responses.add(responses.POST, f"{BASE}/api/license/activate",
                  json={"error": "invalid key format"}, status=400)
    client = TachyClient(BASE)
    import pytest
    with pytest.raises(Exception):
        client.activate_license("BAD-KEY", "secret")


@responses.activate
def test_prompt_complete():
    responses.add(responses.POST, f"{BASE}/api/complete",
                  json={"completion": "Hello, world!", "model": "llama3.2"}, status=200)
    client = TachyClient(BASE)
    text = client.prompt_complete("Say hello")
    assert text == "Hello, world!"
    body = json.loads(responses.calls[0].request.body)
    assert body["prompt"] == "Say hello"
    assert body["max_tokens"] == 2048


@responses.activate
def test_prompt_complete_with_model_override():
    responses.add(responses.POST, f"{BASE}/api/complete",
                  json={"completion": "Hi!", "model": "qwen2.5"}, status=200)
    client = TachyClient(BASE)
    text = client.prompt_complete("Say hi", model="qwen2.5", max_tokens=100)
    assert text == "Hi!"
    body = json.loads(responses.calls[0].request.body)
    assert body["model"] == "qwen2.5"
    assert body["max_tokens"] == 100


# ---------------------------------------------------------------------------
# Conversation management
# ---------------------------------------------------------------------------

@responses.activate
def test_list_conversations():
    conversations = [{"id": "conv-1", "title": "Hello", "messages": []}]
    responses.add(responses.GET, f"{BASE}/api/conversations",
                  json={"conversations": conversations}, status=200)
    client = TachyClient(BASE)
    result = client.list_conversations()
    assert len(result) == 1
    assert result[0]["id"] == "conv-1"


@responses.activate
def test_create_conversation():
    responses.add(responses.POST, f"{BASE}/api/conversations",
                  json={"id": "conv-1", "title": "My Chat", "messages": []}, status=201)
    client = TachyClient(BASE)
    conv = client.create_conversation("My Chat")
    assert conv["id"] == "conv-1"
    body = json.loads(responses.calls[0].request.body)
    assert body["title"] == "My Chat"


@responses.activate
def test_get_conversation():
    responses.add(responses.GET, f"{BASE}/api/conversations/conv-1",
                  json={"id": "conv-1", "title": "Test", "messages": []}, status=200)
    client = TachyClient(BASE)
    conv = client.get_conversation("conv-1")
    assert conv["id"] == "conv-1"


@responses.activate
def test_get_conversation_not_found():
    responses.add(responses.GET, f"{BASE}/api/conversations/conv-999",
                  json={"error": "not found"}, status=404)
    client = TachyClient(BASE)
    with pytest.raises(Exception):
        client.get_conversation("conv-999")


@responses.activate
def test_delete_conversation():
    responses.add(responses.DELETE, f"{BASE}/api/conversations/conv-1",
                  status=204)
    client = TachyClient(BASE)
    ok = client.delete_conversation("conv-1")
    assert ok is True


@responses.activate
def test_delete_conversation_not_found():
    responses.add(responses.DELETE, f"{BASE}/api/conversations/conv-999",
                  json={"error": "not found"}, status=404)
    client = TachyClient(BASE)
    ok = client.delete_conversation("conv-999")
    assert ok is False


# ---------------------------------------------------------------------------
# Extended agent management
# ---------------------------------------------------------------------------

@responses.activate
def test_delete_agent():
    responses.add(responses.DELETE, f"{BASE}/api/agents/agent-1",
                  status=204)
    client = TachyClient(BASE)
    ok = client.delete_agent("agent-1")
    assert ok is True


@responses.activate
def test_delete_agent_not_found():
    responses.add(responses.DELETE, f"{BASE}/api/agents/agent-999",
                  status=404)
    client = TachyClient(BASE)
    ok = client.delete_agent("agent-999")
    assert ok is False


@responses.activate
def test_cancel_agent():
    responses.add(responses.POST, f"{BASE}/api/agents/agent-1/cancel",
                  json={"id": "agent-1", "status": "Failed"}, status=200)
    client = TachyClient(BASE)
    result = client.cancel_agent("agent-1")
    assert result["status"] == "Failed"
    assert result["id"] == "agent-1"


@responses.activate
def test_cancel_agent_not_found():
    responses.add(responses.POST, f"{BASE}/api/agents/agent-999/cancel",
                  json={"error": "not found"}, status=404)
    client = TachyClient(BASE)
    with pytest.raises(Exception):
        client.cancel_agent("agent-999")


# ---------------------------------------------------------------------------
# Codebase index
# ---------------------------------------------------------------------------

@responses.activate
def test_index_status_ready():
    responses.add(responses.GET, f"{BASE}/api/index",
                  json={"status": "ready", "file_count": 42, "workspace": "/code"}, status=200)
    client = TachyClient(BASE)
    result = client.index_status()
    assert result["status"] == "ready"
    assert result["file_count"] == 42


@responses.activate
def test_index_status_not_built():
    responses.add(responses.GET, f"{BASE}/api/index",
                  json={"status": "not_built", "file_count": 0, "workspace": "/code"}, status=200)
    client = TachyClient(BASE)
    result = client.index_status()
    assert result["status"] == "not_built"


@responses.activate
def test_build_index():
    responses.add(responses.POST, f"{BASE}/api/index",
                  json={"status": "built", "file_count": 58, "workspace": "/code"}, status=202)
    client = TachyClient(BASE)
    result = client.build_index()
    assert result["status"] == "built"
    assert result["file_count"] == 58


# ---------------------------------------------------------------------------
# Inference stats
# ---------------------------------------------------------------------------

@responses.activate
def test_inference_stats():
    stats = {"avg_ttft_ms": 120.5, "avg_tokens_per_sec": 32.1, "total_requests": 7, "total_tokens": 1024}
    responses.add(responses.GET, f"{BASE}/api/inference/stats",
                  json=stats, status=200)
    client = TachyClient(BASE)
    result = client.inference_stats()
    assert result["total_requests"] == 7
    assert result["avg_ttft_ms"] == 120.5


# ---------------------------------------------------------------------------
# Cloud / Swarm
# ---------------------------------------------------------------------------

@responses.activate
def test_list_cloud_jobs():
    responses.add(responses.GET, f"{BASE}/api/cloud/jobs",
                  json={"jobs": [{"id": "job-1", "status": "RUNNING"}]}, status=200)
    client = TachyClient(BASE)
    jobs = client.list_cloud_jobs()
    assert len(jobs) == 1
    assert jobs[0]["id"] == "job-1"


@responses.activate
def test_list_swarm_runs():
    responses.add(responses.GET, f"{BASE}/api/swarm/runs",
                  json={"runs": [{"id": "swarm-1", "tasks": 3}]}, status=200)
    client = TachyClient(BASE)
    runs = client.list_swarm_runs()
    assert len(runs) == 1
    assert runs[0]["id"] == "swarm-1"
