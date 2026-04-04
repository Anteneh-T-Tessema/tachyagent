# Tachy — VS Code Extension

Local AI coding agent powered by Gemma 4 + Ollama. Everything runs on your machine.

## Features

- **Chat panel** in the sidebar — ask questions, get code help
- **Right-click actions** — Explain Selection, Fix Selection, Review File
- **Status bar** — shows connection status to the Tachy daemon
- **Zero cloud** — all inference runs locally via Ollama

## Setup

1. Install Tachy CLI: `curl -fsSL https://tachy.dev/install.sh | sh`
2. Start the daemon: `tachy serve`
3. Install this extension
4. The chat panel appears in the sidebar (robot icon)

## Commands

- `Tachy: New Chat` — start a fresh conversation
- `Tachy: Review Current File` — AI code review of the active file
- `Tachy: Explain Selection` — explain selected code
- `Tachy: Fix Selection` — fix issues in selected code
- `Tachy: Start Daemon` — start `tachy serve` in a terminal

## Configuration

- `tachy.daemonUrl` — daemon URL (default: `http://127.0.0.1:7777`)
- `tachy.model` — default model (default: `gemma4:26b`)
- `tachy.autoStart` — auto-start daemon on VS Code launch
