#!/usr/bin/env bash
# One-line Tachy installer (E3)
#
# Usage (one-liner):
#   curl -sSfL https://tachy.dev/install | bash
#   -or-
#   bash install.sh [--model <model>] [--no-ollama] [--prefix <dir>]
#
# What this does:
#   1. Detects OS/arch
#   2. Optionally installs Ollama (local LLM server)
#   3. Downloads and installs the tachy binary
#   4. Initialises a starter workspace (.tachy/)
#   5. Prints a "you're ready" summary

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────────────────
TACHY_VERSION="${TACHY_VERSION:-latest}"
INSTALL_PREFIX="${INSTALL_PREFIX:-/usr/local/bin}"
DEFAULT_MODEL="${DEFAULT_MODEL:-gemma4:26b}"
INSTALL_OLLAMA=true
GITHUB_REPO="tachy-dev/tachy"
TACHY_DIR=".tachy"

# ── Colour helpers ─────────────────────────────────────────────────────────
if [ -t 1 ]; then
  RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
  CYAN='\033[0;36m'; BOLD='\033[1m'; RESET='\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; CYAN=''; BOLD=''; RESET=''
fi

info()    { printf "${CYAN}[tachy]${RESET} %s\n" "$*"; }
success() { printf "${GREEN}[tachy]${RESET} ✓ %s\n" "$*"; }
warn()    { printf "${YELLOW}[tachy]${RESET} ⚠ %s\n" "$*"; }
die()     { printf "${RED}[tachy]${RESET} ✗ %s\n" "$*" >&2; exit 1; }

# ── Argument parsing ────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --model)        DEFAULT_MODEL="$2"; shift 2 ;;
    --model=*)      DEFAULT_MODEL="${1#*=}"; shift ;;
    --prefix)       INSTALL_PREFIX="$2"; shift 2 ;;
    --prefix=*)     INSTALL_PREFIX="${1#*=}"; shift ;;
    --no-ollama)    INSTALL_OLLAMA=false; shift ;;
    --version)      TACHY_VERSION="$2"; shift 2 ;;
    -h|--help)
      printf "Usage: bash install.sh [--model MODEL] [--prefix DIR] [--no-ollama] [--version VER]\n"
      exit 0 ;;
    *) die "Unknown option: $1" ;;
  esac
done

# ── Detect OS / arch ────────────────────────────────────────────────────────
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) die "Unsupported architecture: $ARCH" ;;
esac
case "$OS" in
  linux)  PLATFORM="linux-${ARCH}" ;;
  darwin) PLATFORM="darwin-${ARCH}" ;;
  *) die "Unsupported OS: $OS (Windows: use WSL2 or the .msi installer)" ;;
esac

printf "\n${BOLD}⚡ Tachy Installer${RESET}\n"
printf "   Platform : %s\n" "$PLATFORM"
printf "   Version  : %s\n" "$TACHY_VERSION"
printf "   Model    : %s\n" "$DEFAULT_MODEL"
printf "   Prefix   : %s\n\n" "$INSTALL_PREFIX"

# ── Dependency check ────────────────────────────────────────────────────────
for cmd in curl; do
  command -v "$cmd" >/dev/null 2>&1 || die "Required tool not found: $cmd"
done

# ── Step 1: Install Ollama ──────────────────────────────────────────────────
if $INSTALL_OLLAMA; then
  if command -v ollama >/dev/null 2>&1; then
    success "Ollama already installed ($(ollama --version 2>/dev/null | head -1))"
  else
    info "Installing Ollama…"
    case "$OS" in
      linux)
        curl -fsSL https://ollama.com/install.sh | sh
        ;;
      darwin)
        if command -v brew >/dev/null 2>&1; then
          brew install ollama
        else
          warn "Homebrew not found — download Ollama from https://ollama.com/download"
        fi
        ;;
    esac
    if command -v ollama >/dev/null 2>&1; then
      success "Ollama installed"
    else
      warn "Ollama installation may have failed — install manually if needed"
    fi
  fi

  # Start Ollama in the background if not running
  if ! curl -sf http://localhost:11434/api/tags >/dev/null 2>&1; then
    info "Starting Ollama server…"
    ollama serve >/dev/null 2>&1 &
    sleep 3
    if curl -sf http://localhost:11434/api/tags >/dev/null 2>&1; then
      success "Ollama server started"
    else
      warn "Ollama server not yet responding — run 'ollama serve' manually"
    fi
  else
    success "Ollama server already running"
  fi

  # Pull default model
  if ! ollama list 2>/dev/null | grep -q "$DEFAULT_MODEL"; then
    info "Pulling model ${DEFAULT_MODEL} (this may take a few minutes)…"
    ollama pull "$DEFAULT_MODEL" && success "Model $DEFAULT_MODEL ready" \
      || warn "Model pull failed — run: ollama pull $DEFAULT_MODEL"
  else
    success "Model $DEFAULT_MODEL already available"
  fi
fi

# ── Step 2: Install tachy binary ────────────────────────────────────────────
BINARY_NAME="tachy"
if [ "$OS" = "linux" ]; then
  BINARY_NAME="tachy-${PLATFORM}"
fi

if [ "$TACHY_VERSION" = "latest" ]; then
  DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/${BINARY_NAME}"
else
  DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${TACHY_VERSION}/${BINARY_NAME}"
fi

INSTALL_TARGET="${INSTALL_PREFIX}/tachy"

# Check if we need sudo
if [ -w "$INSTALL_PREFIX" ]; then
  SUDO=""
else
  SUDO="sudo"
  info "Installing to ${INSTALL_PREFIX} (may prompt for sudo)"
fi

info "Downloading tachy ${TACHY_VERSION}…"
TMP="$(mktemp)"
if curl -fsSL --progress-bar "$DOWNLOAD_URL" -o "$TMP" 2>/dev/null; then
  chmod +x "$TMP"
  $SUDO mv "$TMP" "$INSTALL_TARGET"
  success "tachy installed to ${INSTALL_TARGET}"
else
  rm -f "$TMP"
  # Fallback: try to build from source if Cargo is present
  if command -v cargo >/dev/null 2>&1; then
    warn "Binary download failed — building from source…"
    # BASH_SOURCE[0] is empty when piped through curl | bash, fall back to $PWD
    if [ -n "${BASH_SOURCE[0]:-}" ] && [ "${BASH_SOURCE[0]}" != "bash" ]; then
      SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    else
      SCRIPT_DIR="$PWD"
    fi
    if [ -f "${SCRIPT_DIR}/rust/Cargo.toml" ]; then
      cargo build --manifest-path "${SCRIPT_DIR}/rust/Cargo.toml" \
        --release -p tachy-cli --quiet \
        && $SUDO cp "${SCRIPT_DIR}/rust/target/release/tachy" "$INSTALL_TARGET" \
        && success "tachy built and installed from source"
    else
      die "Cannot download binary and no Cargo.toml found for source build. Clone the repo first: git clone https://github.com/${GITHUB_REPO}"
    fi
  else
    die "Binary download failed and Cargo not found. Download manually from: https://tachy.dev/download"
  fi
fi

# ── Step 3: Verify installation ─────────────────────────────────────────────
if command -v tachy >/dev/null 2>&1; then
  success "tachy is on PATH"
elif [ -x "$INSTALL_TARGET" ]; then
  warn "tachy installed but not on PATH — add to PATH: export PATH=\"${INSTALL_PREFIX}:\$PATH\""
fi

# ── Step 4: Workspace initialisation ────────────────────────────────────────
if [ ! -d "$TACHY_DIR" ]; then
  info "Initialising workspace…"
  mkdir -p "${TACHY_DIR}/sessions" "${TACHY_DIR}/memory"
  cat > "${TACHY_DIR}/config.json" <<JSON
{
  "model": "${DEFAULT_MODEL}",
  "api_listen": "127.0.0.1:7777",
  "default_model": "${DEFAULT_MODEL}"
}
JSON
  success "Workspace initialised (${TACHY_DIR}/)"
else
  success "Workspace already initialised"
fi

# ── Step 5: Summary ─────────────────────────────────────────────────────────
printf "\n${BOLD}You're ready!${RESET}\n\n"
printf "  Start the REPL    : ${CYAN}tachy${RESET}\n"
printf "  One-shot prompt   : ${CYAN}tachy prompt \"fix the failing tests\"${RESET}\n"
printf "  Start daemon      : ${CYAN}tachy serve${RESET}\n"
printf "  Open web UI       : ${CYAN}open http://localhost:7777${RESET}\n"
printf "  Switch model      : ${CYAN}tachy models${RESET}\n"
printf "  Docs              : ${CYAN}https://tachy.dev/docs${RESET}\n\n"
