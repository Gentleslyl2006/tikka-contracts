# Contributing

Thanks for your interest in contributing to Tikka! This project targets Stellar/Soroban smart contracts and welcomes PRs for improvements, tests, and docs.

## Getting Started

1. Fork the repository and create a feature branch.
2. Make your changes with clear, focused commits.
3. Run `cargo fmt --all` to format code before committing.
4. Run tests locally before opening a PR.
5. Install the recommended VS Code extensions when prompted and keep format-on-save enabled.
6. Install the local hooks with `pip install pre-commit && pre-commit install`.

Setup problems (missing WASM target, Stellar CLI vs SDK 23 mismatch, Node 20 for `oracle/`, `stellar` vs `soroban` naming, deploy script paths) are answered in [`docs/FAQ.md`](docs/FAQ.md).

## Finding an Issue

New contributors should start by finding an issue labeled **`good first issue`**. This label marks tasks scoped for learning the codebase with minimal risk.

### Issue Labels & Difficulty

- **`good first issue`**: Scoped for newcomers. Self-contained, well-documented, and low risk. Start here.
- **`help wanted`**: Contribution welcome but may require some codebase familiarity.
- **`difficulty: easy`**: Straightforward task, likely in one module or document.
- **`difficulty: medium`**: Requires understanding multiple components or APIs.
- **`difficulty: hard`**: Complex, cross-cutting, or touches contract internals.
- **`type: bug`**: Defect that needs fixing.
- **`type: feature`**: New capability or enhancement.
- **`type: docs`**: Documentation improvement.

### Assignment Etiquette

Before starting work on an issue:

1. **Check if it's assigned**: If someone is already working on it, pick a different issue.
2. **Comment to claim it**: Reply with "I'd like to work on this" or similar. This signals your intent and prevents duplicate work.
3. **Get guidance if unsure**: Ask for clarification on scope or approach in the issue comments. The maintainers will help.

### Finding Good First Issues

**GitHub filter**: [Good first issue filter](https://github.com/stellar/tikka-contracts/issues?q=is%3Aissue+is%3Aopen+label%3A"good+first+issue")

You can also filter by `good first issue` and `type:docs` to start with documentation improvements, which are lower-risk and help the community.

### Terminology Reference

Unfamiliar with a term in the issue? Check [`docs/GLOSSARY.md`](docs/GLOSSARY.md) for one-paragraph definitions with code references.

## Development Expectations

- Keep changes scoped and easy to review.
- Write tests for new behavior when possible.
- Every new privileged entrypoint must have both a positive authorization test for the configured admin and a negative test proving a non-admin is rejected. Keep these checks table-driven where the entrypoints share setup so missing coverage is visible in review.
- Update documentation if behavior or APIs change.
- Include the corresponding documentation update in the same PR whenever
	behavior or an API changes; mark unfinished behavior as unimplemented.

## Tests

```bash
cargo test -p raffle-factory
cargo test -p raffle-instance
```

## Error Documentation Sync

If you modify or add any variants to the `Error` enum in `contracts/raffle-instance/src/lib.rs`, regenerate `docs/ERRORS.md` before committing:

```bash
python scripts/generate_error_docs.py
```

CI will fail if `docs/ERRORS.md` is out of sync with the Rust `Error` enum.

## Markdown

Run markdownlint before opening a PR to keep documentation style consistent:

```bash
npx markdownlint-cli2 "**/*.md"
```

The configuration lives in `.markdownlint.jsonc`. Auto-fixable issues can be resolved with `npx markdownlint-cli2 --fix "**/*.md"`.

## Pull Requests

- Provide a concise summary of what changed and why.
- Link any relevant issues.
- Note any follow-up work or limitations.
- Use the PR template at `.github/PULL_REQUEST_TEMPLATE.md` to ensure all required information is included.

## Dependency updates

Dependabot opens weekly PRs for **GitHub Actions**, **Cargo**, **npm** (`oracle/`), and
**Docker** (`oracle/Dockerfile`). Review policy:

| Ecosystem | Grouping | Review expectations |
|-----------|----------|---------------------|
| **Cargo — production** | Individual PRs for `soroban-sdk`, `ed25519-dalek`, `sha2`, and other runtime/crypto deps | Rebuild WASM (`cargo build --target wasm32-unknown-unknown --release`), run the full test suite, and verify host behaviour before merge. Soroban SDK minors can change contract semantics. |
| **Cargo — dev tooling** | Grouped (`proptest`, test helpers, fuzz crates, etc.) | Run `cargo test --workspace` and `cargo clippy`. |
| **npm — production** | Individual PRs for `@stellar/stellar-sdk`, `dotenv` | Confirm SDK major matches the Soroban protocol (see `oracle/README.md`), run `npm test` and `npm run build`. |
| **npm — dev tooling** | Grouped (Jest, TypeScript, Prettier, types) | Run `npm test` and `npm run format:check`. |
| **Docker** | Individual PRs for base image bumps | Rebuild the oracle image and smoke-test `/health`. |
| **GitHub Actions** | Individual PRs per action bump | Ensure SHA pins include a version comment; `actionlint` must pass. |

All dependency PRs route to `@crackedstudio/maintainers` via CODEOWNERS. Do not merge
supply-chain bumps without maintainer approval.

## Supply-chain policy (`cargo-deny`)

CI runs [`cargo-deny`](https://embarkstudios.github.io/cargo-deny/) on every PR using
the root [`deny.toml`](deny.toml). The policy enforces allowed licenses, warns on
duplicate crate versions, and denies known advisories. Advisories are also refreshed
on a weekly schedule.

### Adding an exception

If `cargo-deny` reports a finding that cannot be resolved immediately:

1. Prefer fixing the dependency (upgrade, replace, or remove) over allowlisting.
2. If an allowlist entry is unavoidable, add it to the relevant section in `deny.toml`
   with an inline `reason` (or `ignore` entry for advisories) explaining the risk
   accepted and the planned remediation.
3. Obtain approval from a `@crackedstudio/maintainers` reviewer — exceptions require
   explicit maintainer sign-off in the PR.
4. Link any related RUSTSEC advisory ID in the PR description.

Run locally before opening a PR:

```bash
cargo install cargo-deny --locked
cargo deny check
```

## Stale issues and PRs

To keep the contribution queue healthy, we run GitHub's [`actions/stale`](https://github.com/actions/stale) bot (see [`.github/workflows/stale.yml`](.github/workflows/stale.yml)):

- Issues and pull requests with no activity for **21 days** are marked `stale` with a friendly reminder.
- If there is still no activity for **7 more days**, they are closed automatically.
- Items labeled `critical` or assigned to a milestone are exempt.

If your issue or PR is marked stale and you are still working on it, leave a comment or push an update and we will gladly keep it open.

## Code of Conduct

Please read and follow our [Code of Conduct](CODE_OF_CONDUCT.md).
