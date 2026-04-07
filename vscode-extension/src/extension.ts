import * as vscode from "vscode";
import { TachyCompletionProvider } from "./completionProvider";
import { TachyChatProvider } from "./chatProvider";
import { TachyClient, ModelInfo } from "./client";
import { registerFixProvider } from "./fixProvider";

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
