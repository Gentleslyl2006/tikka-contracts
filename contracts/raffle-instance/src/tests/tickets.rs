use super::*;
use soroban_sdk::testutils::Address as _;

struct TicketSetup<'a> {
    client: ContractClient<'a>,
    contract_id: Address,
    admin: Address,
    creator: Address,
    buyer: Address,
    recipient: Address,
    token: token::StellarAssetClient<'a>,
}

fn setup(env: &Env, cap: u32, allow_multiple: bool, max_tickets: u32) -> TicketSetup<'_> {
    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(env, &contract_id);
    let factory = env.register(MockFactory, ());
    let admin = Address::generate(env);
    let creator = Address::generate(env);
    let buyer = Address::generate(env);
    let recipient = Address::generate(env);
    let token_admin = Address::generate(env);
    let (payment_token, token) = create_token(env, &token_admin);

    token.mint(&creator, &1_000_000_000);
    token.mint(&buyer, &1_000_000_000);
    let config = RaffleConfig {
        description: String::from_str(env, "ticket cap"),
        end_time: 0,
        no_deadline: true,
        max_tickets,
        max_tickets_per_tx: max_tickets,
        max_tickets_per_address: cap,
        min_tickets: 1,
        allow_multiple,
        ticket_price: MIN_TICKET_PRICE,
        payment_token,
        prize_amount: MIN_TICKET_PRICE * max_tickets as i128,
        prizes: vec![env, 10_000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(env, &[1; 32]),
        claim_lockup_seconds: Some(0),
        swap_deadline_seconds: Some(0),
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
        category: None,
        unique_winners: false,
        bundles: Vec::new(env),
        prize_token: None,
        nft_contract: None,
    };

    client.init(&factory, &admin, &creator, &config);
    env.as_contract(&contract_id, || env.storage().instance().remove(&DataKey::Factory));
    client.deposit_prize();

    TicketSetup {
        client,
        contract_id,
        admin,
        creator,
        buyer,
        recipient,
        token,
    }
}

#[test]
fn buying_exactly_the_cap_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let setup = setup(&env, 5, true, 10);

    assert_eq!(setup.client.buy_tickets(&setup.buyer, &5), 5);
    assert_eq!(setup.client.get_remaining_ticket_allowance(&setup.buyer), 0);
}

#[test]
fn buying_beyond_the_cap_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let setup = setup(&env, 5, true, 10);

    setup.client.buy_tickets(&setup.buyer, &5);
    assert_eq!(
        setup.client.try_buy_tickets(&setup.buyer, &1),
        Err(Ok(Error::ExceedsMaxTicketsPerAddress))
    );
}

#[test]
fn cap_is_enforced_across_transactions() {
    let env = Env::default();
    env.mock_all_auths();
    let setup = setup(&env, 5, true, 10);

    setup.client.buy_tickets(&setup.buyer, &3);
    assert_eq!(
        setup.client.try_buy_tickets(&setup.buyer, &3),
        Err(Ok(Error::ExceedsMaxTicketsPerAddress))
    );
    assert_eq!(setup.client.get_remaining_ticket_allowance(&setup.buyer), 2);
}

#[test]
fn zero_cap_is_unlimited_up_to_raffle_capacity() {
    let env = Env::default();
    env.mock_all_auths();
    let setup = setup(&env, 0, true, 10);

    setup.client.buy_tickets(&setup.buyer, &5);
    assert_eq!(setup.client.buy_tickets(&setup.buyer, &5), 10);
    assert_eq!(setup.client.get_remaining_ticket_allowance(&setup.buyer), 0);
}

#[test]
fn configured_cap_supersedes_allow_multiple() {
    let env = Env::default();
    env.mock_all_auths();
    let setup = setup(&env, 3, false, 10);

    assert_eq!(setup.client.buy_tickets(&setup.buyer, &2), 2);
    assert_eq!(setup.client.get_remaining_ticket_allowance(&setup.buyer), 1);
}

#[test]
fn gifted_tickets_count_against_recipient_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let setup = setup(&env, 2, true, 10);

    setup.client.buy_tickets_for(&setup.buyer, &setup.recipient, &2);
    assert_eq!(setup.client.get_remaining_ticket_allowance(&setup.recipient), 0);
    assert_eq!(setup.client.get_remaining_ticket_allowance(&setup.buyer), 2);
    assert_eq!(
        setup.client.try_buy_tickets_for(&setup.buyer, &setup.recipient, &1),
        Err(Ok(Error::ExceedsMaxTicketsPerAddress))
    );
}

#[test]
fn cap_cannot_exceed_max_tickets() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);
    let factory = env.register(MockFactory, ());
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (payment_token, _token) = create_token(&env, &token_admin);
    let config = RaffleConfig {
        description: String::from_str(&env, "invalid cap"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 2,
        max_tickets_per_tx: 2,
        max_tickets_per_address: 3,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token,
        prize_amount: MIN_TICKET_PRICE * 2,
        prizes: vec![&env, 10_000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[1; 32]),
        claim_lockup_seconds: Some(0),
        swap_deadline_seconds: Some(0),
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
        category: None,
        unique_winners: false,
        bundles: Vec::new(&env),
        prize_token: None,
        nft_contract: None,
    };

    assert_eq!(
        client.try_init(&factory, &admin, &creator, &config),
        Err(Ok(Error::InvalidParameters))
    );
}

fn setup_commit_reveal(env: &Env, max_tickets: u32) -> TicketSetup<'_> {
    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(env, &contract_id);
    let factory = env.register(MockFactory, ());
    let admin = Address::generate(env);
    let creator = Address::generate(env);
    let buyer = Address::generate(env);
    let recipient = Address::generate(env);
    let token_admin = Address::generate(env);
    let (payment_token, token) = create_token(env, &token_admin);

    token.mint(&creator, &1_000_000_000);
    token.mint(&buyer, &1_000_000_000);
    let config = RaffleConfig {
        description: String::from_str(env, "commit reveal"),
        end_time: 0,
        no_deadline: true,
        max_tickets,
        max_tickets_per_tx: max_tickets,
        max_tickets_per_address: max_tickets,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token,
        prize_amount: MIN_TICKET_PRICE * max_tickets as i128,
        prizes: vec![env, 10_000u32],
        randomness_source: RandomnessSource::CommitReveal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(env, &[1; 32]),
        claim_lockup_seconds: Some(0),
        swap_deadline_seconds: Some(0),
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
        category: None,
        unique_winners: false,
        bundles: Vec::new(env),
        prize_token: None,
        nft_contract: None,
    };

    client.init(&factory, &admin, &creator, &config);
    env.as_contract(&contract_id, || env.storage().instance().remove(&DataKey::Factory));
    client.deposit_prize();

    TicketSetup {
        client,
        contract_id,
        admin,
        creator,
        buyer,
        recipient,
        token,
    }
}

#[test]
fn submit_commit_rejects_overwrite_attack() {
    let env = Env::default();
    env.mock_all_auths();
    // Leave room so the raffle stays Active after one ticket purchase.
    let setup = setup_commit_reveal(&env, 10);

    let sold = setup.client.buy_tickets(&setup.buyer, &1);
    assert_eq!(sold, 1);
    let ticket_id: u32 = 1;

    let hash1 = BytesN::from_array(&env, &[1u8; 32]);
    let hash2 = BytesN::from_array(&env, &[2u8; 32]);

    assert_eq!(setup.client.submit_commit(&ticket_id, &hash1), ());

    assert_eq!(
        setup.client.try_submit_commit(&ticket_id, &hash2),
        Err(Ok(Error::CommitAlreadySubmitted))
    );

    // Original commitment must still be stored (not overwritten).
    let stored: crate::CommitRevealEntry = env.as_contract(&setup.contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::CommitEntry(ticket_id))
            .expect("commit entry must exist")
    });
    assert_eq!(stored.hash, hash1);
}

#[test]
fn submit_commit_rejects_when_not_active() {
    let env = Env::default();
    env.mock_all_auths();
    // Fill the raffle so status moves toward Drawing when capacity is reached
    // (matches existing sell-out behaviour in this crate).
    let setup = setup_commit_reveal(&env, 1);

    let sold = setup.client.buy_tickets(&setup.buyer, &1);
    assert_eq!(sold, 1);

    let hash = BytesN::from_array(&env, &[3u8; 32]);
    // After the last ticket is sold, status should no longer be Active for commits.
    assert_eq!(
        setup.client.try_submit_commit(&1u32, &hash),
        Err(Ok(Error::InvalidStatus))
    );
}
