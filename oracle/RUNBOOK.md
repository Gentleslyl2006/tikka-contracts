# Oracle Transaction Submission Runbook

This document describes how the oracle service handles Soroban transaction submission failures and what operators should do when they see each classification.

## Retry Policy Overview

The oracle uses an exponential backoff with full jitter strategy. The base delay is configurable (default 500ms), capped at 30s, with a maximum attempt count (default 5). Jitter prevents multiple oracles in a quorum from colliding after a shared outage.

## Error Classifications

| Classification | Meaning | Operator Action |
|----------------|---------|-----------------|
| `transient` | RPC unreachable, 5xx, `TRY_AGAIN_LATER`, network timeouts, connection resets | No action required. The oracle will retry automatically. If failures persist, check RPC endpoint health and network connectivity. |
| `sequence-collision` | Account sequence mismatch (`AccountSequenceMismatch`, sequence errors) | No action required. The oracle automatically clears its sequence cache and refreshes from the ledger before retrying. If this recurs frequently, investigate concurrent submissions from other sources using the same account. |
| `insufficient-fee` | Transaction fee too low (`InsufficientFee`) | No action required. The oracle automatically doubles the fee on each retry. If this persists, the network may be congested; consider raising the base fee in configuration. |
| `tx-expired` | Transaction exceeded ledger bounds or timeout (`TxTooLate`, expired, not confirmed within timeout) | No action required. The oracle automatically rebuilds the transaction with wider timeout bounds and retries. If this recurs, the RPC may be slow; consider increasing the base timeout or checking RPC latency. |
| `already-satisfied` | Randomness already requested or duplicate detected (`RandomnessAlreadyRequested`, duplicate) | **Do not retry.** The request is already fulfilled. Log for audit and move on. |
| `invalid-state` | No matching randomness request or invalid contract state (`NoRandomnessRequest`, invalid state) | **Do not retry.** Dead-letter the request. Investigate why the contract rejects the call (e.g., wrong request ID, raffle already closed). |
| `fatal` | Any other unrecoverable error | **Do not retry.** Review the error message, check contract state, and escalate if the cause is unclear. |

## Alerting

The oracle tracks consecutive submission failures. When the count reaches `ALERT_FAILURE_THRESHOLD` (default: 3), it emits an alert to `stderr`.

**Operator response to alerts:**
1. Check the most recent error classification in logs.
2. If `transient`: verify RPC endpoint health and network.
3. If `sequence-collision`: check for duplicate oracle instances or manual submissions.
4. If `insufficient-fee`: review network fee conditions.
5. If `tx-expired`: review RPC latency and timeout settings.
6. If `already-satisfied` or `invalid-state`: investigate contract state and request queue.
7. If `fatal`: review full error context and escalate.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `ALERT_FAILURE_THRESHOLD` | `3` | Consecutive failures before alerting |
| `STELLAR_RPC_URL` | `https://soroban-testnet.stellar.org` | Soroban RPC endpoint |
| `STELLAR_NETWORK_PASSPHRASE` | `Test SDF Network ; September 2015` | Network passphrase |

## Recovery Procedures

### RPC Outage
If the RPC endpoint is unreachable, the oracle will back off and retry. No manual intervention is needed unless the outage persists beyond the max attempt window. In that case, verify the RPC URL and network path, then restart the oracle.

### Stuck Sequence
If sequence collisions persist, the oracle may be sharing an account with another process. Ensure only one oracle instance submits transactions for a given account, or use a dedicated oracle account.

### Repeated Fee Bumps
If the oracle is continuously bumping fees, the network may be under heavy load. Consider raising the base fee or pausing non-critical submissions until congestion subsides.

### Dead-Lettered Requests
Requests classified as `already-satisfied` or `invalid-state` are not retried. Operators should inspect the raffle contract state to determine if the request needs to be re-queued or if the raffle has already advanced.
