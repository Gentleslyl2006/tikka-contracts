#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

NETWORK="${STELLAR_NETWORK:-testnet}"
CONTRACT_ID="${RAFFLE_CONTRACT_ADDRESS:-}"

if [[ -z "${CONTRACT_ID}" ]]; then
    echo "Error: RAFFLE_CONTRACT_ADDRESS is required" >&2
    exit 1
fi

if [[ -z "${1:-}" ]]; then
    echo "Usage: ./scripts/invoke.sh <function_name> [args...]" >&2
    echo "Example: ./scripts/invoke.sh buy_ticket --source \$DEPLOYER_SECRET_KEY" >&2
    exit 1
fi

FUNCTION_NAME="$1"
shift

echo "Invoking ${FUNCTION_NAME} on contract ${CONTRACT_ID} (${NETWORK})..."

stellar contract invoke \
  --id "${CONTRACT_ID}" \
  --network "${NETWORK}" \
  --source "${DEPLOYER_SECRET_KEY:-}" \
  -- "${FUNCTION_NAME}" "$@"
