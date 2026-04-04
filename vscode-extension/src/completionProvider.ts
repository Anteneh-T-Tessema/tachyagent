import * as vscode from "vscode";
import { TachyClient } from "./client";

export class TachyCompletionProvider
  implements vscode.InlineCompletionItemProvider
{
  private client: TachyClient;
  private debounceTimer: ReturnType<typeof setTimeout> | undefined;
  private lastRequestId = 0;

  constructor(client: TachyClient) {
    this.client = client;
  }

  async provideInlineCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position,
    context: vscode.InlineCompletionContext,
    token: vscode.CancellationToken
  ): Promise<vscode.InlineCompletionItem[] | undefined> {
    const config = vscode.workspace.getConfiguration("tachy");
    if (!config.get<boolean>("enabled", true)) {
      return undefined;
    }

    // Debounce — cancel previous pending request
    const requestId = ++this.lastRequestId;
    const debounceMs = config.get<number>("debounceMs", 300);
    await new Promise<void>((resolve) => {
      if (this.debounceTimer) {
        clearTimeout(this.debounceTimer);
      }
      this.debounceTimer = setTimeout(resolve, debounceMs);
    });

    // Check if cancelled during debounce
    if (token.isCancellationRequested || requestId !== this.lastRequestId) {
      return undefined;
    }

    // Build context: prefix (up to 60 lines before cursor) and suffix (up to 20 lines after)
    const prefixRange = new vscode.Range(
      new vscode.Position(Math.max(0, position.line - 60), 0),
      position
    );
    const suffixRange = new vscode.Range(
      position,
      new vscode.Position(
        Math.min(document.lineCount - 1, position.line + 20),
        0
      )
    );

    const prefix = document.getText(prefixRange);
    const suffix = document.getText(suffixRange);

    // Skip if line is empty or just whitespace (avoid noisy completions)
    const currentLine = document.lineAt(position.line).text;
    if (currentLine.trim().length === 0 && position.character === 0) {
      return undefined;
    }

    const language = document.languageId;
    const filePath = vscode.workspace.asRelativePath(document.uri);
    const maxTokens = config.get<number>("maxTokens", 128);

    try {
      const completion = await this.client.complete({
        prefix,
        suffix,
        language,
        filePath,
        maxTokens,
      });

      if (
        token.isCancellationRequested ||
        requestId !== this.lastRequestId
      ) {
        return undefined;
      }

      if (!completion || completion.trim().length === 0) {
        return undefined;
      }

      // Clean up the completion — remove markdown fences if the model added them
      const cleaned = cleanCompletion(completion, language);
      if (!cleaned) {
        return undefined;
      }

      return [
        new vscode.InlineCompletionItem(
          cleaned,
          new vscode.Range(position, position)
        ),
      ];
    } catch {
      // Silently fail — don't interrupt the user's typing
      return undefined;
    }
  }
}

/**
 * Clean model output to extract just the code completion.
 * Strips markdown fences, explanatory text, and leading/trailing noise.
 */
function cleanCompletion(raw: string, language: string): string | undefined {
  let text = raw.trim();

  // Remove markdown code fences
  const fencePattern = new RegExp(
    "^```(?:" + language + ")?\\s*\\n?([\\s\\S]*?)\\n?```$"
  );
  const fenceMatch = text.match(fencePattern);
  if (fenceMatch) {
    text = fenceMatch[1];
  }

  // Remove leading ``` without closing
  if (text.startsWith("```")) {
    const firstNewline = text.indexOf("\n");
    if (firstNewline > 0) {
      text = text.substring(firstNewline + 1);
    }
  }

  // Remove trailing ```
  if (text.endsWith("```")) {
    text = text.substring(0, text.length - 3);
  }

  text = text.trimEnd();

  // Skip if it looks like an explanation rather than code
  if (
    text.startsWith("Here") ||
    text.startsWith("The ") ||
    text.startsWith("This ") ||
    text.startsWith("I ")
  ) {
    return undefined;
  }

  // Skip empty or too-short completions
  if (text.length < 2) {
    return undefined;
  }

  return text;
}
