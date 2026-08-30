mod governance;

    use super::*;
    use raffle_shared::{RandomnessSource, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT};
    use soroban_sdk::{String, Vec as SdkVec, Val, IntoVal, Symbol};

    pub fn assert_event<T: IntoVal<Env, Val>>(
        env: &Env,
        expected_contract: &Address,
        expected_topic: &str,
        expected_payload: T,
    ) {
        let events = env.events().all();
        let last = events.last().unwrap();
        assert_eq!(&last.0, expected_contract);
        assert_eq!(last.1.get(0).unwrap(), Symbol::new(env, "tikka").into_val(env));
        assert_eq!(last.1.get(1).unwrap(), Symbol::new(env, expected_topic).into_val(env));
        assert_eq!(last.2, expected_payload.into_val(env));
    }
    use raffle_shared::{LeaderboardMetric, RandomnessSource, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT};
    use soroban_sdk::{String, Vec as SdkVec};
    use soroban_sdk::testutils::{Ledger, MockAuth, MockAuthInvoke};

    fn setup_factory(env: &Env) -> (RaffleFactoryClient<'_>, Address, Address) {
        let admin = Address::generate(env);
        let treasury = Address::generate(env);
        let wasm_hash = BytesN::from_array(env, &[0u8; 32]);

        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(env, &contract_id);
        env.mock_all_auths();
        client.init_factory(&admin, &wasm_hash, &0u32, &treasury);
        client.set_creation_delay(&0u64);

        (client, admin, treasury)
    }

    fn test_raffle_config(env: &Env, payment_token: &Address) -> RaffleConfig {
        RaffleConfig {
            description: String::from_str(env, "Test Raffle"),
            end_time: 0,
            no_deadline: true,
            max_tickets: 10,
            max_tickets_per_tx: 10,
            min_tickets: 1,
            allow_multiple: true,
            ticket_price: 10_000,
            payment_token: payment_token.clone(),
            prize_amount: 10_000,
            prizes: SdkVec::from_array(env, [10_000u32]),
            randomness_source: RandomnessSource::Internal,
            oracle_address: None,
            protocol_fee_bp: 0,
            treasury_address: None,
            swap_router: None,
            tikka_token: None,
            metadata_hash: BytesN::from_array(env, &[1u8; 32]),
            claim_lockup_seconds: 0,
            swap_deadline_seconds: 0,
            early_bird_ticket_percentage: 0,
            early_bird_discount_bp: 0,
            category: None,
            unique_winners: false,
        }
    }

    fn create_raffles_via_factory(
        env: &Env,
        client: &RaffleFactoryClient<'_>,
        admin: &Address,
        treasury: &Address,
        creator: &Address,
        count: u32,
    ) -> SdkVec<Address> {
        use raffle_instance::ContractClient as RaffleInstanceClient;

        let factory_address = client.address.clone();
        let token_admin = Address::generate(env);
        let payment_token = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let protocol_fee_bp: u32 = env.as_contract(&factory_address, || {
            env.storage()
                .persistent()
                .get(&DataKey::ProtocolFeeBP)
                .unwrap_or(0)
        });

        let mut addrs = SdkVec::new(env);
        for _ in 0..count {
            let mut config = test_raffle_config(env, &payment_token);
            config.protocol_fee_bp = protocol_fee_bp;
            config.treasury_address = Some(treasury.clone());

            let raffle_address = env.register(raffle_instance::Contract, ());
            RaffleInstanceClient::new(env, &raffle_address).init(
                &factory_address,
                admin,
                creator,
                &config,
            );

            env.as_contract(&factory_address, || {
                let stable_id: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::NextRaffleId)
                    .unwrap_or(0u32);
                env.storage()
                    .persistent()
                    .set(&DataKey::RaffleById(stable_id), &raffle_address);
                env.storage()
                    .persistent()
                    .set(&DataKey::NextRaffleId, &(stable_id.saturating_add(1)));
                let live_count: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::RaffleCount)
                    .unwrap_or(0u32)
                    .saturating_add(1);
                env.storage()
                    .persistent()
                    .set(&DataKey::RaffleCount, &live_count);
            });

            addrs.push_back(raffle_address);
        }
        addrs
    }

    #[test]
    fn test_init_factory() {
        let env = Env::default();
        env.mock_all_auths();
        
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);

        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);
        
        let start_events = env.events().all().len();
        client.init_factory(&admin, &wasm_hash, &0u32, &treasury);
        assert_eq!(env.events().all().len(), start_events + 1);

        assert_event(
            &env,
            &client.address,
            "factory_initialized",
            events::FactoryInitialized {
                admin: admin.clone(),
                protocol_fee_bp: 0,
                treasury: treasury.clone(),
                timestamp: env.ledger().timestamp(),
            },
        );

        assert_eq!(client.get_admin(), admin);
    }

    #[test]
    fn test_record_volume_overflow() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let asset = Address::generate(&env);

        client.record_volume(&asset, &(i128::MAX - 1));
        assert_eq!(client.get_total_volume(&asset), i128::MAX - 1);
        let start_events = env.events().all().len();
        assert!(client.try_record_volume(&asset, &2).is_err());
        assert_eq!(env.events().all().len(), start_events);
        assert_eq!(client.get_total_volume(&asset), i128::MAX - 1);
    }

    #[test]
    fn test_propose_fee_change_rejects_excessive_protocol_fee() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let excessive_fee = MAX_PROTOCOL_FEE_BP + 1;

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_propose_fee_change(&excessive_fee),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_init_factory_rejects_second_call() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);
        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);

        client.init_factory(&admin, &wasm_hash, &0u32, &treasury);
        let start_events = env.events().all().len();
        assert_eq!(
            client.try_init_factory(&admin, &wasm_hash, &0u32, &treasury),
            Err(Ok(ContractError::AlreadyInitialized))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    /// Strkey of the all-zero contract id (the "zero address").
    const ZERO_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4";

    fn zero_address(env: &Env) -> Address {
        Address::from_string(&String::from_str(env, ZERO_CONTRACT))
    }

    #[test]
    fn test_init_factory_rejects_zero_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let treasury = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);
        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_init_factory(&zero_address(&env), &wasm_hash, &0u32, &treasury),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_init_factory_rejects_zero_treasury() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);
        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_init_factory(&admin, &wasm_hash, &0u32, &zero_address(&env)),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_init_factory_rejects_self_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let treasury = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);
        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_init_factory(&contract_id, &wasm_hash, &0u32, &treasury),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_init_factory_rejects_self_treasury() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);
        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_init_factory(&admin, &wasm_hash, &0u32, &contract_id),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_transfer_factory_admin_rejects_zero_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_transfer_factory_admin(&zero_address(&env)),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_transfer_factory_admin_rejects_self() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let self_address = client.address.clone();

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_transfer_factory_admin(&self_address),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_propose_config_change_rejects_zero_treasury() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_propose_config_change(&ConfigKey::Treasury, &zero_address(&env)),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_propose_config_change_rejects_self_treasury() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let self_address = client.address.clone();

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_propose_config_change(&ConfigKey::Treasury, &self_address),
            Err(Ok(ContractError::InvalidParameters))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_upgrade_requires_admin_authorization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);
        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);
        env.mock_all_auths();
        client.init_factory(&admin, &wasm_hash, &0u32, &treasury);

        let new_hash = BytesN::from_array(&env, &[9u8; 32]);
        // Without auth for the admin address, upgrade must not succeed.
        env.set_auths(&[]);
        let start_events = env.events().all().len();
        assert!(client.try_upgrade(&new_hash).is_err());
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_upgrade_lifecycle_preserves_state() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);
        let contract_id = env.register(RaffleFactory, ());
        let client = RaffleFactoryClient::new(&env, &contract_id);
        client.init_factory(&admin, &wasm_hash, &0u32, &treasury);

        let creator = Address::generate(&env);
        let payment_token = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
        let mut config = test_raffle_config(&env, &payment_token);
        config.protocol_fee_bp = 0;
        config.treasury_address = Some(treasury.clone());
        let raffle_address = client.create_raffle(&creator, &config);

        let new_hash = BytesN::from_array(&env, &[9u8; 32]);
        let op_id = client.propose_wasm_upgrade(&new_hash);
        assert_eq!(client.get_pending_op(&op_id).unwrap().op, AdminOp::UpdateWasmHash(new_hash.clone()));

        let err = client.try_execute_config_change(&op_id);
        assert_eq!(err.err(), Some(Ok(ContractError::TimelockNotElapsed)));

        env.ledger().with_mut(|l| l.timestamp += TIMELOCK_DELAY_SECONDS + 1);
        client.execute_config_change(&op_id);

        let pending = client.get_pending_op(&op_id);
        assert!(pending.is_none());
        let raffle = raffle_instance::ContractClient::new(&env, &raffle_address);
        let raffle_state = raffle.get_raffle();
        assert_eq!(raffle_state.creator, creator);
        assert_eq!(raffle_state.treasury_address, Some(treasury.clone()));
    }

    // -----------------------------------------------------------------------
    // Stable-index storage tests (new with #426)
    //
    // These tests exercise the new storage layout directly via `env.as_contract`
    // to avoid the Soroban limitation that `env.register_at` cannot be called
    // from within an active contract invocation (which the test shim in
    // `create_raffle` does).  This approach tests the storage semantics cleanly.
    // -----------------------------------------------------------------------

    /// Seed the factory's stable-map storage with `n` synthetic raffle entries.
    fn seed_raffles(env: &Env, factory_id: &Address, n: u32) -> Vec<Address> {
        let mut addrs = Vec::new(env);
        env.as_contract(factory_id, || {
            for i in 0..n {
                let addr = Address::generate(env);
                env.storage()
                    .persistent()
                    .set(&DataKey::RaffleById(i), &addr);
                addrs.push_back(addr);
            }
            env.storage().persistent().set(&DataKey::NextRaffleId, &n);
            env.storage().persistent().set(&DataKey::RaffleCount, &n);
        });
        addrs
    }

    #[test]
    fn test_stable_ids_initial_state() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        // Before any raffle: NextRaffleId == 0, RaffleCount == 0.
        assert_eq!(client.get_next_raffle_id(), 0u32);
        assert_eq!(client.get_raffle_count(), 0u32);
        assert_eq!(client.get_raffle_by_id(&0u32), None);
    }

    #[test]
    fn test_stable_ids_seeded_lookup() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let addrs = seed_raffles(&env, &client.address, 3);

        assert_eq!(client.get_next_raffle_id(), 3u32);
        assert_eq!(client.get_raffle_count(), 3u32);
        assert_eq!(client.get_raffle_by_id(&0u32), Some(addrs.get(0).unwrap()));
        assert_eq!(client.get_raffle_by_id(&1u32), Some(addrs.get(1).unwrap()));
        assert_eq!(client.get_raffle_by_id(&2u32), Some(addrs.get(2).unwrap()));
        // Non-existent ID returns None.
        assert_eq!(client.get_raffle_by_id(&99u32), None);
    }

    #[test]
    fn test_get_raffles_page_returns_correct_slice() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let addrs = seed_raffles(&env, &client.address, 5);

        // Page 0: offset=0, limit=3 → IDs 0,1,2.
        let page = client.get_raffles_page(&raffle_shared::PaginationParams {
            limit: 3,
            offset: 0,
        });
        assert_eq!(page.items.len(), 3u32);
        assert_eq!(page.items.get(0).unwrap(), addrs.get(0).unwrap());
        assert_eq!(page.items.get(2).unwrap(), addrs.get(2).unwrap());
        assert!(page.has_more);

        // Page 1: offset=3, limit=3 → IDs 3,4 (only 2 remain).
        let page2 = client.get_raffles_page(&raffle_shared::PaginationParams {
            limit: 3,
            offset: 3,
        });
        assert_eq!(page2.items.len(), 2u32);
        assert_eq!(page2.items.get(0).unwrap(), addrs.get(3).unwrap());
        assert_eq!(page2.items.get(1).unwrap(), addrs.get(4).unwrap());
        assert!(!page2.has_more);

        // Out-of-range offset → empty.
        let page3 = client.get_raffles_page(&raffle_shared::PaginationParams {
            limit: 10,
            offset: 99,
        });
        assert_eq!(page3.items.len(), 0u32);
        assert!(!page3.has_more);
    }

    #[test]
    fn test_get_raffles_page_skips_tombstoned_slots() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let addrs = seed_raffles(&env, &client.address, 3);

        // Tombstone slot 1 directly in storage.
        env.as_contract(&client.address, || {
            env.storage()
                .persistent()
                .remove(&DataKey::RaffleById(1u32));
            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::RaffleCount)
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&DataKey::RaffleCount, &count.saturating_sub(1));
        });

        assert_eq!(client.get_raffle_count(), 2u32);
        assert_eq!(client.get_next_raffle_id(), 3u32); // monotonic, unchanged
        assert_eq!(client.get_raffle_by_id(&1u32), None);

        // Page over all IDs; tombstoned slot 1 is skipped.
        let page = client.get_raffles_page(&raffle_shared::PaginationParams {
            limit: 10,
            offset: 0,
        });
        assert_eq!(page.items.len(), 2u32);
        assert_eq!(page.items.get(0).unwrap(), addrs.get(0).unwrap());
        assert_eq!(page.items.get(1).unwrap(), addrs.get(2).unwrap());
    }

    #[test]
    fn get_raffles_page_empty_list() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let page = client.get_raffles_page(&PaginationParams {
            limit: 10,
            offset: 0,
        });
        assert_eq!(page.items.len(), 0u32);
        assert_eq!(page.total, 0u32);
        assert!(!page.has_more);
    }

    #[test]
    fn get_raffles_page_first_page() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        create_raffles_via_factory(&env, &client, &_admin, &_treasury, &creator, 15);

        let page = client.get_raffles_page(&PaginationParams {
            limit: 10,
            offset: 0,
        });
        assert_eq!(page.items.len(), 10u32);
        assert_eq!(page.total, 15u32);
        assert!(page.has_more);
    }

    #[test]
    fn get_raffles_page_last_page() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        create_raffles_via_factory(&env, &client, &_admin, &_treasury, &creator, 15);

        let page = client.get_raffles_page(&PaginationParams {
            limit: 10,
            offset: 10,
        });
        assert_eq!(page.items.len(), 5u32);
        assert_eq!(page.total, 15u32);
        assert!(!page.has_more);
    }

    #[test]
    fn get_raffles_page_offset_beyond_total() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        create_raffles_via_factory(&env, &client, &_admin, &_treasury, &creator, 5);

        let page = client.get_raffles_page(&PaginationParams {
            limit: 10,
            offset: 10,
        });
        assert_eq!(page.items.len(), 0u32);
        assert_eq!(page.total, 5u32);
        assert!(!page.has_more);
    }

    #[test]
    fn get_raffles_page_limit_zero_uses_default() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        create_raffles_via_factory(&env, &client, &_admin, &_treasury, &creator, 150);

        let page = client.get_raffles_page(&PaginationParams {
            limit: 0,
            offset: 0,
        });
        assert_eq!(page.items.len(), DEFAULT_PAGE_LIMIT);
        assert_eq!(page.total, 150u32);
        assert!(page.has_more);
    }

    #[test]
    fn get_raffles_page_limit_above_max_is_clamped() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        create_raffles_via_factory(&env, &client, &_admin, &_treasury, &creator, 250);

        let page = client.get_raffles_page(&PaginationParams {
            limit: 999,
            offset: 0,
        });
        assert_eq!(page.items.len(), MAX_PAGE_LIMIT);
        assert_eq!(page.total, 250u32);
        assert!(page.has_more);
    }

    #[test]
    fn test_clean_old_raffle_invalid_id_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        // No raffles → any ID is invalid.
        let start_events = env.events().all().len();
        assert_eq!(
            client.try_clean_old_raffle(&0u32),
            Err(Ok(ContractError::InvalidRaffleId))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_clean_old_raffle_already_tombstoned_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        seed_raffles(&env, &client.address, 3);

        // Tombstone slot 1.
        env.as_contract(&client.address, || {
            env.storage()
                .persistent()
                .remove(&DataKey::RaffleById(1u32));
        });

        // Trying to clean it again must return InvalidRaffleId.
        let start_events = env.events().all().len();
        assert_eq!(
            client.try_clean_old_raffle(&1u32),
            Err(Ok(ContractError::InvalidRaffleId))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    // -----------------------------------------------------------------------
    // Creator index tests
    // -----------------------------------------------------------------------

    /// Seed the per-creator index directly in storage with `addrs`.
    fn seed_creator_index(env: &Env, factory_id: &Address, creator: &Address, addrs: &[Address]) {
        env.as_contract(factory_id, || {
            let mut v: Vec<Address> = Vec::new(env);
            for a in addrs {
                v.push_back(a.clone());
            }
            env.storage()
                .persistent()
                .set(&DataKey::CreatorRaffles(creator.clone()), &v);
        });
    }

    #[test]
    fn test_get_raffles_by_creator_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);

        let page = client.get_raffles_by_creator(
            &creator,
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(page.items.len(), 0u32);
        assert_eq!(page.total, 0u32);
        assert!(!page.has_more);
    }

    #[test]
    fn test_get_raffles_by_creator_basic() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let creator_a = Address::generate(&env);
        let creator_b = Address::generate(&env);

        // 5 raffles for A, 3 for B.
        let mut a_addrs = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];
        let b_addrs = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        seed_creator_index(&env, &client.address, &creator_a, &a_addrs);
        seed_creator_index(&env, &client.address, &creator_b, &b_addrs);

        // Creator A: full page.
        let page_a = client.get_raffles_by_creator(
            &creator_a,
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(page_a.total, 5u32);
        assert_eq!(page_a.items.len(), 5u32);
        assert!(!page_a.has_more);
        for (i, addr) in a_addrs.iter().enumerate() {
            assert_eq!(page_a.items.get(i as u32).unwrap(), addr.clone());
        }

        // Creator B: full page.
        let page_b = client.get_raffles_by_creator(
            &creator_b,
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(page_b.total, 3u32);
        assert_eq!(page_b.items.len(), 3u32);
        assert!(!page_b.has_more);
        for (i, addr) in b_addrs.iter().enumerate() {
            assert_eq!(page_b.items.get(i as u32).unwrap(), addr.clone());
        }
    }

    #[test]
    fn test_get_raffles_by_creator_pagination() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let creator = Address::generate(&env);
        let addrs = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];
        seed_creator_index(&env, &client.address, &creator, &addrs);

        // Page 0: offset=0, limit=3 → items 0,1,2; has_more=true.
        let p0 = client.get_raffles_by_creator(
            &creator,
            &raffle_shared::PaginationParams {
                limit: 3,
                offset: 0,
            },
        );
        assert_eq!(p0.items.len(), 3u32);
        assert_eq!(p0.total, 5u32);
        assert!(p0.has_more);
        assert_eq!(p0.items.get(0).unwrap(), addrs[0].clone());
        assert_eq!(p0.items.get(2).unwrap(), addrs[2].clone());

        // Page 1: offset=3, limit=3 → items 3,4; has_more=false.
        let p1 = client.get_raffles_by_creator(
            &creator,
            &raffle_shared::PaginationParams {
                limit: 3,
                offset: 3,
            },
        );
        assert_eq!(p1.items.len(), 2u32);
        assert_eq!(p1.total, 5u32);
        assert!(!p1.has_more);
        assert_eq!(p1.items.get(0).unwrap(), addrs[3].clone());
        assert_eq!(p1.items.get(1).unwrap(), addrs[4].clone());

        // Out-of-range offset → empty, has_more=false.
        let p_oor = client.get_raffles_by_creator(
            &creator,
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 99,
            },
        );
        assert_eq!(p_oor.items.len(), 0u32);
        assert!(!p_oor.has_more);

        // Exact boundary: offset=5 (== total) → empty.
        let p_exact = client.get_raffles_by_creator(
            &creator,
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 5,
            },
        );
        assert_eq!(p_exact.items.len(), 0u32);
        assert!(!p_exact.has_more);
    }

    #[test]
    fn test_creator_index_isolates_separate_creators() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let creator_a = Address::generate(&env);
        let creator_b = Address::generate(&env);

        let a_addrs = [Address::generate(&env), Address::generate(&env)];
        let b_addrs = [Address::generate(&env)];

        seed_creator_index(&env, &client.address, &creator_a, &a_addrs);
        seed_creator_index(&env, &client.address, &creator_b, &b_addrs);

        // A sees only its own raffles.
        let pa = client.get_raffles_by_creator(
            &creator_a,
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(pa.total, 2u32);

        // B sees only its own raffle.
        let pb = client.get_raffles_by_creator(
            &creator_b,
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(pb.total, 1u32);
        assert_eq!(pb.items.get(0).unwrap(), b_addrs[0].clone());
    }

    // -----------------------------------------------------------------------
    // Factory admin two-step transfer tests (#453)
    // -----------------------------------------------------------------------

    #[test]
    fn test_admin_transfer_two_step_completes_correctly() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let new_admin = Address::generate(&env);

        let start_events = env.events().all().len();
        client.transfer_factory_admin(&new_admin);
        
        assert_event(
            &env,
            &client.address,
            "admin_transfer_proposed",
            events::AdminTransferProposed {
                current_admin: _admin.clone(),
                proposed_admin: new_admin.clone(),
                timestamp: env.ledger().timestamp(),
            },
        );

        client.accept_factory_admin();
        
        assert_event(
            &env,
            &client.address,
            "admin_transfer_accepted",
            events::AdminTransferAccepted {
                old_admin: _admin.clone(),
                new_admin: new_admin.clone(),
                timestamp: env.ledger().timestamp(),
            },
        );
        assert_eq!(env.events().all().len(), start_events + 2);

        let actual: Address = env.as_contract(&client.address, || {
            env.storage().persistent().get(&DataKey::Admin).unwrap()
        });
        assert_eq!(actual, new_admin);

        let pending_still_exists: bool = env.as_contract(&client.address, || {
            env.storage().persistent().has(&DataKey::PendingAdmin)
        });
        assert!(!pending_still_exists);
    }

    #[test]
    fn test_admin_transfer_rejected_if_pending_already_exists() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);
        let admin_b = Address::generate(&env);
        let admin_c = Address::generate(&env);

        client.transfer_factory_admin(&admin_b);

        let start_events = env.events().all().len();
        assert_eq!(
            client.try_transfer_factory_admin(&admin_c),
            Err(Ok(ContractError::AdminTransferPending))
        );
        assert_eq!(env.events().all().len(), start_events);
    }

    #[test]
    fn test_admin_accept_fails_if_wrong_address_accepts() {
        let env = Env::default();
        let (client, _admin, _treasury) = setup_factory(&env);
        let admin_b = Address::generate(&env);
        let admin_c = Address::generate(&env);

        env.mock_all_auths();
        client.transfer_factory_admin(&admin_b);

        env.mock_auths(&[MockAuth {
            address: &admin_c,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "accept_factory_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }]);
        assert!(client.try_accept_factory_admin().is_err());
    }

    #[test]
    fn test_admin_transfer_to_same_address_clears_pending() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _treasury) = setup_factory(&env);
        let new_admin = Address::generate(&env);

        client.transfer_factory_admin(&new_admin);

        let pending_before: bool = env.as_contract(&client.address, || {
            env.storage().persistent().has(&DataKey::PendingAdmin)
        });
        assert!(pending_before);

        // Proposing the current admin clears the pending entry
        let start_events = env.events().all().len();
        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "transfer_factory_admin",
                args: (&admin,).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.transfer_factory_admin(&admin);
        assert_eq!(env.events().all().len(), start_events);

        let pending_after: bool = env.as_contract(&client.address, || {
            env.storage().persistent().has(&DataKey::PendingAdmin)
        });
        assert!(!pending_after);

        let actual: Address = env.as_contract(&client.address, || {
            env.storage().persistent().get(&DataKey::Admin).unwrap()
        });
        assert_eq!(actual, admin);
    }

    #[test]
    fn test_only_new_admin_can_accept_transfer() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _treasury) = setup_factory(&env);
        let new_admin = Address::generate(&env);

        client.transfer_factory_admin(&new_admin);

        // Old admin tries to accept — should fail because require_auth checks caller == PendingAdmin
        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "accept_factory_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }]);
        assert!(client.try_accept_factory_admin().is_err());
    }

    // -----------------------------------------------------------------------
    // Rate limiter tests (#447)
    //
    // The rate limiter lives inside `create_raffle` and gates non-whitelisted
    // creators to at most one creation per `MinCreationDelay` seconds.  These
    // tests exercise the full `create_raffle` path (deploying a real instance
    // via the test shim) so the guard is validated end-to-end.
    // -----------------------------------------------------------------------

    /// A complete, valid `RaffleConfig` for rate-limiter tests.  Prize tiers sum
    /// to 10_000 bp and `prize_amount >= ticket_price`, satisfying instance init.
    fn rate_limit_config(env: &Env, payment_token: &Address, desc: &str) -> RaffleConfig {
        RaffleConfig {
            description: String::from_str(env, desc),
            end_time: 0,
            no_deadline: true,
            max_tickets: 10,
            max_tickets_per_tx: 10,
            min_tickets: 1,
            allow_multiple: true,
            ticket_price: 10_000,
            payment_token: payment_token.clone(),
            prize_amount: 10_000,
            prizes: SdkVec::from_array(env, [10_000u32]),
            randomness_source: RandomnessSource::Internal,
            oracle_address: None,
            protocol_fee_bp: 0,
            treasury_address: None,
            swap_router: None,
            tikka_token: None,
            metadata_hash: BytesN::from_array(env, &[1u8; 32]),
            claim_lockup_seconds: None,
            swap_deadline_seconds: None,
            early_bird_ticket_percentage: 0,
            early_bird_discount_bp: 0,
            category: None,
        }
    }

    /// Register a payment token the instance init will accept.
    fn make_token(env: &Env) -> Address {
        let token_admin = Address::generate(env);
        env.register_stellar_asset_contract_v2(token_admin)
            .address()
    }

    #[test]
    fn non_whitelisted_creator_is_rate_limited() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let (client, _admin, _treasury) = setup_factory(&env);
        // Use a real, non-zero delay (setup_factory zeroes it by default).
        let delay: u64 = 300;
        client.set_creation_delay(&delay);

        let creator = Address::generate(&env);
        let token = make_token(&env);

        // 1. First creation succeeds.
        client.create_raffle(&creator, &rate_limit_config(&env, &token, "r1"));

        // 2. Immediate second creation is rate-limited.
        let start_events = env.events().all().len();
        // Since create_raffle emits CreationRateLimited *before* returning the error,
        // we check that length increases by 1.
        assert_eq!(
            client.try_create_raffle(&creator, &rate_limit_config(&env, &token, "r2")),
            Err(Ok(ContractError::RateLimitExceeded))
        );
        assert_eq!(env.events().all().len(), start_events + 1);

        // 3. Advance time by exactly MinCreationDelay.
        env.ledger().set_timestamp(1_000 + delay);

        // 4. Creation succeeds again once the window has elapsed.
        client.create_raffle(&creator, &rate_limit_config(&env, &token, "r3"));
    }

    #[test]
    fn whitelisted_partner_bypasses_rate_limit() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let (client, _admin, _treasury) = setup_factory(&env);
        client.set_creation_delay(&300u64);

        let creator = Address::generate(&env);
        let token = make_token(&env);

        // Whitelist the creator, then create twice back-to-back with no time
        // advance — both must succeed because the whitelist bypasses the limiter.
        client.set_whitelist_status(&creator, &true);
        client.create_raffle(&creator, &rate_limit_config(&env, &token, "w1"));
        client.create_raffle(&creator, &rate_limit_config(&env, &token, "w2"));
    }

    #[test]
    fn partner_dashboard_tracks_stats_across_creations() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(5_000);

        let (client, _admin, _treasury) = setup_factory(&env);
        let partner = Address::generate(&env);
        let outsider = Address::generate(&env);
        let token = make_token(&env);

        // Non-partners get None.
        assert!(client.get_partner_stats(&partner).is_none());

        client.set_whitelist_status(&partner, &true);

        // Whitelisted with no raffles yet → zeroed stats.
        let empty = client.get_partner_stats(&partner).unwrap();
        assert_eq!(empty.total_raffles, 0);
        assert_eq!(empty.total_volume, 0);
        assert_eq!(empty.total_fees_generated, 0);

        let partners = client.get_all_partners(&PaginationParams {
            limit: 10,
            offset: 0,
        });
        assert_eq!(partners.len(), 1);
        assert_eq!(partners.get(0).unwrap(), partner);

        // Create three raffles at distinct timestamps.
        client.create_raffle(&partner, &rate_limit_config(&env, &token, "p1"));
        env.ledger().set_timestamp(5_100);
        client.create_raffle(&partner, &rate_limit_config(&env, &token, "p2"));
        env.ledger().set_timestamp(5_200);
        client.create_raffle(&partner, &rate_limit_config(&env, &token, "p3"));

        let stats = client.get_partner_stats(&partner).unwrap();
        assert_eq!(stats.total_raffles, 3);
        assert_eq!(stats.first_raffle_at, 5_000);
        assert_eq!(stats.latest_raffle_at, 5_200);
        assert_eq!(stats.total_volume, 0);
        assert_eq!(stats.total_fees_generated, 0);

        // Outsider still None; de-whitelisting hides stats.
        assert!(client.get_partner_stats(&outsider).is_none());
        client.set_whitelist_status(&partner, &false);
        assert!(client.get_partner_stats(&partner).is_none());
        assert_eq!(
            client
                .get_all_partners(&PaginationParams {
                    limit: 10,
                    offset: 0,
                })
                .len(),
            0
        );
    }

    #[test]
    fn set_creation_delay_affects_rate_limiter() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let (client, _admin, _treasury) = setup_factory(&env);
        client.set_creation_delay(&60u64);

        let creator = Address::generate(&env);
        let token = make_token(&env);

        // 1. Create at t=1000.
        client.create_raffle(&creator, &rate_limit_config(&env, &token, "d1"));

        // 2. Advance 59 seconds — still inside the window, second creation fails.
        env.ledger().set_timestamp(1_000 + 59);
        let start_events = env.events().all().len();
        assert_eq!(
            client.try_create_raffle(&creator, &rate_limit_config(&env, &token, "d2")),
            Err(Ok(ContractError::RateLimitExceeded))
        );
        assert_eq!(env.events().all().len(), start_events + 1);

        // 3. Advance 1 more second (60 total) — the window has elapsed, succeeds.
        env.ledger().set_timestamp(1_000 + 60);
        client.create_raffle(&creator, &rate_limit_config(&env, &token, "d3"));
    }

    // -----------------------------------------------------------------------
    // Category index tests (#439)
    // -----------------------------------------------------------------------

    /// Seed the per-category index directly in storage (mirrors
    /// `seed_creator_index`) so `get_raffles_by_category` can be validated
    /// without going through the `create_raffle` deploy shim.
    fn seed_category_index(env: &Env, factory_id: &Address, category: &str, addrs: &[Address]) {
        let cat = String::from_str(env, category);
        env.as_contract(factory_id, || {
            let mut v: Vec<Address> = Vec::new(env);
            for a in addrs {
                v.push_back(a.clone());
            }
            env.storage()
                .persistent()
                .set(&DataKey::CategoryRaffles(cat.clone()), &v);
        });
    }

    #[test]
    fn get_raffles_by_category_unknown_is_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let page = client.get_raffles_by_category(
            &String::from_str(&env, "gaming"),
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(page.items.len(), 0u32);
        assert_eq!(page.total, 0u32);
        assert!(!page.has_more);
    }

    #[test]
    fn get_raffles_by_category_returns_only_matching() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        // 3 raffles tagged "gaming", 2 tagged "art".
        let gaming = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];
        let art = [Address::generate(&env), Address::generate(&env)];

        seed_category_index(&env, &client.address, "gaming", &gaming);
        seed_category_index(&env, &client.address, "art", &art);

        let gaming_page = client.get_raffles_by_category(
            &String::from_str(&env, "gaming"),
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(gaming_page.total, 3u32);
        assert_eq!(gaming_page.items.len(), 3u32);
        assert!(!gaming_page.has_more);
        for (i, addr) in gaming.iter().enumerate() {
            assert_eq!(gaming_page.items.get(i as u32).unwrap(), addr.clone());
        }

        let art_page = client.get_raffles_by_category(
            &String::from_str(&env, "art"),
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(art_page.total, 2u32);
        assert_eq!(art_page.items.len(), 2u32);
        assert!(!art_page.has_more);

        // A category with no raffles yields an empty page.
        let charity_page = client.get_raffles_by_category(
            &String::from_str(&env, "charity"),
            &raffle_shared::PaginationParams {
                limit: 10,
                offset: 0,
            },
        );
        assert_eq!(charity_page.total, 0u32);
        assert_eq!(charity_page.items.len(), 0u32);
    }

    #[test]
    fn get_raffles_by_category_paginates() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        let addrs = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];
        seed_category_index(&env, &client.address, "gaming", &addrs);

        // Page 0: offset=0, limit=3 → items 0,1,2; has_more=true.
        let p0 = client.get_raffles_by_category(
            &String::from_str(&env, "gaming"),
            &raffle_shared::PaginationParams {
                limit: 3,
                offset: 0,
            },
        );
        assert_eq!(p0.items.len(), 3u32);
        assert_eq!(p0.total, 5u32);
        assert!(p0.has_more);
        assert_eq!(p0.items.get(0).unwrap(), addrs[0].clone());
        assert_eq!(p0.items.get(2).unwrap(), addrs[2].clone());

        // Page 1: offset=3, limit=3 → items 3,4; has_more=false.
        let p1 = client.get_raffles_by_category(
            &String::from_str(&env, "gaming"),
            &raffle_shared::PaginationParams {
                limit: 3,
                offset: 3,
            },
        );
        assert_eq!(p1.items.len(), 2u32);
        assert!(!p1.has_more);
        assert_eq!(p1.items.get(0).unwrap(), addrs[3].clone());
        assert_eq!(p1.items.get(1).unwrap(), addrs[4].clone());
    }

    // -----------------------------------------------------------------------
    // Recurring (subscription) raffle tests (#487)
    // -----------------------------------------------------------------------

    fn recurring_config(env: &Env, base: RaffleConfig) -> RecurringRaffleConfig {
        RecurringRaffleConfig {
            base_config: base,
            interval_seconds: 86_400, // 1 day
            max_rounds: 3,
            auto_fund: false,
        }
    }

    fn make_payment_token(env: &Env) -> Address {
        let token_admin = Address::generate(env);
        env.register_stellar_asset_contract_v2(token_admin)
            .address()
    }

    fn valid_base_config(env: &Env, payment_token: &Address) -> RaffleConfig {
        RaffleConfig {
            description: String::from_str(env, "Recurring Raffle"),
            end_time: 0,
            no_deadline: true,
            max_tickets: 10,
            max_tickets_per_tx: 10,
            min_tickets: 1,
            allow_multiple: true,
            ticket_price: 10_000,
            payment_token: payment_token.clone(),
            prize_amount: 10_000,
            prizes: SdkVec::from_array(env, [10_000u32]),
            randomness_source: RandomnessSource::Internal,
            oracle_address: None,
            protocol_fee_bp: 0,
            treasury_address: None,
            swap_router: None,
            tikka_token: None,
            metadata_hash: BytesN::from_array(env, &[1u8; 32]),
            claim_lockup_seconds: 0,
            swap_deadline_seconds: 0,
            early_bird_ticket_percentage: 0,
            early_bird_discount_bp: 0,
            category: None,
        }
    }

    #[test]
    fn test_create_recurring_raffle() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let rc = recurring_config(&env, base);

        let recurring_id = client.create_recurring_raffle(&creator, &rc);
        assert_eq!(recurring_id, 0u32);

        let entry = client
            .get_recurring_raffle(&recurring_id)
            .expect("recurring entry should exist");
        assert_eq!(entry.creator, creator);
        assert_eq!(entry.config.max_rounds, 3);
        assert!(entry.active);
        assert_eq!(entry.current_round, 0);
        assert!(entry.last_raffle_address.is_none());
        assert_eq!(entry.next_due, 1_000_000 + 86_400);
    }

    #[test]
    fn test_create_recurring_raffle_increments_id() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);

        let id0 = client.create_recurring_raffle(&creator, &recurring_config(&env, base.clone()));
        let id1 = client.create_recurring_raffle(&creator, &recurring_config(&env, base));
        assert_eq!(id0, 0u32);
        assert_eq!(id1, 1u32);
    }

    #[test]
    fn test_create_recurring_raffle_rejects_invalid_interval() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);

        let too_short = RecurringRaffleConfig {
            interval_seconds: 3_599,
            ..recurring_config(&env, base.clone())
        };
        assert_eq!(
            client.try_create_recurring_raffle(&creator, &too_short),
            Err(Ok(ContractError::InvalidParameters))
        );

        let too_long = RecurringRaffleConfig {
            interval_seconds: 31_536_001,
            ..recurring_config(&env, base)
        };
        assert_eq!(
            client.try_create_recurring_raffle(&creator, &too_long),
            Err(Ok(ContractError::InvalidParameters))
        );
    }

    #[test]
    fn test_create_recurring_raffle_rejects_auto_fund_infinite() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);

        let bad = RecurringRaffleConfig {
            max_rounds: 0,
            auto_fund: true,
            ..recurring_config(&env, base)
        };
        assert_eq!(
            client.try_create_recurring_raffle(&creator, &bad),
            Err(Ok(ContractError::InvalidParameters))
        );
    }

    #[test]
    fn test_trigger_next_round() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        // Advance past the interval.
        env.ledger().set_timestamp(1_000_000 + 86_400);

        let addr = client.trigger_next_round(&recurring_id);
        assert!(addr != Address::zero(&env));

        let entry = client
            .get_recurring_raffle(&recurring_id)
            .expect("entry exists");
        assert_eq!(entry.current_round, 1);
        assert_eq!(entry.next_due, 1_000_000 + 86_400 + 86_400);
        assert_eq!(entry.last_raffle_address, Some(addr.clone()));

        let instances = client.get_recurring_instances(&recurring_id);
        assert_eq!(instances.len(), 1u32);
        assert_eq!(instances.get(0).unwrap(), addr);
    }

    #[test]
    fn test_trigger_next_round_multiple_rounds() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        let mut addrs = Vec::new(&env);
        for round in 1..=3 {
            env.ledger().set_timestamp(1_000_000 + 86_400 * round as u64);
            let addr = client.trigger_next_round(&recurring_id);
            addrs.push_back(addr);
        }

        let entry = client
            .get_recurring_raffle(&recurring_id)
            .expect("entry exists");
        assert_eq!(entry.current_round, 3);

        let instances = client.get_recurring_instances(&recurring_id);
        assert_eq!(instances.len(), 3u32);
        assert_eq!(instances, addrs);
    }

    #[test]
    fn test_trigger_next_round_interval_not_elapsed() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        // Try trigger at same timestamp — interval not elapsed.
        assert_eq!(
            client.try_trigger_next_round(&recurring_id),
            Err(Ok(ContractError::IntervalNotElapsed))
        );
    }

    #[test]
    fn test_trigger_next_round_max_rounds_reached() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);

        let limited = RecurringRaffleConfig {
            max_rounds: 1,
            ..recurring_config(&env, base)
        };
        let recurring_id = client.create_recurring_raffle(&creator, &limited);

        // Advance past interval and trigger first round.
        env.ledger().set_timestamp(1_000_000 + 86_400);
        let _addr = client.trigger_next_round(&recurring_id);

        // Advance past the next interval and try to trigger again.
        env.ledger().set_timestamp(1_000_000 + 86_400 * 2);
        assert_eq!(
            client.try_trigger_next_round(&recurring_id),
            Err(Ok(ContractError::MaxRoundsReached))
        );
    }

    #[test]
    fn test_trigger_next_round_not_found() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);

        assert_eq!(
            client.try_trigger_next_round(&999u32),
            Err(Ok(ContractError::RecurringNotFound))
        );
    }

    #[test]
    fn test_cancel_recurring_raffle() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        client.cancel_recurring_raffle(&recurring_id, &creator);

        let entry = client
            .get_recurring_raffle(&recurring_id)
            .expect("entry exists");
        assert!(!entry.active);
    }

    #[test]
    fn test_cancel_recurring_raffle_by_admin() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        client.cancel_recurring_raffle(&recurring_id, &admin);

        let entry = client
            .get_recurring_raffle(&recurring_id)
            .expect("entry exists");
        assert!(!entry.active);
    }

    #[test]
    fn test_cancel_recurring_raffle_not_authorized() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let stranger = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        // Stranger tries to cancel — the contract requires auth and the caller
        // is neither creator nor admin, so NotAuthorized must be returned.
        env.mock_auths(&[&stranger]);
        assert_eq!(
            client.try_cancel_recurring_raffle(&recurring_id, &stranger),
            Err(Ok(ContractError::NotAuthorized))
        );
    }

    #[test]
    fn test_cancel_recurring_raffle_not_found() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let caller = Address::generate(&env);

        assert_eq!(
            client.try_cancel_recurring_raffle(&999u32, &caller),
            Err(Ok(ContractError::RecurringNotFound))
        );
    }

    #[test]
    fn test_trigger_recurring_when_inactive() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        // Cancel first.
        client.cancel_recurring_raffle(&recurring_id, &creator);

        // Advance time and try to trigger.
        env.ledger().set_timestamp(1_000_000 + 86_400);
        assert_eq!(
            client.try_trigger_next_round(&recurring_id),
            Err(Ok(ContractError::RecurringInactive))
        );
    }

    #[test]
    fn test_get_recurring_instances_empty_for_new() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);
        let recurring_id = client.create_recurring_raffle(
            &creator,
            &recurring_config(&env, base),
        );

        let instances = client.get_recurring_instances(&recurring_id);
        assert_eq!(instances.len(), 0u32);
        assert_eq!(instances, Vec::new(&env));
    }

    #[test]
    fn test_infinite_recurring_raffle() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_payment_token(&env);
        let base = valid_base_config(&env, &token);

        let infinite = RecurringRaffleConfig {
            max_rounds: 0,
            ..recurring_config(&env, base)
        };
        let recurring_id = client.create_recurring_raffle(&creator, &infinite);

        // Trigger 5 rounds — max_rounds=0 means no cap.
        for round in 1..=5 {
            env.ledger().set_timestamp(1_000_000 + 86_400 * round as u64);
            client.trigger_next_round(&recurring_id);
        }

        let entry = client
            .get_recurring_raffle(&recurring_id)
            .expect("entry exists");
        assert_eq!(entry.current_round, 5);
    }

    #[test]
    fn test_get_recurring_raffle_nonexistent() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        assert!(client.get_recurring_raffle(&999u32).is_none());
    }

    // -----------------------------------------------------------------------
    // Creation-only pause tests (#611)
    // -----------------------------------------------------------------------

    #[test]
    fn test_set_creation_paused_blocks_create_raffle() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_token(&env);

        assert!(!client.is_creation_paused());

        client.set_creation_paused(&true);
        assert!(client.is_creation_paused());

        assert_eq!(
            client.try_create_raffle(&creator, &rate_limit_config(&env, &token, "cp1")),
            Err(Ok(ContractError::CreationPaused))
        );
    }

    #[test]
    fn test_set_creation_paused_unpause_allows_create_raffle() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let (client, _admin, _treasury) = setup_factory(&env);
        let creator = Address::generate(&env);
        let token = make_token(&env);

        client.set_creation_paused(&true);
        client.set_creation_paused(&false);
        assert!(!client.is_creation_paused());

        client.create_raffle(&creator, &rate_limit_config(&env, &token, "cp2"));
    }

    #[test]
    fn test_creation_paused_does_not_affect_full_pause() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _treasury) = setup_factory(&env);

        client.set_creation_paused(&true);
        // The factory-wide pause flag is independent and remains false.
        assert!(!client.is_factory_paused());
    }

    #[test]
    fn test_only_admin_can_set_creation_paused() {
        let env = Env::default();
        let (client, _admin, _treasury) = setup_factory(&env);
        let stranger = Address::generate(&env);

        env.mock_auths(&[MockAuth {
            address: &stranger,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "set_creation_paused",
                args: (true,).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        assert_eq!(
            client.try_set_creation_paused(&true),
            Err(Ok(ContractError::NotAuthorized))
        );
    }
}
