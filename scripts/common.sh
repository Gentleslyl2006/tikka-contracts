#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

load_env() {
  if [[ -f "${REPO_ROOT}/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "${REPO_ROOT}/.env"
    set +a
  fi
}

require_env() {
  local name="$1"
  local detail="${2:-}"
  if [[ -z "${!name:-}" ]]; then
    echo "Error: ${name} is required${detail:+ (${detail})}" >&2
    exit 1
  fi
}

usage() {
  echo "Usage: $1" >&2
}
