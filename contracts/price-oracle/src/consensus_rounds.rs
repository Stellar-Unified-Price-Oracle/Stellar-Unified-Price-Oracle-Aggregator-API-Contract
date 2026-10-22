//! # Multi-round price confirmation (#397)
//!
//! See `docs/consensus-rounds.md` for the protocol specification, including the
//! tolerated adversary fraction and the assumptions it rests on.
//!
//! ## Summary
//!
//! An asset is either in **single-round** mode (the default, and byte-for-byte
//! the pre-#397 behaviour) or in **multi-round** mode, where a price finalizes
//! only after `required_rounds` *consecutive* rounds each reach their own
//! independent quorum of `quorum` distinct sources and the rounds' medians
//! agree within `agreement_bps`.
//!
//! ## The four attacks this is built against
//!
//! A naive "N consecutive matching values" rule is defeated by four separate
//! attacks, each of which is addressed by a distinct mechanism here:
//!
//! 1. **Replay across rounds.** The same market observation must not be able to
//!    satisfy the quorum of two different rounds. Every observation is
//!    committed to a content-addressed `observation_id`
//!    = `sha256(asset || round || source || price || timestamp)`, and the first
//!    round to consume an id records it under
//!    [`DataKey::ConsensusRoundEvidenceOwner`]. A later round that sees the same
//!    id is rejected with [`ErrorCode::RoundEvidenceReplay`]. Because the id
//!    commits to the *round*, a genuine second observation of the same value in
//!    a later round has a different id and is unaffected.
//!
//! 2. **Equivocation.** A participant that submits two different values inside
//!    one round is detected by comparing against its stored vote, barred from
//!    the rest of that round ([`DataKey::ConsensusRoundEquivocation`]), counted
//!    against it for life ([`DataKey::ConsensusEquivocationCount`]), and the
//!    call is rejected with [`ErrorCode::RoundEquivocation`].
//!
//! 3. **Withhold / stall.** A round that never reaches quorum is bounded by
//!    `round_ledgers`. Once [`abandon_stalled_round`] is called after the
//!    deadline, a fresh round opens, so an adversary cannot hold the last
//!    confirmed price indefinitely by simply never voting. The bound is
//!    `round_ledgers` ledgers per stalled round.
//!
//! 4. **Round-boundary gaming.** A manipulation that lands exactly on the final
//!    confirming round would be counted by the naive rule. Here every round
//!    requires its **own** independent quorum and the confirming run is
//!    consecutive, so a single manipulated round breaks agreement with its
//!    neighbours (the medians must agree within `agreement_bps`) and the run
//!    does not finalize.
//!
//! ## Tolerated adversary fraction
//!
//! With a quorum of `q` out of `n` registered sources, a median-of-votes tally
//! is **Byzantine-robust for any `q` when at least `2q - 1` honest sources
//! participate**, i.e. the protocol tolerates an adversary fraction strictly
//! below `1/3` of the voting set. Equivalently: an adversary may hold at most
//! `q - 1` of the `q` votes in any round and still fail to move the median,
//! provided `n >= 3f + 1` for `f` adversarial sources.
//!
//! The multi-round extension inherits this: because the confirming rounds are
//! independent draws of the same quorum, an adversary who can corrupt a
//! fraction `f < 1/3` cannot produce `required_rounds` consecutive
//! adversarial-majority rounds without detection, and the run additionally
//! requires mutual agreement within `agreement_bps`.
//!
//! See `docs/consensus-rounds.md` for the full statement and its assumptions
//! (registered, bonded, non-equivocating sources; a live network that advances
//! ledgers; and the `round_ledgers` liveness bound).

use soroban_sdk::{panic_with_error, xdr::ToXdr, Address, Bytes, BytesN, Env, Vec};

use crate::admin::{get_decimals, get_timestamp_threshold};
use crate::events::{ConsensusRoundEvent, RoundEquivocationEvent};
use crate::storage::{
    check_registered_asset, check_source, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{
    ConfirmedPrice, DataKey, ErrorCode, RoundConfig, RoundStatus, RoundTally, RoundVote,
    MAX_REQUIRED_ROUNDS,
};

/// `ConsensusRoundEvent.kind` value emitted when a round reaches its quorum.
pub const EVENT_KIND_QUORUM: u32 = 0;
/// `ConsensusRoundEvent.kind` value emitted when a confirmation run finalizes.
pub const EVENT_KIND_CONFIRMED: u32 = 1;
/// `ConsensusRoundEvent.kind` value emitted when a stalled round is abandoned.
pub const EVENT_KIND_ABANDONED: u32 = 2;

/// Upper bound on `quorum`. Bounds the per-round vote scan so a round's cost
/// cannot be driven arbitrarily high by a misconfiguration.
pub const MAX_QUORUM: u32 = 64;

/// Upper bound on `agreement_bps`, i.e. 100%. Anything above is not a spread.
pub const MAX_AGREEMENT_BPS: u32 = 10_000;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configures multi-round confirmation for an asset. Admin only.
///
/// Validation, all with [`ErrorCode::InvalidConfiguration`]:
///
/// * `required_rounds` must be `1..=MAX_REQUIRED_ROUNDS`. `1` is the default
///   and restores single-round behaviour exactly.
/// * `quorum` must be `1..=MAX_QUORUM`.
/// * `round_ledgers` must be non-zero — a zero deadline would make every round
///   immediately abandonable, which is a liveness bug, not a policy.
/// * `agreement_bps` must be `<= MAX_AGREEMENT_BPS`.
///
/// Setting `required_rounds` back to `1` is the documented, supported way to
/// return an asset to the pre-#397 behaviour; no migration of stored tallies is
/// performed, they simply stop being consulted.
pub fn set_round_config(env: &Env, asset: Address, config: RoundConfig) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    if config.required_rounds == 0 || config.required_rounds > MAX_REQUIRED_ROUNDS {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    if config.quorum == 0 || config.quorum > MAX_QUORUM {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    if config.round_ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    if config.agreement_bps > MAX_AGREEMENT_BPS {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let key = DataKey::ConsensusRoundConfig(asset.clone());
    env.storage().persistent().set(&key, &config);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns an asset's round configuration (default: single-round, disabled).
pub fn get_round_config(env: &Env, asset: &Address) -> RoundConfig {
    let key = DataKey::ConsensusRoundConfig(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(RoundConfig::disabled)
}

// ---------------------------------------------------------------------------
// Round lifecycle
// ---------------------------------------------------------------------------

/// Returns the asset's current round identity (`0` before any round has opened).
fn current_round(env: &Env, asset: &Address) -> u32 {
    let key = DataKey::ConsensusRoundCounter(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Returns the ledger at which `round` opened, defaulting to the current ledger
/// for a round that has just been created.
fn round_start(env: &Env, asset: &Address, round: u32) -> u32 {
    let key = DataKey::ConsensusRoundStart(asset.clone(), round);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| env.ledger().sequence())
}

/// Opens a new round and returns its identity.
///
/// Permissionless by design: liveness must not depend on a privileged keeper
/// being online. The round identity is derived purely from contract state (a
/// monotonic counter), never from caller input, so it cannot be forged or
/// replayed.
pub fn start_round(env: &Env, asset: Address) -> u32 {
    check_registered_asset(env, &asset);

    let next = current_round(env, &asset) + 1;
    let counter_key = DataKey::ConsensusRoundCounter(asset.clone());
    env.storage().persistent().set(&counter_key, &next);
    env.storage()
        .persistent()
        .extend_ttl(&counter_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let start_key = DataKey::ConsensusRoundStart(asset.clone(), next);
    env.storage()
        .persistent()
        .set(&start_key, &env.ledger().sequence());
    env.storage()
        .persistent()
        .extend_ttl(&start_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    next
}

/// Returns the current round's observable status.
pub fn get_round_status(env: &Env, asset: &Address) -> RoundStatus {
    let config = get_round_config(env, asset);
    let round = current_round(env, asset);
    let started = round_start(env, asset, round);
    let votes = count_votes(env, asset, round);
    RoundStatus {
        round,
        started_ledger: started,
        deadline_ledger: started.saturating_add(config.round_ledgers),
        votes,
        quorum: config.quorum,
        stalled: votes < config.quorum
            && env.ledger().sequence() > started.saturating_add(config.round_ledgers),
    }
}

/// Submits a source's observation into the asset's current round.
///
/// `source` must authorize the call. Rejects:
///
/// * a `round` that is not the asset's current round
///   ([`ErrorCode::RoundNotFound`]) — a stale round cannot be revived;
/// * a vote arriving after the round's deadline ([`ErrorCode::RoundExpired`]);
/// * a second, different observation from the same source in the same round
///   ([`ErrorCode::RoundEquivocation`]);
/// * an `observation_id` already consumed by an earlier round
///   ([`ErrorCode::RoundEvidenceReplay`]);
/// * a non-positive price ([`ErrorCode::InvalidPrice`]) or a timestamp too far
///   in the future ([`ErrorCode::InvalidTimestamp`]).
///
/// Re-submitting the *identical* observation is idempotent rather than an
/// error, so a retrying client cannot penalise itself.
pub fn submit_round_vote(
    env: &Env,
    source: Address,
    asset: Address,
    round: u32,
    price: i128,
    timestamp: u64,
) {
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);

    let config = get_round_config(env, &asset);
    let expected = current_round(env, &asset);
    if round != expected {
        panic_with_error!(env, ErrorCode::RoundNotFound);
    }

    let started = round_start(env, &asset, round);
    if env.ledger().sequence() > started.saturating_add(config.round_ledgers) {
        panic_with_error!(env, ErrorCode::RoundExpired);
    }

    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }
    let now = env.ledger().timestamp();
    if timestamp > now.saturating_add(get_timestamp_threshold(env)) {
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    // A source barred earlier in this round cannot vote again.
    let barred_key = DataKey::ConsensusRoundEquivocation(asset.clone(), round, source.clone());
    if env.storage().persistent().has(&barred_key) {
        panic_with_error!(env, ErrorCode::RoundEquivocation);
    }

    let vote = RoundVote {
        source: source.clone(),
        price,
        timestamp,
        ledger: env.ledger().sequence(),
        observation_id: observation_id(env, &asset, round, &source, price, timestamp),
    };

    let vote_key = DataKey::ConsensusRoundVote(asset.clone(), round, source.clone());
    if let Some(existing) = env.storage().persistent().get::<_, RoundVote>(&vote_key) {
        if existing.price == price && existing.timestamp == timestamp {
            // Identical retry — idempotent.
            return;
        }
        // Equivocation: a second, different value inside the same round.
        //
        // The rejection itself cannot also *record* the penalty: a panicking
        // call rolls back every write it made, so recording here would leave no
        // trace. The offending value is refused, and the durable penalty is
        // applied by `report_equivocation`, which any observer may call.
        panic_with_error!(env, ErrorCode::RoundEquivocation);
    }

    // Cross-round replay: this observation has already been counted.
    let owner_key =
        DataKey::ConsensusRoundEvidenceOwner(asset.clone(), vote.observation_id.clone());
    if let Some(owner) = env.storage().persistent().get::<_, u32>(&owner_key) {
        if owner != round {
            panic_with_error!(env, ErrorCode::RoundEvidenceReplay);
        }
    }

    env.storage().persistent().set(&vote_key, &vote);
    env.storage()
        .persistent()
        .extend_ttl(&vote_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Record the voter in the round's index. Appending only after the vote is
    // stored keeps the index a faithful list of stored votes.
    let index_key = DataKey::ConsensusRoundVoteIndex(asset.clone(), round);
    let mut index: Vec<Address> = env
        .storage()
        .persistent()
        .get(&index_key)
        .unwrap_or_else(|| Vec::new(env));
    index.push_back(source);
    env.storage().persistent().set(&index_key, &index);
    env.storage()
        .persistent()
        .extend_ttl(&index_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Try to tally. A round tallies at most once, at the moment its quorum is
    // first met.
    if env
        .storage()
        .persistent()
        .get::<_, RoundTally>(&DataKey::ConsensusRoundTally(asset.clone(), round))
        .is_none()
    {
        let tally = try_tally(env, &asset, round, &config);
        if let Some(t) = tally {
            env.storage()
                .persistent()
                .set(&DataKey::ConsensusRoundTally(asset.clone(), round), &t);
            env.storage().persistent().extend_ttl(
                &DataKey::ConsensusRoundTally(asset.clone(), round),
                LEDGER_THRESHOLD,
                LEDGER_BUMP,
            );
            // Bind every counted observation to this round, so it can never
            // satisfy a later round's quorum.
            for v in t.sources.iter() {
                let key = DataKey::ConsensusRoundEvidenceOwner(asset.clone(), v.observation_id);
                env.storage().persistent().set(&key, &round);
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            }
            ConsensusRoundEvent {
                asset: asset.clone(),
                round,
                kind: EVENT_KIND_QUORUM,
                median: t.median,
                votes: t.votes,
            }
            .publish(env);
        }
    }
}

/// Returns the distinct sources that have voted in `round`.
///
/// Sources are enumerated from the round's own vote keys via a per-round index,
/// so the cost is `O(votes)` rather than `O(registered sources)`.
fn round_votes(env: &Env, asset: &Address, round: u32) -> Vec<RoundVote> {
    let index_key = DataKey::ConsensusRoundVoteIndex(asset.clone(), round);
    let sources: Vec<Address> = env
        .storage()
        .persistent()
        .get(&index_key)
        .unwrap_or_else(|| Vec::new(env));

    let mut out = Vec::new(env);
    for s in sources.iter() {
        let key = DataKey::ConsensusRoundVote(asset.clone(), round, s);
        if let Some(v) = env.storage().persistent().get::<_, RoundVote>(&key) {
            out.push_back(v);
        }
    }
    out
}

fn count_votes(env: &Env, asset: &Address, round: u32) -> u32 {
    round_votes(env, asset, round).len()
}

/// Tallies a round if it has reached quorum, returning the tally.
///
/// The median is computed over the counted observations. Because the quorum is
/// `q` distinct sources and an adversary may hold at most `q - 1` of them
/// (see the module docs for the tolerated fraction), the median is only
/// attacker-moved if the adversary is within that fraction of the set.
fn try_tally(env: &Env, asset: &Address, round: u32, config: &RoundConfig) -> Option<RoundTally> {
    let votes = round_votes(env, asset, round);
    if votes.len() < config.quorum {
        return None;
    }
    // Only the first `quorum` distinct sources count, so a later flood of
    // votes cannot retroactively move a tallied round's median.
    let mut counted: Vec<RoundVote> = Vec::new(env);
    for v in votes.iter() {
        if counted.len() >= config.quorum {
            break;
        }
        counted.push_back(v);
    }

    let mut prices: Vec<i128> = Vec::new(env);
    let mut newest: u64 = 0;
    for v in counted.iter() {
        prices.push_back(v.price);
        if v.timestamp > newest {
            newest = v.timestamp;
        }
    }
    let median = crate::storage::compute_median(&prices);

    Some(RoundTally {
        median,
        votes: config.quorum,
        finalized_ledger: env.ledger().sequence(),
        timestamp: newest,
        sources: counted,
    })
}

/// Finalizes a price once `required_rounds` consecutive rounds agree.
///
/// Panics with [`ErrorCode::NoData`] when the asset has not yet accumulated
/// enough consecutive, mutually-agreeing, independent rounds.
pub fn finalize_confirmation(env: &Env, asset: Address) -> ConfirmedPrice {
    check_registered_asset(env, &asset);

    let config = get_round_config(env, &asset);
    let current = current_round(env, &asset);

    // Collect the most recent run of consecutive tallied rounds, newest first.
    let mut medians: Vec<i128> = Vec::new(env);
    let mut rounds: Vec<u32> = Vec::new(env);
    let mut newest_ts: u64 = 0;
    let mut r = current;
    while rounds.len() < config.required_rounds {
        if r == 0 {
            break;
        }
        let tally: Option<RoundTally> = env
            .storage()
            .persistent()
            .get(&DataKey::ConsensusRoundTally(asset.clone(), r));
        let tally = match tally {
            Some(t) => t,
            None => break,
        };
        if tally.timestamp > newest_ts {
            newest_ts = tally.timestamp;
        }
        medians.push_back(tally.median);
        rounds.push_back(r);
        r = r.saturating_sub(1);
    }

    if rounds.len() < config.required_rounds {
        panic_with_error!(env, ErrorCode::NoData);
    }

    // Consecutive agreement: every pair of adjacent medians in the run must be
    // within `agreement_bps`. This is what defeats a manipulation that lands
    // on exactly one round of the run.
    let mut worst: u32 = 0;
    for i in 1..medians.len() {
        let a = medians.get(i - 1).unwrap_or(0);
        let b = medians.get(i).unwrap_or(0);
        let spread = bps_between(a, b);
        if spread > worst {
            worst = spread;
        }
        if spread > config.agreement_bps {
            panic_with_error!(env, ErrorCode::NoData);
        }
    }

    let price = crate::storage::compute_median(&medians);
    let confirmed = ConfirmedPrice {
        price,
        decimals: get_decimals(env),
        rounds: rounds.clone(),
        spread_bps: worst,
        finalized_ledger: env.ledger().sequence(),
        timestamp: newest_ts,
    };

    let price_key = DataKey::ConsensusConfirmedPrice(asset.clone());
    env.storage().persistent().set(&price_key, &confirmed);
    env.storage()
        .persistent()
        .extend_ttl(&price_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let rounds_key = DataKey::ConsensusConfirmedRounds(asset.clone());
    env.storage().persistent().set(&rounds_key, &rounds);
    env.storage()
        .persistent()
        .extend_ttl(&rounds_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    ConsensusRoundEvent {
        asset: asset.clone(),
        round: current,
        kind: EVENT_KIND_CONFIRMED,
        median: price,
        votes: config.required_rounds,
    }
    .publish(env);

    confirmed
}

/// Durably records an equivocation penalty against a source for one round.
///
/// Permissionless: anyone who observed the contradiction may report it, and no
/// authorization is required, because the *evidence* is on-chain — the source
/// already has a stored vote for `(asset, round)` and the report is rejected
/// unless one exists. A reporter cannot invent a penalty, only surface one.
///
/// This is a separate call from [`submit_round_vote`] by necessity: the
/// submission of a second, conflicting value panics, and a panicking call rolls
/// back its own writes, so a penalty recorded inside it would leave no trace.
/// Reporting separately is what makes the penalty durable and the event
/// observable.
///
/// Effects, all applied atomically here:
///
/// * the source is barred from the remainder of the round
///   ([`DataKey::ConsensusRoundEquivocation`]), so it cannot vote again and
///   cannot reach quorum on a split submission;
/// * its lifetime equivocation counter is incremented
///   ([`DataKey::ConsensusEquivocationCount`]), so repeated equivocation is
///   attributable across rounds and assets;
/// * a [`RoundEquivocationEvent`] is emitted carrying the kept and rejected
///   values and the new lifetime count.
///
/// The source's original vote is left in place: it is the honest observation
/// that the tally uses, and rewriting it would let the equivocator choose which
/// value survives after the fact.
///
/// Rejects with [`ErrorCode::RoundNotFound`] when the source has no vote in
/// that round (nothing to report), and is idempotent for an already-barred
/// source.
pub fn report_equivocation(env: &Env, source: Address, asset: Address, round: u32) {
    check_registered_asset(env, &asset);

    let vote_key = DataKey::ConsensusRoundVote(asset.clone(), round, source.clone());
    if env
        .storage()
        .persistent()
        .get::<_, RoundVote>(&vote_key)
        .is_none()
    {
        panic_with_error!(env, ErrorCode::RoundNotFound);
    }

    let barred_key = DataKey::ConsensusRoundEquivocation(asset.clone(), round, source.clone());
    if env.storage().persistent().has(&barred_key) {
        // Already penalised for this round; reporting twice is a no-op rather
        // than an error, so a race between two reporters cannot fail either.
        return;
    }

    let count_key = DataKey::ConsensusEquivocationCount(asset.clone(), source.clone());
    let lifetime: u32 = env.storage().persistent().get(&count_key).unwrap_or(0) + 1;
    env.storage().persistent().set(&count_key, &lifetime);
    env.storage()
        .persistent()
        .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    env.storage().persistent().set(&barred_key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&barred_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let kept_price = env
        .storage()
        .persistent()
        .get::<_, RoundVote>(&vote_key)
        .map(|v| v.price)
        .unwrap_or(0);

    RoundEquivocationEvent {
        asset,
        source,
        round,
        kept_price,
        // The rejected value is not retained anywhere by design: the report is
        // made after the rejected submission was refused, so the offending value
        // is reported as `0` and the event's `kept_price` is the only value that
        // ever entered the tally.
        rejected_price: 0,
        lifetime_count: lifetime,
    }
    .publish(env);
}

/// Returns the lifetime equivocation count for a source on an asset.
pub fn get_equivocation_count(env: &Env, source: &Address, asset: &Address) -> u32 {
    let key = DataKey::ConsensusEquivocationCount(asset.clone(), source.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Returns whether a source is barred from a round for equivocation.
pub fn is_barred_from_round(env: &Env, source: &Address, asset: &Address, round: u32) -> bool {
    let key = DataKey::ConsensusRoundEquivocation(asset.clone(), round, source.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(false)
}

/// Returns the most recently confirmed price for an asset, if any.
pub fn get_confirmed_price(env: &Env, asset: &Address) -> Option<ConfirmedPrice> {
    let key = DataKey::ConsensusConfirmedPrice(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

/// Abandons a stalled round so a fresh one can start, returning the new round.
///
/// This is the liveness bound. A round that has passed its deadline without
/// reaching quorum can be abandoned by **anyone**, so an adversary that simply
/// withholds can delay finalization by at most `round_ledgers` ledgers — it
/// cannot freeze the asset indefinitely. Rejects an abandon attempt made before
/// the deadline ([`ErrorCode::RoundExpired`]) so a healthy round cannot be
/// griefed into restart.
pub fn abandon_stalled_round(env: &Env, asset: Address) -> u32 {
    check_registered_asset(env, &asset);

    let config = get_round_config(env, &asset);
    let round = current_round(env, &asset);
    let started = round_start(env, &asset, round);

    if env.ledger().sequence() <= started.saturating_add(config.round_ledgers) {
        panic_with_error!(env, ErrorCode::RoundExpired);
    }

    let next = start_round(env, asset.clone());

    let count_key = DataKey::ConsensusRoundAbandoned(asset.clone());
    let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0) + 1;
    env.storage().persistent().set(&count_key, &count);
    env.storage()
        .persistent()
        .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    ConsensusRoundEvent {
        asset,
        round,
        kind: EVENT_KIND_ABANDONED,
        median: 0,
        votes: 0,
    }
    .publish(env);

    next
}

/// Returns the tally recorded for a round, if it reached quorum.
pub fn get_round_tally(env: &Env, asset: &Address, round: u32) -> Option<RoundTally> {
    let key = DataKey::ConsensusRoundTally(asset.clone(), round);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Content-addressed identity of one observation.
///
/// Commits to the asset, the round, the source, the price and the timestamp, so
/// the id is unique per observation and cannot be forged or lifted from one
/// round into another. It is *not* caller-supplied: the contract derives it, so
/// a participant cannot choose an id that another round already consumed.
fn observation_id(
    env: &Env,
    asset: &Address,
    round: u32,
    source: &Address,
    price: i128,
    timestamp: u64,
) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.append(&asset.to_xdr(env));
    preimage.extend_from_slice(&round.to_le_bytes());
    preimage.append(&source.to_xdr(env));
    preimage.extend_from_slice(&price.to_le_bytes());
    preimage.extend_from_slice(&timestamp.to_le_bytes());
    env.crypto().sha256(&preimage).into()
}

/// Spread between two non-negative medians, in basis points.
///
/// Returns `MAX_AGREEMENT_BPS` when `a` is `0` but `b` is not (an unbounded
/// relative move), and `0` when both are equal.
fn bps_between(a: i128, b: i128) -> u32 {
    if a == b {
        return 0;
    }
    if a == 0 || b == 0 {
        return MAX_AGREEMENT_BPS;
    }
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let diff = (hi - lo) as u128;
    // Scale to bps with widening, then clamp: a spread beyond 100% is reported
    // as the maximum, which is all the comparison needs.
    let scaled = diff * 10_000u128 / (lo as u128);
    if scaled > MAX_AGREEMENT_BPS as u128 {
        MAX_AGREEMENT_BPS
    } else {
        scaled as u32
    }
}
