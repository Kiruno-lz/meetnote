#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

usage() {
  cat <<'EOF'
Usage: script/dev.sh [--check|--frontend|--tauri]

  --check     Verify environment and repository structure.
  --frontend  Start Bun frontend dev server (http://localhost:1420).
  --tauri     Start Tauri desktop app in dev mode (requires cargo-tauri).

This script never installs dependencies or downloads models.

Setup:
  bun install (in frontend/) to install dev deps
  cargo install tauri-cli --version "^2" to install Tauri CLI
EOF
}

status_ok()    { printf '[OK]   %s\n' "$1"; }
status_warn()  { printf '[WARN] %s\n' "$1"; }
status_info()  { printf '[INFO] %s\n' "$1"; }
status_error() { printf '[ERROR] %s\n' "$1" >&2; }

check_command() {
  local cmd="$1" label="$2"
  if command -v "$cmd" >/dev/null 2>&1; then
    status_ok "$label: $(command -v "$cmd")"
  else
    status_warn "$label: not found"
  fi
}

require_command() {
  local cmd="$1" label="$2" hint="${3:-}"
  if command -v "$cmd" >/dev/null 2>&1; then
    status_ok "$label: $(command -v "$cmd")"
  else
    status_error "$label: not found${hint:+ — $hint}"
    return 1
  fi
}

check_path() {
  local path="$1" label="$2"
  if [ -e "$path" ]; then
    status_ok "$label"
  else
    status_warn "$label: missing"
  fi
}

check_tauri_cli() {
  if cargo tauri --version >/dev/null 2>&1; then
    status_ok "cargo-tauri: $(cargo tauri --version)"
  else
    status_warn "cargo-tauri: not installed (run: cargo install tauri-cli --version \"^2\")"
  fi
}

run_checks() {
  status_info "=== meetnote environment check ==="

  status_info "Runtime tools:"
  check_command bun  "bun"
  check_command node "node"
  check_command cargo "cargo"
  check_command rustc "rustc"
  check_command python3 "python3"
  check_command uv "uv"
  check_tauri_cli

  status_info "Repository structure:"
  check_path "README.md"                 "README.md"
  check_path "PLAN.md"                   "PLAN.md"
  check_path "frontend/package.json"     "frontend/package.json"
  check_path "frontend/src/main.ts"      "frontend/src/main.ts"
  check_path "frontend/src/mock-pipeline.ts" "frontend/src/mock-pipeline.ts"
  check_path "src-tauri/Cargo.toml"      "src-tauri/Cargo.toml"
  check_path "src-tauri/tauri.conf.json" "src-tauri/tauri.conf.json"
  check_path "tests/e2e"                 "tests/e2e/"
  check_path "script/dev.sh"             "script/dev.sh"

  status_info "=== check complete ==="
  status_info "To start dev: script/dev.sh --tauri  (or --frontend for browser-only)"
}

main() {
  case "${1:---check}" in
    --check)
      run_checks
      ;;
    --frontend)
      require_command bun "bun" "install from https://bun.sh"
      status_info "Starting frontend dev server on http://localhost:1420"
      cd frontend
      exec bun run dev
      ;;
    --tauri)
      require_command bun "bun" "install from https://bun.sh"
      require_command cargo "cargo" "install Rust via rustup.rs"
      if ! cargo tauri --version >/dev/null 2>&1; then
        status_error "cargo-tauri not installed."
        status_error "Run: cargo install tauri-cli --version \"^2\""
        exit 1
      fi
      status_info "Starting Tauri desktop app (cargo tauri dev)..."
      exec cargo tauri dev
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
