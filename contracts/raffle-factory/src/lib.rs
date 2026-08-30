#![no_std]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, xdr::ToXdr, Address, Bytes, BytesN,
    Env, IntoVal, Symbol, Vec,
};

#[cfg(test)]
use soroban_sdk::testutils::Address as _;

mod events;

use raffle_shared::{
    effective_limit, AdminOp, FairnessData, PageResultRaffles, PaginationParams, RaffleConfig,
    RecurringRaffleConfig,
};

use raffle_shared::constants::{
    CHECKPOINT_INTERVAL, MAX_PROTOCOL_FEE_BP, MAX_RECURRING_INTERVAL_SECONDS,
    MIN_RECURRING_INTERVAL_SECONDS, TIMELOCK_DELAY_SECONDS,
};

/// A timelocked administrative operation queued for future execution.
///
/// Created by [`RaffleFactory::set_config`] and stored under
/// [`DataKey::PendingOp`] until either executed by
/// [`RaffleFactory::execute_config_change`] or cancelled by
/// [`RaffleFactory::cancel_config_change`].
///
/// See also: [`docs/EVENTS.md`](../../../docs/EVENTS.md) — `AdminOpProposed`,
/// `AdminOpExecuted`, `AdminOpCancelled`.
#[derive(Clone)]
#[contracttype]
pub struct PendingOp {
    /// The operation payload to apply once the timelock elapses.
    pub op: AdminOp,
    /// Unix timestamp (seconds) at which the operation becomes executable.
    /// Equals the ledger timestamp at proposal time plus
    /// [`TIMELOCK_DELAY_SECONDS`] (48 hours).
    pub effective_timestamp: u64,
    /// Address of the admin who proposed this operation.
    pub proposed_by: Address,
}

/// On-chain creator profile with display name, verified badge, and track record.
///
/// Creators can self-set a display name via [`RaffleFactory::set_profile_name`],
/// and the admin can grant a verified badge via [`RaffleFactory::set_verified`].
/// The `raffles_created` counter is automatically incremented on each successful
/// [`RaffleFactory::create_raffle`] call.
///
/// Frontends can query profiles with [`RaffleFactory::get_profile`] to show
/// creator reputation, verified status, and activity level without off-chain
/// infrastructure.
#[derive(Clone)]
#[contracttype]
pub struct CreatorProfile {
    /// Self-set display name (max length [`MAX_DESCRIPTION_LENGTH`]).
    /// Empty string if never set.
    pub name: soroban_sdk::String,
    /// Admin-granted verified badge. `true` indicates a trusted/reputable organizer.
    pub verified: bool,
    /// Number of raffles this creator has successfully launched.
    pub raffles_created: u32,
}

/// A periodic state snapshot recording factory health at a milestone raffle
/// count.
///
/// A checkpoint is automatically created every
/// [`CHECKPOINT_INTERVAL`] (1 000) total raffles. The
/// `aggregate_hash` is a SHA-256 digest of `raffle_count ‖ ledger_sequence ‖
/// ledger_timestamp`, giving indexers a compact, tamper-evident anchor.
///
/// Retrieve checkpoints with [`RaffleFactory::get_checkpoint`] and
/// [`RaffleFactory::get_latest_checkpoint_index`].
///
/// See also: [`docs/EVENTS.md`](../../../docs/EVENTS.md) — `CheckpointCreated`.
#[derive(Clone)]
#[contracttype]
pub struct StateCheckpoint {
    /// Sequential 1-based checkpoint index (`raffle_count / CHECKPOINT_INTERVAL`).
    pub index: u32,
    /// Total number of raffles created when this checkpoint was taken.
    pub raffle_count: u32,
    /// Ledger timestamp (Unix seconds) when this checkpoint was recorded.
    pub ledger_timestamp: u64,
    /// SHA-256 digest of `raffle_count ‖ ledger_sequence ‖ ledger_timestamp`.
    /// Used by off-chain monitors to detect storage tampering.
    pub aggregate_hash: BytesN<32>,
}

/// Persistent storage keys used by the factory contract.
///
/// Each variant maps to exactly one storage slot, keeping reads and writes
/// O(1). The stable-map design (`RaffleById` / `NextRaffleId`) means that
/// adding or removing a raffle never touches any other raffle's slot.
#[derive(Clone)]
#[contracttype]
pub struct RecurringRaffleEntry {
    pub creator: Address,
    pub config: RecurringRaffleConfig,
    pub next_due: u64,
    pub current_round: u32,
    pub active: bool,
    pub last_raffle_address: Option<Address>,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Flag set to `true` after the first successful [`RaffleFactory::init_factory`]
    /// call. Guards against re-initialization.
    Initialized,
    /// Current admin [`Address`]. Updated by a completed two-step transfer or
    /// directly by [`RaffleFactory::accept_factory_admin`].
    Admin,
    /// Stable map: stable_id (u32) → raffle Address.
    /// Replaces the old RaffleInstances Vec — each entry is an independent
    /// storage slot so reads and writes are always O(1).
    RaffleById(u32),
    /// Monotonic counter: the stable_id that will be assigned to the *next*
    /// raffle.  Starts at 0 and is never decremented.
    NextRaffleId,
    /// Number of live (non-tombstoned) raffles.  Used for stats only.
    RaffleCount,
    /// WASM hash of the raffle-instance contract deployed by
    /// [`RaffleFactory::create_raffle`].
    InstanceWasmHash,
    /// Protocol fee in basis points applied to every new raffle instance.
    ProtocolFeeBP,
    /// Treasury [`Address`] that receives protocol fees.
    Treasury,
    /// Boolean pause flag stored in instance storage. When `true`,
    /// [`RaffleFactory::create_raffle`] is blocked.
    Paused,
    /// Pending admin [`Address`] set by
    /// [`RaffleFactory::transfer_factory_admin`]; cleared on acceptance or
    /// cancellation.
    PendingAdmin,
    /// Timelocked operation keyed by its auto-incrementing `op_id`.
    PendingOp(u32),
    /// Monotonic counter that assigns unique IDs to pending operations.
    OpCounter,
    /// State checkpoint keyed by its sequential index.
    Checkpoint(u32),
    /// Index of the most recently written [`StateCheckpoint`].
    LatestCheckpointIndex,
    /// Cumulative count of all raffles ever created (never decremented).
    /// Used as input to the checkpoint trigger.
    TotalRafflesCreated,
    /// Per-address flag (`true`) recording that an address has participated.
    /// Used to maintain the unique-participant count without double-counting.
    UniqueParticipant(Address),
    /// Running count of unique participant addresses across all raffles.
    TotalUniqueParticipants,
    /// Minimum seconds between raffle creations for non-whitelisted creators.
    /// Defaults to 300 s when absent.
    MinCreationDelay,
    /// Unix timestamp of the most recent successful raffle creation for each
    /// non-whitelisted creator address. Used by the rate limiter.
    LastCreationTime(Address),
    /// Whitelist flag for partner addresses. When `true`, the address bypasses
    /// the creation rate limiter entirely.
    WhitelistedPartner(Address),
    /// Aggregate dashboard stats for a whitelisted partner (#488).
    PartnerStats(Address),
    /// Ordered list of currently whitelisted partner addresses for
    /// [`RaffleFactory::get_all_partners`] pagination.
    PartnersList,
    /// Cumulative ticket-sale volume denominated in a specific asset. Updated
    /// by [`RaffleFactory::record_volume`] on every ticket purchase.
    TotalVolumePerAsset(Address),
    /// Kept for test-only address generation; not used for indexing.
    RaffleInstancesCount,
    /// Per-creator raffle index: creator Address → Vec<Address> of raffle addresses.
    /// Appended to on every successful `create_raffle`.
    CreatorRaffles(Address),
    /// Per-category raffle index (#439): category String → Vec<Address> of raffle
    /// addresses. Appended to on every successful `create_raffle` whose config
    /// carries a category, enabling `get_raffles_by_category` queries without an
    /// off-chain indexer.
    CategoryRaffles(soroban_sdk::String),
    /// Recurring (subscription) raffle state by ID.
    RecurringRaffle(u32),
    /// Monotonic counter assigned to the next recurring raffle.
    NextRecurringId,
    /// ID → list of raffle addresses created across all rounds so far.
    RecurringRaffleInstances(u32),
    /// Whether creation of new raffles is currently paused (#611). Distinct
    /// from `DataKey::Paused`, which halts the entire factory; this flag only
    /// blocks `create_raffle`, leaving all other admin operations, reads, and
    /// any raffles already in flight unaffected.
    CreationPaused,
}

/// A read-only snapshot of key factory metrics returned by
/// [`RaffleFactory::get_protocol_stats`].
#[derive(Clone)]
#[contracttype]
pub struct ProtocolStats {
    /// Cumulative number of raffle instances ever created by this factory.
    pub total_raffles_created: u32,
    /// Current protocol fee in basis points (100 = 1 %).
    pub protocol_fee_bp: u32,
    /// Whether the factory is currently paused (`create_raffle` is blocked).
    pub paused: bool,
    /// Number of unique participant addresses tracked across all raffles.
    pub total_unique_participants: u32,
}

/// Per-partner aggregate statistics for the partner dashboard API (#488).
///
/// Updated on every successful [`RaffleFactory::create_raffle`] by a
/// whitelisted partner. `total_volume` / `total_fees_generated` start at zero
/// and are reserved for future fee/volume attribution hooks.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PartnerStats {
    /// Number of raffles created by this partner while whitelisted.
    pub total_raffles: u32,
    /// Cumulative ticket-sale volume attributed to this partner.
    pub total_volume: i128,
    /// Cumulative protocol fees generated by this partner's raffles.
    pub total_fees_generated: i128,
    /// Ledger timestamp of the partner's first raffle creation.
    pub first_raffle_at: u64,
    /// Ledger timestamp of the partner's most recent raffle creation.
    pub latest_raffle_at: u64,
}

/// Errors returned by the factory contract.
///
/// Each variant maps to a unique `u32` discriminant so that Stellar clients and
/// off-chain integrations can match on numeric codes without parsing strings.
/// See [`docs/ERRORS.md`](../../../docs/ERRORS.md) for the complete reference.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ContractError {
    /// `init_factory` was called on an already-initialized contract. Code 1.
    AlreadyInitialized = 1,
    /// Caller is not the admin or the operation requires admin authorization.
    /// Code 2.
    NotAuthorized = 2,
    /// The factory is paused; raffle creation is blocked until unpaused.
    /// Code 3.
    ContractPaused = 3,
    /// A supplied parameter is out of range or otherwise invalid (e.g., fee
    /// exceeds [`MAX_PROTOCOL_FEE_BP`], zero/self address). Code 4.
    InvalidParameters = 4,
    /// The requested raffle stable-ID does not map to an existing contract.
    /// Code 5.
    RaffleNotFound = 5,
    /// A two-step admin transfer is already in progress; the current proposal
    /// must be accepted or cancelled before a new one can be opened. Code 11.
    AdminTransferPending = 11,
    /// `accept_factory_admin` was called but there is no pending transfer.
    /// Code 12.
    NoPendingTransfer = 12,
    /// A non-whitelisted creator attempted to create a raffle before the
    /// [`MinCreationDelay`](DataKey::MinCreationDelay) window elapsed. Code 13.
    RateLimitExceeded = 13,
    /// `execute_config_change` or `cancel_config_change` was called with an
    /// `op_id` that has no pending operation. Code 14.
    NoPendingOp = 14,
    /// `execute_config_change` was called before `effective_timestamp` was
    /// reached. Code 15.
    TimelockNotElapsed = 15,
    /// `clean_old_raffle` was called with an ID that is not in the stable-map
    /// (never assigned or already tombstoned). Code 16.
    InvalidRaffleId = 16,
    /// Reserved for future use — a raffle does not meet eligibility criteria
    /// for the requested operation. Code 17.
    RaffleNotEligible = 17,
    /// A `checked_add` overflow occurred while accumulating volume. Code 18.
    ArithmeticOverflow = 18,
    /// `create_raffle` could not read the treasury address (factory not fully
    /// initialized). Code 19.
    TreasuryNotSet = 19,
    RecurringNotFound = 20,
    IntervalNotElapsed = 21,
    MaxRoundsReached = 22,
    RecurringInactive = 23,
    /// `create_raffle` was called while creation is paused via
    /// `set_creation_paused` (#611). Distinct from `ContractPaused`, which
    /// blocks the whole factory. Code 24.
    CreationPaused = 24,
}

pub const LEADERBOARD_CAP: u32 = 10;

#[contract]
pub struct RaffleFactory;

raffle_shared::impl_require_admin!(ContractError, ContractError::NotAuthorized);
raffle_shared::impl_require_not_paused!(
    ContractError,
    ContractError::ContractPaused,
    require_factory_not_paused
);

fn maybe_create_checkpoint(env: &Env, raffle_count: u32) {
    if raffle_count == 0 || !raffle_count.is_multiple_of(CHECKPOINT_INTERVAL) {
        return;
    }

    let index = raffle_count / CHECKPOINT_INTERVAL;
    let ledger_timestamp = env.ledger().timestamp();
    let ledger_sequence = env.ledger().sequence();

    let mut input = Bytes::new(env);
    input.extend_from_array(&raffle_count.to_be_bytes());
    input.extend_from_array(&ledger_sequence.to_be_bytes());
    input.extend_from_array(&ledger_timestamp.to_be_bytes());

    let aggregate_hash = env.crypto().sha256(&input);

    let checkpoint = StateCheckpoint {
        index,
        raffle_count,
        ledger_timestamp,
        aggregate_hash: aggregate_hash.clone().into(),
    };

    env.storage()
        .persistent()
        .set(&DataKey::Checkpoint(index), &checkpoint);
    env.storage()
        .persistent()
        .set(&DataKey::LatestCheckpointIndex, &index);

    events::CheckpointCreated {
        index,
        raffle_count,
        ledger_timestamp,
        aggregate_hash: aggregate_hash.into(),
    }
    .publish(env);
}

/// Derive a deterministic 32-byte deployment salt from `creator` and `nonce`.
///
/// The salt is `SHA-256(XDR(creator) ‖ XDR(nonce))`. Combined with
/// [`Env::deployer`]`.`with_current_contract`, this yields a stable instance
/// address that clients can compute before `create_raffle` lands.
///
/// `nonce` is the factory's [`DataKey::NextRaffleId`] at creation time (see
/// [`RaffleFactory::get_next_raffle_id`] / [`RaffleFactory::predict_raffle_address`]).
fn compute_raffle_salt(env: &Env, creator: &Address, nonce: u64) -> BytesN<32> {
    let payload = (creator.clone(), nonce).to_xdr(env);
    env.crypto().sha256(&payload).into()
}

/// Validate that an address is usable for a privileged role (admin/treasury).
///
/// Rejects the zero contract address (all-zero 32-byte hash) and the factory's
/// own address to prevent a self-referential admin or treasury that would brick
/// the contract.  Account (keypair) addresses are always accepted.
fn require_valid_role_address(env: &Env, address: &Address) -> Result<(), ContractError> {
    #[cfg(not(test))]
    if !address.exists() {
        return Err(ContractError::InvalidParameters);
    }
    // In test mode the exists() check is skipped, but we still reject the
    // all-zeros contract id (the "zero address") explicitly.
    #[cfg(test)]
    {
        use soroban_sdk::String;
        const ZERO_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4";
        let zero = Address::from_string(&String::from_str(env, ZERO_CONTRACT));
        if *address == zero {
            return Err(ContractError::InvalidParameters);
        }
    }
    if *address == env.current_contract_address() {
        return Err(ContractError::InvalidParameters);
    }
    Ok(())
}

fn create_raffle_internal(
    env: &Env,
    creator: Address,
    config: RaffleConfig,
) -> Result<Address, ContractError> {
    let admin: Address = env
        .storage()
        .persistent()
        .get(&DataKey::Admin)
        .ok_or(ContractError::NotAuthorized)?;
    let factory_address = env.current_contract_address();

    #[cfg(not(test))]
    let raffle_address = {
        let wasm_hash: BytesN<32> = env
            .storage()
            .persistent()
            .get(&DataKey::InstanceWasmHash)
            .ok_or(ContractError::InvalidParameters)?;
        let salt = env
            .crypto()
            .sha256(&(creator.clone(), config.description.clone()).to_xdr(env));
        env.deployer()
            .with_address(factory_address.clone(), salt)
            .deploy_v2(wasm_hash, ())
    };

    #[cfg(test)]
    let raffle_address = {
        let mut count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::RaffleInstancesCount)
            .unwrap_or(0);
        count += 1;
        env.storage()
            .persistent()
            .set(&DataKey::RaffleInstancesCount, &count);
        let mut id = Address::generate(env);
        for _ in 0..count {
            id = Address::generate(env);
        }
        env.register_at(&id, raffle_instance::Contract, ());
        id
    };

    let category = config.category.clone();
    env.invoke_contract::<()>(
        &raffle_address,
        &Symbol::new(env, "init"),
        (factory_address, admin, creator.clone(), config).into_val(env),
    );

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

    let mut creator_raffles: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::CreatorRaffles(creator.clone()))
        .unwrap_or_else(|| Vec::new(env));
    creator_raffles.push_back(raffle_address.clone());
    env.storage()
        .persistent()
        .set(&DataKey::CreatorRaffles(creator), &creator_raffles);

    if let Some(ref category) = category {
        let mut cat_raffles: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::CategoryRaffles(category.clone()))
            .unwrap_or_else(|| Vec::new(env));
        cat_raffles.push_back(raffle_address.clone());
        env.storage()
            .persistent()
            .set(&DataKey::CategoryRaffles(category.clone()), &cat_raffles);
    }

    let live_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::RaffleCount)
        .unwrap_or(0u32)
        .saturating_add(1);
    env.storage()
        .persistent()
        .set(&DataKey::RaffleCount, &live_count);

    let mut count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::TotalRafflesCreated)
        .unwrap_or(0);
    count += 1;
    env.storage()
        .persistent()
        .set(&DataKey::TotalRafflesCreated, &count);

    maybe_create_checkpoint(env, count);

    Ok(raffle_address)
}

#[contractimpl]
impl RaffleFactory {
    /// Initialize the factory contract.
    ///
    /// Must be called exactly once immediately after deployment. Subsequent
    /// calls return [`ContractError::AlreadyInitialized`].
    ///
    /// # Parameters
    ///
    /// - `admin` — Privileged address that may call admin-only functions.
    ///   Must not be the zero contract address or the factory's own address.
    /// - `wasm_hash` — WASM hash of the raffle-instance contract that will be
    ///   deployed by [`create_raffle`](Self::create_raffle).
    /// - `protocol_fee_bp` — Initial protocol fee in basis points
    ///   (max [`MAX_PROTOCOL_FEE_BP`] = 2 000, i.e. 20 %).
    /// - `treasury` — Address that receives protocol fees. Must not be the
    ///   zero contract address or the factory's own address.
    ///
    /// # Errors
    ///
    /// - [`ContractError::AlreadyInitialized`] — factory was already
    ///   initialized.
    /// - [`ContractError::InvalidParameters`] — `protocol_fee_bp` exceeds the
    ///   cap, or `admin`/`treasury` is the zero address or the factory itself.
    ///
    /// # Events
    ///
    /// Emits [`events::FactoryInitialized`] on success.
    ///
    /// See also: [`docs/EVENTS.md`](../../../docs/EVENTS.md) —
    /// `FactoryInitialized`.
    pub fn init_factory(
        env: Env,
        admin: Address,
        wasm_hash: BytesN<32>,
        protocol_fee_bp: u32,
        treasury: Address,
    ) -> Result<(), ContractError> {
        if env.storage().persistent().has(&DataKey::Initialized) {
            return Err(ContractError::AlreadyInitialized);
        }
        if protocol_fee_bp > MAX_PROTOCOL_FEE_BP {
            return Err(ContractError::InvalidParameters);
        }
        require_valid_role_address(&env, &admin)?;
        require_valid_role_address(&env, &treasury)?;
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::InstanceWasmHash, &wasm_hash);
        env.storage()
            .persistent()
            .set(&DataKey::ProtocolFeeBP, &protocol_fee_bp);
        env.storage()
            .persistent()
            .set(&DataKey::Treasury, &treasury);
        env.storage().persistent().set(&DataKey::Initialized, &true);

        events::FactoryInitialized {
            admin,
            protocol_fee_bp,
            treasury,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    /// Propose a protocol-configuration change under a 48-hour timelock.
    ///
    /// The change is **not** applied immediately. It is stored as a
    /// [`PendingOp`] and becomes executable only after
    /// [`TIMELOCK_DELAY_SECONDS`] (48 hours) have elapsed. Call
    /// [`execute_config_change`](Self::execute_config_change) with the
    /// returned `op_id` to apply it, or
    /// [`cancel_config_change`](Self::cancel_config_change) to discard it.
    ///
    /// # Auth
    ///
    /// Requires authorization from the current admin address.
    ///
    /// # Parameters
    ///
    /// - `protocol_fee_bp` — New protocol fee in basis points (max
    ///   [`MAX_PROTOCOL_FEE_BP`] = 2 000).
    /// - `treasury` — New treasury address. Must not be the zero contract
    ///   address or the factory's own address.
    ///
    /// # Returns
    ///
    /// The auto-incremented `op_id` that identifies this pending operation.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotAuthorized`] — caller is not the admin.
    /// - [`ContractError::InvalidParameters`] — fee exceeds cap or treasury
    ///   address is invalid.
    ///
    /// # Events
    ///
    /// Emits [`events::AdminOpProposed`] on success.
    ///
    /// See also: [`docs/EVENTS.md`](../../../docs/EVENTS.md) —
    /// `AdminOpProposed`.
    pub fn set_config(
        env: Env,
        key: ConfigKey,
        address: Address,
    ) -> Result<u32, ContractError> {
        let admin = require_admin(&env)?;
        require_valid_role_address(&env, &address)?;

        let op_id = env
            .storage()
            .persistent()
            .get::<_, u32>(&DataKey::OpCounter)
            .unwrap_or(0)
            .saturating_add(1);

        env.storage().persistent().set(&DataKey::OpCounter, &op_id);

        let effective_timestamp = env.ledger().timestamp() + TIMELOCK_DELAY_SECONDS;
        let op = AdminOp::SetConfig(key, address);
        let pending = PendingOp {
            op: op.clone(),
            effective_timestamp,
            proposed_by: admin.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::PendingOp(op_id), &pending);

        events::AdminOpProposed {
            op_id,
            op,
            effective_timestamp,
            proposed_by: admin,
        }
        .publish(&env);

        Ok(op_id)
    }

    pub fn propose_fee_change(env: Env, protocol_fee_bp: u32) -> Result<u32, ContractError> {
        let admin = require_admin(&env)?;
        if protocol_fee_bp > MAX_PROTOCOL_FEE_BP {
            return Err(ContractError::InvalidParameters);
        }

        let op_id = env
            .storage()
            .persistent()
            .get::<_, u32>(&DataKey::OpCounter)
            .unwrap_or(0)
            .saturating_add(1);

        env.storage().persistent().set(&DataKey::OpCounter, &op_id);

        let effective_timestamp = env.ledger().timestamp() + TIMELOCK_DELAY_SECONDS;
        let op = AdminOp::SetProtocolFeeBP(protocol_fee_bp);
        let pending = PendingOp {
            op: op.clone(),
            effective_timestamp,
            proposed_by: admin.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::PendingOp(op_id), &pending);

        events::AdminOpProposed {
            op_id,
            op,
            effective_timestamp,
            proposed_by: admin,
        }
        .publish(&env);

        Ok(op_id)
    }

    pub fn propose_wasm_upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<u32, ContractError> {
        let admin = require_admin(&env)?;
        let op_id = env
            .storage()
            .persistent()
            .get::<_, u32>(&DataKey::OpCounter)
            .unwrap_or(0)
            .saturating_add(1);

        env.storage().persistent().set(&DataKey::OpCounter, &op_id);

        let effective_timestamp = env.ledger().timestamp() + TIMELOCK_DELAY_SECONDS;
        let pending = PendingOp {
            op: AdminOp::UpdateWasmHash(new_wasm_hash.clone()),
            effective_timestamp,
            proposed_by: admin.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::PendingOp(op_id), &pending);

        events::AdminOpProposed {
            op_id,
            op: AdminOp::UpdateWasmHash(new_wasm_hash),
            effective_timestamp,
            proposed_by: admin,
        }
        .publish(&env);

        Ok(op_id)
    }

    pub fn execute_config_change(env: Env, op_id: u32) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;

        let pending: PendingOp = env
            .storage()
            .persistent()
            .get(&DataKey::PendingOp(op_id))
            .ok_or(ContractError::NoPendingOp)?;

        if env.ledger().timestamp() < pending.effective_timestamp {
            return Err(ContractError::TimelockNotElapsed);
        }

        match pending.op.clone() {
            AdminOp::SetConfig(key, address) => {
                require_valid_role_address(&env, &address)?;
                match key {
                    ConfigKey::Treasury => {
                        env.storage()
                            .persistent()
                            .set(&DataKey::Treasury, &address);
                    }
                    ConfigKey::Oracle => {
                        // The factory doesn't store a global Oracle address right now,
                        // but if we add it to DataKey in the future we would set it here.
                        // Currently, this is a placeholder per the user's request.
                    }
                    ConfigKey::SwapRouter => {
                        // Same here, placeholder.
                    }
                }
            }
            AdminOp::SetProtocolFeeBP(protocol_fee_bp) => {
                if protocol_fee_bp > MAX_PROTOCOL_FEE_BP {
                    return Err(ContractError::InvalidParameters);
                }
                env.storage()
                    .persistent()
                    .set(&DataKey::ProtocolFeeBP, &protocol_fee_bp);
            }
            AdminOp::UpdateWasmHash(new_hash) => {
                env.storage()
                    .persistent()
                    .set(&DataKey::InstanceWasmHash, &new_hash);
            }
            AdminOp::ApproveOracle(oracle) => {
                env.storage()
                    .persistent()
                    .set(&DataKey::ApprovedOracle(oracle.clone()), &true);
                events::OracleApproved {
                    oracle,
                    approved_by: admin.clone(),
                    timestamp: env.ledger().timestamp(),
                }
                .publish(&env);
            }
            AdminOp::RemoveOracle(oracle) => {
                env.storage()
                    .persistent()
                    .remove(&DataKey::ApprovedOracle(oracle.clone()));
                events::OracleRemoved {
                    oracle,
                    removed_by: admin.clone(),
                    timestamp: env.ledger().timestamp(),
                }
                .publish(&env);
            }
        }

        env.storage()
            .persistent()
            .remove(&DataKey::PendingOp(op_id));

        events::AdminOpExecuted {
            op_id,
            op: pending.op,
            executed_by: admin,
            executed_at: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    /// Cancel a pending timelocked configuration change.
    ///
    /// Removes the [`PendingOp`] stored under `op_id` without applying it.
    /// The operation cannot be recovered after cancellation.
    ///
    /// # Auth
    ///
    /// Requires authorization from the current admin address.
    ///
    /// # Parameters
    ///
    /// - `op_id` — Identifier returned by [`set_config`](Self::set_config).
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotAuthorized`] — caller is not the admin.
    /// - [`ContractError::NoPendingOp`] — no pending operation for `op_id`.
    ///
    /// # Events
    ///
    /// Emits [`events::AdminOpCancelled`] on success.
    ///
    /// See also: [`docs/EVENTS.md`](../../../docs/EVENTS.md) —
    /// `AdminOpCancelled`.
    pub fn cancel_config_change(env: Env, op_id: u32) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;

        if !env.storage().persistent().has(&DataKey::PendingOp(op_id)) {
            return Err(ContractError::NoPendingOp);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::PendingOp(op_id));

        events::AdminOpCancelled {
            op_id,
            cancelled_by: admin,
            cancelled_at: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    /// Return the pending operation for `op_id`, or `None` if it has been
    /// executed, cancelled, or never created.
    pub fn get_pending_op(env: Env, op_id: u32) -> Option<PendingOp> {
        env.storage().persistent().get(&DataKey::PendingOp(op_id))
    }

    /// Return the current operation counter value.
    ///
    /// The next call to [`set_config`](Self::set_config) will produce an
    /// `op_id` equal to this value plus one.
    pub fn get_op_counter(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::OpCounter)
            .unwrap_or(0u32)
    }

    /// Deploy a new raffle-instance contract and register it with the factory.
    ///
    /// This is the primary entry point for raffle creators. The function:
    ///
    /// 1. Checks the factory is not paused.
    /// 2. Enforces the creation rate limiter for non-whitelisted creators
    ///    (default 300 s cooldown, configurable via
    ///    [`set_creation_delay`](Self::set_creation_delay)).
    /// 3. Injects the current `protocol_fee_bp` and `treasury` into the config.
    /// 4. Deploys a new raffle-instance WASM contract with a deterministic
    ///    address derived from `creator` + current [`DataKey::NextRaffleId`]
    ///    (see [`predict_raffle_address`](Self::predict_raffle_address)).
    /// 5. Calls `init` on the deployed instance.
    /// 6. Registers the address in the O(1) stable-ID map and the per-creator
    ///    index. If the config declares a `category`, also appends to the
    ///    per-category index.
    /// 7. Triggers a [`StateCheckpoint`] every
    ///    [`CHECKPOINT_INTERVAL`] total raffles.
    ///
    /// # Auth
    ///
    /// Requires authorization from `creator`.
    ///
    /// # Parameters
    ///
    /// - `creator` — Address that will own the raffle and receive any creator
    ///   privileges within the instance.
    /// - `config` — Full raffle configuration. `protocol_fee_bp` and
    ///   `treasury_address` fields are **overwritten** by the factory's stored
    ///   values regardless of what the caller provides.
    ///
    /// # Returns
    ///
    /// The [`Address`] of the newly deployed raffle-instance contract.
    ///
    /// # Errors
    ///
    /// - [`ContractError::ContractPaused`] — factory is paused.
    /// - [`ContractError::RateLimitExceeded`] — non-whitelisted creator is
    ///   within the cooldown window (also emits [`events::CreationRateLimited`]).
    /// - [`ContractError::TreasuryNotSet`] — factory treasury address not
    ///   initialized.
    /// - [`ContractError::NotAuthorized`] — factory admin address missing
    ///   (should not occur after `init_factory`).
    /// - [`ContractError::InvalidParameters`] — WASM hash not set (production
    ///   only).
    ///
    /// # Events
    ///
    /// - [`events::CreationRateLimited`] when a non-whitelisted creator is
    ///   rate-limited (returned together with
    ///   [`ContractError::RateLimitExceeded`]).
    /// - The deployed instance emits `RaffleCreated` on its own `init` call.
    ///
    /// See also: [`docs/EVENTS.md`](../../../docs/EVENTS.md) —
    /// `CreationRateLimited`.
    pub fn create_raffle(
        env: Env,
        creator: Address,
        config: RaffleConfig,
    ) -> Result<Address, ContractError> {
        creator.require_auth();
        require_factory_not_paused(&env)?;

        let creation_paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::CreationPaused)
            .unwrap_or(false);
        if creation_paused {
            return Err(ContractError::CreationPaused);
        }

        let is_whitelisted = env
            .storage()
            .persistent()
            .get(&DataKey::WhitelistedPartner(creator.clone()))
            .unwrap_or(false);

        if !is_whitelisted {
            let now = env.ledger().timestamp();
            let min_delay = env
                .storage()
                .persistent()
                .get(&DataKey::MinCreationDelay)
                .unwrap_or(300);

            let last_creation: u64 = env
                .storage()
                .persistent()
                .get(&DataKey::LastCreationTime(creator.clone()))
                .unwrap_or(0);

            if now < last_creation + min_delay {
                let unlock_timestamp = last_creation + min_delay;
                events::CreationRateLimited {
                    creator: creator.clone(),
                    unlock_timestamp,
                    timestamp: now,
                }
                .publish(&env);
                return Err(ContractError::RateLimitExceeded);
            }

            env.storage()
                .persistent()
                .set(&DataKey::LastCreationTime(creator.clone()), &now);
        }

        let protocol_fee_bp: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ProtocolFeeBP)
            .unwrap_or(0);
        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .ok_or(ContractError::TreasuryNotSet)?;

        let mut final_config = config;
        final_config.protocol_fee_bp = protocol_fee_bp;
        final_config.treasury_address = Some(treasury);

        create_raffle_internal(&env, creator, final_config)
    }

    pub fn create_recurring_raffle(
        env: Env,
        creator: Address,
        config: RecurringRaffleConfig,
    ) -> Result<u32, ContractError> {
        creator.require_auth();
        require_factory_not_paused(&env)?;

        if config.interval_seconds < MIN_RECURRING_INTERVAL_SECONDS
            || config.interval_seconds > MAX_RECURRING_INTERVAL_SECONDS
        {
            return Err(ContractError::InvalidParameters);
        }

        if config.max_rounds == 0 && config.auto_fund {
            return Err(ContractError::InvalidParameters);
        }

        let recurring_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextRecurringId)
            .unwrap_or(0u32);

        let now = env.ledger().timestamp();
        let interval = config.interval_seconds;
        let entry = RecurringRaffleEntry {
            creator: creator.clone(),
            config,
            next_due: now.saturating_add(interval),
            current_round: 0,
            active: true,
            last_raffle_address: None,
        };

        env.storage()
            .persistent()
            .set(&DataKey::RecurringRaffle(recurring_id), &entry);
        env.storage()
            .persistent()
            .set(&DataKey::NextRecurringId, &(recurring_id.saturating_add(1)));
        env.storage()
            .persistent()
            .set(&DataKey::RecurringRaffleInstances(recurring_id), &Vec::new(&env));

        events::RecurringRaffleCreated {
            recurring_id,
            creator,
            interval_seconds: entry.config.interval_seconds,
            max_rounds: entry.config.max_rounds,
            auto_fund: entry.config.auto_fund,
            next_due: entry.next_due,
            timestamp: now,
        }
        .publish(&env);

        Ok(recurring_id)
    }

    pub fn trigger_next_round(
        env: Env,
        recurring_id: u32,
    ) -> Result<Address, ContractError> {
        require_factory_not_paused(&env)?;

        let mut entry: RecurringRaffleEntry = env
            .storage()
            .persistent()
            .get(&DataKey::RecurringRaffle(recurring_id))
            .ok_or(ContractError::RecurringNotFound)?;

        if !entry.active {
            return Err(ContractError::RecurringInactive);
        }

        let now = env.ledger().timestamp();
        if now < entry.next_due {
            return Err(ContractError::IntervalNotElapsed);
        }

        if entry.config.max_rounds > 0 && entry.current_round >= entry.config.max_rounds {
            return Err(ContractError::MaxRoundsReached);
        }

        let raffle_address = self::create_raffle_internal(
            &env,
            entry.creator.clone(),
            entry.config.base_config.clone(),
        )?;

        entry.current_round = entry.current_round.saturating_add(1);
        entry.next_due = now.saturating_add(entry.config.interval_seconds);
        entry.last_raffle_address = Some(raffle_address.clone());

        let mut instances: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::RecurringRaffleInstances(recurring_id))
            .unwrap_or_else(|| Vec::new(&env));
        instances.push_back(raffle_address.clone());
        env.storage()
            .persistent()
            .set(&DataKey::RecurringRaffleInstances(recurring_id), &instances);

        env.storage()
            .persistent()
            .set(&DataKey::RecurringRaffle(recurring_id), &entry);

        events::RecurringRoundTriggered {
            recurring_id,
            round: entry.current_round,
            raffle_address: raffle_address.clone(),
            next_due: entry.next_due,
            timestamp: now,
        }
        .publish(&env);

        // --- partner dashboard stats (#488) ---
        if is_whitelisted {
            let now = env.ledger().timestamp();
            let mut stats: PartnerStats = env
                .storage()
                .persistent()
                .get(&DataKey::PartnerStats(creator.clone()))
                .unwrap_or(PartnerStats {
                    total_raffles: 0,
                    total_volume: 0,
                    total_fees_generated: 0,
                    first_raffle_at: now,
                    latest_raffle_at: 0,
                });
            if stats.total_raffles == 0 {
                stats.first_raffle_at = now;
            }
            stats.total_raffles = stats.total_raffles.saturating_add(1);
            stats.latest_raffle_at = now;
            env.storage()
                .persistent()
                .set(&DataKey::PartnerStats(creator), &stats);
        }

        Ok(raffle_address)
    }

    pub fn cancel_recurring_raffle(
        env: Env,
        recurring_id: u32,
        caller: Address,
    ) -> Result<(), ContractError> {
        let entry: RecurringRaffleEntry = env
            .storage()
            .persistent()
            .get(&DataKey::RecurringRaffle(recurring_id))
            .ok_or(ContractError::RecurringNotFound)?;

        if caller != entry.creator {
            let admin = require_admin(&env)?;
            if caller != admin {
                return Err(ContractError::NotAuthorized);
            }
        }
        caller.require_auth();

        env.storage()
            .persistent()
            .set(
                &DataKey::RecurringRaffle(recurring_id),
                &RecurringRaffleEntry {
                    active: false,
                    ..entry
                },
            );

        events::RecurringRaffleCancelled {
            recurring_id,
            cancelled_by: caller,
            rounds_completed: entry.current_round,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    pub fn get_recurring_raffle(env: Env, recurring_id: u32) -> Option<RecurringRaffleEntry> {
        env.storage()
            .persistent()
            .get(&DataKey::RecurringRaffle(recurring_id))
    }

    pub fn get_recurring_instances(
        env: Env,
        recurring_id: u32,
    ) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::RecurringRaffleInstances(recurring_id))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Return a snapshot of key factory metrics.
    ///
    /// This is a read-only call with no auth requirement. All fields default to
    /// zero/false when the factory has just been initialized and no raffles have
    /// been created.
    pub fn get_protocol_stats(env: Env) -> ProtocolStats {
        let total_raffles_created: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalRafflesCreated)
            .unwrap_or(0);
        let protocol_fee_bp: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ProtocolFeeBP)
            .unwrap_or(0);
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        let total_unique_participants: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalUniqueParticipants)
            .unwrap_or(0);

        ProtocolStats {
            total_raffles_created,
            protocol_fee_bp,
            paused,
            total_unique_participants,
        }
    }

    /// O(1) direct lookup of a raffle address by its stable ID.
    /// Returns `None` if the ID was never assigned or has been cleaned up.
    pub fn get_raffle_by_id(env: Env, raffle_id: u32) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::RaffleById(raffle_id))
    }

    /// Returns the stable ID that will be assigned to the next raffle.
    /// IDs in [0, next_raffle_id) have been assigned at least once.
    ///
    /// Pass this value as `nonce` to [`predict_raffle_address`](Self::predict_raffle_address)
    /// to precompute the instance address for the next [`create_raffle`](Self::create_raffle).
    pub fn get_next_raffle_id(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::NextRaffleId)
            .unwrap_or(0u32)
    }

    /// Predict the deterministic contract address for a raffle before deployment.
    ///
    /// The address is derived from this factory's address and
    /// `SHA-256(XDR(creator) ‖ XDR(nonce))` via
    /// [`Env::deployer`]`.`with_current_contract`.
    ///
    /// For the next raffle a creator will receive, use
    /// `nonce = get_next_raffle_id() as u64`.
    ///
    /// # Parameters
    ///
    /// - `creator` — Address that will create the raffle.
    /// - `nonce` — Deployment salt input; must match the factory's
    ///   [`get_next_raffle_id`](Self::get_next_raffle_id) at `create_raffle` time.
    pub fn predict_raffle_address(env: Env, creator: Address, nonce: u64) -> Address {
        let salt = compute_raffle_salt(&env, &creator, nonce);
        env.deployer()
            .with_current_contract(salt)
            .deployed_address()
    }

    /// Returns the current count of live (non-tombstoned) raffles.
    pub fn get_raffle_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::RaffleCount)
            .unwrap_or(0u32)
    }

    /// Return the cumulative ticket-sale volume for a specific `asset` token.
    ///
    /// Returns `0` when no volume has been recorded for `asset`. No auth
    /// required.
    pub fn get_total_volume(env: Env, asset: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalVolumePerAsset(asset))
            .unwrap_or(0)
    }

    /// Accumulate `amount` into the running volume counter for `asset`.
    ///
    /// Called by raffle instances on every successful ticket purchase to
    /// update the factory-level per-asset volume metric. This is an internal
    /// cross-contract call — end users do not call it directly.
    ///
    /// # Auth
    ///
    /// No explicit admin check; the instance is trusted by the factory because
    /// the factory deployed it. The instance itself validates that the caller
    /// is the ticket buyer.
    ///
    /// # Errors
    ///
    /// - [`ContractError::ArithmeticOverflow`] — adding `amount` to the
    ///   current total would exceed `i128::MAX`.
    pub fn record_volume(env: Env, asset: Address, amount: i128) -> Result<(), ContractError> {
        let total_volume: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalVolumePerAsset(asset.clone()))
            .unwrap_or(0);
        let total_volume = total_volume
            .checked_add(amount)
            .ok_or(ContractError::ArithmeticOverflow)?;
        env.storage()
            .persistent()
            .set(&DataKey::TotalVolumePerAsset(asset), &total_volume);
        Ok(())
    }

    /// Return the current admin address.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotAuthorized`] — admin key is missing (factory not
    ///   initialized).
    pub fn get_admin(env: Env) -> Result<Address, ContractError> {
        env.storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::NotAuthorized)
    }

    /// Return a paginated slice of all live raffle addresses.
    ///
    /// Iterates over the stable-ID space `[offset, offset + limit)` and
    /// returns only slots that still hold a live address (tombstoned entries
    /// from [`clean_old_raffle`](Self::clean_old_raffle) are silently skipped).
    /// Each iteration step is a single O(1) storage lookup.
    ///
    /// # Parameters
    ///
    /// - `params.offset` — First stable-ID to include. Acts as a cursor into
    ///   the ever-increasing ID space (not the live-raffle count).
    /// - `params.limit` — Maximum results per page. Clamped to
    ///   `[1, MAX_PAGE_LIMIT]`; `0` uses `DEFAULT_PAGE_LIMIT` (100).
    ///
    /// # Returns
    ///
    /// A [`PageResultRaffles`] whose `total` field reflects the number of
    /// **live** raffles (not the total IDs ever assigned), and `has_more` is
    /// `true` when the stable-ID space extends beyond the returned window.
    pub fn get_raffles_page(env: Env, params: PaginationParams) -> PageResultRaffles {
        // `NextRaffleId` is the exclusive upper bound on all ever-assigned IDs.
        // It equals the total number of raffles ever created (including any that
        // have been cleaned up / tombstoned).
        let next_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextRaffleId)
            .unwrap_or(0u32);

        let lim = effective_limit(params.limit);
        let offset = params.offset;

        // `total` here is the live-raffle count (tombstoned entries excluded),
        // reported to the caller for UI pagination purposes.
        let total: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::RaffleCount)
            .unwrap_or(0u32);

        if offset >= next_id {
            return PageResultRaffles {
                items: Vec::new(&env),
                total,
                has_more: false,
            };
        }

        // Walk the stable ID space [offset, offset + lim) and collect only
        // slots that still hold a live address (non-tombstoned).  Each read
        // is a single O(1) storage lookup; the loop is bounded by `lim`.
        let end = offset.saturating_add(lim).min(next_id);
        let mut items: Vec<Address> = Vec::new(&env);
        for id in offset..end {
            if let Some(addr) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::RaffleById(id))
            {
                items.push_back(addr);
            }
        }

        let has_more = end < next_id;
        PageResultRaffles {
            items,
            total,
            has_more,
        }
    }

    /// Return a paginated list of raffle addresses created by `creator`.
    ///
    /// `params.offset` is an index into the creator's personal raffle list
    /// (not the global stable-ID space).  `params.limit` is clamped by
    /// `effective_limit` (1–200, default 100).
    pub fn get_raffles_by_creator(
        env: Env,
        creator: Address,
        params: PaginationParams,
    ) -> PageResultRaffles {
        let creator_raffles: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::CreatorRaffles(creator))
            .unwrap_or_else(|| Vec::new(&env));

        let total = creator_raffles.len();
        let lim = effective_limit(params.limit);
        let offset = params.offset;

        if offset >= total {
            return PageResultRaffles {
                items: Vec::new(&env),
                total,
                has_more: false,
            };
        }

        let end = offset.saturating_add(lim).min(total);
        let mut items: Vec<Address> = Vec::new(&env);
        for i in offset..end {
            if let Some(addr) = creator_raffles.get(i) {
                items.push_back(addr);
            }
        }

        let has_more = end < total;
        PageResultRaffles {
            items,
            total,
            has_more,
        }
    }

    /// Return a paginated list of raffle addresses tagged with `category` (#439).
    ///
    /// `params.offset` is an index into the category's raffle list.
    /// `params.limit` is clamped by `effective_limit` (1–200, default 100).
    /// An unknown category simply yields an empty page.
    pub fn get_raffles_by_category(
        env: Env,
        category: soroban_sdk::String,
        params: PaginationParams,
    ) -> PageResultRaffles {
        let category_raffles: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::CategoryRaffles(category))
            .unwrap_or_else(|| Vec::new(&env));

        let total = category_raffles.len();
        let lim = effective_limit(params.limit);
        let offset = params.offset;

        if offset >= total {
            return PageResultRaffles {
                items: Vec::new(&env),
                total,
                has_more: false,
            };
        }

        let end = offset.saturating_add(lim).min(total);
        let mut items: Vec<Address> = Vec::new(&env);
        for i in offset..end {
            if let Some(addr) = category_raffles.get(i) {
                items.push_back(addr);
            }
        }

        let has_more = end < total;
        PageResultRaffles {
            items,
            total,
            has_more,
        }
    }

    /// Pause the factory, blocking new raffle creation.
    ///
    /// While paused, [`create_raffle`](Self::create_raffle) returns
    /// [`ContractError::ContractPaused`]. All other reads and admin operations
    /// remain available.
    ///
    /// # Auth
    ///
    /// Requires authorization from the current admin address.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotAuthorized`] — caller is not the admin.
    ///
    /// # Events
    ///
    /// Emits [`events::ContractPaused`].
    ///
    /// See also: [`docs/EVENTS.md`](../../../docs/EVENTS.md) — `ContractPaused`.
    pub fn pause_factory(env: Env) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &true);

        events::ContractPaused {
            paused_by: admin,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    pub fn unpause_factory(env: Env) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        env.storage().instance().set(&DataKey::Paused, &false);

        events::ContractUnpaused {
            unpaused_by: admin,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    pub fn is_factory_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn is_creation_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::CreationPaused)
            .unwrap_or(false)
    }

    pub fn transfer_factory_admin(env: Env, new_admin: Address) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;

        if new_admin == admin {
            env.storage().persistent().remove(&DataKey::PendingAdmin);
            return Ok(());
        }

        require_valid_role_address(&env, &new_admin)?;

        if env.storage().persistent().has(&DataKey::PendingAdmin) {
            return Err(ContractError::AdminTransferPending);
        }

        env.storage()
            .persistent()
            .set(&DataKey::PendingAdmin, &new_admin);

        events::AdminTransferProposed {
            current_admin: admin,
            proposed_admin: new_admin,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    pub fn accept_factory_admin(env: Env) -> Result<(), ContractError> {
        let pending: Address = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdmin)
            .ok_or(ContractError::NoPendingTransfer)?;
        pending.require_auth();

        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(ContractError::NotAuthorized)?;

        env.storage().persistent().set(&DataKey::Admin, &pending);
        env.storage().persistent().remove(&DataKey::PendingAdmin);

        events::AdminTransferAccepted {
            old_admin,
            new_admin: pending,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    pub fn get_checkpoint(env: Env, index: u32) -> Option<StateCheckpoint> {
        env.storage().persistent().get(&DataKey::Checkpoint(index))
    }

    pub fn get_latest_checkpoint_index(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::LatestCheckpointIndex)
            .unwrap_or(0u32)
    }

    pub fn sync_admin(env: Env, instance_address: Address) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        env.invoke_contract::<()>(
            &instance_address,
            &Symbol::new(&env, "set_admin"),
            (admin,).into_val(&env),
        );
        Ok(())
    }

    pub fn pause_instance(env: Env, instance_address: Address) -> Result<(), ContractError> {
        require_admin(&env)?;
        env.invoke_contract::<()>(
            &instance_address,
            &Symbol::new(&env, "pause"),
            ().into_val(&env),
        );
        Ok(())
    }

    pub fn unpause_instance(env: Env, instance_address: Address) -> Result<(), ContractError> {
        require_admin(&env)?;
        env.invoke_contract::<()>(
            &instance_address,
            &Symbol::new(&env, "unpause"),
            ().into_val(&env),
        );
        Ok(())
    }

    pub fn track_participant(env: Env, participant: Address) -> Result<(), ContractError> {
        participant.require_auth();

        let key = DataKey::UniqueParticipant(participant.clone());
        if !env.storage().persistent().has(&key) {
            env.storage().persistent().set(&key, &true);
            let mut count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::TotalUniqueParticipants)
                .unwrap_or(0);
            count += 1;
            env.storage()
                .persistent()
                .set(&DataKey::TotalUniqueParticipants, &count);
        }
        Ok(())
    }

    pub fn get_unique_participants(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalUniqueParticipants)
            .unwrap_or(0)
    }

    pub fn get_raffle_fairness_data(
        env: Env,
        raffle_id: Address,
    ) -> Result<FairnessData, ContractError> {
        Ok(env.invoke_contract::<FairnessData>(
            &raffle_id,
            &Symbol::new(&env, "get_fairness_data"),
            ().into_val(&env),
        ))
    }

    pub fn set_creation_delay(env: Env, delay_seconds: u64) -> Result<(), ContractError> {
        require_admin(&env)?;
        env.storage()
            .persistent()
            .set(&DataKey::MinCreationDelay, &delay_seconds);
        Ok(())
    }

    /// Pause or resume creation of new raffles only (#611).
    ///
    /// Unlike [`pause_factory`](Self::pause_factory), which halts the entire
    /// factory, this only blocks [`create_raffle`](Self::create_raffle) —
    /// admin operations, views, and any raffles already in flight are
    /// unaffected.
    ///
    /// # Auth
    ///
    /// Requires authorization from the current admin address.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotAuthorized`] — caller is not the admin.
    ///
    /// # Events
    ///
    /// Emits [`events::CreationPaused`] or [`events::CreationUnpaused`].
    pub fn set_creation_paused(env: Env, paused: bool) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::CreationPaused, &paused);

        let timestamp = env.ledger().timestamp();
        if paused {
            events::CreationPaused {
                paused_by: admin,
                timestamp,
            }
            .publish(&env);
        } else {
            events::CreationUnpaused {
                unpaused_by: admin,
                timestamp,
            }
            .publish(&env);
        }

        Ok(())
    }

    pub fn set_whitelist_status(
        env: Env,
        partner: Address,
        status: bool,
    ) -> Result<(), ContractError> {
        require_admin(&env)?;
        env.storage()
            .persistent()
            .set(&DataKey::WhitelistedPartner(partner.clone()), &status);

        // Keep PartnersList in sync so get_all_partners can paginate without
        // scanning every address key.
        let mut partners: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::PartnersList)
            .unwrap_or_else(|| Vec::new(&env));

        let mut existing: Option<u32> = None;
        for i in 0..partners.len() {
            if partners.get(i).as_ref() == Some(&partner) {
                existing = Some(i);
                break;
            }
        }

        if status {
            if existing.is_none() {
                partners.push_back(partner);
                env.storage()
                    .persistent()
                    .set(&DataKey::PartnersList, &partners);
            }
        } else if let Some(idx) = existing {
            let mut next = Vec::new(&env);
            for i in 0..partners.len() {
                if i != idx {
                    if let Some(addr) = partners.get(i) {
                        next.push_back(addr);
                    }
                }
            }
            env.storage()
                .persistent()
                .set(&DataKey::PartnersList, &next);
        }

        Ok(())
    }

    /// Return aggregate stats for a whitelisted partner, or `None` if the
    /// address is not currently on the partner whitelist (#488).
    pub fn get_partner_stats(env: Env, partner: Address) -> Option<PartnerStats> {
        let is_whitelisted = env
            .storage()
            .persistent()
            .get(&DataKey::WhitelistedPartner(partner.clone()))
            .unwrap_or(false);
        if !is_whitelisted {
            return None;
        }
        Some(
            env.storage()
                .persistent()
                .get(&DataKey::PartnerStats(partner))
                .unwrap_or(PartnerStats {
                    total_raffles: 0,
                    total_volume: 0,
                    total_fees_generated: 0,
                    first_raffle_at: 0,
                    latest_raffle_at: 0,
                }),
        )
    }

    /// Return a paginated page of currently whitelisted partner addresses (#488).
    ///
    /// `params.limit` is clamped by [`effective_limit`] (1–200, default 100).
    pub fn get_all_partners(env: Env, params: PaginationParams) -> Vec<Address> {
        let partners: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::PartnersList)
            .unwrap_or_else(|| Vec::new(&env));

        let total = partners.len();
        let lim = effective_limit(params.limit);
        let offset = params.offset;

        if offset >= total {
            return Vec::new(&env);
        }

        let end = offset.saturating_add(lim).min(total);
        let mut items: Vec<Address> = Vec::new(&env);
        for i in offset..end {
            if let Some(addr) = partners.get(i) {
                items.push_back(addr);
            }
        }
        items
    }

    /// Standard Soroban upgrade entry point for the factory contract WASM.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());

        events::FactoryUpgraded {
            admin,
            new_wasm_hash,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    /// Sweep tokens accidentally sent to the factory contract.
    pub fn rescue_tokens(
        env: Env,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;

        if amount <= 0 {
            return Err(ContractError::InvalidParameters);
        }

        let token_client = token::Client::new(&env, &token);
        let _ = token_client
            .try_transfer(&env.current_contract_address(), &recipient, &amount)
            .map_err(|_| ContractError::InvalidParameters)?;

        events::FactoryTokensRescued {
            rescued_by: admin,
            token,
            recipient,
            amount,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    fn upsert_leaderboard(
        env: &Env,
        key: &DataKey,
        raffle: Address,
        metric: i128,
    ) {
        let mut board: Vec<(Address, i128)> = env
            .storage()
            .persistent()
            .get(key)
            .unwrap_or_else(|| Vec::new(env));

        let mut next = Vec::new(env);
        for i in 0..board.len() {
            let entry = board.get(i).unwrap();
            if entry.0 != raffle {
                next.push_back(entry);
            }
        }
        next.push_back((raffle, metric));

        let len = next.len();
        for i in 0..len {
            for j in (i + 1)..len {
                let left = next.get(i).unwrap();
                let right = next.get(j).unwrap();
                if right.1 > left.1 {
                    next.set(i, right);
                    next.set(j, left);
                }
            }
        }

        while next.len() > LEADERBOARD_CAP {
            next.pop_back();
        }

        env.storage().persistent().set(key, &next);
    }

    /// Called by a raffle instance after finalization (#484).
    pub fn record_leaderboard_entry(
        env: Env,
        raffle_address: Address,
        tickets_sold: i128,
        prize_amount: i128,
        total_volume: i128,
    ) -> Result<(), ContractError> {
        raffle_address.require_auth();
        Self::upsert_leaderboard(&env, &DataKey::TopByTickets, raffle_address.clone(), tickets_sold);
        Self::upsert_leaderboard(&env, &DataKey::TopByPrize, raffle_address.clone(), prize_amount);
        Self::upsert_leaderboard(&env, &DataKey::TopByVolume, raffle_address, total_volume);
        Ok(())
    }

    pub fn get_leaderboard(env: Env, metric: LeaderboardMetric) -> Vec<(Address, i128)> {
        let key = match metric {
            LeaderboardMetric::TicketsSold => DataKey::TopByTickets,
            LeaderboardMetric::PrizeAmount => DataKey::TopByPrize,
            LeaderboardMetric::TotalVolume => DataKey::TopByVolume,
        };
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env))
    }

    pub fn emergency_pause_all(env: Env, reason: soroban_sdk::String) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        env.storage()
            .persistent()
            .set(&DataKey::GlobalEmergencyPause, &true);
        events::GlobalEmergencyPaused {
            paused_by: admin,
            reason,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);
        Ok(())
    }

    pub fn emergency_unpause_all(env: Env) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        env.storage()
            .persistent()
            .set(&DataKey::GlobalEmergencyPause, &false);
        events::GlobalEmergencyUnpaused {
            unpaused_by: admin,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);
        Ok(())
    }

    pub fn is_global_paused(env: Env) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::GlobalEmergencyPause)
            .unwrap_or(false)
    }

    pub fn clean_old_raffle(env: Env, raffle_id: u32) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;

        // Look up the raffle by its stable ID.  A missing entry means the ID
        // was never assigned or has already been cleaned up.
        let raffle_address: Address = env
            .storage()
            .persistent()
            .get(&DataKey::RaffleById(raffle_id))
            .ok_or(ContractError::InvalidRaffleId)?;

        env.invoke_contract::<()>(
            &raffle_address,
            &Symbol::new(&env, "wipe_storage"),
            ().into_val(&env),
        );

        // Tombstone: remove the stable-map entry so the slot is freed and
        // `get_raffles_page` will skip it.  The stable_id is never reused so
        // other IDs are completely unaffected — no shifting, no reindexing.
        env.storage()
            .persistent()
            .remove(&DataKey::RaffleById(raffle_id));

        // Decrement the live count (floor at 0 for safety).
        let live_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::RaffleCount)
            .unwrap_or(0u32);
        env.storage()
            .persistent()
            .set(&DataKey::RaffleCount, &live_count.saturating_sub(1));

        events::RaffleCleanedUp {
            raffle_address,
            cleaned_by: admin,
            finish_time: 0,
            cleaned_at: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    /// Set the display name for the caller's creator profile.
    ///
    /// Creators can self-service update their profile name to provide a
    /// human-readable identity for frontends. The name is capped at
    /// [`MAX_DESCRIPTION_LENGTH`] (1 000 bytes).
    ///
    /// # Auth
    ///
    /// Requires authorization from the creator address whose profile is being
    /// updated.
    ///
    /// # Parameters
    ///
    /// - `creator` — Address of the profile owner.
    /// - `name` — Display name string (max 1 000 bytes).
    ///
    /// # Errors
    ///
    /// - [`ContractError::InvalidParameters`] — name exceeds
    ///   [`MAX_DESCRIPTION_LENGTH`].
    ///
    /// # Events
    ///
    /// Emits [`events::ProfileNameSet`] on success.
    pub fn set_profile_name(
        env: Env,
        creator: Address,
        name: soroban_sdk::String,
    ) -> Result<(), ContractError> {
        creator.require_auth();

        if name.len() > MAX_DESCRIPTION_LENGTH {
            return Err(ContractError::InvalidParameters);
        }

        let mut profile: CreatorProfile = env
            .storage()
            .persistent()
            .get(&DataKey::CreatorProfile(creator.clone()))
            .unwrap_or(CreatorProfile {
                name: soroban_sdk::String::from_str(&env, ""),
                verified: false,
                raffles_created: 0,
            });

        profile.name = name.clone();
        env.storage()
            .persistent()
            .set(&DataKey::CreatorProfile(creator.clone()), &profile);

        events::ProfileNameSet {
            creator,
            name,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    /// Grant or revoke the verified badge for a creator profile.
    ///
    /// The admin can set the `verified` flag on any creator's profile to
    /// signal trustworthiness and reputation to frontends. This provides a
    /// lightweight on-chain trust signal without requiring off-chain
    /// infrastructure.
    ///
    /// # Auth
    ///
    /// Requires authorization from the current admin address.
    ///
    /// # Parameters
    ///
    /// - `creator` — Address of the profile to update.
    /// - `verified` — `true` to grant the badge, `false` to revoke it.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotAuthorized`] — caller is not the admin.
    ///
    /// # Events
    ///
    /// Emits [`events::VerifiedStatusSet`] on success.
    pub fn set_verified(
        env: Env,
        creator: Address,
        verified: bool,
    ) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;

        let mut profile: CreatorProfile = env
            .storage()
            .persistent()
            .get(&DataKey::CreatorProfile(creator.clone()))
            .unwrap_or(CreatorProfile {
                name: soroban_sdk::String::from_str(&env, ""),
                verified: false,
                raffles_created: 0,
            });

        profile.verified = verified;
        env.storage()
            .persistent()
            .set(&DataKey::CreatorProfile(creator.clone()), &profile);

        events::VerifiedStatusSet {
            creator,
            verified,
            set_by: admin,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        Ok(())
    }

    /// Retrieve the creator profile for a given address.
    ///
    /// Returns the on-chain profile containing the creator's display name,
    /// verified status, and number of raffles created. If no profile exists
    /// for the address, returns a default profile with an empty name,
    /// `verified = false`, and `raffles_created = 0`.
    ///
    /// # Parameters
    ///
    /// - `creator` — Address to query.
    ///
    /// # Returns
    ///
    /// [`CreatorProfile`] containing name, verified badge, and track record.
    pub fn get_profile(env: Env, creator: Address) -> CreatorProfile {
        env.storage()
            .persistent()
            .get(&DataKey::CreatorProfile(creator))
            .unwrap_or(CreatorProfile {
                name: soroban_sdk::String::from_str(&env, ""),
                verified: false,
                raffles_created: 0,
            })
    }
}

#[cfg(test)]
mod tests;

