# Deployment Guide

This document covers the `scripts/` deployment toolchain and the `deployments/` registry. A contributor can fund an account, deploy the factory to testnet, initialize it, create a raffle, invoke functions, and verify the on-chain WASM using only the steps below.

## Prerequisites

- Rust toolchain via [rustup](https://rustup.rs/)
- Stellar CLI **v23.x** (must match workspace `soroban-sdk = "23"`)
- WASM target used by your toolchain (see [FAQ](FAQ.md) for `wasm32-unknown-unknown` vs `wasm32v1-none`)
- A funded Stellar account secret key for the target network

```bash
rustup target add wasm32-unknown-unknown
# Newer stellar-cli builds may also require:
# rustup target add wasm32v1-none

cargo install --locked stellar-cli --features opt
stellar --version   # expect 23.x
```

## Environment variables

Copy the example env file and fill in the values you need:

```bash
cp .env.example .env
```

Every script under `scripts/` loads `.env` when present (`export $(cat .env | xargs)`).

| Variable                               | Required by                | Purpose                                        |
| -------------------------------------- | -------------------------- | ---------------------------------------------- |
| `DEPLOYER_SECRET_KEY`                  | `deploy-*.sh`, `invoke.sh` | Account that signs deploy/invoke txs (`S...`)  |
| `RAFFLE_CONTRACT_ADDRESS`              | `invoke.sh`, `verify.sh`   | Contract ID to invoke or verify (`C...`)       |
| `STELLAR_NETWORK`                      | `invoke.sh`, `verify.sh`   | Network name (`testnet` default, or `mainnet`) |
| `STELLAR_RPC_URL`                      | oracle / manual CLI        | Soroban RPC endpoint                           |
| `STELLAR_HORIZON_URL`                  | optional                   | Horizon endpoint                               |
| `FACTORY_CONTRACT_ID`                  | oracle service             | Factory ID the oracle listens on               |
| `ORACLE_ADDRESS` / `ORACLE_SECRET_KEY` | oracle / External raffles  | Oracle identity for `provide_randomness`       |
| `ADMIN_ADDRESS`                        | manual init                | Factory admin G-address                        |

> **Security:** Never commit `.env` or secret keys. `DEPLOYER_SECRET_KEY` and `ORACLE_SECRET_KEY` must stay local or in a secrets manager.

---

## Scripts reference

### `scripts/fund-testnet.sh`

**Purpose:** Request testnet XLM from Friendbot for a public key.

**Required args:** `<stellar_public_key>` (G-address)

**Env vars:** none required

**Example:**

```bash
./scripts/fund-testnet.sh GD....YOUR_PUBLIC_KEY
```

**Expected output:** Friendbot JSON response, then `Funding complete!`

---

### `scripts/deploy-testnet.sh`

**Purpose:** Build WASM with `stellar contract build`, deploy `raffle-factory.wasm` to testnet, and record the result under `deployments/`.

**Required env:** `DEPLOYER_SECRET_KEY`

**Example:**

```bash
export DEPLOYER_SECRET_KEY="S..."
./scripts/deploy-testnet.sh
```

**Expected output:**

```text
Building WASM...
Deploying to Testnet...
Deployment successful!
Contract ID: C...
Saved deployment info to deployments/testnet.json
```

**WASM path expected by the script:** `target/wasm32v1-none/release/raffle-factory.wasm`

If that path is missing after `stellar contract build`, see [FAQ](FAQ.md) (WASM target mismatch).

---

### `scripts/deploy-mainnet.sh`

**Purpose:** Same as testnet deploy, but targets `mainnet` and writes `deployments/mainnet.json`.

**Required env:** `DEPLOYER_SECRET_KEY`

**Safety:** Prompts `WARNING: You are deploying to MAINNET. Proceed? (y/N)` and aborts unless you confirm with `y` / `yes`.

**Example:**

```bash
export DEPLOYER_SECRET_KEY="S..."
./scripts/deploy-mainnet.sh
# type y when prompted
```

---

### `scripts/invoke.sh`

**Purpose:** Thin wrapper around `stellar contract invoke` for the contract in `RAFFLE_CONTRACT_ADDRESS`.

**Required env:** `RAFFLE_CONTRACT_ADDRESS`, `DEPLOYER_SECRET_KEY`  
**Optional env:** `STELLAR_NETWORK` (default `testnet`)

**Usage:**

```bash
./scripts/invoke.sh <function_name> [args...]
```

**Example:**

```bash
export RAFFLE_CONTRACT_ADDRESS="C..."
export DEPLOYER_SECRET_KEY="S..."
./scripts/invoke.sh get_raffle
```

Arguments after the function name are forwarded to the Stellar CLI as contract function args.

---

### `scripts/verify.sh`

**Purpose:** Fetch the on-chain WASM for `RAFFLE_CONTRACT_ADDRESS` and compare its SHA-256 hash to the local factory WASM.

**Required env:** `RAFFLE_CONTRACT_ADDRESS`  
**Optional env:** `STELLAR_NETWORK` (default `testnet`)  
**Local artifact:** `target/wasm32v1-none/release/raffle-factory.wasm` (build first)

**Example:**

```bash
export RAFFLE_CONTRACT_ADDRESS="C..."
stellar contract build   # or cargo build --release --target ...
./scripts/verify.sh
```

**Expected output on match:**

```text
Local WASM Hash:  <hex>
Remote WASM Hash: <hex>
Verification Result: Match: YES
```

Exit code `0` on match, `1` on mismatch or fetch failure. Temporary file `remote.wasm` is deleted after comparison.

---

## End-to-end testnet flow

Order of operations from a clean machine:

### 1. Configure env

```bash
cp .env.example .env
# Set DEPLOYER_SECRET_KEY, ADMIN_ADDRESS, and network URLs
```

### 2. Fund the deployer

```bash
# Derive public key from your secret (example with stellar CLI)
stellar keys address <alias-or-secret>
./scripts/fund-testnet.sh <YOUR_PUBLIC_KEY>
```

### 3. Deploy the factory

```bash
./scripts/deploy-testnet.sh
```

Note the printed `Contract ID` and confirm `deployments/testnet.json` was updated.

### 4. Point tooling at the factory

```bash
# Use the factory ID for verify/invoke against the factory itself
export RAFFLE_CONTRACT_ADDRESS="$(jq -r .contractId deployments/testnet.json)"
export FACTORY_CONTRACT_ID="$RAFFLE_CONTRACT_ADDRESS"
```

### 5. Initialize the factory

`deploy-*.sh` only uploads/deploys WASM — it does **not** call `init_factory`. Initialize once:

```bash
# Upload instance WASM and capture its hash (needed by init_factory)
stellar contract install \
  --wasm target/wasm32v1-none/release/raffle_instance.wasm \
  --source "$DEPLOYER_SECRET_KEY" \
  --network testnet
# → prints WASM hash

./scripts/invoke.sh init_factory \
  --admin "$ADMIN_ADDRESS" \
  --wasm_hash <INSTANCE_WASM_HASH_HEX> \
  --protocol_fee_bp 250 \
  --treasury "$ADMIN_ADDRESS"
```

Exact CLI flag spelling follows `stellar contract invoke --help` for your CLI version. `protocol_fee_bp` is basis points (250 = 2.5%); see [FEE_MODEL.md](FEE_MODEL.md).

### Upgrade procedure

1. Propose a new instance WASM hash through the factory timelock with `propose_wasm_upgrade`.
2. Confirm the proposal is pending and cannot be executed before `TIMELOCK_DELAY_SECONDS` elapses.
3. Advance the ledger time past the delay, then invoke `execute_config_change` to apply the upgrade.
4. Verify the new instance WASM is active and that existing raffles remain readable after the upgrade.

### 6. Create a raffle

```bash
./scripts/invoke.sh create_raffle \
  --creator "$ADMIN_ADDRESS" \
  --config '<RaffleConfig JSON / XDR per CLI>'
```

`create_raffle` deploys a new **raffle instance** and returns its address. Set `RAFFLE_CONTRACT_ADDRESS` to that instance for ticket/draw calls, or invoke with `--id <instance>` directly via `stellar contract invoke`.

### 7. Fund prize, sell tickets, finalize

Typical instance lifecycle (see [ARCHITECTURE.md](ARCHITECTURE.md)):

1. `deposit_prize` — creator escrows the prize (`PendingPrize` → `Active`)
1. `buy_tickets` — buyers purchase entries
1. `finalize_raffle` — starts the draw (Internal / External / CommitReveal)
1. For `External`: run the `oracle/` service so it calls `provide_randomness`
1. `claim_prize` — winners withdraw after claim lockup

Example finalize:

```bash
export RAFFLE_CONTRACT_ADDRESS="<INSTANCE_C_ADDRESS>"
./scripts/invoke.sh finalize_raffle
```

### 8. Verify factory WASM

```bash
export RAFFLE_CONTRACT_ADDRESS="$(jq -r .contractId deployments/testnet.json)"
./scripts/verify.sh
```

---

## The `deployments/` directory

Deployment scripts write a small JSON receipt after a successful deploy:

| File                       | Written by          | Contents                                            |
| -------------------------- | ------------------- | --------------------------------------------------- |
| `deployments/testnet.json` | `deploy-testnet.sh` | `network`, `contractId`, `timestamp` (UTC ISO-8601) |
| `deployments/mainnet.json` | `deploy-mainnet.sh` | same shape for mainnet                              |

Example (`deployments/testnet.json`):

```json
{
  "network": "testnet",
  "contractId": "CCTCPMI66REXIJQPVOPNTNUZBCMSRM7TZLMIPQROZIID44XNP2P2MKFZ",
  "timestamp": "2026-02-24T18:05:54Z"
}
```

### Recording a new deployment

1. Run `./scripts/deploy-testnet.sh` or `./scripts/deploy-mainnet.sh`.
1. The script overwrites the corresponding JSON file with the new `contractId` and timestamp.
1. Commit the updated JSON when the deployment is meant to be the shared reference address for the team (optional; treat secrets separately).
1. Update `.env` (`RAFFLE_CONTRACT_ADDRESS` / `FACTORY_CONTRACT_ID`) to match.

The scripts currently record the **factory** contract ID only. Instance addresses from `create_raffle` should be tracked separately (notes, frontend config, or an extended deployments file of your own).

---

## Related docs

- [DEVELOPMENT.md](DEVELOPMENT.md) — local build and repository workflow
- [ARCHITECTURE.md](ARCHITECTURE.md) — factory → instance → oracle flow
- [RANDOMNESS.md](RANDOMNESS.md) — choose Internal / External / CommitReveal before create
- [FAQ.md](FAQ.md) — CLI naming, WASM targets, Node 20 for `oracle/`
