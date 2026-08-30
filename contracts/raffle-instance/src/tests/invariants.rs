use super::*;
use proptest::prelude::*;
use soroban_sdk::{Address, BytesN, Env, String, Vec};

fn valid_prize_weights() -> impl Strategy<Value = std::vec::Vec<u32>> {
    prop::collection::vec(0u32..=10_000, 0..=99)
        .prop_filter("basis points must leave room for the final tier", |weights| {
            weights.iter().copied().sum::<u32>() <= 10_000
        })
        .prop_map(|mut weights| {
            let allocated = weights.iter().copied().sum::<u32>();
            weights.push(10_000 - allocated);
            weights
        })
}

fn test_raffle(env: &Env, weights: &[u32], prize_amount: i128) -> Raffle {
    let mut prizes = Vec::new(env);
    for weight in weights {
        prizes.push_back(*weight);
    }

    Raffle {
        creator: Address::generate(env),
        description: String::from_str(env, "tier invariant"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        max_tickets_per_address: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: Address::generate(env),
        prize_token: Address::generate(env),
        prize_amount,
        prizes,
        tickets_sold: 0,
        status: RaffleStatus::PendingPrize,
        prize_deposited: false,
        winners: Vec::new(env),
        randomness_source: raffle_shared::RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        finalized_at: None,
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        ticket_sales_paused: false,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
        metadata_hash: BytesN::from_array(env, &[1; 32]),
        unique_winners: false,
        nft_contract: None,
    }
}

fn assert_tier_sum(weights: &[u32], prize_amount: i128) {
    let env = Env::default();
    let raffle = test_raffle(&env, weights, prize_amount);
    let mut total = 0i128;

    for index in 0..raffle.prizes.len() {
        let amount = calculate_tier_prize(&raffle, index).unwrap();
        assert!(amount >= 0, "tier {index} computed a negative prize");
        total += amount;
    }

    assert_eq!(total, prize_amount);
}

proptest! {
    #[test]
    fn tier_prizes_sum_to_prize_amount(
        weights in valid_prize_weights(),
        prize_amount in MIN_TICKET_PRICE..=MAX_PRIZE_AMOUNT,
    ) {
        assert_tier_sum(&weights, prize_amount);
    }
}

#[test]
fn one_hundred_equal_tiers_sum_exactly() {
    assert_tier_sum(&[100; 100], 1_000_003);
}

#[test]
fn one_tier_receives_the_entire_prize() {
    assert_tier_sum(&[10_000], MAX_PRIZE_AMOUNT);
}

#[test]
fn final_tier_absorbs_maximum_rounding_dust() {
    assert_tier_sum(&[101; 99].iter().copied().chain([1]).collect::<std::vec::Vec<_>>(), 10_000);
}
