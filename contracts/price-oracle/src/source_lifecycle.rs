//! #402 — Automated source onboarding / offboarding
//!
//! ## Onboarding (identity → bond → probation)
//!
//! 1. **Identity** — [`onboard`] binds the source to an identity fingerprint
//!    (e.g. the hash of its #211 verification record / DID). The fingerprint
//!    must be unused and never revoked, and the address must never have been
//!    offboarded. Pass/fail evidence: [`OnboardingChecklist::identity_verified`].
//! 2. **Bond** — the source deposits the configured bond through the existing
//!    `deposit_source_bond`. Evidence: `bond_posted` / `bond_amount`.
//! 3. **Probation** — lasts `probation_secs`. While on probation a source's
//!    quorum influence is hard-capped: at most [`PROBATION_MAX_COUNTED`] probation
//!    value is counted per aggregation round across *all* probation sources,
//!    and probation sources cannot cast volatility-blackout signals (#400).
//!    [`graduate`] lifts probation once both the time and the bond steps pass.
//!
//! ## Offboarding (one atomic step)
//!
//! [`offboard`] runs in a single transaction: optional evidence-based slashing,
//! revocation of every piece of derived state (per-asset submissions and
//! submission ledgers, compliance flags, reputation pinned to zero, lifecycle
//! record), de-registration, tombstoning of both the address and the identity
//! fingerprint, and recomputation of every asset from the surviving sources.
//! There is no intermediate state in which the source still counts.
//!
//! ## Slashing
//!
//! Slashing requires on-chain evidence: the source's *stored* submission for
//! `evidence_asset` must deviate from that asset's current published aggregate
//! by more than `slash_deviation_bps`. The contract re-derives the deviation
//! itself — the admin cannot assert it. On success the full bond is forfeited
//! to the treasury.

use soroban_sdk::{contractevent, contracttype, panic_with_error, Address, BytesN, Env, String};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregatePrice, DataKey, ErrorCode, PriceEntry};

/// Probation values counted per aggregation round, across all probation sources.
pub const PROBATION_MAX_COUNTED: u32 = 1;
/// Default probation period (7 days).
pub const DEFAULT_PROBATION_SECS: u64 = 7 * 86_400;
/// Default slashing deviation threshold (20 %).
pub const DEFAULT_SLASH_DEVIATION_BPS: u32 = 2_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleConfig {
    pub probation_secs: u64,
    pub slash_deviation_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLifecycle {
    pub identity: BytesN<32>,
    pub onboarded_at: u64,
    pub probation_until: u64,
    pub graduated: bool,
}

/// Pass/fail evidence for every onboarding step.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingChecklist {
    pub identity_verified: bool,
    pub bond_posted: bool,
    pub bond_amount: i128,
    pub probation_complete: bool,
    pub graduated: bool,
}

#[contracttype]
#[derive(Clone)]
enum LcKey {
    Config,
    Record(Address),
    IdentityOwner(BytesN<32>),
    RevokedIdentity(BytesN<32>),
    RevokedAddress(Address),
    AnyProbation,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceOnboardedEvent {
    #[topic]
    pub source: Address,
    pub identity: BytesN<32>,
    pub probation_until: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceGraduatedEvent {
    #[topic]
    pub source: Address,
    pub bond_amount: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceOffboardedEvent {
    #[topic]
    pub source: Address,
    pub identity: Option<BytesN<32>>,
    pub slashed: bool,
    pub assets_revoked: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceSlashedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub evidence_asset: Address,
    pub submitted_price: i128,
    pub aggregate_price: i128,
    pub deviation_bps: i128,
    pub threshold_bps: u32,
    pub bond_forfeited: i128,
}

pub fn get_config(env: &Env) -> LifecycleConfig {
    env.storage()
        .persistent()
        .get(&LcKey::Config)
        .unwrap_or(LifecycleConfig {
            probation_secs: DEFAULT_PROBATION_SECS,
            slash_deviation_bps: DEFAULT_SLASH_DEVIATION_BPS,
        })
}

pub fn set_config(env: &Env, cfg: LifecycleConfig) {
    get_admin(env).require_auth();
    if cfg.slash_deviation_bps == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage().persistent().set(&LcKey::Config, &cfg);
}

pub fn get_record(env: &Env, source: &Address) -> Option<SourceLifecycle> {
    env.storage()
        .persistent()
        .get(&LcKey::Record(source.clone()))
}

pub fn is_revoked(env: &Env, source: &Address) -> bool {
    env.storage()
        .persistent()
        .has(&LcKey::RevokedAddress(source.clone()))
}

/// Blocks registration (and key rotation) onto an offboarded address.
pub fn check_not_revoked(env: &Env, source: &Address) {
    if is_revoked(env, source) {
        panic_with_error!(env, ErrorCode::IdentityRevoked);
    }
}

/// Carries the lifecycle record (probation state and identity binding) across
/// a key rotation, so rotating cannot shed probation or the identity record.
pub fn on_key_rotated(env: &Env, old: &Address, new: &Address) {
    let Some(record) = get_record(env, old) else {
        return;
    };
    let storage = env.storage().persistent();
    storage.remove(&LcKey::Record(old.clone()));
    storage.set(&LcKey::Record(new.clone()), &record);
    storage.set(&LcKey::IdentityOwner(record.identity), new);
}

/// Cheap global guard read once per aggregation round.
pub fn any_probation(env: &Env) -> bool {
    env.storage().persistent().has(&LcKey::AnyProbation)
}

pub fn on_probation(env: &Env, source: &Address) -> bool {
    any_probation(env) && get_record(env, source).is_some_and(|r| !r.graduated)
}

/// Registers `source` with an identity fingerprint and starts probation.
pub fn onboard(env: &Env, source: Address, name: String, identity: BytesN<32>) {
    get_admin(env).require_auth();
    let revoked = env
        .storage()
        .persistent()
        .has(&LcKey::RevokedIdentity(identity.clone()));
    if revoked {
        panic_with_error!(env, ErrorCode::IdentityRevoked);
    }
    if env
        .storage()
        .persistent()
        .has(&LcKey::IdentityOwner(identity.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceAlreadyExists);
    }
    crate::sources::add_source(env, source.clone(), name);

    let now = env.ledger().timestamp();
    let record = SourceLifecycle {
        identity: identity.clone(),
        onboarded_at: now,
        probation_until: now.saturating_add(get_config(env).probation_secs),
        graduated: false,
    };
    let key = LcKey::Record(source.clone());
    env.storage().persistent().set(&key, &record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    env.storage()
        .persistent()
        .set(&LcKey::IdentityOwner(identity.clone()), &source);
    env.storage().persistent().set(&LcKey::AnyProbation, &true);
    SourceOnboardedEvent {
        source,
        identity,
        probation_until: record.probation_until,
    }
    .publish(env);
}

pub fn checklist(env: &Env, source: &Address) -> OnboardingChecklist {
    let record = get_record(env, source);
    let bond_amount = crate::sources::get_source_deposited_bond(env, source.clone());
    let required = crate::sources::get_source_bond(env);
    OnboardingChecklist {
        identity_verified: record.is_some() && !is_revoked(env, source),
        bond_posted: bond_amount > 0 && bond_amount >= required,
        bond_amount,
        probation_complete: record
            .as_ref()
            .is_some_and(|r| env.ledger().timestamp() >= r.probation_until),
        graduated: record.is_some_and(|r| r.graduated),
    }
}

/// Lifts probation once the probation period has elapsed and the bond is
/// posted. Permissionless: it only checks recorded evidence.
pub fn graduate(env: &Env, source: Address) {
    let mut record = get_record(env, &source)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::SourceNotFound));
    let list = checklist(env, &source);
    if record.graduated || !list.identity_verified || !list.bond_posted || !list.probation_complete
    {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    record.graduated = true;
    env.storage()
        .persistent()
        .set(&LcKey::Record(source.clone()), &record);
    SourceGraduatedEvent {
        source,
        bond_amount: list.bond_amount,
    }
    .publish(env);
}

fn deviation_bps(price: i128, reference: i128) -> i128 {
    (price - reference).saturating_abs().saturating_mul(10_000) / reference
}

/// Atomically offboards `source`, optionally slashing on evidence from
/// `evidence_asset`. See the module docs for the full sequence.
pub fn offboard(env: &Env, source: Address, evidence_asset: Option<Address>) {
    get_admin(env).require_auth();
    if !crate::sources::is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    // 1. Slashing — evidence is read before any state is revoked.
    let slashed = if let Some(asset) = evidence_asset {
        let entry: PriceEntry = env
            .storage()
            .persistent()
            .get(&DataKey::Submission(asset.clone(), source.clone()))
            .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));
        let aggregate: AggregatePrice = env
            .storage()
            .persistent()
            .get(&DataKey::Aggregate(asset.clone()))
            .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));
        let threshold = get_config(env).slash_deviation_bps;
        if aggregate.price <= 0 {
            panic_with_error!(env, ErrorCode::NoData);
        }
        let dev = deviation_bps(entry.price, aggregate.price);
        if dev <= threshold as i128 {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
        let bond = crate::sources::get_source_deposited_bond(env, source.clone());
        crate::sources::forfeit_source_bond_internal(env, source.clone());
        SourceSlashedEvent {
            source: source.clone(),
            evidence_asset: asset,
            submitted_price: entry.price,
            aggregate_price: aggregate.price,
            deviation_bps: dev,
            threshold_bps: threshold,
            bond_forfeited: bond,
        }
        .publish(env);
        true
    } else {
        false
    };

    // 2. Revoke all derived per-asset state.
    let assets = crate::storage::read_registered_assets(env);
    let storage = env.storage().persistent();
    for asset in assets.iter() {
        storage.remove(&DataKey::Submission(asset.clone(), source.clone()));
        storage.remove(&DataKey::SubmissionLedger(asset.clone(), source.clone()));
        storage.remove(&DataKey::LastSubmissionLedger(
            source.clone(),
            asset.clone(),
        ));
        storage.remove(&DataKey::SourceNonCompliant(source.clone(), asset.clone()));
    }
    // Reputation is pinned to zero rather than deleted: deleting would reset
    // it to the default score, laundering the record.
    storage.set(&DataKey::SourceReputation(source.clone()), &0i128);

    // 3. Tombstone address and identity so neither can be re-onboarded.
    let identity = get_record(env, &source).map(|r| r.identity);
    storage.remove(&LcKey::Record(source.clone()));
    storage.set(&LcKey::RevokedAddress(source.clone()), &true);
    if let Some(id) = identity.clone() {
        storage.remove(&LcKey::IdentityOwner(id.clone()));
        storage.set(&LcKey::RevokedIdentity(id), &true);
    }

    // 4. De-register and recompute every asset from the surviving sources.
    crate::sources::remove_source_inner(env, source.clone());
    crate::recompute::recompute_all(env, crate::recompute::RecomputeReason::Removed);

    SourceOffboardedEvent {
        source,
        identity,
        slashed,
        assets_revoked: assets.len(),
    }
    .publish(env);
}
