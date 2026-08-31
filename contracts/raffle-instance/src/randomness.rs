//! Winner-selection strategies and seed construction for raffle draws.
//!
//! # Overview
//!
//! This module provides everything the contract needs to select winners once
//! a draw is triggered.  There are two concrete strategies:
//!
//! | Strategy | Type | When used |
//! |---|---|---|
//! | [`PrngWinnerSelection`] | On-chain PRNG | `RandomnessSource::Internal` and `CommitReveal` fallback |
//! | [`OracleSeedWinnerSelection`] | External VRF seed | `RandomnessSource::External` via [`provide_randomness`] |
//!
//! Both implement the [`WinnerSelectionStrategy`] trait. The finalize path
//! currently dispatches explicitly in `draw.rs`; the trait is not itself a
//! runtime selector.
//!
//! # Internal seed construction
//!
//! [`build_internal_seed`] mixes four base inputs. `PrngWinnerSelection` then
//! adds `tickets_sold` in a second hash before passing the result to
//! `env.prng().seed()`:
//!
//! ```text
//! ledger_timestamp  ─┐
//! ledger_sequence   ─┤
//! network_id        ─┼─► XDR-pack ─► SHA-256 ─► BytesN<32>
//! raffle_id (XDR)   ─┤
//! raffle_id         ─┘
//! ```
//!
//! Using XDR serialisation for packing eliminates length-ambiguity collisions
//! between differently-typed fields.  The SHA-256 step distributes the entropy
//! uniformly across all 32 bytes.
//!
//! **⚠️ Security caveat — low-stakes raffles only.**
//! The seed is deterministic and visible on-chain: anyone who knows the ledger
//! state at draw time can reproduce the outcome.  Validators can also influence
//! `ledger_timestamp` and `ledger_sequence` to bias the result.  For
//! high-stakes raffles use [`RandomnessSource::External`] so an off-chain VRF
//! oracle delivers a seed that cannot be predicted or manipulated before
//! [`provide_randomness`](crate::draw::provide_randomness) is called.
//!
//! # Oracle-timeout fallback
//!
//! When an external-randomness raffle's oracle does not respond within
//! [`ORACLE_TIMEOUT_LEDGERS`](crate::ORACLE_TIMEOUT_LEDGERS) (~17 minutes),
//! the creator or admin can call
//! [`trigger_randomness_fallback`](crate::draw::trigger_randomness_fallback):
//!
//! - `do_refund = true` — cancels the raffle and allows ticket holders to
//!   claim refunds via `refund_ticket`.
//! - `do_refund = false` — finalizes the raffle using an internal seed built
//!   from the current ledger state, recorded with
//!   [`RandomnessType::Fallback`](raffle_shared::RandomnessType::Fallback).
//!
//! # VRF proof binding
//!
//! [`build_vrf_proof_message`] constructs the Ed25519 message that the oracle
//! must sign when submitting randomness.  It binds the proof to this specific
//! raffle contract address **and** the request ID so that a valid proof for
//! one raffle cannot be replayed against a different raffle or request.
//!
//! See [`docs/RANDOMNESS.md`](../../../../docs/RANDOMNESS.md) for a
//! higher-level comparison of all three modes, and
//! [`docs/COMMIT_REVEAL.md`](../../../../docs/COMMIT_REVEAL.md) for the
//! commit-reveal protocol specification.

use soroban_sdk::{xdr::ToXdr, Address, Bytes, BytesN, Env, Vec};

// ============================================================================
// build_internal_seed
// ============================================================================
//
// ⚠️  LOW-STAKES RAFFLES ONLY
//
// This seed is deterministic and visible on-chain.  Any participant who knows
// the ledger state at the time `finalize_raffle` is called can reproduce the
// exact output.  Miners / validators can also influence the ledger timestamp and
// sequence to bias the result.
//
// For high-stakes or high-value raffles, use `RandomnessSource::External` so
// that a VRF oracle provides a verifiably-unbiased seed that cannot be
// predicted or manipulated before `provide_randomness` is called.
//
// Base entropy sources mixed into the seed:
//   1. `ledger_timestamp`  – wall-clock time in seconds
//   2. `ledger_sequence`   – monotonically-increasing ledger counter
//   3. `network_id`        – SHA-256 of the network passphrase (32 bytes),
//                            ensuring seeds are network-partitioned (mainnet ≠
//                            testnet ≠ futurenet)
//   4. `raffle_id`         – the raffle contract address in XDR encoding,
//                            making every raffle's draw independent even when
//                            finalized in the same ledger
// `PrngWinnerSelection::seed_bytes` adds `tickets_sold` in a second hash.
//
// The four base inputs are packed together and passed through
// `env.crypto().sha256`; `PrngWinnerSelection` then incorporates the ticket
// count in a second hash before calling `env.prng().seed()`.

/// Build a 32-byte base internal PRNG seed by hashing four entropy sources
/// together. The ticket count is added by [`PrngWinnerSelection::seed_bytes`].
///
/// The four base sources are (in order):
///
/// 1. `ledger_timestamp` — wall-clock time in seconds; changes every ~5 s.
/// 2. `ledger_sequence` — monotonically-increasing ledger counter.
/// 3. `network_id` — 32-byte SHA-256 of the network passphrase, ensuring
///    that seeds on mainnet, testnet, and futurenet are all different even
///    for an identical raffle and ledger state.
/// 4. `raffle_id` (XDR-encoded) — the current contract's address, making
///    concurrent raffles finalised in the same ledger produce distinct seeds.
/// `PrngWinnerSelection::seed_bytes` then adds `tickets_sold` as an additional
/// XDR-packed field in a second hash.
///
/// The base sources are XDR-serialised into a single byte buffer before being
/// passed to `env.crypto().sha256`. XDR encoding is unambiguous and
/// length-delimited so there are no field-boundary collision attacks.
///
/// # Returns
///
/// A [`BytesN<32>`] suitable for passing directly to `env.prng().seed()`.
///
/// # Panics
///
/// Panics if `env.crypto().sha256` returns the all-zero hash (which would
/// indicate a crypto subsystem failure) to prevent silently falling back to a
/// trivially predictable seed.
///
/// # Security note
///
/// **For low-stakes raffles only.** The seed is deterministic given ledger
/// state, so any observer can reproduce the winner selection.  Validators can
/// also marginally bias `ledger_timestamp` and `ledger_sequence`.  Use
/// [`RandomnessSource::External`] for high-value draws.
///
/// See also: module-level documentation and
/// [`docs/RANDOMNESS.md`](../../../../docs/RANDOMNESS.md).
pub fn build_internal_seed(env: &Env, raffle_id: &Address) -> BytesN<32> {
    let timestamp = env.ledger().timestamp();
    let sequence = env.ledger().sequence();
    let network_id: BytesN<32> = env.ledger().network_id();

    // Pack all sources into a single byte buffer, then SHA-256 hash it.
    // Using XDR serialisation guarantees an unambiguous, length-delimited
    // encoding so there are no collisions between differently-typed fields.
    let raw: Bytes = (timestamp, sequence, network_id, raffle_id.clone()).to_xdr(env);
    hash_bytes32(env, &raw)
}

/// Hash `input` with SHA-256 and verify the result is non-zero.
///
/// A zero hash would be indistinguishable from a crypto subsystem failure, so
/// this function panics rather than returning silently — a zeroed seed would
/// make winner selection trivially reproducible and insecure.
fn hash_bytes32(env: &Env, input: &Bytes) -> BytesN<32> {
    let hash: BytesN<32> = env.crypto().sha256(input).into();
    if hash.to_array() == [0u8; 32] {
        panic!("crypto.sha256() failed: invalid hash output");
    }
    hash
}

/// Common interface for winner-index selection algorithms.
///
/// Implemented by both [`PrngWinnerSelection`] (on-chain PRNG) and
/// [`OracleSeedWinnerSelection`] (external VRF seed), so that
/// [`do_finalize_with_seed`](crate::helpers::do_finalize_with_seed) can
/// select winners through a single call-site regardless of the randomness
/// source.
pub trait WinnerSelectionStrategy {
    /// Return `winner_count` distinct zero-based ticket indices chosen
    /// uniformly at random from `[0, total_tickets)`.
    ///
    /// If `winner_count > total_tickets`, at most `total_tickets` indices are
    /// returned (no duplicates are possible beyond that cap).  An empty `Vec`
    /// is returned when either argument is zero.
    fn select_winner_indices(&self, env: &Env, total_tickets: u32, winner_count: u32) -> Vec<u32>;
}

/// On-chain PRNG-based winner selection using a multi-source seed.
///
/// Used for [`RandomnessSource::Internal`] raffles and as the fallback when
/// the commit-reveal path has no commits.  The same inputs always produce the
/// same winners, which allows off-chain auditors to verify the draw by
/// replaying the seed construction against the known ledger state.
///
/// **For low-stakes raffles only** — see [`build_internal_seed`] and the
/// module documentation for the full security caveat.
pub struct PrngWinnerSelection {
    pub raffle_id: Address,
    /// Number of tickets sold at draw time, mixed into the seed so that
    /// identical raffle setups with different participation produce different
    /// outcomes.
    pub tickets_sold: u32,
}

impl PrngWinnerSelection {
    /// Create a new `PrngWinnerSelection` for the given raffle and ticket count.
    pub fn new(raffle_id: Address, tickets_sold: u32) -> Self {
        Self { raffle_id, tickets_sold }
    }

    /// Return a compact `u64` fingerprint of the draw seed.
    ///
    /// The fingerprint is derived from the first 8 bytes of a second SHA-256
    /// hash over the base seed, giving the same domain separation as the full
    /// seed while fitting in a `u64` for compact on-chain storage in
    /// [`FairnessMetadata`](crate::FairnessMetadata).
    ///
    /// The fingerprint changes whenever `raffle_id`, `tickets_sold`, or any
    /// ledger field changes, so it can be used to spot-check draws off-chain.
    pub fn seed_fingerprint(&self, env: &Env) -> u64 {
        let hashed = hash_bytes32(env, &self.seed_bytes(env));
        let arr = hashed.to_array();
        u64::from_be_bytes([
            arr[0], arr[1], arr[2], arr[3], arr[4], arr[5], arr[6], arr[7],
        ])
    }

    /// Build the raw 32-byte seed `Bytes` that is passed to `env.prng().seed()`.
    ///
    /// Extends [`build_internal_seed`] by XDR-packing `base_seed ‖
    /// tickets_sold` and re-hashing so that the ticket-count entropy is
    /// included without truncating any of the four base sources.
    fn seed_bytes(&self, env: &Env) -> Bytes {
        let base: BytesN<32> = build_internal_seed(env, &self.raffle_id);
        // XDR-pack the base seed + tickets_sold and re-hash to include the
        // extra entropy source without truncating the network_id contribution.
        let combined: Bytes = (base, self.tickets_sold).to_xdr(env);
        hash_bytes32(env, &combined).into()
    }
}

impl WinnerSelectionStrategy for PrngWinnerSelection {
    fn select_winner_indices(&self, env: &Env, total_tickets: u32, winner_count: u32) -> Vec<u32> {
        let mut indices = Vec::new(env);
        if total_tickets == 0 || winner_count == 0 {
            return indices;
        }

        // Seed the PRNG with the multi-source hash — see build_internal_seed
        // for details on the entropy inputs.
        env.prng().seed(self.seed_bytes(env));

        let effective_count = winner_count.min(total_tickets);
        for _ in 0..effective_count {
            // Keep sampling until we find an index that hasn't been selected yet
            loop {
                #[allow(deprecated)]
                let idx = env.prng().u64_in_range(0..(total_tickets as u64)) as u32;

                // Check if this index is already in the selected indices
                let mut found = false;
                for i in 0..indices.len() {
                    if indices.get(i) == Some(idx) {
                        found = true;
                        break;
                    }
                }

                // If not found, add it and break; otherwise resample
                if !found {
                    indices.push_back(idx);
                    break;
                }
            }
        }

        indices
    }
}

/// Build the Ed25519 message that binds a VRF proof to a specific raffle and
/// request.
///
/// The oracle must sign exactly this byte sequence when calling
/// [`provide_randomness`](crate::draw::provide_randomness).  The message
/// contains:
///
/// - The current contract address (`env.current_contract_address()`) — binds
///   the proof to **this** raffle; a proof generated for raffle A cannot be
///   replayed against raffle B.
/// - `request_id` — binds the proof to the specific randomness request; a
///   stale or recycled proof from an earlier draw cannot be accepted.
/// - `random_seed` — the oracle's VRF output being delivered.
///
/// All three fields are XDR-serialised together so the encoding is
/// unambiguous and length-delimited.
///
/// # Parameters
///
/// - `request_id` — The unique request ID stored in
///   [`DataKey::RandomnessRequestId`](crate::DataKey::RandomnessRequestId).
/// - `random_seed` — The VRF output (random seed) being delivered.
///
/// # Returns
///
/// A [`Bytes`] value that should be passed to `env.crypto().ed25519_verify`.
///
/// See also: [`docs/RANDOMNESS.md`](../../../../docs/RANDOMNESS.md) — External
/// / VRF mode.
pub fn build_vrf_proof_message(env: &Env, request_id: u64, random_seed: u64) -> Bytes {
    (env.current_contract_address(), request_id, random_seed).to_xdr(env)
}

/// Oracle-backed winner selection using an externally provided VRF seed.
///
/// Used by [`provide_randomness`](crate::draw::provide_randomness) after the
/// oracle has delivered a cryptographically-verified random value.  Unlike
/// [`PrngWinnerSelection`], this strategy is not subject to on-chain
/// manipulation: the seed is committed off-chain via VRF before the draw
/// starts and delivered back to the contract via a signed proof.
///
/// ## Rejection sampling
///
/// To eliminate modulo bias, the selection uses rejection sampling.  A sample
/// is only accepted when it falls below
/// `floor(u64::MAX / total_tickets) * total_tickets` — the largest multiple
/// of `total_tickets` that fits in a `u64`.  Samples in the biased tail are
/// discarded and the seed is advanced using an LCG step.
///
/// ## LCG advance
///
/// Between samples the internal state is advanced with the LCG:
/// ```text
/// state = state * 6364136223846793005 + 1442695040888963407
/// ```
/// (Knuth's constants, same as those used in many standard libraries.)
pub struct OracleSeedWinnerSelection {
    seed: u64,
}

impl OracleSeedWinnerSelection {
    /// Create a new selector seeded with the oracle-provided VRF output.
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Pure (no-`Env`) version of [`select_winner_indices`] used in tests and
    /// off-chain tooling.  Available only when `std` is in scope.
    #[cfg(any(test, feature = "std"))]
    pub fn select_winner_indices_pure(
        &self,
        total_tickets: u32,
        winner_count: u32,
    ) -> std::vec::Vec<u32> {
        let mut indices = std::vec::Vec::new();
        if total_tickets == 0 || winner_count == 0 {
            return indices;
        }

        let n = total_tickets as u64;
        let largest_multiple = (u64::MAX / n) * n;

        let mut current_seed = self.seed;
        for _ in 0..winner_count {
            let idx = loop {
                if current_seed < largest_multiple {
                    break (current_seed % n) as u32;
                }
                current_seed = current_seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
            };
            indices.push(idx);
            current_seed = current_seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
        }

        indices
    }
}

impl WinnerSelectionStrategy for OracleSeedWinnerSelection {
    fn select_winner_indices(&self, env: &Env, total_tickets: u32, winner_count: u32) -> Vec<u32> {
        let mut indices = Vec::new(env);
        if total_tickets == 0 || winner_count == 0 {
            return indices;
        }

        // #257: Use rejection sampling to eliminate modulo bias.
        // We discard samples that fall in the biased tail so every ticket in
        // [0, total_tickets) is chosen with exactly equal probability.
        //
        // largest_multiple = floor(u64::MAX / total_tickets) * total_tickets
        // Any sample >= largest_multiple is rejected and the seed advanced.
        let n = total_tickets as u64;
        let largest_multiple = (u64::MAX / n) * n;

        let effective_count = winner_count.min(total_tickets);
        let mut current_seed = self.seed;
        for _ in 0..effective_count {
            let idx = loop {
                let candidate = loop {
                    if current_seed < largest_multiple {
                        break (current_seed % n) as u32;
                    }
                    current_seed = current_seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                };
                let mut found = false;
                for i in 0..indices.len() {
                    if indices.get(i) == Some(candidate) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    break candidate;
                }
                current_seed = current_seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
            };
            indices.push_back(idx);
            current_seed = current_seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
        }

        indices
    }
}

/// Aggregate multiple oracle seeds into a single deterministic seed.
///
/// Uses SHA-256 over the concatenation of all delivered seeds
/// in submission order.  The first 8 bytes of the hash become the `u64` seed.
///
/// # Security
///
/// As long as at least one of the seeds was provided by an honest oracle,
/// the SHA-256 output is cryptographically uniform and cannot be biased.
pub fn aggregate_quorum_seeds(env: &Env, seeds: &Vec<(Address, u64)>) -> u64 {
    if seeds.is_empty() {
        return 0u64;
    }

    let mut combined = Bytes::new(env);
    for i in 0..seeds.len() {
        if let Some((_, seed)) = seeds.get(i) {
            combined.extend_from_array(&seed.to_be_bytes());
        }
    }

    let hash: BytesN<32> = env.crypto().sha256(&combined).into();
    let arr = hash.to_array();
    let mut seed_bytes = [0u8; 8];
    seed_bytes.copy_from_slice(&arr[..8]);
    u64::from_be_bytes(seed_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    /// build_internal_seed produces different values for different raffle IDs.
    #[test]
    fn build_internal_seed_differs_by_raffle_id() {
        let env = Env::default();
        let id_a = Address::generate(&env);
        let id_b = Address::generate(&env);
        let contract = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        let (seed_a, seed_b) = env.as_contract(&contract, || {
            (
                build_internal_seed(&env, &id_a),
                build_internal_seed(&env, &id_b),
            )
        });

        assert_ne!(
            seed_a, seed_b,
            "different raffle IDs must produce different seeds"
        );
    }

    /// build_internal_seed is deterministic: same inputs → same output.
    #[test]
    fn build_internal_seed_is_deterministic() {
        let env = Env::default();
        let raffle_id = Address::generate(&env);
        let contract = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        let (first, second) = env.as_contract(&contract, || {
            (
                build_internal_seed(&env, &raffle_id),
                build_internal_seed(&env, &raffle_id),
            )
        });

        assert_eq!(first, second, "same inputs must always yield the same seed");
    }

    /// build_internal_seed output is exactly 32 bytes.
    #[test]
    fn build_internal_seed_is_32_bytes() {
        let env = Env::default();
        let raffle_id = Address::generate(&env);
        let contract = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        let seed = env.as_contract(&contract, || build_internal_seed(&env, &raffle_id));
        // BytesN<32> is always 32 bytes by construction; this is a compile-time
        // guarantee, but we also verify the array conversion is loss-free.
        assert_eq!(seed.to_array().len(), 32);
    }

    /// build_internal_seed must not produce the all-zero hash.
    #[test]
    fn build_internal_seed_is_not_zero() {
        let env = Env::default();
        let raffle_id = Address::generate(&env);
        let contract = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        let seed = env.as_contract(&contract, || build_internal_seed(&env, &raffle_id));
        assert_ne!(
            seed.to_array(),
            [0u8; 32],
            "sha256 output must not be all zero"
        );
    }

    /// PRNG selections fall within [0, total_tickets).
    #[test]
    fn prng_selection_is_in_ticket_range() {
        let env = Env::default();
        let raffle_id = Address::generate(&env);
        let strategy = PrngWinnerSelection::new(raffle_id, 17);

        let contract_id = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        let indices = env.as_contract(&contract_id, || {
            strategy.select_winner_indices(&env, 17, 25)
        });
        assert_eq!(indices.len(), 17);
        for idx in indices.iter() {
            assert!(idx < 17, "winner index {idx} must be < total_tickets 17");
        }
    }

    /// Same PRNG inputs always produce the same winner sequence.
    #[test]
    fn prng_selection_is_deterministic_for_same_inputs() {
        let env = Env::default();
        let raffle_id = Address::generate(&env);

        let contract_id = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        let first = env.as_contract(&contract_id, || {
            PrngWinnerSelection::new(raffle_id.clone(), 17).select_winner_indices(&env, 17, 8)
        });
        let second = env.as_contract(&contract_id, || {
            PrngWinnerSelection::new(raffle_id, 17).select_winner_indices(&env, 17, 8)
        });

        assert_eq!(
            first, second,
            "identical inputs must yield identical winners"
        );
    }

    /// Seed fingerprint changes when raffle_id changes.
    #[test]
    fn seed_fingerprint_differs_by_raffle_id() {
        let env = Env::default();
        let id_a = Address::generate(&env);
        let id_b = Address::generate(&env);
        let contract = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        let (fp_a, fp_b) = env.as_contract(&contract, || {
            let s_a = PrngWinnerSelection::new(id_a, 10);
            let s_b = PrngWinnerSelection::new(id_b, 10);
            (s_a.seed_fingerprint(&env), s_b.seed_fingerprint(&env))
        });

        assert_ne!(
            fp_a, fp_b,
            "fingerprints must differ for different raffle IDs"
        );
    }

    /// Seed fingerprint changes when ticket count changes.
    #[test]
    fn seed_fingerprint_differs_by_ticket_count() {
        let env = Env::default();
        let raffle_id = Address::generate(&env);
        let contract = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        let (fp_a, fp_b) = env.as_contract(&contract, || {
            let s_a = PrngWinnerSelection::new(raffle_id.clone(), 10);
            let s_b = PrngWinnerSelection::new(raffle_id, 11);
            (s_a.seed_fingerprint(&env), s_b.seed_fingerprint(&env))
        });

        assert_ne!(
            fp_a, fp_b,
            "fingerprints must differ for different ticket counts"
        );
    }

    /// Deliberately biased winner selector used to verify that the Chi-squared test
    /// correctly detects modulo / index distribution bias (#633).
    struct BiasedWinnerSelection {
        seed: u64,
    }

    impl BiasedWinnerSelection {
        fn select_winner_indices_biased(&self, total_tickets: u32) -> u32 {
            let n = total_tickets as u64;
            // Intentionally introduces modulo bias by wrapping around an asymmetric range
            ((self.seed % (n + 1)) % n) as u32
        }
    }

    /// Computes the Chi-squared statistic for a frequency histogram against a uniform distribution.
    fn compute_chi_squared(histogram: &[u32], total_samples: u32) -> f64 {
        let k = histogram.len() as f64;
        let expected = total_samples as f64 / k;
        let mut chi2 = 0.0;
        for &count in histogram {
            let diff = count as f64 - expected;
            chi2 += (diff * diff) / expected;
        }
        chi2
    }

    /// Critical values for Chi-squared distribution at alpha = 0.001 (significance level 99.9%).
    fn critical_value_999(degrees_of_freedom: usize) -> f64 {
        match degrees_of_freedom {
            4 => 18.47,  // 5 tickets - 1
            8 => 26.12,  // 9 tickets - 1
            32 => 62.49, // 33 tickets - 1
            df => (df as f64) + 3.0 * (2.0 * df as f64).sqrt(),
        }
    }

    /// Helper running the Chi-squared goodness-of-fit test for OracleSeedWinnerSelection.
    fn run_uniformity_simulation(ticket_counts: &[u32], total_draws: u32) {
        for &n in ticket_counts {
            let mut histogram = std::vec![0u32; n as usize];
            for seed in 1..=(total_draws as u64) {
                let strategy = OracleSeedWinnerSelection::new(seed);
                let winners = strategy.select_winner_indices_pure(n, 1);
                assert_eq!(winners.len(), 1);
                histogram[winners[0] as usize] += 1;
            }
            let df = (n - 1) as usize;
            let chi2 = compute_chi_squared(&histogram, total_draws);
            let crit = critical_value_999(df);
            assert!(
                chi2 < crit,
                "Real winner selector failed Chi-squared uniformity test for ticket_count={n}: chi2={chi2} >= critical={crit}"
            );
        }
    }

    /// Statistical uniformity test (CI variant: 5,000 samples per ticket count).
    /// Tests ticket counts chosen to stress modulo bias (just above powers of two: 5, 9, 33).
    #[test]
    fn test_statistical_uniformity_ci() {
        run_uniformity_simulation(&[5, 9, 33], 5_000);
    }

    /// Statistical uniformity test (Full simulation variant: 100,000 samples per ticket count).
    /// Marked as #[ignore] by default to keep CI fast.
    #[test]
    #[ignore]
    fn test_statistical_uniformity_full() {
        run_uniformity_simulation(&[5, 9, 33], 100_000);
    }

    /// Acceptance criterion test: verifies that the Chi-squared test REJECTS a biased selector.
    #[test]
    fn test_statistical_uniformity_rejects_biased_selector() {
        let total_draws = 5_000u32;
        for &n in &[5u32, 9u32, 33u32] {
            let mut histogram = std::vec![0u32; n as usize];
            for seed in 1..=(total_draws as u64) {
                let biased = BiasedWinnerSelection { seed };
                let winner = biased.select_winner_indices_biased(n);
                histogram[winner as usize] += 1;
            }
            let df = (n - 1) as usize;
            let chi2 = compute_chi_squared(&histogram, total_draws);
            let crit = critical_value_999(df);
            assert!(
                chi2 >= crit,
                "Chi-squared test must REJECT biased selector for ticket_count={n}: chi2={chi2} expected >= critical={crit}"
            );
        }
    }
}
