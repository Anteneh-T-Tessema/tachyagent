"""Yaya expert platform client for Tachy integrations."""

from __future__ import annotations

from typing import Optional

import requests

from tachy.models import YayaCitation, YayaExpert, YayaExpertResponse, YayaRetrievalPreferences


class YayaError(Exception):
    """Raised when the Yaya API returns an error."""

    def __init__(self, status: int, message: str):
        self.status = status
        super().__init__(f"[{status}] {message}")


class YayaClient:
    """Client for the Yaya expert platform."""

    def __init__(self, base_url: str = "http://localhost:8000", api_key: Optional[str] = None):
        self.base_url = base_url.rstrip("/")
        self.session = requests.Session()
        if api_key:
            self.session.headers["x-api-key"] = api_key

    def _get(self, path: str, **params) -> dict | list:
        resp = self.session.get(f"{self.base_url}{path}", params=params)
        if resp.status_code >= 400:
            raise YayaError(resp.status_code, resp.text)
        return resp.json()

    def _post(self, path: str, body: dict) -> dict:
        resp = self.session.post(f"{self.base_url}{path}", json=body)
        if resp.status_code >= 400:
            raise YayaError(resp.status_code, resp.text)
        return resp.json()

    def auth_session(self) -> dict:
        return self._get("/auth/session")

    def list_workspaces(self) -> list[dict]:
        data = self._get("/workspaces")
        return list(data)

    def list_experts(self, workspace: str) -> list[YayaExpert]:
        data = self._get("/experts", workspace=workspace)
        return [
            YayaExpert(
                workspace=item["workspace"],
                subject=item["subject"],
                active_version=item.get("active_version"),
                model_path=item.get("model_path"),
                latest_evaluation_passed=item.get("latest_evaluation_passed"),
                latest_evaluation_version=item.get("latest_evaluation_version"),
                latest_trained_at=item.get("latest_trained_at"),
            )
            for item in data
        ]

    def expert_chat(
        self,
        workspace: str,
        subject: str,
        message: str,
        execution_context: Optional[dict] = None,
        actor: Optional[dict] = None,
    ) -> YayaExpertResponse:
        data = self._post(
            "/expert/chat",
            {
                "workspace": workspace,
                "subject": subject,
                "message": message,
                "execution_context": execution_context or {},
                "actor": actor or {},
            },
        )
        citations = [
            YayaCitation(
                source=item["source"],
                label=item["label"],
                page=item.get("page"),
                chunk=item.get("chunk"),
                score=item.get("score"),
                semantic_score=item.get("semantic_score"),
                lexical_score=item.get("lexical_score"),
                retrieval_mode=item.get("retrieval_mode"),
            )
            for item in data.get("citations", [])
        ]
        return YayaExpertResponse(
            workspace=data["workspace"],
            subject=data["subject"],
            response=data["response"],
            citations=citations,
            model_type=data.get("model_type", "expert"),
            used_fallback=bool(data.get("used_fallback", False)),
            retrieval_mode=data.get("retrieval_mode", "none"),
            grounded=bool(data.get("grounded", False)),
            retrieval_preferences=(
                YayaRetrievalPreferences(
                    strategy=data.get("retrieval_preferences", {}).get("strategy", "workspace_wide"),
                    preferred_sources=list(data.get("retrieval_preferences", {}).get("preferred_sources", [])),
                    preferred_source_terms=list(data.get("retrieval_preferences", {}).get("preferred_source_terms", [])),
                    explicit_preferred_sources=list(data.get("retrieval_preferences", {}).get("explicit_preferred_sources", [])),
                    explicit_preferred_source_terms=list(data.get("retrieval_preferences", {}).get("explicit_preferred_source_terms", [])),
                    inferred_preferred_sources=list(data.get("retrieval_preferences", {}).get("inferred_preferred_sources", [])),
                    inferred_preferred_source_terms=list(data.get("retrieval_preferences", {}).get("inferred_preferred_source_terms", [])),
                    approved_example_count=int(data.get("retrieval_preferences", {}).get("approved_example_count", 0)),
                    updated_at=data.get("retrieval_preferences", {}).get("updated_at"),
                )
                if data.get("retrieval_preferences")
                else None
            ),
        )

    def submit_training_example(
        self,
        workspace: str,
        subject: str,
        prompt: str,
        answer: str,
        *,
        citations: Optional[list[dict]] = None,
        approved: bool = True,
        source: str = "tachy",
        audit_reference: Optional[str] = None,
        metadata: Optional[dict] = None,
    ) -> dict:
        return self._post(
            "/training/examples",
            {
                "workspace": workspace,
                "subject": subject,
                "examples": [
                    {
                        "prompt": prompt,
                        "answer": answer,
                        "citations": citations or [],
                        "approved": approved,
                        "source": source,
                        "audit_reference": audit_reference,
                        "metadata": metadata or {},
                    }
                ],
            },
        )

    def sync_knowledge(self, workspace: str) -> dict:
        return self._post("/knowledge/sync", {"workspace": workspace})
