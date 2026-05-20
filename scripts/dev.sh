#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

usage() {
  cat <<'EOF'
Usage: script/dev.sh [--check|--frontend|--tauri]

--check     Perform environment and repository structure checks.
--frontend  Start the Bun frontend dev server.
--tauri     Start Tauri dev mode.

This script never installs dependencies or downloads models.
EOF
}

status_ok() {
  printf '[OK] %s\n' "$1"
}

status_warn() {
  printf '[WARN] %s\n' "$1"
}

status_info() {
  printf '[INFO] %s\n' "$1"
}

status_error() {
  printf '[ERROR] %s\n' "$1" >&2
}

check_command() {
  local cmd="$1"
  local label="$2"
  if command -v "$cmd" >/dev/null 2>&1; then
    status_ok "$label: found ($(command -v "$cmd"))"
  else
    status_warn "$label: missing"
  fi
}

require_command() {
  local cmd="$1"
  local label="$2"
  if command -v "$cmd" >/dev/null 2>&1; then
    status_ok "$label: found ($(command -v "$cmd"))"
  else
    status_error "$label: missing"
    return 1
  fi
}

check_path() {
  local path="$1"
  local label="$2"
  if [ -e "$path" ]; then
    status_ok "$label: present"
  else
    status_warn "$label: missing"
  fi
}

run_checks() {
  status_info "meetnote init check"
  status_info "TS-first, Rust optional"
  check_command node "node"
  check_command bun "bun"
  check_command npm "npm"
  check_command python3 "python3"
  check_command uv "uv"
  check_command cargo "cargo"
  check_command rustc "rustc"

  status_info "repository structure"
  check_path "README.md" "README.md"
  check_path "AGENTS.md" "AGENTS.md"
  check_path "PLAN.md" "PLAN.md"
  check_path "docs/decisions.md" "docs/decisions.md"
  check_path "docs/structure.md" "docs/structure.md"
  check_path "frontend/package.json" "frontend/package.json"
  check_path "frontend/src/main.ts" "frontend/src/main.ts"
  check_path "src-tauri/Cargo.toml" "src-tauri/Cargo.toml"
  check_path "src-tauri/tauri.conf.json" "src-tauri/tauri.conf.json"
  check_path "frontend" "frontend/"
  check_path "src-tauri" "src-tauri/"
  check_path "python" "python/"
  check_path "tests" "tests/"
  check_path "script" "script/"
  check_path "docs" "docs/"
  check_path "examples" "examples/"

  status_info "No installation, build, service startup, or model download was attempted."
}

main() {
  case "${1:---check}" in
    --check)
      run_checks
      ;;
    --frontend)
      require_command bun "bun"
      status_info "Starting frontend dev server without installing dependencies."
      cd frontend
      exec bun run dev
      ;;
    --tauri)
      require_command bun "bun"
      require_command cargo "cargo"
      if ! cargo tauri --version >/dev/null 2>&1; then
        status_error "cargo-tauri is missing. Install the Tauri CLI before running --tauri."
        exit 1
      fi
      status_info "Starting Tauri dev mode without installing dependencies."
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
