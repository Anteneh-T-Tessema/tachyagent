"""Tachy HTTP API client."""

from __future__ import annotations

import time
from typing import Optional

import requests

from tachy.models import (
    Agent,
    AgentRun,
    FileLock,
    Model,
    ParallelRun,
    ParallelTask,
    PendingApproval,
    Template,
)


class TachyError(Exception):
    """Raised when the Tachy API returns an error."""

    def __init__(self, status: int, message: str):
        self.status = status
        super().__init__(f"[{status}] {message}")


class TachyClient:
    """Client for the Tachy daemon HTTP API.

    Usage::

        client = TachyClient("http://localhost:7777")
        run = client.run_agent("code-reviewer", "review src/main.rs")
        agent = client.wait_for_agent(run.agent_id)
        print(agent.summary)
    """

    def __init__(self, base_url: str = "http://localhost:7777", api_key: Optional[str] = None):
        self.base_url = base_url.rstrip("/")
        self.session = requests.Session()
        if api_key:
            self.session.headers["Authorization"] = f"Bearer {api_key}"

    # -- helpers --

    def _get(self, path: str) -> dict:
        resp = self.session.get(f"{self.base_url}{path}")
        if resp.status_code >= 400:
            raise TachyError(resp.status_code, resp.text)
        return resp.json()

    def _post(self, path: str, body: dict) -> dict:
        resp = self.session.post(f"{self.base_url}{path}", json=body)
        if resp.status_code >= 400:
            raise TachyError(resp.status_code, resp.text)
        return resp.json()

    # -- health --

    def health(self) -> dict:
        """Check daemon health."""
        return self._get("/health")

    # -- models & templates --

    def list_models(self) -> list[Model]:
        """List available LLM models."""
        data = self._get("/api/models")
        return [Model(name=m["name"], backend=m["backend"],
                      context_window=m.get("context_window", 0)) for m in data]

    def list_templates(self) -> list[Template]:
        """List available agent templates."""
        data = self._get("/api/templates")
        return [Template(name=t["name"], model=t["model"],
                         description=t.get("description", ""),
                         allowed_tools=t.get("allowed_tools", [])) for t in data]

    # -- agents --

    def run_agent(self, template: str, prompt: str, model: Optional[str] = None) -> AgentRun:
        """Start an agent run (async — returns immediately)."""
        body: dict = {"template": template, "prompt": prompt}
        if model:
            body["model"] = model
        data = self._post("/api/agents/run", body)
        return AgentRun(agent_id=data["agent_id"], status=data["status"],
                        message=data.get("message", ""))

    def get_agent(self, agent_id: str) -> Agent:
        """Get an agent's current status."""
        data = self._get(f"/api/agents/{agent_id}")
        if isinstance(data, str):
            import json
            data = json.loads(data)
        return Agent(
            agent_id=data.get("agent_id", agent_id),
            template=data.get("template", ""),
            status=data.get("status", "unknown"),
            iterations=data.get("iterations", 0),
            tool_invocations=data.get("tool_invocations", 0),
            summary=data.get("summary"),
        )

    def list_agents(self) -> list[Agent]:
        """List all agents."""
        data = self._get("/api/agents")
        return [Agent(agent_id=a["agent_id"], template=a["template"],
                      status=a["status"]) for a in data]

    def wait_for_agent(self, agent_id: str, timeout: float = 300, poll_interval: float = 1.0) -> Agent:
        """Poll until an agent completes or times out."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            agent = self.get_agent(agent_id)
            if agent.status in ("Completed", "Failed", "completed", "failed"):
                return agent
            time.sleep(poll_interval)
        raise TimeoutError(f"agent {agent_id} did not complete within {timeout}s")

    # -- parallel runs --

    def run_parallel(self, tasks: list[ParallelTask], max_concurrency: int = 4) -> ParallelRun:
        """Submit a parallel run with a DAG of tasks."""
        task_dicts = [
            {
                "template": t.template,
                "prompt": t.prompt,
                **({"model": t.model} if t.model else {}),
                **({"deps": t.deps} if t.deps else {}),
                "priority": t.priority,
            }
            for t in tasks
        ]
        data = self._post("/api/parallel/run", {
            "tasks": task_dicts,
            "max_concurrency": max_concurrency,
        })
        return ParallelRun(run_id=data["run_id"], status=data["status"],
                           task_count=data.get("task_count", 0))

    def get_parallel_run(self, run_id: str) -> ParallelRun:
        """Get a parallel run's status."""
        data = self._get(f"/api/parallel/runs/{run_id}")
        return ParallelRun(run_id=data["run_id"], status=data["status"],
                           task_count=data.get("task_count", 0),
                           tasks=data.get("tasks", []))

    def cancel_parallel_run(self, run_id: str, task_id: Optional[str] = None) -> dict:
        """Cancel a parallel run or a specific task within it."""
        body: dict = {}
        if task_id:
            body["task_id"] = task_id
        return self._post(f"/api/parallel/runs/{run_id}/cancel", body)

    def list_parallel_runs(self) -> list[dict]:
        """List all parallel runs."""
        data = self._get("/api/parallel/runs")
        return data.get("runs", [])

    # -- governance --

    def pending_approvals(self) -> list[PendingApproval]:
        """List pending approvals (agents + patches)."""
        data = self._get("/api/pending-approvals")
        items = data.get("pending", data) if isinstance(data, dict) else data
        result = []
        for item in items:
            pa = PendingApproval(
                type=item.get("type", "agent"),
                id=item.get("patch_id") or item.get("agent_id", ""),
                reason=item.get("reason", ""),
                file_path=item.get("file_path"),
                agent_id=item.get("agent_id"),
            )
            result.append(pa)
        return result

    def approve(self, *, agent_id: Optional[str] = None, patch_id: Optional[str] = None) -> dict:
        """Approve a pending agent or patch."""
        body: dict = {"approved": True}
        if patch_id:
            body["patch_id"] = patch_id
        elif agent_id:
            body["agent_id"] = agent_id
        return self._post("/api/approve", body)

    def reject(self, *, agent_id: Optional[str] = None, patch_id: Optional[str] = None) -> dict:
        """Reject a pending agent or patch."""
        body: dict = {"approved": False}
        if patch_id:
            body["patch_id"] = patch_id
        elif agent_id:
            body["agent_id"] = agent_id
        return self._post("/api/approve", body)

    # -- file locks --

    def list_file_locks(self) -> list[FileLock]:
        """List active file locks."""
        data = self._get("/api/file-locks")
        return [FileLock(file=l["file"], agent_id=l["agent_id"])
                for l in data.get("locks", [])]

    # -- audit & metrics --

    def audit_log(self) -> str:
        """Get the audit log."""
        resp = self.session.get(f"{self.base_url}/api/audit")
        return resp.text

    def metrics(self) -> str:
        """Get Prometheus-format metrics."""
        resp = self.session.get(f"{self.base_url}/api/metrics")
        return resp.text

    # -- webhooks --

    def add_webhook(self, url: str, events: list[str]) -> dict:
        """Register a webhook."""
        return self._post("/api/webhooks", {"url": url, "events": events})

    def list_webhooks(self) -> list[dict]:
        """List registered webhooks."""
        data = self._get("/api/webhooks")
        return data if isinstance(data, list) else data.get("webhooks", [])
