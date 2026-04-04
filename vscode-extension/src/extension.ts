import * as vscode from "vscode";
import { TachyCompletionProvider } from "./completionProvider";
import { TachyClient } from "./client";

let completionProvider: vscode.Disposable | undefined;

export function activate(context: vscode.ExtensionContext) {
  const client = new TachyClient();

  // Register inline completion provider
  registerProvider(context, client);

  // Re-register when config changes
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("tachy")) {
        client.reloadConfig();
        if (completionProvider) {
          completionProvider.dispose();
        }
        registerProvider(context, client);
      }
    })
  );

  // Toggle command
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

  // Health check command
  context.subscriptions.push(
    vscode.commands.registerCommand("tachy.checkHealth", async () => {
      try {
        const health = await client.health();
        vscode.window.showInformationMessage(
          `Tachy daemon: ${health.status} (${health.models} models)`
        );
      } catch (e: any) {
        vscode.window.showErrorMessage(
          `Tachy daemon unreachable: ${e.message}`
        );
      }
    })
  );

  // Status bar
  const statusBar = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    100
  );
  statusBar.text = "$(sparkle) Tachy";
  statusBar.tooltip = "Tachy AI Autocomplete";
  statusBar.command = "tachy.toggleAutocomplete";
  statusBar.show();
  context.subscriptions.push(statusBar);
}

function registerProvider(
  context: vscode.ExtensionContext,
  client: TachyClient
) {
  const provider = new TachyCompletionProvider(client);
  completionProvider = vscode.languages.registerInlineCompletionItemProvider(
    { pattern: "**" },
    provider
  );
  context.subscriptions.push(completionProvider);
}

export function deactivate() {
  completionProvider?.dispose();
}
