import * as vscode from 'vscode';
import { TachyChatProvider } from './chatProvider';
import { TachyClient } from './client';

let client: TachyClient;

export function activate(context: vscode.ExtensionContext) {
    const config = vscode.workspace.getConfiguration('tachy');
    const daemonUrl = config.get<string>('daemonUrl', 'http://127.0.0.1:7777');
    const defaultModel = config.get<string>('model', 'gemma4:26b');

    client = new TachyClient(daemonUrl);

    // Register the chat webview provider
    const chatProvider = new TachyChatProvider(context.extensionUri, client, defaultModel);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider('tachy.chat', chatProvider)
    );

    // Register commands
    context.subscriptions.push(
        vscode.commands.registerCommand('tachy.newChat', () => {
            chatProvider.newChat();
        })
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('tachy.reviewFile', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) { return; }
            const filePath = editor.document.uri.fsPath;
            const content = editor.document.getText();
            const prompt = `Review this file for bugs, security issues, and improvements:\n\nFile: ${filePath}\n\`\`\`\n${content.substring(0, 8000)}\n\`\`\``;
            chatProvider.sendMessage(prompt);
        })
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('tachy.explainSelection', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) { return; }
            const selection = editor.document.getText(editor.selection);
            if (!selection) { return; }
            chatProvider.sendMessage(`Explain this code:\n\`\`\`\n${selection}\n\`\`\``);
        })
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('tachy.fixSelection', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) { return; }
            const selection = editor.document.getText(editor.selection);
            if (!selection) { return; }
            chatProvider.sendMessage(`Fix any issues in this code and explain what you changed:\n\`\`\`\n${selection}\n\`\`\``);
        })
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('tachy.startDaemon', async () => {
            const terminal = vscode.window.createTerminal('Tachy Daemon');
            terminal.sendText('tachy serve');
            terminal.show();
        })
    );

    // Check daemon connection on startup
    checkDaemonConnection(client);

    // Status bar item
    const statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBar.text = '$(hubot) Tachy';
    statusBar.tooltip = 'Tachy — Local AI Coding Agent';
    statusBar.command = 'tachy.newChat';
    statusBar.show();
    context.subscriptions.push(statusBar);

    // Update status bar with connection status
    setInterval(async () => {
        const health = await client.health();
        if (health) {
            statusBar.text = `$(hubot) Tachy ✓`;
            statusBar.tooltip = `Connected — ${health.models} models`;
        } else {
            statusBar.text = `$(hubot) Tachy ✗`;
            statusBar.tooltip = 'Disconnected — run: tachy serve';
        }
    }, 30000);
}

async function checkDaemonConnection(client: TachyClient) {
    const health = await client.health();
    if (!health) {
        const action = await vscode.window.showWarningMessage(
            'Tachy daemon is not running. Start it?',
            'Start Daemon', 'Dismiss'
        );
        if (action === 'Start Daemon') {
            vscode.commands.executeCommand('tachy.startDaemon');
        }
    }
}

export function deactivate() {}
