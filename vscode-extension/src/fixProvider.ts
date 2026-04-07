import * as vscode from 'vscode';
import { TachyClient } from './client';

/**
 * Provides the "⚡ Fix with Tachy" code action on diagnostics (errors / warnings).
 * When the user invokes it, the error message + surrounding code are sent to the
 * chat sidebar as a /fix prompt — giving instant visual feedback while the agent works.
 */
export class TachyFixProvider implements vscode.CodeActionProvider {
    static readonly providedCodeActionKinds = [vscode.CodeActionKind.QuickFix];

    constructor(private readonly client: TachyClient) {}

    provideCodeActions(
        document: vscode.TextDocument,
        range: vscode.Range,
        context: vscode.CodeActionContext
    ): vscode.CodeAction[] {
        if (!context.diagnostics.length) {
            return [];
        }

        return context.diagnostics.map((diagnostic) => {
            const action = new vscode.CodeAction(
                `⚡ Fix with Tachy: ${diagnostic.message.slice(0, 60)}${diagnostic.message.length > 60 ? '…' : ''}`,
                vscode.CodeActionKind.QuickFix
            );
            action.command = {
                command: 'tachy.fixDiagnostic',
                title: 'Fix with Tachy',
                arguments: [document, diagnostic],
            };
            action.diagnostics = [diagnostic];
            action.isPreferred = false;
            return action;
        });
    }
}

/**
 * Registers the fix provider and the `tachy.fixDiagnostic` command.
 */
export function registerFixProvider(
    context: vscode.ExtensionContext,
    client: TachyClient,
    sendToChat: (text: string) => void
): void {
    context.subscriptions.push(
        vscode.languages.registerCodeActionsProvider(
            { scheme: 'file' },
            new TachyFixProvider(client),
            { providedCodeActionKinds: TachyFixProvider.providedCodeActionKinds }
        )
    );

    context.subscriptions.push(
        vscode.commands.registerCommand(
            'tachy.fixDiagnostic',
            (document: vscode.TextDocument, diagnostic: vscode.Diagnostic) => {
                const diagRange = diagnostic.range;
                // Include a few lines of context around the error
                const contextStart = Math.max(0, diagRange.start.line - 5);
                const contextEnd = Math.min(document.lineCount - 1, diagRange.end.line + 5);
                const code = document.getText(
                    new vscode.Range(
                        new vscode.Position(contextStart, 0),
                        new vscode.Position(contextEnd, 0)
                    )
                );
                const relPath = vscode.workspace.asRelativePath(document.uri);
                const prompt =
                    `/fix ${diagnostic.message}\n\n` +
                    `File: ${relPath} (lines ${contextStart + 1}–${contextEnd + 1})\n` +
                    `\`\`\`${document.languageId}\n${code}\n\`\`\``;

                // Show the chat panel then send the message
                vscode.commands.executeCommand('tachy.chatView.focus').then(() => {
                    sendToChat(prompt);
                });
            }
        )
    );
}
