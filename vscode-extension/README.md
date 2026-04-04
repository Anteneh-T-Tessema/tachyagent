# Tachy Autocomplete — VS Code Extension

AI-powered inline code completions from your local Tachy agent. Runs entirely on your machine via Ollama — no cloud, no data leaves your laptop.

## Setup

1. Install and start the Tachy daemon: `tachy serve`
2. Install this extension
3. Start typing — completions appear as ghost text

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `tachy.endpoint` | `http://localhost:7777` | Daemon URL |
| `tachy.apiKey` | (empty) | API key if auth is enabled |
| `tachy.model` | `gemma4:26b` | LLM model for completions |
| `tachy.enabled` | `true` | Enable/disable completions |
| `tachy.maxTokens` | `128` | Max tokens per completion |
| `tachy.debounceMs` | `300` | Debounce delay before requesting |

## Commands

- **Tachy: Toggle Autocomplete** — Enable/disable inline completions
- **Tachy: Check Daemon Health** — Verify the daemon is running

## How It Works

1. As you type, the extension captures ~60 lines of context before your cursor and ~20 lines after
2. It sends this to the local Tachy daemon with a fill-in-the-middle prompt
3. The daemon runs the LLM locally via Ollama
4. The completion appears as ghost text you can accept with Tab

## Development

```bash
npm install
npm run compile
# Press F5 in VS Code to launch Extension Development Host
```
