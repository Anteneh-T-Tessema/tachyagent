#!/bin/bash
# Tachy Installer — fully automatic, graceful degradation on every failure
# Usage: curl -fsSL https://tachy.dev/install.sh | bash

# Do NOT use set -e — we handle every error ourselves
REPO="your-org/tachy"  # TODO: replace with actual repo
BOLD="\033[1m"
GREEN="\033[32m"
YELLOW="\033[33m"
RED="\033[31m"
CYAN="\033[36m"
RESET="\033[0m"

info()  { echo -e "${CYAN}▸${RESET} $1"; }
ok()    { echo -e "${GREEN}✓${RESET} $1"; }
warn()  { echo -e "${YELLOW}⚠${RESET} $1"; }
err()   { echo -e "${RED}✗${RESET} $1"; }

# Track what succeeded for the final summary
TACHY_OK=false
OLLAMA_OK=false
MODEL_OK=false
WORKSPACE_OK=false
WARMUP_OK=false
MANUAL_STEPS=""

echo -e "${BOLD}⚡ Tachy Installer${RESET}"
echo ""

# ── Step 1: Detect platform ─────────────────────────────────────
OS=$(uname -s 2>/dev/null | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m 2>/dev/null)

case "$ARCH" in
  x86_64|amd64)  ARCH_NAME="x86_64" ;;
  aarch64|arm64)  ARCH_NAME="arm64" ;;
  *)
    err "Unsupported architecture: $ARCH"
    echo "  Tachy supports: x86_64, arm64"
    echo "  Download manually: https://github.com/${REPO}/releases"
    exit 1
    ;;
esac

case "$OS" in
  linux)   PLATFORM="linux" ;;
  darwin)  PLATFORM="macos" ;;
  mingw*|msys*|cygwin*)
    err "This script doesn't support Windows directly."
    echo "  Download the Windows binary from: https://github.com/${REPO}/releases"
    echo "  Then install Ollama from: https://ollama.com/download"
    exit 1
    ;;
  *)
    err "Unsupported OS: $OS"
    exit 1
    ;;
esac

info "Platform: ${PLATFORM}/${ARCH_NAME}"

# Detect RAM
RAM_GB=16  # safe default
if [ "$PLATFORM" = "macos" ]; then
  RAM_BYTES=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
  if [ "$RAM_BYTES" -gt 0 ] 2>/dev/null; then
    RAM_GB=$((RAM_BYTES / 1073741824))
  fi
elif [ "$PLATFORM" = "linux" ]; then
  RAM_KB=$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}')
  if [ -n "$RAM_KB" ] && [ "$RAM_KB" -gt 0 ] 2>/dev/null; then
    RAM_GB=$((RAM_KB / 1048576))
  fi
fi
info "RAM: ${RAM_GB} GB"

# Pick model based on RAM
if [ "$RAM_GB" -ge 32 ]; then
  MODEL="gemma4:26b"
  MODEL_DESC="Gemma 4 26B MoE (frontier quality)"
elif [ "$RAM_GB" -ge 16 ]; then
  MODEL="qwen3:8b"
  MODEL_DESC="Qwen3 8B (good balance)"
elif [ "$RAM_GB" -ge 8 ]; then
  MODEL="gemma4:e4b"
  MODEL_DESC="Gemma 4 E4B (fast, efficient)"
else
  MODEL="llama3.2:3b"
  MODEL_DESC="Llama 3.2 3B (lightweight)"
fi
info "Recommended model: ${MODEL} — ${MODEL_DESC}"
echo ""

# ── Step 2: Install Tachy binary ────────────────────────────────
INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR" 2>/dev/null

TARBALL="tachy-${PLATFORM}-${ARCH_NAME}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${TARBALL}"

info "Downloading Tachy..."
TMP_DIR=$(mktemp -d 2>/dev/null || echo "/tmp/tachy-install-$$")
mkdir -p "$TMP_DIR" 2>/dev/null

DOWNLOADED=false
if command -v curl &>/dev/null; then
  if curl -fsSL "$URL" -o "${TMP_DIR}/${TARBALL}" 2>/dev/null; then
    DOWNLOADED=true
  fi
elif command -v wget &>/dev/null; then
  if wget -q "$URL" -O "${TMP_DIR}/${TARBALL}" 2>/dev/null; then
    DOWNLOADED=true
  fi
fi

if [ "$DOWNLOADED" = true ] && [ -f "${TMP_DIR}/${TARBALL}" ]; then
  tar xzf "${TMP_DIR}/${TARBALL}" -C "${TMP_DIR}" 2>/dev/null
  if [ -f "${TMP_DIR}/tachy" ]; then
    mv "${TMP_DIR}/tachy" "${INSTALL_DIR}/tachy"
  elif [ -f "${TMP_DIR}/tachy-cli" ]; then
    mv "${TMP_DIR}/tachy-cli" "${INSTALL_DIR}/tachy"
  fi
  chmod +x "${INSTALL_DIR}/tachy" 2>/dev/null
fi
rm -rf "$TMP_DIR" 2>/dev/null

# Fallback: check for local build
if [ ! -x "${INSTALL_DIR}/tachy" ]; then
  if [ -f "./rust/target/release/tachy-cli" ]; then
    warn "GitHub release not found — using local build"
    cp "./rust/target/release/tachy-cli" "${INSTALL_DIR}/tachy"
    chmod +x "${INSTALL_DIR}/tachy"
  fi
fi

# Add to PATH
if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
  export PATH="${INSTALL_DIR}:$PATH"
  for RC in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.profile"; do
    if [ -f "$RC" ]; then
      grep -q '.local/bin' "$RC" 2>/dev/null || echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$RC"
      break
    fi
  done
fi

if [ -x "${INSTALL_DIR}/tachy" ]; then
  ok "Tachy installed to ${INSTALL_DIR}/tachy"
  TACHY_OK=true
else
  err "Tachy download failed"
  MANUAL_STEPS="${MANUAL_STEPS}\n  1. Download Tachy from https://github.com/${REPO}/releases"
  MANUAL_STEPS="${MANUAL_STEPS}\n     Extract and move to ~/.local/bin/tachy"
fi
echo ""

# ── Step 3: Install Ollama ──────────────────────────────────────
if curl -sf http://localhost:11434/api/version &>/dev/null; then
  OLLAMA_OK=true
  OLLAMA_VER=$(curl -sf http://localhost:11434/api/version 2>/dev/null | grep -o '"version":"[^"]*"' | cut -d'"' -f4)
  ok "Ollama already running (v${OLLAMA_VER:-unknown})"
elif command -v ollama &>/dev/null; then
  info "Ollama installed but not running — will start it"
else
  info "Installing Ollama..."
  INSTALL_SUCCEEDED=false

  if [ "$PLATFORM" = "linux" ]; then
    # Try official script (handles sudo internally)
    if curl -fsSL https://ollama.com/install.sh | sh 2>&1 | tail -3; then
      INSTALL_SUCCEEDED=true
    fi
    # Fallback: direct binary
    if [ "$INSTALL_SUCCEEDED" = false ]; then
      OLLAMA_ARCH="amd64"
      [ "$ARCH_NAME" = "arm64" ] && OLLAMA_ARCH="arm64"
      OLLAMA_BIN="$HOME/.local/bin/ollama"
      if curl -fsSL "https://ollama.com/download/ollama-linux-${OLLAMA_ARCH}" -o "$OLLAMA_BIN" 2>/dev/null; then
        chmod +x "$OLLAMA_BIN"
        INSTALL_SUCCEEDED=true
        warn "Installed Ollama to ${OLLAMA_BIN} (no systemd service)"
      fi
    fi
  elif [ "$PLATFORM" = "macos" ]; then
    # Try brew (check both ARM and Intel paths)
    BREW=""
    [ -x "/opt/homebrew/bin/brew" ] && BREW="/opt/homebrew/bin/brew"
    [ -x "/usr/local/bin/brew" ] && BREW="/usr/local/bin/brew"
    command -v brew &>/dev/null && BREW="brew"

    if [ -n "$BREW" ]; then
      if $BREW install ollama 2>&1 | tail -3; then
        INSTALL_SUCCEEDED=true
      fi
    fi
    # Fallback: download app
    if [ "$INSTALL_SUCCEEDED" = false ]; then
      if curl -fsSL https://ollama.com/download/Ollama-darwin.zip -o /tmp/ollama-dl.zip 2>/dev/null; then
        unzip -oq /tmp/ollama-dl.zip -d /tmp/ollama-app 2>/dev/null
        if [ -d "/tmp/ollama-app/Ollama.app" ]; then
          rm -rf /Applications/Ollama.app 2>/dev/null
          mv /tmp/ollama-app/Ollama.app /Applications/ 2>/dev/null && INSTALL_SUCCEEDED=true
        fi
        rm -rf /tmp/ollama-dl.zip /tmp/ollama-app 2>/dev/null
      fi
    fi
  fi

  if [ "$INSTALL_SUCCEEDED" = true ] || command -v ollama &>/dev/null; then
    ok "Ollama installed"
  else
    err "Ollama auto-install failed"
    MANUAL_STEPS="${MANUAL_STEPS}\n  • Install Ollama: https://ollama.com/download"
  fi
fi

# ── Step 4: Start Ollama server ─────────────────────────────────
if ! curl -sf http://localhost:11434/api/version &>/dev/null; then
  info "Starting Ollama server..."

  # Try platform-specific methods
  if [ "$PLATFORM" = "macos" ] && [ -d "/Applications/Ollama.app" ]; then
    open /Applications/Ollama.app 2>/dev/null
  fi
  if [ "$PLATFORM" = "linux" ]; then
    systemctl start ollama 2>/dev/null || sudo systemctl start ollama 2>/dev/null || true
  fi

  # Check if it started
  sleep 2
  if ! curl -sf http://localhost:11434/api/version &>/dev/null; then
    # Last resort: run directly in background
    if command -v ollama &>/dev/null; then
      nohup ollama serve >/dev/null 2>&1 &
      disown 2>/dev/null || true
    fi
  fi

  # Wait for server
  WAITED=0
  while [ $WAITED -lt 20 ]; do
    if curl -sf http://localhost:11434/api/version &>/dev/null; then
      OLLAMA_OK=true
      break
    fi
    sleep 1
    WAITED=$((WAITED + 1))
    # Show progress every 5 seconds
    if [ $((WAITED % 5)) -eq 0 ]; then
      info "Waiting for Ollama... (${WAITED}s)"
    fi
  done

  if [ "$OLLAMA_OK" = true ]; then
    ok "Ollama server running"
  else
    err "Ollama server didn't start"
    MANUAL_STEPS="${MANUAL_STEPS}\n  • Start Ollama: ollama serve"
  fi
else
  OLLAMA_OK=true
fi
echo ""

# ── Step 5: Pull model ──────────────────────────────────────────
if [ "$OLLAMA_OK" = true ]; then
  HAS_MODEL=$(curl -sf http://localhost:11434/api/tags 2>/dev/null | grep -c "\"${MODEL}\"" || echo 0)

  if [ "$HAS_MODEL" -gt 0 ] 2>/dev/null; then
    ok "Model ${MODEL} already available"
    MODEL_OK=true
  else
    info "Pulling ${MODEL} (${MODEL_DESC})..."
    info "This may take a few minutes depending on your connection..."
    if ollama pull "$MODEL" 2>&1; then
      # Verify it actually pulled
      HAS_MODEL=$(curl -sf http://localhost:11434/api/tags 2>/dev/null | grep -c "\"${MODEL}\"" || echo 0)
      if [ "$HAS_MODEL" -gt 0 ] 2>/dev/null; then
        ok "Model ${MODEL} ready"
        MODEL_OK=true
      else
        err "Model pull completed but model not found"
        MANUAL_STEPS="${MANUAL_STEPS}\n  • Pull model: ollama pull ${MODEL}"
      fi
    else
      err "Model pull failed (network issue or Ollama version too old)"
      warn "Try updating Ollama: https://ollama.com/download"
      MANUAL_STEPS="${MANUAL_STEPS}\n  • Pull model: ollama pull ${MODEL}"
    fi
  fi
else
  warn "Skipping model pull (Ollama not running)"
  MANUAL_STEPS="${MANUAL_STEPS}\n  • Start Ollama and pull model: ollama serve && ollama pull ${MODEL}"
fi
echo ""

# ── Step 6: Initialize workspace ────────────────────────────────
if [ "$TACHY_OK" = true ]; then
  info "Initializing workspace..."
  if "${INSTALL_DIR}/tachy" init 2>&1 | grep -qE "✓|Initialized"; then
    ok "Workspace ready"
    WORKSPACE_OK=true
  else
    warn "Workspace init had issues (may need write permission to current directory)"
    MANUAL_STEPS="${MANUAL_STEPS}\n  • Initialize workspace: tachy init"
  fi
else
  warn "Skipping workspace init (Tachy not installed)"
fi
echo ""

# ── Step 7: Warm up model ───────────────────────────────────────
if [ "$TACHY_OK" = true ] && [ "$MODEL_OK" = true ]; then
  info "Warming up model..."
  if timeout 120 "${INSTALL_DIR}/tachy" warmup "$MODEL" 2>&1; then
    WARMUP_OK=true
  else
    warn "Warmup timed out or failed (model will load on first use)"
  fi
else
  warn "Skipping warmup"
fi
echo ""

# ── Summary ─────────────────────────────────────────────────────
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"

if [ "$TACHY_OK" = true ] && [ "$OLLAMA_OK" = true ] && [ "$MODEL_OK" = true ]; then
  echo -e "${GREEN}${BOLD}  ✓ Tachy is ready!${RESET}"
  echo ""
  echo -e "  ${BOLD}tachy${RESET}                    Interactive REPL"
  echo -e "  ${BOLD}tachy serve${RESET}              Web UI at http://localhost:7777"
  echo -e "  ${BOLD}tachy doctor${RESET}             Check system status"
  echo ""
  echo -e "  Model: ${CYAN}${MODEL}${RESET} (${RAM_GB} GB RAM detected)"
  echo -e "  7-day free trial — no credit card required"
elif [ "$TACHY_OK" = true ]; then
  echo -e "${YELLOW}${BOLD}  ⚠ Tachy installed but needs manual steps:${RESET}"
  echo -e "$MANUAL_STEPS"
  echo ""
  echo -e "  After fixing the above, run: ${BOLD}tachy setup${RESET}"
else
  echo -e "${RED}${BOLD}  ✗ Installation incomplete. Manual steps needed:${RESET}"
  echo -e "$MANUAL_STEPS"
  echo ""
  echo -e "  Full instructions: https://github.com/${REPO}#quick-start"
fi

echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
