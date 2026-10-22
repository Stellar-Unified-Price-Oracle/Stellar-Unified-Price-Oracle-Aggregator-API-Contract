//! # Canonical Cross-Chain Asset Registry
//!
//! A single source of truth mapping a Stellar asset address to its
//! representation(s) on foreign chains (contract/token address, decimals).
//! Cross-chain modules — Axelar GMP ([`crate::axelar_gmp`]), LayerZero
//! ([`crate::layerzero`]), and manual cross-reference checks
//! ([`crate::cross_chain_verify`]) — resolve foreign asset identifiers
//! through this registry instead of maintaining their own ad-hoc mappings.
//!
//! ## Schema
//!
//! A mapping is keyed by `(chain, foreign_address)`:
//! * `chain` — a short canonical chain identifier (e.g. `"ethereum"`,
//!   `"polygon"`). Bridge integrations that identify chains differently
//!   (LayerZero's numeric `src_eid`, for example) translate their own
//!   identifier onto this canonical namespace rather than inventing a
//!   parallel one — see [`crate::layerzero::set_lz_chain_name`].
//! * `foreign_address` — the 32-byte canonical representation of the
//!   asset's contract/token address on that chain (left-padded for
//!   shorter addresses such as 20-byte EVM addresses).
//!
//! Each mapping also carries the foreign asset's `decimals` so callers can
//! rescale a price without a second lookup, and an `enabled` flag so admins
//! can suspend a mapping without losing its history.
//!
//! ## Onboarding sandbox
//!
//! Before a source is approved it can run a non-production certification
//! pass with [`certify_source`]. The sandbox validates authentication,
//! asset identifiers, units (decimals), and timing against the live
//! registry and returns a [`SourceCertification`] report. It never writes
//! to production storage: the report is returned to the caller and no
//! mapping, price, or source record is mutated.
//!
//! ## Source removal impact preview
//!
//! Before an administrator removes a source, [`preview_source_removal`]
//! computes — without mutating state — which assets the source currently
//! serves, the remaining quorum after removal, and whether any asset would
//! lose service. The preview is a pure read so it can be run repeatedly and
//! compared against the actual post-removal state.

use soroban_sdk::{panic_with_error, Address, BytesN, Env, String, Vec};

use crate::events::{
    ForeignAssetMappedEvent, ForeignAssetMappingRemovedEvent, ForeignAssetMappingUpdatedEvent,
};
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, ForeignAssetMapping};

/// Maximum number of decimals a foreign asset may declare. Mirrors the
/// widest representation the oracle can rescale without overflow.
pub const MAX_FOREIGN_DECIMALS: u32 = 18;

/// Result of a sandbox certification run for a prospective source.
///
/// Every field is a diagnostic flag so a test source receives clear,
/// actionable feedback before approval. A source is certified only when
/// all flags are `true`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCertification {
    /// The source's authentication was accepted (address is non-zero and
    /// the caller authorized the sandbox run).
    pub auth_ok: bool,
    /// The Stellar asset identifier is registered in the oracle.
    pub asset_ok: bool,
    /// The foreign asset identifier resolves to a known mapping.
    pub foreign_asset_ok: bool,
    /// The declared decimals are within the supported unit range.
    pub units_ok: bool,
    /// The declared observation timestamp is not in the future relative to
    /// the ledger close time.
    pub timing_ok: bool,
    /// Aggregate: `true` only when every individual check passed.
    pub certified: bool,
}

/// Impact preview for removing a source from the oracle.
///
/// Produced by [`preview_source_removal`] without mutating any state. It
/// reports which assets the source currently serves, the quorum that would
/// remain after removal, and whether any asset would lose service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRemovalPreview {
    /// Assets the source currently contributes to.
    pub affected_assets: Vec<Address>,
    /// Number of sources serving the oracle before removal.
    pub sources_before: u32,
    /// Number of sources that would remain after removal.
    pub sources_after: u32,
    /// Quorum required before removal.
    pub quorum_before: u32,
    /// Quorum that would remain after removal.
    pub quorum_after: u32,
    /// `true` when the remaining sources can still meet quorum.
    pub quorum_ok: bool,
    /// `true` when at least one affected asset would lose service (no
    /// remaining source can serve it).
    pub loses_service: bool,
    /// `true` when the removal is safe: quorum is preserved and no asset
    /// loses service.
    pub safe: bool,
}

fn mapping_key(chain: &String, foreign_address: &BytesN<32>) -> DataKey {
    DataKey::ForeignAssetMapping(chain.clone(), foreign_address.clone())
}

fn append_index(env: &Env, key: DataKey, entry: (String, BytesN<32>)) {
    let mut list: Vec<(String, BytesN<32>)> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    list.push_back(entry);
    env.storage().persistent().set(&key, &list);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn remove_from_index(env: &Env, key: DataKey, entry: &(String, BytesN<32>)) {
    let list: Vec<(String, BytesN<32>)> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    let mut updated: Vec<(String, BytesN<32>)> = Vec::new(env);
    for item in list.iter() {
        if item != *entry {
            updated.push_back(item);
        }
    }
    env.storage().persistent().set(&key, &updated);
}

/// Computes the impact of removing `source` from the oracle without
/// mutating any state.
///
/// The preview reads the current source set and per-asset source
/// assignments, then reports the assets the source serves, the quorum that
/// would remain, and whether any asset would lose service. Because it is a
/// pure read, the result matches the actual post-removal state produced by
/// the corresponding removal operation.
///
/// # Arguments
/// * `source` — the source address proposed for removal.
///
/// # Returns
/// A [`SourceRemovalPreview`] describing the operational consequences.
pub fn preview_source_removal(env: &Env, source: Address) -> SourceRemovalPreview {
    let sources: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::SourceList)
        .unwrap_or(Vec::new(env));

    let quorum_before: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::Quorum)
        .unwrap_or(0);

    let sources_before = sources.len();
    let mut sources_after = 0u32;
    for s in sources.iter() {
        if s != source {
            sources_after += 1;
        }
    }

    // Quorum cannot exceed the number of remaining sources; the effective
    // quorum after removal is the configured quorum clamped to the
    // remaining source count.
    let quorum_after = if quorum_before > sources_after {
        sources_after
    } else {
        quorum_before
    };
    let quorum_ok = sources_after >= quorum_before;

    // Collect the assets the source currently serves and check whether any
    // of them would be left without a remaining source.
    let assets: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::AssetList)
        .unwrap_or(Vec::new(env));

    let mut affected_assets: Vec<Address> = Vec::new(env);
    let mut loses_service = false;
    for asset in assets.iter() {
        let asset_sources: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AssetSources(asset.clone()))
            .unwrap_or(Vec::new(env));

        let mut serves = false;
        let mut remaining = 0u32;
        for s in asset_sources.iter() {
            if s == source {
                serves = true;
            } else {
                remaining += 1;
            }
        }

        if serves {
            affected_assets.push_back(asset.clone());
            if remaining == 0 {
                loses_service = true;
            }
        }
    }

    let safe = quorum_ok && !loses_service;

    SourceRemovalPreview {
        affected_assets,
        sources_before,
        sources_after,
        quorum_before,
        quorum_after,
        quorum_ok,
        loses_service,
        safe,
    }
}

/// Runs a non-production certification pass for a prospective source.
///
/// The sandbox validates the source's authentication, asset identifiers,
/// units (decimals), and timing against the live registry and returns a
/// [`SourceCertification`] report. It performs **no** writes: no mapping,
/// price, or source record is created or mutated, so a test source cannot
/// write to production during certification.
///
/// # Arguments
/// * `source` — the prospective source address; must authorize the run.
/// * `stellar_asset` — the Stellar asset the source intends to report on.
/// * `chain` / `foreign_address` — the foreign identifier to resolve.
/// * `decimals` — the units the source will report in.
/// * `observed_at` — the source's observation timestamp (ledger seconds).
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — `source` did not authorize the run.
/// * [`ErrorCode::InvalidString`] — `chain` is not a canonical identifier.
pub fn certify_source(
    env: &Env,
    source: Address,
    stellar_asset: Address,
    chain: String,
    foreign_address: BytesN<32>,
    decimals: u32,
    observed_at: u64,
) -> SourceCertification {
    // Authentication: the source must authorize the sandbox run. This is a
    // read-only check — no source record is persisted.
    source.require_auth();
    crate::input_validation::validate_string(env, "certify_source", "chain", &chain);

    let asset_ok = env
        .storage()
        .persistent()
        .has(&DataKey::AssetConfig(stellar_asset.clone()));

    let foreign_asset_ok = env
        .storage()
        .persistent()
        .has(&mapping_key(&chain, &foreign_address));

    let units_ok = decimals <= MAX_FOREIGN_DECIMALS;

    let now = env.ledger().timestamp();
    let timing_ok = observed_at <= now;

    let certified = asset_ok && foreign_asset_ok && units_ok && timing_ok;

    SourceCertification {
        auth_ok: true,
        asset_ok,
        foreign_asset_ok,
        units_ok,
        timing_ok,
        certified,
    }
}

/// Registers a new canonical mapping from `stellar_asset` to its representation
/// on `chain`. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — `stellar_asset` is not a registered asset.
/// * [`ErrorCode::ForeignAssetAlreadyMapped`] — a mapping already exists for
///   `(chain, foreign_address)`.
pub fn register_foreign_asset_mapping(
    env: &Env,
    stellar_asset: Address,
    chain: String,
    foreign_address: BytesN<32>,
    decimals: u32,
) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::input_validation::validate_string(
        env,
        "register_foreign_asset_mapping",
        "chain",
        &chain,
    );
    check_registered_asset(env, &stellar_asset);

    let key = mapping_key(&chain, &foreign_address);
    if env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::ForeignAssetAlreadyMapped);
    }

    let mapping = ForeignAssetMapping {
        stellar_asset: stellar_asset.clone(),
        chain: chain.clone(),
        foreign_address: foreign_address.clone(),
        decimals,
        enabled: true,
    };
    env.storage().persistent().set(&key, &mapping);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    append_index(
        env,
        DataKey::ForeignAssetRegistryList,
        (chain.clone(), foreign_address.clone()),
    );
    append_index(
        env,
        DataKey::AssetForeignMappings(stellar_asset.clone()),
        (chain.clone(), foreign_address.clone()),
    );

    ForeignAssetMappedEvent {
        stellar_asset,
        chain,
        foreign_address,
        decimals,
    }
 

/* … truncated 3652 chars — edit only what you need near the top … */
