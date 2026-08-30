/// Submit a one-time hash commitment for a ticket (Commit-Reveal mode only).
///
/// # Rules
/// - Randomness source must be `CommitReveal`.
/// - Raffle status must be `Active` only (commits close when `Drawing` starts).
/// - Instance + global pause are enforced.
/// - At most **one** commit per `ticket_id`; a second call returns
///   [`Error::CommitAlreadySubmitted`].
/// - Persistent commit entry TTL is bumped so long-running raffles do not
///   archive the commitment before finalize.
pub(crate) fn submit_commit(env: Env, ticket_id: u32, hash: BytesN<32>) -> Result<(), Error> {
    let raffle = crate::read_raffle(&env)?;

    if raffle.randomness_source != RandomnessSource::CommitReveal {
        return Err(Error::InvalidParameters);
    }

    // Commit window: Active only — no last-look after Drawing begins.
    if raffle.status != RaffleStatus::Active {
        return Err(Error::InvalidStatus);
    }

    require_not_paused(&env)?;
    crate::require_global_not_paused(&env)?;

    let ticket: Ticket = env
        .storage()
        .persistent()
        .get(&DataKey::Ticket(ticket_id))
        .ok_or(Error::TicketNotFound)?;
    ticket.owner.require_auth();

    let key = DataKey::CommitEntry(ticket_id);
    if env.storage().persistent().has(&key) {
        return Err(Error::CommitAlreadySubmitted);
    }

    env.storage().persistent().set(
        &key,
        &CommitRevealEntry {
            committer: ticket.owner,
            hash,
        },
    );

    // Keep the commit live through a long Active period (adjust if the repo
    // already defines shared TTL constants — prefer those).
    env.storage().persistent().extend_ttl(&key, 100, 535_680);

    Ok(())
}
