import * as vscode from "vscode";
import { TachyCompletionProvider } from "./completionProvider";
import { TachyChatProvider } from "./chatProvider";
import { TachyClient, ModelInfo } from "./client";
import { registerFixProvider } from "./fixProvider";
import { DagTreeProvider } from "./dagProvider";
import { AuditTreeProvider, PolicyDiagnosticProvider } from "./auditProvider";

let completionProvider: vscode.Disposable | undefined;
let statusBar: vscode.StatusBarItem;

export function activate(context: vscode.ExtensionContext) {
  const client = new TachyClient();

  // ── Inline completion provider ────────────────────────────────────────
  registerCompletionProvider(context, client);

  // ── Chat sidebar panel ────────────────────────────────────────────────
  const chatProvider = new TachyChatProvider(
    client,
    vscode.workspace.getConfiguration("tachy").get<string>("model", "gemma4:26b")
  );
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider("tachy.chatView", chatProvider)
  );

  // ── "Fix with Tachy" code action ──────────────────────────────────────
  registerFixProvider(context, client, (text) => chatProvider.sendMessage(text));

  // ── Status bar ────────────────────────────────────────────────────────
  statusBar = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    100
  );
  updateStatusBar(client);
  statusBar.command = "tachy.selectModel";
  statusBar.show();
  context.subscriptions.push(statusBar);

  // ── Commands ──────────────────────────────────────────────────────────

  // Toggle completions
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.toggleAutocomplete", () => {
      const config = vscode.workspace.getConfiguration("tachy");
      const current = config.get<boolean>("enabled", true);
      config.update("enabled", !current, vscode.ConfigurationTarget.Global);
      vscode.window.showInformationMessage(
        `Tachy Autocomplete ${!current ? "enabled" : "disabled"}`
      );
    })
  );

  // Health check
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.checkHealth", async () => {
      try {
        const health = await client.health();
        vscode.window.showInformationMessage(
          `Tachy daemon: ${health.status} · ${health.models} models registered`
        );
      } catch (e: any) {
        vscode.window.showErrorMessage(
          `Tachy daemon unreachable: ${e.message}. Run: tachy serve`
        );
      }
    })
  );

  // Model picker — shows all models from daemon, grouped by backend
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.selectModel", async () => {
      const models = await client.listModels().catch(() => [] as ModelInfo[]);
      const config = vscode.workspace.getConfiguration("tachy");
      const current = config.get<string>("model", "gemma4:26b");

      if (models.length === 0) {
        // Fallback static list when daemon is offline — all local Ollama, no cloud
        const staticModels = [
          // Gemma 4
          { label: "$(server) gemma4:31b",        description: "Gemma · Ollama · 256K ctx · frontier",         model: "gemma4:31b" },
          { label: "$(server) gemma4:26b",        description: "Gemma · Ollama · 256K ctx · default (MoE)",    model: "gemma4:26b" },
          { label: "$(server) gemma4:e4b",        description: "Gemma · Ollama · 128K ctx · fast edge",        model: "gemma4:e4b" },
          { label: "$(server) gemma4:e2b",        description: "Gemma · Ollama · 128K ctx · ultra-fast",       model: "gemma4:e2b" },
          // Qwen
          { label: "$(server) qwen3-coder:30b",   description: "Qwen  · Ollama · 32K ctx  · coding frontier",  model: "qwen3-coder:30b" },
          { label: "$(server) qwen3:8b",          description: "Qwen  · Ollama · 32K ctx  · general purpose",  model: "qwen3:8b" },
          { label: "$(server) qwen2.5-coder:14b", description: "Qwen  · Ollama · 32K ctx  · coding standard",  model: "qwen2.5-coder:14b" },
          { label: "$(server) qwen2.5-coder:7b",  description: "Qwen  · Ollama · 32K ctx  · coding fast",      model: "qwen2.5-coder:7b" },
          // Llama
          { label: "$(server) llama3.1:70b",      description: "Llama · Ollama · 128K ctx · frontier (40GB+)", model: "llama3.1:70b" },
          { label: "$(server) llama3.1:8b",       description: "Llama · Ollama · 128K ctx · standard",         model: "llama3.1:8b" },
          { label: "$(server) llama3.2:3b",       description: "Llama · Ollama · 128K ctx · fast",             model: "llama3.2:3b" },
          // Mistral
          { label: "$(server) codestral:22b",     description: "Mistral · Ollama · 32K ctx · fill-in-middle",  model: "codestral:22b" },
          { label: "$(server) mistral:7b",        description: "Mistral · Ollama · 32K ctx · general",         model: "mistral:7b" },
        ];
        const picked = await vscode.window.showQuickPick(staticModels, {
          title: "Select Tachy Model",
          placeHolder: `Current: ${current}`,
        });
        if (picked) {
          await config.update("model", picked.model, vscode.ConfigurationTarget.Global);
          client.reloadConfig();
          updateStatusBar(client);
          vscode.window.showInformationMessage(`Tachy model set to: ${picked.model}`);
        }
        return;
      }

      // Group models by backend
      const items = models.map((m) => {
        const isCloud = m.backend === "gemini" || m.backend === "openai_compat";
        const icon = isCloud ? "$(cloud)" : "$(server)";
        const ctxK = m.context_window >= 1_000_000
          ? `${(m.context_window / 1_000_000).toFixed(0)}M ctx`
          : `${Math.round(m.context_window / 1024)}K ctx`;
        const check = m.name === current ? "$(check) " : "";
        return {
          label: `${check}${icon} ${m.name}`,
          description: `${m.tier} · ${ctxK}${m.notes ? " · " + m.notes : ""}`,
          model: m.name,
        };
      });

      const picked = await vscode.window.showQuickPick(items, {
        title: "Select Tachy Model",
        placeHolder: `Current: ${current}`,
        matchOnDescription: true,
      });

      if (picked) {
        await config.update("model", picked.model, vscode.ConfigurationTarget.Global);
        client.reloadConfig();
        updateStatusBar(client);
        vscode.window.showInformationMessage(`Tachy model set to: ${picked.model}`);
      }
    })
  );

  // Explain selection — sends selected code to chat panel
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.explainSelection", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        vscode.window.showWarningMessage("No active editor");
        return;
      }
      const selection = editor.document.getText(editor.selection);
      if (!selection.trim()) {
        vscode.window.showWarningMessage("Select some code first");
        return;
      }
      const lang = editor.document.languageId;
      const prompt = `Explain this ${lang} code:\n\`\`\`${lang}\n${selection}\n\`\`\``;
      await vscode.commands.executeCommand("tachy.chatView.focus");
      chatProvider.sendMessage(prompt);
    })
  );

  // Fix/refactor selection
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.fixSelection", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        vscode.window.showWarningMessage("No active editor");
        return;
      }
      const selection = editor.document.getText(editor.selection);
      if (!selection.trim()) {
        vscode.window.showWarningMessage("Select some code first");
        return;
      }
      const lang = editor.document.languageId;
      const prompt = `Find and fix bugs in this ${lang} code. Show the corrected version:\n\`\`\`${lang}\n${selection}\n\`\`\``;
      await vscode.commands.executeCommand("tachy.chatView.focus");
      chatProvider.sendMessage(prompt);
    })
  );

  // ── DAG panel (execution graph) ───────────────────────────────────────
  const pollMs = vscode.workspace
    .getConfiguration("tachy")
    .get<number>("pollIntervalMs", 3000);

  const dagProvider = new DagTreeProvider(client);
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("tachy.dagView", dagProvider)
  );
  dagProvider.startPolling(pollMs);

  // ── Audit trail panel ─────────────────────────────────────────────────
  const auditProvider = new AuditTreeProvider(client);
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("tachy.auditView", auditProvider)
  );
  auditProvider.startPolling(pollMs);

  // ── Policy diagnostic squiggles ───────────────────────────────────────
  const policyProvider = new PolicyDiagnosticProvider(client);
  policyProvider.startPolling(pollMs);
  context.subscriptions.push({ dispose: () => policyProvider.dispose() });

  // ── Run Swarm Refactor on open files ──────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.runSwarm", async () => {
      const goal = await vscode.window.showInputBox({
        prompt: "Swarm goal (e.g. 'Add structured logging to all HTTP handlers')",
        placeHolder: "Describe the refactor goal…",
      });
      if (!goal) { return; }

      // Use open file paths as the target set, falling back to workspace files
      const openFiles = vscode.workspace.textDocuments
        .filter((d) => !d.isUntitled && d.uri.scheme === "file")
        .map((d) => vscode.workspace.asRelativePath(d.uri));

      const files = openFiles.length > 0 ? openFiles : [];
      if (files.length === 0) {
        vscode.window.showWarningMessage("Open the files you want to swarm-refactor first.");
        return;
      }

      vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: `Tachy Swarm: ${goal}`, cancellable: false },
        async (progress) => {
          progress.report({ message: `Planning DAG for ${files.length} files…` });
          try {
            const runId = await client.startSwarmRun(goal, files);
            progress.report({ message: `Run ${runId} started — watch the DAG panel` });
            dagProvider.refresh();
            vscode.window.showInformationMessage(`Swarm run ${runId} started. Monitor in the DAG panel.`);
          } catch (e: any) {
            vscode.window.showErrorMessage(`Swarm failed: ${e.message}`);
          }
        }
      );
    })
  );

  // ── Validate policy ───────────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.validatePolicy", async () => {
      const result = await client.validatePolicy();
      if (result.error) {
        vscode.window.showErrorMessage(`Policy error: ${result.error}`);
      } else {
        const violations = result.violations?.length ?? 0;
        vscode.window.showInformationMessage(
          violations === 0
            ? "Policy valid — no violations found."
            : `Policy has ${violations} violation(s). Check the Audit Trail panel.`
        );
        policyProvider.refresh();
        auditProvider.refresh();
      }
    })
  );

  // ── Follow a teammate's active mission session ────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.followSession", async () => {
      const smolagentEndpoint = vscode.workspace
        .getConfiguration("tachy")
        .get<string>("smolagentEndpoint", "http://localhost:8100");

      const sessionId = await vscode.window.showInputBox({
        prompt: "Session ID to follow (leave blank to follow all active missions)",
        placeHolder: "mission-abc123 or empty for broadcast feed",
      });
      if (sessionId === undefined) { return; } // cancelled

      const developer = vscode.env.machineId.slice(0, 8);
      const streamUrl = `${smolagentEndpoint}/sessions/live`;

      // Join endpoint announces presence and returns recent history
      try {
        const joinResp = await fetch(`${smolagentEndpoint}/sessions/join`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ session_id: sessionId, developer }),
        });
        const joinData = await joinResp.json() as { history_events?: number };
        const historyCount = joinData.history_events ?? 0;
        vscode.window.showInformationMessage(
          `Following session${sessionId ? ` "${sessionId}"` : " (all)"}. ` +
          `${historyCount} recent events replayed. DAG panel updating…`
        );
      } catch {
        vscode.window.showWarningMessage(
          `SmolAgent not reachable at ${smolagentEndpoint} — is webhook_receiver.py running?`
        );
        return;
      }

      // Subscribe the DAG panel to the SSE stream so it updates in real time.
      // The dagProvider polls TachyCode directly; here we additionally forward
      // SmolAgent mission events (from teammates) into the same refresh cycle.
      dagProvider.followExternalStream(streamUrl);

      vscode.window.showInformationMessage(
        `DAG panel now streaming from ${streamUrl}. Teammate actions appear in real time.`
      );
    })
  );

  // ── Open web dashboard ────────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.openDashboard", () => {
      const url = vscode.workspace
        .getConfiguration("tachy")
        .get<string>("endpoint", "http://localhost:7777");
      vscode.env.openExternal(vscode.Uri.parse(url));
    })
  );

  // Reload config + re-register providers when settings change
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("tachy")) {
        client.reloadConfig();
        updateStatusBar(client);
        if (completionProvider) {
          completionProvider.dispose();
        }
        registerCompletionProvider(context, client);
      }
    })
  );
}

function registerCompletionProvider(
  context: vscode.ExtensionContext,
  client: TachyClient
) {
  const provider = new TachyCompletionProvider(client);
  provider.onCompletionSuccess = (latencyMs) => {
    updateStatusBar(client, latencyMs);
  };
  completionProvider = vscode.languages.registerInlineCompletionItemProvider(
    { pattern: "**" },
    provider
  );
  context.subscriptions.push(completionProvider);
}

function updateStatusBar(client: TachyClient, latencyMs?: number) {
  const model = client.getModel();
  const latency = latencyMs ?? client.getLastLatencyMs();
  const latencyStr = latency > 0 ? ` | ${latency}ms` : "";
  statusBar.text = `⚡ Tachy: ${model}${latencyStr}`;
  statusBar.tooltip = `Tachy · ${model} · local Ollama${latency > 0 ? ` · last completion ${latency}ms` : ""} · click to change model`;
}

export function deactivate() {
  completionProvider?.dispose();
}
