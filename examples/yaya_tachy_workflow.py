"""Simple end-to-end workflow that uses Yaya for grounded expertise and Tachy for governed execution."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re

from tachy import TachyClient, YayaClient


PROFILE_SETTINGS = {
    "default": {
        "template": "yaya-memo-writer",
        "title": "{subject} Expert Memo",
        "signal_heading": "Key Source Signals",
        "limits_heading": "Operational Limits",
        "tone_note": "Use ATX headings only, keep the tone neutral and operational, and do not add a references section.",
    },
    "finance": {
        "template": "yaya-finance-brief-writer",
        "title": "Finance Expert Brief",
        "signal_heading": "Control Signals",
        "limits_heading": "Escalations And Limits",
        "tone_note": "Use ATX headings only, keep the tone board-ready and control-focused, and do not add a references section.",
    },
}


def profile_config(profile: str, subject: str) -> dict:
    config = dict(PROFILE_SETTINGS.get(profile, PROFILE_SETTINGS["default"]))
    config["title"] = config["title"].format(subject=subject.title())
    return config


def build_tachy_prompt(subject: str, question: str, expert_answer: str, citations: list[dict], output_path: str) -> str:
    return build_tachy_prompt_for_profile("default", subject, question, expert_answer, citations, output_path)


def build_tachy_prompt_for_profile(
    profile: str,
    subject: str,
    question: str,
    expert_answer: str,
    citations: list[dict],
    output_path: str,
) -> str:
    config = profile_config(profile, subject)
    citation_lines = "\n".join(
        f"- {item.get('label', 'citation')} -> {item.get('source', 'unknown source')}"
        for item in citations
    ) or "- No citations returned"

    return (
        f"You are preparing a regulated-team deliverable for the {subject} domain.\n\n"
        f"REQUIRED_OUTPUT_PATH: {output_path}\n\n"
        f"User request:\n{question}\n\n"
        f"Grounded expert guidance from Yaya:\n{expert_answer}\n\n"
        f"Supporting citations:\n{citation_lines}\n\n"
        f"Create a concise markdown memo at {output_path}.\n"
        "Use this exact section order:\n"
        f"# {config['title']}\n"
        "## Executive Summary\n"
        f"## {config['signal_heading']}\n"
        f"## {config['limits_heading']}\n\n"
        f"{config['tone_note']}"
    )


def normalize_workflow_output(text: str) -> str:
    marker = "\nCall write_file tool with the final markdown memo:"
    if marker in text:
        text = text.split(marker, 1)[0]
    return text.rstrip() + "\n"


def strip_markdown_links(text: str) -> str:
    return re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r"\1", text)


def _extract_section(text: str, heading: str) -> str:
    pattern = rf"^##\s+{re.escape(heading)}\s*$\n(?P<body>.*?)(?=^##\s+|\Z)"
    match = re.search(pattern, text.strip(), flags=re.MULTILINE | re.DOTALL)
    if not match:
        return ""
    return match.group("body").strip()


def _bulletize(text: str) -> list[str]:
    lines = []
    for raw in text.splitlines():
        cleaned = raw.strip()
        if not cleaned:
            continue
        cleaned = re.sub(r"^[*-]\s*", "", cleaned)
        lines.append(cleaned)
    if not lines and text.strip():
        lines = [text.strip()]
    return lines


def citation_line(item: dict) -> str:
    source = item.get("source", "unknown source")
    label = item.get("label") or source
    chunk = item.get("chunk")
    page = item.get("page")
    location = []
    if chunk is not None:
        location.append(f"chunk {chunk}")
    if page is not None:
        location.append(f"page {page}")
    if location and not any(token in label.lower() for token in ("chunk", "page")):
        suffix = f" ({', '.join(location)})"
    else:
        suffix = ""
    return f"- `{label}` -> `{source}`{suffix}"


def normalize_memo_markdown(text: str, citations: list[dict]) -> str:
    text = normalize_workflow_output(strip_markdown_links(text)).strip()
    text = re.sub(r"(?m)^[=-]{3,}\s*$", "", text)
    text = re.sub(r"\n#{1,6}\s*(References|Sources|Supporting Citations)\b[\s\S]*$", "", text, flags=re.IGNORECASE)
    text = re.sub(r"\n\*?Written by Yaya.*$", "", text, flags=re.IGNORECASE)
    text = text.rstrip()
    citation_block = "\n".join(citation_line(item) for item in citations) or "- No citations provided"
    return f"{text}\n\n## Supporting Citations\n{citation_block}\n"


def build_canonical_memo(
    subject: str,
    expert_answer: str,
    citations: list[dict],
    drafted_text: str | None = None,
    profile: str = "default",
) -> str:
    config = profile_config(profile, subject)
    expert_markdown = normalize_workflow_output(strip_markdown_links(expert_answer)).strip()
    direct_answer = _extract_section(expert_markdown, "Direct Answer")
    grounded_evidence = _extract_section(expert_markdown, "Grounded Evidence")
    limits = _extract_section(expert_markdown, "Limits")

    if not direct_answer and drafted_text:
        normalized_draft = normalize_workflow_output(strip_markdown_links(drafted_text)).strip()
        normalized_draft = re.sub(r"\n#{1,6}\s*(References|Sources|Supporting Citations)\b[\s\S]*$", "", normalized_draft, flags=re.IGNORECASE)
        direct_answer = normalized_draft.split("\n\n", 1)[0].strip()

    evidence_lines = _bulletize(grounded_evidence)
    if not evidence_lines:
        evidence_lines = [f"Retrieved sources support the workspace answer; see the cited materials below."]

    limit_lines = _bulletize(limits)
    if not limit_lines:
        limit_lines = ["No material gaps in the retrieved context."]

    citation_block = "\n".join(citation_line(item) for item in citations) or "- No citations provided"
    evidence_block = "\n".join(f"- {line}" for line in evidence_lines)
    limit_block = "\n".join(f"- {line}" for line in limit_lines)
    summary = direct_answer or "The retrieved workspace context does not support a more specific grounded answer yet."

    return (
        f"# {config['title']}\n\n"
        f"## Executive Summary\n{summary}\n\n"
        f"## {config['signal_heading']}\n{evidence_block}\n\n"
        f"## {config['limits_heading']}\n{limit_block}\n\n"
        f"## Supporting Citations\n{citation_block}\n"
    )


def resolve_output_path(raw_path: str) -> Path:
    path = Path(raw_path)
    if path.is_absolute():
        return path
    return (Path(__file__).resolve().parents[1] / "rust" / path).resolve()


def main() -> None:
    parser = argparse.ArgumentParser(description="Run a small Yaya + Tachy workflow.")
    parser.add_argument("--workspace", default="default")
    parser.add_argument("--subject", required=True)
    parser.add_argument("--question", required=True)
    parser.add_argument("--tachy-url", default="http://localhost:7777")
    parser.add_argument("--tachy-api-key", default=None)
    parser.add_argument("--yaya-url", default="http://localhost:8000")
    parser.add_argument("--yaya-api-key", default=None)
    parser.add_argument("--template", default=None)
    parser.add_argument("--model", default="llama3.2:3b")
    parser.add_argument("--workflow-profile", default="default")
    parser.add_argument("--output-path", default="yaya_grounded_memo.md")
    parser.add_argument("--agent-timeout", type=float, default=900.0)
    parser.add_argument("--poll-interval", type=float, default=2.0)
    parser.add_argument("--direct-yaya", action="store_true")
    parser.add_argument("--submit-training-example", action="store_true")
    args = parser.parse_args()

    output_path = resolve_output_path(args.output_path)
    workflow_profile = args.workflow_profile.strip().lower() or "default"
    selected_template = args.template or profile_config(workflow_profile, args.subject)["template"]

    tachy = TachyClient(args.tachy_url, api_key=args.tachy_api_key)
    yaya = YayaClient(args.yaya_url, api_key=args.yaya_api_key)

    if args.direct_yaya:
        expert = yaya.expert_chat(
            workspace=args.workspace,
            subject=args.subject,
            message=args.question,
            execution_context={"requested_output": str(output_path), "consumer": "tachy", "workflow_profile": workflow_profile},
            actor={"system": "tachy", "template": selected_template},
        )
    else:
        expert = tachy.yaya_chat(
            workspace=args.workspace,
            subject=args.subject,
            message=args.question,
            execution_context={"requested_output": str(output_path), "consumer": "tachy", "workflow_profile": workflow_profile},
            actor={"system": "tachy", "template": selected_template},
        )

    print("=== Yaya Expert Response ===")
    print(expert.response)
    print("\n=== Citations ===")
    for citation in expert.citations:
        print(f"{citation.label} | {citation.source}")
    if expert.retrieval_preferences:
        print("\n=== Retrieval Preferences ===")
        print(f"Strategy: {expert.retrieval_preferences.strategy}")
        if expert.retrieval_preferences.preferred_sources:
            print(f"Preferred sources: {', '.join(expert.retrieval_preferences.preferred_sources)}")
        if expert.retrieval_preferences.preferred_source_terms:
            print(f"Preferred terms: {', '.join(expert.retrieval_preferences.preferred_source_terms)}")

    tachy_prompt = build_tachy_prompt_for_profile(
        workflow_profile,
        args.subject,
        args.question,
        expert.response,
        [
            {
                "label": citation.label,
                "source": citation.source,
            }
            for citation in expert.citations
        ],
        str(output_path),
    )
    run = tachy.run_agent(selected_template, tachy_prompt, model=args.model)
    agent = tachy.wait_for_agent(
        run.agent_id,
        timeout=args.agent_timeout,
        poll_interval=args.poll_interval,
    )

    print("\n=== Tachy Agent Summary ===")
    print(agent.summary or "<no summary>")

    citations = [
        {
            "label": citation.label,
            "source": citation.source,
            "page": citation.page,
            "chunk": citation.chunk,
        }
        for citation in expert.citations
    ]

    final_content = build_canonical_memo(
        args.subject,
        expert.response,
        citations,
        drafted_text=agent.summary,
        profile=workflow_profile,
    )

    if agent.summary and not output_path.exists():
        output_path.write_text(final_content, encoding="utf-8")
        print(f"\n=== Workflow Output Saved ===\n{output_path}")
    elif output_path.exists():
        existing = output_path.read_text(encoding="utf-8")
        if existing.lower().startswith("runtime error:"):
            normalized = build_canonical_memo(args.subject, expert.response, citations, drafted_text=agent.summary, profile=workflow_profile)
        else:
            normalized = build_canonical_memo(
                args.subject,
                expert.response,
                citations,
                drafted_text=normalize_memo_markdown(existing, citations),
                profile=workflow_profile,
            )
        output_path.write_text(normalized, encoding="utf-8")
        print(f"\n=== Workflow Output Normalized ===\n{output_path}")

    if args.submit_training_example:
        if args.direct_yaya:
            payload = yaya.submit_training_example(
                workspace=args.workspace,
                subject=args.subject,
                prompt=args.question,
                answer=expert.response,
                citations=citations,
                approved=True,
                source="tachy_workflow",
                audit_reference=f"agent:{agent.agent_id}",
                metadata={
                    "tachy_template": selected_template,
                    "tachy_status": agent.status,
                    "tachy_summary": agent.summary,
                    "workflow_profile": workflow_profile,
                    "retrieval_preferences": (
                        {
                            "strategy": expert.retrieval_preferences.strategy,
                            "preferred_sources": expert.retrieval_preferences.preferred_sources,
                            "preferred_source_terms": expert.retrieval_preferences.preferred_source_terms,
                            "explicit_preferred_sources": expert.retrieval_preferences.explicit_preferred_sources,
                            "explicit_preferred_source_terms": expert.retrieval_preferences.explicit_preferred_source_terms,
                            "inferred_preferred_sources": expert.retrieval_preferences.inferred_preferred_sources,
                            "inferred_preferred_source_terms": expert.retrieval_preferences.inferred_preferred_source_terms,
                            "approved_example_count": expert.retrieval_preferences.approved_example_count,
                            "updated_at": expert.retrieval_preferences.updated_at,
                        }
                        if expert.retrieval_preferences
                        else {}
                    ),
                },
            )
        else:
            payload = tachy.submit_yaya_training_example(
                workspace=args.workspace,
                subject=args.subject,
                prompt=args.question,
                answer=expert.response,
                citations=citations,
                approved=True,
                source="tachy_workflow",
                audit_reference=f"agent:{agent.agent_id}",
                metadata={
                    "tachy_template": selected_template,
                    "tachy_status": agent.status,
                    "tachy_summary": agent.summary,
                    "workflow_profile": workflow_profile,
                    "retrieval_preferences": (
                        {
                            "strategy": expert.retrieval_preferences.strategy,
                            "preferred_sources": expert.retrieval_preferences.preferred_sources,
                            "preferred_source_terms": expert.retrieval_preferences.preferred_source_terms,
                            "explicit_preferred_sources": expert.retrieval_preferences.explicit_preferred_sources,
                            "explicit_preferred_source_terms": expert.retrieval_preferences.explicit_preferred_source_terms,
                            "inferred_preferred_sources": expert.retrieval_preferences.inferred_preferred_sources,
                            "inferred_preferred_source_terms": expert.retrieval_preferences.inferred_preferred_source_terms,
                            "approved_example_count": expert.retrieval_preferences.approved_example_count,
                            "updated_at": expert.retrieval_preferences.updated_at,
                        }
                        if expert.retrieval_preferences
                        else {}
                    ),
                },
            )
        print("\n=== Training Example Submitted ===")
        print(json.dumps(payload, indent=2))


if __name__ == "__main__":
    main()
