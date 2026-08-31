use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, BytesN, Env, String, Vec};
use tikka_raffle_instance::{Contract, ContractClient, RaffleStatus};
use raffle_shared::{RaffleConfig, RandomnessSource};

pub fn setup(env: &Env, max_tickets: u32) -> (ContractClient<'_>, Address, Address, StellarAssetClient<'_>) {
    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(env, &contract_id);
    let factory = Address::generate(env);
    let admin = Address::generate(env);
    let creator = Address::generate(env);
    let token_admin = Address::generate(env);
    let payment_token = env.register_stellar_asset_contract_v2(token_admin.clone()).address();
    let token = StellarAssetClient::new(env, &payment_token);
    token.mint(&creator, &(max_tickets as i128 * 20_000));

    let config = RaffleConfig {
        description: String::from_str(env, "fuzz"),
        end_time: 0,
        no_deadline: true,
        max_tickets,
        max_tickets_per_tx: max_tickets,
        max_tickets_per_address: 0,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: 10_000,
        payment_token,
        prize_amount: max_tickets as i128 * 10_000,
        prizes: Vec::from_array(env, [10_000]),
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
    env.as_contract(&contract_id, || {
        env.storage().instance().remove(&tikka_raffle_instance::DataKey::Factory);
    });
    client.deposit_prize();
    (client, creator, admin, token)
}

pub fn buy(data: &[u8]) {
    let env = Env::default();
    env.mock_all_auths();
    let (client, creator, _admin, token) = setup(&env, 50);
    let buyer = Address::generate(&env);
    token.mint(&buyer, &1_000_000);
    let quantity = (data.first().copied().unwrap_or(1) as u32 % 5) + 1;
    let _ = client.try_buy_tickets(&buyer, &quantity);
}

pub fn finalize(data: &[u8]) {
    let env = Env::default();
    env.mock_all_auths();
    let (client, creator, _admin, token) = setup(&env, 1);
    token.mint(&creator, &10_000);
    let _ = client.try_finalize_raffle();
    let _ = data;
}

pub fn refund_cancel(data: &[u8]) {
    let env = Env::default();
    env.mock_all_auths();
    let (client, creator, _admin, token) = setup(&env, 10);
    let buyer = Address::generate(&env);
    token.mint(&buyer, &100_000);
    let _ = client.try_buy_tickets(&buyer, &1);
    let _ = client.try_cancel_raffle(&raffle_shared::CancelReason::CreatorCancelled);
    let ticket_id = (data.first().copied().unwrap_or(0) as u32 % 2) + 1;
    let _ = client.try_refund_ticket(&ticket_id);
}

pub fn commit_reveal(data: &[u8]) {
    let env = Env::default();
    env.mock_all_auths();
    let (client, creator, _admin, token) = setup(&env, 2);
    let buyer = Address::generate(&env);
    token.mint(&buyer, &100_000);
    let _ = client.try_buy_tickets(&buyer, &1);
    let hash = BytesN::from_array(&env, &[data.first().copied().unwrap_or(0); 32]);
    let _ = client.try_submit_commit(&1, &hash);
}

pub fn lifecycle(data: &[u8]) {
    let env = Env::default();
    env.mock_all_auths();
    let (client, creator, admin, token) = setup(&env, 20);
    let buyer = Address::generate(&env);
    token.mint(&buyer, &1_000_000);
    let mut paused = false;

    for operation in data.iter().copied() {
        match operation % 6 {
            0 if !paused => {
                let quantity = (operation as u32 % 3) + 1;
                let _ = client.try_buy_tickets(&buyer, &quantity);
            }
            1 => {
                let _ = client.try_finalize_raffle();
            }
            2 => {
                let _ = client.try_claim_prize(&buyer, 0);
            }
            3 => {
                let ticket_id = (operation as u32 % 20) + 1;
                let _ = client.try_refund_ticket(&ticket_id);
            }
            4 => {
                let _ = client.try_cancel_raffle(&raffle_shared::CancelReason::AdminCancelled);
            }
            _ => {
                let result = if paused { client.try_unpause() } else { client.try_pause() };
                if result.is_ok() {
                    paused = !paused;
                }
            }
        }

        if let Ok(Ok(raffle)) = client.try_get_raffle() {
            assert!(raffle.tickets_sold <= raffle.max_tickets);
            assert!(matches!(
                raffle.status,
                RaffleStatus::PendingPrize
                    | RaffleStatus::Active
                    | RaffleStatus::Drawing
                    | RaffleStatus::Finalized
                    | RaffleStatus::Claimed
                    | RaffleStatus::Cancelled
                    | RaffleStatus::Failed
            ));
            assert!(token.balance(&client.address) >= 0);
        }
    }
}
