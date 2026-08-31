#c[cfg(test)]

pub mod budget;
pub mod invariants;

use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::{Bytes, BytesN, Env};

use crate::randomness::build_internal_seed_u64;

#[test]
fn test_seed_is_network_partitioned() {

    // Same raffle ID and ledger position, but different network IDs.
    // This mirrors the security requirement: seeds must be network-partitioned.
    let raffle_id_bytes = [0x42u8; 32];

    let env1 = Env::default();
    let env2 = Env::default();

    let raffle_id1 = BytesN::32>::from_array(&env1, raffle_id_bytes);
    let raffle_id2 = BytesN::32>::from_array(&env2, raffle_id_bytes);

    // Same ledger timestamp and sequence, different network IDs.
    let ledger_info1 = LedgerInfo {
        timestamp: 10_000,
        sequence_number: 42,
        network_id: Bytes::from_slice(&env1, &[1u8; 32]),
        ..Default::default()
    };
    let ledger_info2 = LedgerInfo {
        timestamp: 10_000,
        sequence_number: 42,
        network_id: Bytes::from_slice(&env2, &[2u8; 32]),
        ..Default::default()
    };
    env1.ledger().set(ledger_info1);
    env2.ledger().set(ledger_info2);

    let seed1 = build_internal_seed_u64(&env1, &raffle_id1);
    let seed2 = build_internal_seed_u64(&env2, &raffle_id2);

    assert_ne!(seed1, seed2, "seeds must be partitioned by network id");
}