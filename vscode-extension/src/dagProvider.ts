/**
 * DAG Panel — live execution graph of parallel/swarm runs.
 *
 * Surfaces the Tachy daemon's parallel run state as a VS Code TreeView.
 * Each run is a top-level node; each task is a child coloured by status.
 */

import * as vscode from "vscode";
import { TachyClient } from "./client";

// ── Types ─────────────────────────────────────────────────────────────────────

interface TaskNode {
  id: string;
  status: "pending" | "queued" | "running" | "completed" | "failed" | "cancelled";
  prompt: string;
  deps: string[];
  started_at?: number;
  completed_at?: number;
  result?: { success: boolean; summary: string; iterations: number; tool_invocations: number };
}

interface RunNode {
  id: string;
  status: "running" | "completed" | "partially_completed" | "failed" | "cancelled";
  tasks: TaskNode[];
  created_at: number;
  conflicts?: ConflictNode[];
}

interface ConflictNode {
  file: string;
  line: number;
  message: string;
  severity: string;
  suspected_tasks: string[];
}

// ── Tree item ─────────────────────────────────────────────────────────────────

class DagTreeItem extends vscode.TreeItem {
  constructor(
    label: string,
    public readonly collapsibleState: vscode.TreeItemCollapsibleState,
    public readonly runId?: string,
    public readonly task?: TaskNode,
    public readonly conflict?: ConflictNode,
  ) {
    super(label, collapsibleState);
  }
}

// ── Provider ──────────────────────────────────────────────────────────────────

export class DagTreeProvider implements vscode.TreeDataProvider<DagTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<DagTreeItem | undefined>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  private runs: RunNode[] = [];
  private pollTimer?: NodeJS.Timeout;

  constructor(private readonly client: TachyClient) {}

  startPolling(intervalMs: number) {
    this.refresh();
    this.pollTimer = setInterval(() => this.refresh(), intervalMs);
  }

  stopPolling() {
    if (this.pollTimer) clearInterval(this.pollTimer);
  }

  async refresh() {
    try {
      const [parallel, swarm] = await Promise.all([
        this.client.listParallelRuns(),
        this.client.listSwarmRuns(),
      ]);
      this.runs = [...(parallel || []), ...(swarm || [])];
      this._onDidChangeTreeData.fire(undefined);
    } catch {
      // Daemon offline — keep last known state
    }
  }

  getTreeItem(element: DagTreeItem): vscode.TreeItem {
    return element;
  }

  getChildren(element?: DagTreeItem): DagTreeItem[] {
    if (!element) {
      // Root level — one item per run
      if (this.runs.length === 0) {
        return [new DagTreeItem("No active runs", vscode.TreeItemCollapsibleState.None)];
      }
      return this.runs.map((run) => {
        const icon = runStatusIcon(run.status);
        const conflicts = run.conflicts?.length ?? 0;
        const label = `${icon} ${run.id}  [${run.tasks.length} tasks${conflicts ? ` ⚠ ${conflicts} conflicts` : ""}]`;
        const item = new DagTreeItem(
          label,
          vscode.TreeItemCollapsibleState.Collapsed,
          run.id,
        );
        item.description = run.status;
        item.tooltip = `Created: ${new Date(run.created_at * 1000).toLocaleString()}\nStatus: ${run.status}`;
        item.contextValue = "run";
        return item;
      });
    }

    if (element.runId && !element.task) {
      // Run-level children: tasks + conflicts section
      const run = this.runs.find((r) => r.id === element.runId);
      if (!run) return [];

      const taskItems = run.tasks.map((task) => {
        const icon = taskStatusIcon(task.status);
        const duration = task.started_at && task.completed_at
          ? ` (${task.completed_at - task.started_at}s)`
          : task.status === "running" ? " (running…)" : "";
        const item = new DagTreeItem(
          `${icon} ${task.id}${duration}`,
          vscode.TreeItemCollapsibleState.Collapsed,
          element.runId,
          task,
        );
        item.description = task.status;
        item.tooltip = `Prompt: ${task.prompt.slice(0, 120)}…\nDeps: ${task.deps.join(", ") || "none"}`;
        item.contextValue = "task";
        return item;
      });

      if (run.conflicts && run.conflicts.length > 0) {
        const conflictHeader = new DagTreeItem(
          `⚠ Semantic Conflicts (${run.conflicts.length})`,
          vscode.TreeItemCollapsibleState.Collapsed,
          element.runId,
        );
        conflictHeader.contextValue = "conflictHeader";
        return [...taskItems, conflictHeader];
      }

      return taskItems;
    }

    if (element.task) {
      // Task detail: result summary
      const task = element.task;
      const items: DagTreeItem[] = [];

      if (task.result) {
        const r = task.result;
        items.push(new DagTreeItem(`Summary: ${r.summary.slice(0, 80)}`, vscode.TreeItemCollapsibleState.None));
        items.push(new DagTreeItem(`Iterations: ${r.iterations}  Tools: ${r.tool_invocations}`, vscode.TreeItemCollapsibleState.None));
      }

      if (task.deps.length > 0) {
        items.push(new DagTreeItem(`Depends on: ${task.deps.join(", ")}`, vscode.TreeItemCollapsibleState.None));
      }

      if (items.length === 0) {
        items.push(new DagTreeItem("No result yet", vscode.TreeItemCollapsibleState.None));
      }

      return items;
    }

    // Conflict header children
    if (element.contextValue === "conflictHeader" && element.runId) {
      const run = this.runs.find((r) => r.id === element.runId);
      return (run?.conflicts ?? []).map((c) => {
        const item = new DagTreeItem(
          `${c.severity === "error" ? "✗" : "⚠"} ${c.file}:${c.line}  ${c.message.slice(0, 60)}`,
          vscode.TreeItemCollapsibleState.None,
          element.runId,
          undefined,
          c,
        );
        item.tooltip = `File: ${c.file}:${c.line}\n${c.message}\nSuspected: ${c.suspected_tasks.join(", ")}`;
        item.command = {
          command: "vscode.open",
          title: "Open file",
          arguments: [vscode.Uri.file(c.file), { selection: new vscode.Range(c.line - 1, 0, c.line - 1, 0) }],
        };
        return item;
      });
    }

    return [];
  }
}

function runStatusIcon(status: string): string {
  switch (status) {
    case "running": return "$(sync~spin)";
    case "completed": return "$(check)";
    case "partially_completed": return "$(warning)";
    case "failed": return "$(error)";
    case "cancelled": return "$(circle-slash)";
    default: return "$(clock)";
  }
}

function taskStatusIcon(status: string): string {
  switch (status) {
    case "running": return "$(sync~spin)";
    case "completed": return "$(pass)";
    case "failed": return "$(error)";
    case "cancelled": return "$(circle-slash)";
    case "queued": return "$(clock)";
    default: return "$(circle-outline)";
  }
}
