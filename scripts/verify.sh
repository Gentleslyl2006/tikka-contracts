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
WASM_FILE="target/wasm32v1-none/release/raffle-factory.wasm"

if [[ -z "${CONTRACT_ID}" ]]; then
    echo "Error: RAFFLE_CONTRACT_ADDRESS environment variable is required" >&2
    exit 1
fi

if [[ ! -f "${WASM_FILE}" ]]; then
    echo "Error: Local WASM file not found at ${WASM_FILE}. Please build first." >&2
    exit 1
fi

echo "Verifying contract ${CONTRACT_ID} on ${NETWORK}..."

stellar contract fetch --id "${CONTRACT_ID}" --network "${NETWORK}" --out-file remote.wasm

if [[ ! -f remote.wasm ]]; then
    echo "Error: Failed to fetch remote contract." >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    LOCAL_HASH=$(sha256sum "${WASM_FILE}" | awk '{ print $1 }')
    REMOTE_HASH=$(sha256sum remote.wasm | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    LOCAL_HASH=$(shasum -a 256 "${WASM_FILE}" | awk '{ print $1 }')
    REMOTE_HASH=$(shasum -a 256 remote.wasm | awk '{ print $1 }')
else
    echo "Error: Neither sha256sum nor shasum is available to verify hashes." >&2
    rm -f remote.wasm
    exit 1
fi

echo "Local WASM Hash:  ${LOCAL_HASH}"
echo "Remote WASM Hash: ${REMOTE_HASH}"

rm -f remote.wasm

if [[ "${LOCAL_HASH}" = "${REMOTE_HASH}" ]]; then
    echo "Verification Result: Match: YES"
    exit 0
else
    echo "Verification Result: Match: NO"
    exit 1
fi
