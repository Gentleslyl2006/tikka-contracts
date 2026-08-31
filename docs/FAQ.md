# Contributor FAQ

# Contributor FAQ

Symptom → cause → fix for the setup problems that show up repeatedly in PRs and local builds. Search this page for the exact error text you hit.

---

## 1. `error[E0463]: can't find crate for 'core'` / `can't find crate for 'std'` when building WASM

**Symptom:**

```text
error[E0463]: can't find crate for `core`
  |
  = note: the `wasm32-unknown-unknown` target may not be installed
```

**Cause:** Rust WASM target missing.

**Fix:**

```bash
rustup target add wasm32-unknown-unknown
# If using newer stellar-cli / sdk builds that emit wasm32v1-none:
rustup target add wasm32v1-none
```

---

## 2. `Error: WASM file not found at target/wasm32v1-none/release/raffle-factory.wasm`

**Symptom:** `scripts/deploy-testnet.sh`, `deploy-mainnet.sh`, or `verify.sh` exits after build with that path missing.

**Cause:** Scripts expect the **`wasm32v1-none`** output layout from `stellar contract build`, while CI/`DEVELOPMENT.md` often build with `--target wasm32-unknown-unknown` (different directory).

**Fix (pick one):**

```bash
# A) Use Stellar CLI build (matches scripts)
stellar contract build
ls target/wasm32v1-none/release/*.wasm

# B) Or build the CI target and symlink/copy into the path the script expects
cargo build --target wasm32-unknown-unknown --release -p raffle-factory
mkdir -p target/wasm32v1-none/release
cp target/wasm32-unknown-unknown/release/raffle_factory.wasm \
   target/wasm32v1-none/release/raffle-factory.wasm
```

Confirm the exact `.wasm` filename (`raffle-factory.wasm` vs `raffle_factory.wasm`) after your build — Cargo may emit underscores.

---

## 3. Stellar CLI / Soroban SDK version skew

**Symptom:**

```text
Error: unsupported soroban environment ... 
```

or deploy/invoke fails with cryptic XDR / diagnostic decode errors after a CLI upgrade.

**Cause:** This workspace pins `soroban-sdk = "23"` (`Cargo.toml`). An old `soroban` binary or a CLI major ≠ 23 will disagree with contract bindings.

**Fix:**

```bash
cargo install --locked stellar-cli --features opt
stellar --version    # expect 23.x
# Uninstall legacy binary if it shadows PATH:
# which soroban; which stellar
```

README requires **Stellar CLI v23.x** matched to SDK 23.

---

## 4. `soroban: command not found` but docs mention Soroban CLI

**Symptom:** Shell cannot find `soroban`.

**Cause:** Upstream renamed the CLI to **`stellar`**. Older tutorials still say `soroban contract ...`.

**Fix:** Use `stellar` everywhere:

```bash
stellar contract build
stellar contract deploy ...
stellar contract invoke ...
```

If you have a `soroban` shim, ensure it is the same major version as `stellar`.

---

## 5. Oracle / Node engine errors (`engine-strict`, odd TypeScript failures)

**Symptom:**

```text
error engine "node" ...
The engine "node" is incompatible with this module
```

or `npm ci` / `tsc` fails inside `oracle/`.

**Cause:** Oracle CI and README require **Node.js 20.x** (`oracle/package.json` `@types/node` ^20; workflow `node-version: '20'`).

**Fix:**

```bash
node -v   # should be v20.x
cd oracle && npm ci && npm run build && npm test
```

Use `nvm install 20 && nvm use 20` (or equivalent) if your default Node is 18/22+.

---

## 6. `Error: DEPLOYER_SECRET_KEY is required to deploy`

**Symptom:** Deploy scripts exit immediately with that message.

**Cause:** `.env` missing, not loaded, or key commented out. Scripts only auto-load `.env` from the **repo root**.

**Fix:**

```bash
cp .env.example .env
# Uncomment and set:
# DEPLOYER_SECRET_KEY="S..."
./scripts/deploy-testnet.sh   # run from repo root
```

---

## 7. `Error: RAFFLE_CONTRACT_ADDRESS is required`

**Symptom:** `invoke.sh` / `verify.sh` refuse to run.

**Cause:** Env var unset. Name is historical — value may be the **factory** or an **instance** ID depending on what you are calling.

**Fix:**

```bash
export RAFFLE_CONTRACT_ADDRESS="$(jq -r .contractId deployments/testnet.json)"
# or paste a C... address from create_raffle
./scripts/invoke.sh get_protocol_stats
```

---

## 8. Friendbot / funding failures on testnet

**Symptom:**

```text
Usage: ./scripts/fund-testnet.sh <stellar_public_key>
```

or Friendbot returns an error body / empty funding.

**Cause:** Missing G-address argument, wrong network key, or Friendbot rate limiting.

**Fix:**

```bash
./scripts/fund-testnet.sh G...   # public key, not S... secret
# Retry after a short wait; confirm balance on Horizon testnet
```

---

## 9. Wrong package name after the factory rename

**Symptom:**

```text
error: package ID specification `raffle` did not match any packages
```

or docs/scripts still say `-p raffle` / `raffle.wasm`.

**Cause:** The factory crate was renamed to `raffle-factory` (WASM/`cdylib` name may appear as `raffle_factory`). Older README snippets still use `raffle`.

**Fix:** Use the current package names:

```bash
cargo test -p raffle-factory
cargo test -p raffle-instance
cargo test -p raffle-shared
cargo build --release --target wasm32-unknown-unknown -p raffle-factory -p raffle-instance
```

See [CONTRIBUTING.md](../CONTRIBUTING.md).

---

## 10. Clippy / fmt CI failures on an otherwise green change

**Symptom:** CI step `cargo fmt --all -- --check` or `cargo clippy ... -D warnings` fails.

**Cause:** Local toolchain formatting differs, or warnings treated as errors in CI.

**Fix:**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

Re-commit before pushing.

---

## Still stuck?

1. Read [DEVELOPMENT.md](DEVELOPMENT.md) and [DEPLOYMENT.md](DEPLOYMENT.md).
1. Search existing issues/PRs for the error string.  
1. Open a new issue with: OS, `rustc -V`, `stellar --version`, `node -V`, and the full command + log.
