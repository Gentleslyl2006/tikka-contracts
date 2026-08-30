# Security Policy

## Reporting vulnerabilities

Report suspected vulnerabilities privately to the repository maintainers via GitHub Security Advisories or the contact channel listed in [SUPPORT.md](SUPPORT.md). Do not open public issues for undisclosed security bugs.

## Dependency scanning

| Tool | Scope | Trigger |
|------|-------|---------|
| `cargo audit` | Rust workspace (`contracts/*`) | Daily schedule + every PR (`.github/workflows/security-audit.yml`) |
| `npm audit --omit=dev` | Oracle service production deps | Daily schedule + CI (`ci.yml`, `security-audit.yml`) |

Dev-only npm advisories (Jest, ESLint, etc.) are intentionally excluded from release gates.

### Triage

| Severity | Owner | Target response |
|----------|-------|-----------------|
| Critical (RUSTSEC/npm high affecting runtime crypto or auth) | `@crackedstudio` code owners | 24 hours — assess, patch or document compensating control |
| High | Code owners | 3 business days |
| Medium / Low | Code owners | Next scheduled maintenance window |

When `cargo audit` finds new advisories on the daily schedule, the workflow opens (or updates) a GitHub issue labelled `security`. The assignee must:

1. Confirm whether the advisory applies to code paths we ship (WASM contracts, oracle binary).
2. Upgrade the dependency, replace the crate, or document why the finding is accepted.
3. Close the issue with the remediation commit reference.

Pinned crates such as `ed25519-dalek = "=2.1.1"` never auto-update — scheduled scanning is required to catch advisories against pinned versions.

### Existing findings

Run `cargo audit` and `cd oracle && npm audit --omit=dev` locally before release. Known accepted findings must be recorded in this section with rationale and review date.

| Advisory | Package | Status | Reviewed |
|----------|---------|--------|----------|
| _(none recorded)_ | | | |

## Secure development

- Contract changes affecting fund flows require review from a code owner.
- Randomness and draw paths are covered by invariant and budget regression tests — regressions that exceed committed baselines fail CI.
- WASM artifacts are size-checked against committed baselines on every PR.

## Front-running mitigation

See the existing randomness fulfillment delay notes below.

### Attack Vector

The raffle contract's randomness fulfillment mechanism (`provide_randomness`) was vulnerable to front-running and manipulation attacks. In the original implementation, an oracle could submit randomness immediately after a raffle transitioned to `Drawing`, potentially allowing an attacker to:

1. Observe the pending raffle finalization
1. Manipulate oracle behavior to favor specific outcomes
1. Execute malicious transactions in the same block

### Mitigation Implemented

To address this vulnerability, we've implemented a minimum ledger delay between randomness request and fulfillment:

- A constant `RANDOMNESS_MIN_DELAY_LEDGERS = 10` is enforced
- When randomness is requested (during the Drawing phase transition), the current ledger sequence is stored under `DataKey::RandomnessRequestLedger`
- In `provide_randomness`, we check that the current ledger sequence is at least 10 ledgers higher than the request ledger
- If fulfillment is attempted too early, the transaction is rejected with `Error::RandomnessTooEarly`

This delay ensures there's sufficient time for:

- The market and participants to stabilize
- No same-block manipulation
- A clear window between request and fulfillment

### Other Security Considerations

- **Drawing Lock**: Exclusive lock to prevent concurrent state transitions
- **Oracle Timeout**: Fallback mechanism if oracle doesn't respond within 200 ledgers
- **Reentrancy Guard**: Prevents reentrant attacks
