#!/usr/bin/env python3
"""Archive historical Yaya/Tachy workflow artifacts while preserving auditability."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
import shutil


ROOT = Path(__file__).resolve().parents[1]
RUST_ROOT = ROOT / "rust"
STATE_PATH = RUST_ROOT / ".tachy" / "state.json"
ARCHIVE_ROOT = RUST_ROOT / ".tachy" / "archive" / "yaya_workflows"


def memo_version(path: Path) -> tuple[int, str]:
    stem = path.stem
    if "_v" in stem:
        try:
            return (int(stem.rsplit("_v", 1)[1]), stem)
        except ValueError:
            pass
    return (0, stem)


def latest_memo_file() -> Path | None:
    candidates = sorted(RUST_ROOT.glob("yaya_grounded_memo*.md"), key=memo_version)
    return candidates[-1] if candidates else None


def should_archive_agent(agent: dict) -> bool:
    template_name = (((agent.get("config") or {}).get("template") or {}).get("name"))
    status = (agent.get("status") or "").lower()
    summary = (agent.get("result_summary") or "").lower()

    if status == "failed" and "recovered after daemon restart" in summary:
        return True

    if template_name != "yaya-memo-writer":
        return False
    if status == "failed":
        return True
    noisy_markers = (
        "runtime error:",
        "https://",
        "written by yaya",
        "call write_file tool",
        "# references",
        "=====================================",
    )
    if any(marker in summary for marker in noisy_markers):
        return True
    return not summary.startswith("# legal expert memo")


def archive_history() -> dict:
    latest = latest_memo_file()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    archive_dir = ARCHIVE_ROOT / stamp
    archive_dir.mkdir(parents=True, exist_ok=True)

    manifest = {
        "created_at": stamp,
        "kept_latest_memo": latest.name if latest else None,
        "archived_memos": [],
        "archived_agents": [],
    }

    for memo_path in sorted(RUST_ROOT.glob("yaya_grounded_memo*.md"), key=memo_version):
        if latest and memo_path.resolve() == latest.resolve():
            continue
        destination = archive_dir / memo_path.name
        shutil.move(str(memo_path), destination)
        manifest["archived_memos"].append({"from": str(memo_path), "to": str(destination)})

    if latest:
        canonical = RUST_ROOT / "yaya_grounded_memo.md"
        latest_text = latest.read_text(encoding="utf-8")
        canonical.write_text(latest_text, encoding="utf-8")

    if STATE_PATH.exists():
        state = json.loads(STATE_PATH.read_text(encoding="utf-8"))
        agents = state.get("agents", {})
        archived_agents = {}
        for agent_id in list(agents):
            agent = agents[agent_id]
            if should_archive_agent(agent):
                archived_agents[agent_id] = agents.pop(agent_id)
                manifest["archived_agents"].append(agent_id)

        if archived_agents:
            (archive_dir / "archived_agents.json").write_text(
                json.dumps(archived_agents, indent=2, sort_keys=True),
                encoding="utf-8",
            )
            STATE_PATH.write_text(json.dumps(state, indent=2, sort_keys=True), encoding="utf-8")

    (archive_dir / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


def main() -> None:
    manifest = archive_history()
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
