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

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "Error: ${name} is required to deploy" >&2
    exit 1
  fi
}

require_env DEPLOYER_SECRET_KEY

echo "Building WASM..."
stellar contract build

WASM_FILE="target/wasm32v1-none/release/raffle-factory.wasm"

if [[ ! -f "${WASM_FILE}" ]]; then
    echo "Error: WASM file not found at ${WASM_FILE}" >&2
    exit 1
fi

echo "Deploying to Testnet..."

CONTRACT_ID=$(stellar contract deploy \
  --wasm "${WASM_FILE}" \
  --source "${DEPLOYER_SECRET_KEY}" \
  --network testnet)

echo "Deployment successful!"
echo "Contract ID: ${CONTRACT_ID}"

mkdir -p deployments
cat <<EOF > deployments/testnet.json
{
  "network": "testnet",
  "contractId": "${CONTRACT_ID}",
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}
EOF

echo "Saved deployment info to deployments/testnet.json"
