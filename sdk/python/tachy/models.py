"""Data models for the Tachy SDK."""

from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional


@dataclass
class Model:
    name: str
    backend: str
    context_window: int = 0


@dataclass
class Template:
    name: str
    model: str
    description: str = ""
    allowed_tools: list[str] = field(default_factory=list)


@dataclass
class Agent:
    agent_id: str
    template: str
    status: str
    iterations: int = 0
    tool_invocations: int = 0
    summary: Optional[str] = None


@dataclass
class AgentRun:
    agent_id: str
    status: str
    message: str = ""


@dataclass
class ParallelTask:
    template: str
    prompt: str
    model: Optional[str] = None
    deps: list[str] = field(default_factory=list)
    priority: int = 5


@dataclass
class ParallelRun:
    run_id: str
    status: str
    task_count: int = 0
    tasks: list[dict] = field(default_factory=list)


@dataclass
class PendingApproval:
    type: str
    id: str
    reason: str = ""
    file_path: Optional[str] = None
    agent_id: Optional[str] = None


@dataclass
class FileLock:
    file: str
    agent_id: str


@dataclass
class YayaCitation:
    source: str
    label: str
    page: Optional[int] = None
    chunk: Optional[int] = None
    score: Optional[float] = None
    semantic_score: Optional[float] = None
    lexical_score: Optional[float] = None
    retrieval_mode: Optional[str] = None


@dataclass
class YayaExpert:
    workspace: str
    subject: str
    active_version: Optional[str] = None
    model_path: Optional[str] = None
    latest_evaluation_passed: Optional[bool] = None
    latest_evaluation_version: Optional[str] = None
    latest_trained_at: Optional[str] = None


@dataclass
class YayaRetrievalPreferences:
    strategy: str = "workspace_wide"
    preferred_sources: list[str] = field(default_factory=list)
    preferred_source_terms: list[str] = field(default_factory=list)
    explicit_preferred_sources: list[str] = field(default_factory=list)
    explicit_preferred_source_terms: list[str] = field(default_factory=list)
    inferred_preferred_sources: list[str] = field(default_factory=list)
    inferred_preferred_source_terms: list[str] = field(default_factory=list)
    approved_example_count: int = 0
    updated_at: Optional[str] = None


@dataclass
class YayaExpertResponse:
    workspace: str
    subject: str
    response: str
    citations: list[YayaCitation] = field(default_factory=list)
    model_type: str = "expert"
    used_fallback: bool = False
    retrieval_mode: str = "none"
    grounded: bool = False
    retrieval_preferences: Optional[YayaRetrievalPreferences] = None
