"""Convenience wrapper for the packaged finance Yaya + Tachy workflow."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


DEFAULT_QUESTION = (
    "Prepare a finance brief covering close controls, escalation thresholds, "
    "cash forecast triggers, and board reporting expectations."
)


def main() -> None:
    workflow_script = Path(__file__).with_name("yaya_tachy_workflow.py")
    forwarded = sys.argv[1:]
    command = [
        sys.executable,
        str(workflow_script),
        "--subject",
        "finance",
        "--workflow-profile",
        "finance",
        "--question",
        DEFAULT_QUESTION,
        "--output-path",
        "finance_expert_brief.md",
    ]
    command.extend(forwarded)
    raise SystemExit(subprocess.call(command))


if __name__ == "__main__":
    main()
