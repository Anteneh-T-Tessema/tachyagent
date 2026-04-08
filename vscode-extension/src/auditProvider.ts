/**
 * Audit Trail Panel — surfaces the Tachy daemon's audit log as:
 * 1. A VS Code TreeView in the sidebar.
 * 2. VS Code Diagnostics — policy violations appear as squiggles in the editor.
 */

import * as vscode from "vscode";
import { TachyClient } from "./client";

// ── Types ─────────────────────────────────────────────────────────────────────

interface AuditEvent {
  seq: number;
  timestamp: string;
  session_id: string;
  kind: string;
  severity: "info" | "warning" | "critical";
  message: string;
  agent_id?: string;
  tool_name?: string;
  model?: string;
}

interface PolicyViolation {
  file?: string;
  line?: number;
  rule: string;
  message: string;
  severity: "error" | "warning";
}

// ── Audit tree provider ────────────────────────────────────────────────────────

class AuditTreeItem extends vscode.TreeItem {
  constructor(label: string, state: vscode.TreeItemCollapsibleState, public readonly event?: AuditEvent) {
    super(label, state);
  }
}

export class AuditTreeProvider implements vscode.TreeDataProvider<AuditTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<AuditTreeItem | undefined>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  private events: AuditEvent[] = [];
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
      this.events = await this.client.getAuditLog(50);
      this._onDidChangeTreeData.fire(undefined);
    } catch { /* daemon offline */ }
  }

  getTreeItem(e: AuditTreeItem): vscode.TreeItem { return e; }

  getChildren(element?: AuditTreeItem): AuditTreeItem[] {
    if (!element) {
      if (this.events.length === 0) {
        return [new AuditTreeItem("No audit events", vscode.TreeItemCollapsibleState.None)];
      }
      return [...this.events].reverse().slice(0, 50).map((ev) => {
        const icon = severityIcon(ev.severity);
        const item = new AuditTreeItem(
          `${icon} [${ev.kind}] ${ev.message.slice(0, 80)}`,
          vscode.TreeItemCollapsibleState.Collapsed,
          ev,
        );
        item.description = ev.timestamp;
        item.tooltip = JSON.stringify(ev, null, 2);
        item.contextValue = "auditEvent";
        return item;
      });
    }

    if (element.event) {
      const ev = element.event;
      const details: AuditTreeItem[] = [
        new AuditTreeItem(`Seq: ${ev.seq}`, vscode.TreeItemCollapsibleState.None),
        new AuditTreeItem(`Session: ${ev.session_id}`, vscode.TreeItemCollapsibleState.None),
        new AuditTreeItem(`Severity: ${ev.severity}`, vscode.TreeItemCollapsibleState.None),
      ];
      if (ev.agent_id) details.push(new AuditTreeItem(`Agent: ${ev.agent_id}`, vscode.TreeItemCollapsibleState.None));
      if (ev.tool_name) details.push(new AuditTreeItem(`Tool: ${ev.tool_name}`, vscode.TreeItemCollapsibleState.None));
      if (ev.model) details.push(new AuditTreeItem(`Model: ${ev.model}`, vscode.TreeItemCollapsibleState.None));
      return details;
    }

    return [];
  }
}

// ── Policy diagnostic provider ─────────────────────────────────────────────────

export class PolicyDiagnosticProvider {
  private readonly collection: vscode.DiagnosticCollection;
  private pollTimer?: NodeJS.Timeout;

  constructor(private readonly client: TachyClient) {
    this.collection = vscode.languages.createDiagnosticCollection("tachy-policy");
  }

  startPolling(intervalMs: number) {
    this.refresh();
    this.pollTimer = setInterval(() => this.refresh(), intervalMs);
  }

  stopPolling() {
    if (this.pollTimer) clearInterval(this.pollTimer);
    this.collection.clear();
  }

  dispose() {
    this.stopPolling();
    this.collection.dispose();
  }

  async refresh() {
    const showInline = vscode.workspace
      .getConfiguration("tachy")
      .get<boolean>("showPolicyWarningsInline", true);

    if (!showInline) {
      this.collection.clear();
      return;
    }

    try {
      const violations: PolicyViolation[] = await this.client.getPolicyViolations();
      this.collection.clear();

      const byFile = new Map<string, vscode.Diagnostic[]>();

      for (const v of violations) {
        if (!v.file) continue;
        const range = new vscode.Range(
          Math.max(0, (v.line ?? 1) - 1), 0,
          Math.max(0, (v.line ?? 1) - 1), 200
        );
        const diag = new vscode.Diagnostic(
          range,
          `[Tachy Policy] ${v.rule}: ${v.message}`,
          v.severity === "error"
            ? vscode.DiagnosticSeverity.Error
            : vscode.DiagnosticSeverity.Warning,
        );
        diag.source = "tachy-policy";

        const uri = vscode.Uri.file(v.file);
        const existing = byFile.get(uri.toString()) ?? [];
        existing.push(diag);
        byFile.set(uri.toString(), existing);
      }

      for (const [uriStr, diags] of byFile.entries()) {
        this.collection.set(vscode.Uri.parse(uriStr), diags);
      }
    } catch { /* daemon offline */ }
  }
}

function severityIcon(severity: string): string {
  switch (severity) {
    case "critical": return "$(error)";
    case "warning": return "$(warning)";
    default: return "$(info)";
  }
}
