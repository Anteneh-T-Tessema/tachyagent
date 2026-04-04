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
