"""Tachy HTTP API client."""

from __future__ import annotations

import json
import time
from typing import Generator, Iterator, Optional

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

    # -- streaming completions --

    def stream_complete(
        self,
        prefix: str,
        suffix: Optional[str] = None,
        model: Optional[str] = None,
        max_tokens: Optional[int] = None,
    ) -> Iterator[str]:
        """Stream an FIM (fill-in-middle) code completion token by token.

        Yields plaintext tokens as they arrive. Raises ``TachyError`` on failure.

        Usage::

            for token in client.stream_complete("def add(a, b):"):
                print(token, end="", flush=True)
        """
        body: dict = {"prefix": prefix}
        if suffix is not None:
            body["suffix"] = suffix
        if model is not None:
            body["model"] = model
        if max_tokens is not None:
            body["max_tokens"] = max_tokens

        with self.session.post(
            f"{self.base_url}/api/complete/stream",
            json=body,
            stream=True,
            timeout=120,
        ) as resp:
            if resp.status_code >= 400:
                raise TachyError(resp.status_code, resp.text)
            yield from self._iter_sse(resp)

    def complete(
        self,
        prefix: str,
        suffix: Optional[str] = None,
        model: Optional[str] = None,
        max_tokens: Optional[int] = None,
    ) -> str:
        """Return a full FIM completion by collecting all stream chunks."""
        return "".join(
            self.stream_complete(prefix, suffix=suffix, model=model, max_tokens=max_tokens)
        )

    def chat_stream(
        self,
        prompt: str,
        model: Optional[str] = None,
    ) -> Iterator[str]:
        """Stream a chat response token by token.

        Yields plaintext tokens as they arrive. Raises ``TachyError`` on failure.

        Usage::

            for token in client.chat_stream("Explain what a closure is"):
                print(token, end="", flush=True)
        """
        body: dict = {"prompt": prompt}
        if model is not None:
            body["model"] = model

        with self.session.post(
            f"{self.base_url}/api/chat/stream",
            json=body,
            stream=True,
            timeout=300,
        ) as resp:
            if resp.status_code >= 400:
                raise TachyError(resp.status_code, resp.text)
            yield from self._iter_sse(resp)

    def chat(self, prompt: str, model: Optional[str] = None) -> str:
        """Return a full chat response by collecting all stream chunks."""
        return "".join(self.chat_stream(prompt, model=model))

    @staticmethod
    def _iter_sse(resp: requests.Response) -> Generator[str, None, None]:
        """Parse Server-Sent Events from a streaming response and yield text tokens."""
        for raw_line in resp.iter_lines(decode_unicode=True):
            if not raw_line or not raw_line.startswith("data:"):
                continue
            payload = raw_line[5:].strip()
            if not payload or payload == "{}":
                continue
            try:
                evt = json.loads(payload)
            except json.JSONDecodeError:
                continue
            if "error" in evt:
                raise TachyError(500, evt["error"])
            if "text" in evt:
                yield evt["text"]

    # -- code search --

    def search(self, query: str, limit: int = 10) -> list[dict]:
        """Search the indexed codebase.

        Returns a list of matching file entries, each with keys:
        ``path``, ``language``, ``lines``, ``exports``, ``summary``.
        """
        data = self._get(f"/api/search?q={requests.utils.quote(query)}&limit={limit}")
        return data.get("results", [])

    # -- policy --

    def get_policy(self) -> dict:
        """Return the current tachy-policy.yaml as a dict."""
        return self._get("/api/policy")

    def set_policy(self, policy: dict) -> dict:
        """Replace the workspace tachy-policy.yaml from *policy* dict."""
        return self._post("/api/policy", policy)

    # -- webhooks --

    def add_webhook(self, url: str, events: list[str]) -> dict:
        """Register a webhook."""
        return self._post("/api/webhooks", {"url": url, "events": events})

    def list_webhooks(self) -> list[dict]:
        """List registered webhooks."""
        data = self._get("/api/webhooks")
        return data if isinstance(data, list) else data.get("webhooks", [])

    # -- tasks --

    def schedule_task(
        self,
        template: str,
        name: str,
        interval_seconds: Optional[int] = None,
    ) -> dict:
        """Schedule a recurring (or one-shot) agent task.

        Args:
            template: Agent template name.
            name: Human-readable task name.
            interval_seconds: Repeat interval in seconds.  Omit for a one-shot task.

        Returns:
            Dict with ``task_id`` and ``name``.
        """
        payload: dict = {"template": template, "name": name}
        if interval_seconds is not None:
            payload["interval_seconds"] = interval_seconds
        return self._post("/api/tasks/schedule", payload)

    # -- license --

    def activate_license(self, key: str, secret: str) -> dict:
        """Activate a Tachy license key.

        Args:
            key: License key in ``TACHY-<payload>-<sig>`` format.
            secret: Shared secret used to verify the key signature.

        Returns:
            Dict with ``status``, ``tier``, and ``expires_at``.
        """
        return self._post("/api/license/activate", {"key": key, "secret": secret})

    # -- non-streaming prompt completion --

    def prompt_complete(
        self,
        prompt: str,
        model: Optional[str] = None,
        max_tokens: int = 2048,
    ) -> str:
        """Blocking single-turn prompt completion via POST /api/complete.

        Unlike :meth:`complete` (FIM/streaming), this sends a plain *prompt*
        and waits for the full response.

        Args:
            prompt: The prompt text.
            model: Override the model.  Uses daemon default if omitted.
            max_tokens: Maximum tokens to generate (capped at 4096 by daemon).

        Returns:
            Completion text string.
        """
        payload: dict = {"prompt": prompt, "max_tokens": max_tokens}
        if model is not None:
            payload["model"] = model
        data = self._post("/api/complete", payload)
        return data.get("completion", "")

    # -- conversation management --

    def list_conversations(self) -> list[dict]:
        """List all stored conversations.

        Returns:
            List of conversation dicts with keys ``id``, ``title``, ``messages``.
        """
        return self._get("/api/conversations").get("conversations", [])

    def create_conversation(self, title: str) -> dict:
        """Create a new conversation.

        Args:
            title: Human-readable title for the conversation.

        Returns:
            Newly created conversation dict.
        """
        return self._post("/api/conversations", {"title": title})

    def get_conversation(self, conv_id: str) -> dict:
        """Fetch a single conversation by ID.

        Args:
            conv_id: Conversation identifier (e.g. ``"conv-1"``).

        Returns:
            Conversation dict.

        Raises:
            TachyError: If the conversation is not found (HTTP 404).
        """
        return self._get(f"/api/conversations/{conv_id}")

    def delete_conversation(self, conv_id: str) -> bool:
        """Delete a conversation by ID.

        Args:
            conv_id: Conversation identifier.

        Returns:
            ``True`` if deleted, ``False`` if not found.
        """
        resp = self.session.delete(f"{self.base_url}/api/conversations/{conv_id}")
        return resp.status_code == 204

    # -- extended agent management --

    def delete_agent(self, agent_id: str) -> bool:
        """Remove an agent from daemon state.

        Args:
            agent_id: Agent identifier (e.g. ``"agent-1"``).

        Returns:
            ``True`` if deleted, ``False`` if not found.
        """
        resp = self.session.delete(f"{self.base_url}/api/agents/{agent_id}")
        return resp.status_code == 204

    def cancel_agent(self, agent_id: str) -> dict:
        """Cancel a running agent (marks it as Failed).

        Args:
            agent_id: Agent identifier.

        Returns:
            Dict with ``id`` and ``status``.

        Raises:
            TachyError: If the agent is not found.
        """
        return self._post(f"/api/agents/{agent_id}/cancel", {})

    # -- codebase index --

    def index_status(self) -> dict:
        """Return the status of the codebase index.

        Returns:
            Dict with ``status`` (``"ready"`` or ``"not_built"``), ``file_count``,
            and ``workspace``.
        """
        return self._get("/api/index")

    def build_index(self) -> dict:
        """Trigger a codebase index (re)build.

        Returns:
            Dict with ``status``, ``file_count``, and ``workspace``.
        """
        return self._post("/api/index", {})

    # -- inference stats --

    def inference_stats(self) -> dict:
        """Return accumulated inference performance statistics.

        Returns:
            Dict with ``avg_ttft_ms``, ``avg_tokens_per_sec``, ``total_requests``,
            and ``total_tokens``.
        """
        return self._get("/api/inference/stats")

    # -- cloud / swarm --

    def list_cloud_jobs(self) -> list[dict]:
        """List cloud batch jobs (AWS Batch or similar).

        Returns:
            List of cloud job dicts.
        """
        return self._get("/api/cloud/jobs").get("jobs", [])

    def list_swarm_runs(self) -> list[dict]:
        """List swarm refactor runs.

        Returns:
            List of swarm run dicts.
        """
        return self._get("/api/swarm/runs").get("runs", [])
