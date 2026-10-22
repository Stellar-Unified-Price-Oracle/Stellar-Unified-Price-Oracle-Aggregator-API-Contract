//! # Data provenance for published aggregates (#493)
//!
//! A published price was an assertion: nothing on chain said *why* it was
//! that value. This module records, for every aggregate, the exact
//! submissions, sources, weights and reference value that produced it, so a
//! consumer can prove a price rather than trust it — the basis for dispute
//! resolution and audit.
//!
//! ## Record
//!
//! One [`ProvenanceRecord`] per published aggregate, keyed by
//! `(asset, ledger)` and carrying:
//!
//! * every contributing submission as a [`ProvenanceEntry`] — source, price,
//!   weight in bps, and the ledger the submission was made in;
//! * `reference`, the unweighted median of the counted prices, i.e. the
//!   value a weighted aggregate was checked against;
//! * the aggregation `method` in force, so a method change cannot silently
//!   re-interpret an old record;
//! * the publication `deferral_ledgers` of each contributor, matching the
//!   latency analytics of #492.
//!
//! ## Integrity
//!
//! `hash` is `SHA-256(domain_sep ‖ previous_hash ‖ fields…)` over every
//! field of the record, and `previous_hash` is the `hash` of the previous
//! record *of the same asset*. The records therefore form a per-asset hash
//! chain: a record cannot be edited, replaced or back-dated without
//! breaking [`verify_record`], because the successor already commits to
//! this record's hash. `id` is the hash itself, so the identifier emitted
//! in [`ProvenanceRecordedEvent`] is a commitment, not an index.
//!
//! ## Corrections and source removal
//!
//! Provenance is written **per publication**, not per price, so a
//! correction produces a new record at a new ledger with its own
//! contributors, chained to its predecessor. Earlier records are immutable
//! and remain retrievable for as long as their history survives, so
//! "what did we publish, and why" stays answerable across a correction.
//! Removing a source does not rewrite history: past records still name it,
//! with the weight it held at the time.
//!
//! ## Pruning
//!
//! Records live at `DataKey::Provenance(asset, ledger)` and are removed by
//! the same history-pruning loop that drops the aggregate's
//! `PriceHistory` entry for that ledger, so provenance can never outlive
//! the price it explains and cannot leak. See `docs/provenance.md`.

use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

use crate::events::ProvenanceRecordedEvent;
use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ProvenanceEntry, ProvenanceHead, ProvenanceRecord};

/// Domain separator, so a provenance hash can never collide with a hash
/// produced by another module over the same field bytes.
const DOMAIN: &[u8] = b"SUPRO1";

/// The zero hash used as `previous_hash` of an asset's first record.
fn env_zero_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

fn head_key(asset: &Address) -> DataKey {
    DataKey::ProvenanceHead(asset.clone())
}

/// The chain head of an asset's provenance, or `None` when it has none.
///
/// The hash and its ledger share one entry: a round that writes provenance
/// would otherwise pay two ledger entries for a single logical write, which
/// is enough on its own to push a wide batch over the network footprint cap.
fn head(env: &Env, asset: &Address) -> Option<ProvenanceHead> {
    let k = head_key(asset);
    let v: Option<ProvenanceHead> = env.storage().persistent().get(&k);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&k, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Computes the commitment over a record's fields and `previous_hash`.
///
/// Every field is XDR-serialized, so no two distinct records can produce the
/// same preimage.
fn commitment(
    env: &Env,
    previous_hash: &BytesN<32>,
    ledger: u32,
    price: i128,
    timestamp: u64,
    reference: i128,
    num_sources: u32,
    method: u32,
    contributors: &Vec<ProvenanceEntry>,
    deferrals: &Vec<u32>,
) -> BytesN<32> {
    use soroban_sdk::xdr::ToXdr;
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, DOMAIN));
    buf.append(&Bytes::from_slice(env, &previous_hash.to_array()));
    buf.append(&ledger.to_xdr(env));
    buf.append(&price.to_xdr(env));
    buf.append(&timestamp.to_xdr(env));
    buf.append(&reference.to_xdr(env));
    buf.append(&num_sources.to_xdr(env));
    buf.append(&method.to_xdr(env));
    for (i, c) in contributors.iter().enumerate() {
        buf.append(&c.source.to_xdr(env));
        buf.append(&c.price.to_xdr(env));
        buf.append(&c.weight_bps.to_xdr(env));
        buf.append(&c.submission_ledger.to_xdr(env));
        buf.append(&deferrals.get(i as u32).unwrap_or(0).to_xdr(env));
    }
    env.crypto().sha256(&buf).into()
}

/// Builds, stores and announces the provenance record of a published
/// aggregate. Called from `aggregate_asset` once the contributors and the
/// final price are known.
#[allow(clippy::too_many_arguments)]
pub fn record(
    env: &Env,
    asset: &Address,
    ledger: u32,
    price: i128,
    timestamp: u64,
    reference: i128,
    method: u32,
    contributors: Vec<ProvenanceEntry>,
    deferrals: Vec<u32>,
) {
    let num_sources = contributors.len();
    // Aggregation can run more than once inside the same ledger, so the chain
    // anchor is the head of the *previous* ledger: a re-publication replaces
    // the record at this ledger and must not chain to the copy it replaces,
    // which would make the replaced copy unverifiable and let a rewrite slip
    // in under a fresh-looking successor.
    let head_ledger = head(env, asset).map(|h| h.ledger).unwrap_or(0);
    let previous_hash = if head_ledger < ledger {
        head(env, asset)
            .map(|h| h.hash)
            .unwrap_or_else(|| env_zero_hash(env))
    } else {
        head_before(env, asset, ledger).0
    };
    let hash = commitment(
        env,
        &previous_hash,
        ledger,
        price,
        timestamp,
        reference,
        num_sources,
        method,
        &contributors,
        &deferrals,
    );
    let record = ProvenanceRecord {
        id: hash.clone(),
        asset: asset.clone(),
        ledger,
        price,
        timestamp,
        reference,
        num_sources,
        method,
        contributors,
        deferral_ledgers: deferrals,
        previous_hash,
        hash: hash.clone(),
    };

    let k = DataKey::Provenance(asset.clone(), ledger);
    env.storage().persistent().set(&k, &record);
    env.storage()
        .persistent()
        .extend_ttl(&k, LEDGER_THRESHOLD, LEDGER_BUMP);

    let hk = head_key(asset);
    let hd = ProvenanceHead {
        hash: hash.clone(),
        ledger,
    };
    env.storage().persistent().set(&hk, &hd);
    env.storage()
        .persistent()
        .extend_ttl(&hk, LEDGER_THRESHOLD, LEDGER_BUMP);

    ProvenanceRecordedEvent {
        asset: asset.clone(),
        ledger,
        price,
        provenance_id: hash,
        num_sources,
    }
    .publish(env);
}

/// Full provenance of the aggregate published for `asset` at `ledger`.
pub fn get_record(env: &Env, asset: &Address, ledger: u32) -> Option<ProvenanceRecord> {
    let k = DataKey::Provenance(asset.clone(), ledger);
    let v: Option<ProvenanceRecord> = env.storage().persistent().get(&k);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&k, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Whether `record`'s commitment still matches its contents.
///
/// Catches any field edited after the fact. It does **not** by itself prove
/// the record is the one that was published — that needs the chain check in
/// [`verify_link`], since a forged record can be internally consistent.
pub fn verify_record(env: &Env, record: &ProvenanceRecord) -> bool {
    // Both digests are public on-chain data (anyone can read the record), so the
    // comparison is not secret-dependent; it is still done in constant shape so
    // that no call site in the contract becomes the one place where a digest
    // compare leaks through the metered instruction count (#507).
    let expected = commitment(
        env,
        &record.previous_hash,
        record.ledger,
        record.price,
        record.timestamp,
        record.reference,
        record.num_sources,
        record.method,
        &record.contributors,
        &record.deferral_ledgers,
    );
    crate::constant_time::digest_eq(&expected, &record.hash)
        && crate::constant_time::digest_eq(&record.id, &record.hash)
}

/// The head hash as of just before `ledger`: the hash of the newest record
/// with a smaller ledger that is still retained, or the zero hash when the
/// asset has no such record.
///
/// `found` distinguishes "the chain is truncated by pruning" from "this is
/// the asset's first record", which verification must tell apart. Read from
/// the history ledger index so the walk stays proportional to the retained
/// history, which the same pruning policy bounds.
fn head_before(env: &Env, asset: &Address, ledger: u32) -> (BytesN<32>, bool) {
    let history: Vec<u32> = env
        .storage()
        .persistent()
        .get(&DataKey::PriceHistoryLedgers(asset.clone()))
        .unwrap_or_else(|| Vec::new(env));
    for i in 0..history.len() {
        let l = history.get_unchecked(i);
        if l >= ledger {
            break;
        }
        if let Some(prev) = get_record(env, asset, l) {
            return (prev.hash, true);
        }
    }
    (env_zero_hash(env), false)
}

/// Whether the record at `ledger` is intact *and* correctly chained: its own
/// commitment still matches its contents, and it commits to the hash its
/// predecessor of the same asset carries. A record that was edited, replaced
/// or back-dated fails one of the two checks.
///
/// A record whose predecessor has been **pruned** cannot be chain-checked —
/// the link it commits to no longer exists on chain. That is truncation, not
/// tampering, so only the commitment is required to hold; `verify_record`
/// alone still detects an edit. Pruning is what bounds provenance storage, so
/// this is the normal state of the oldest retained record.
pub fn verify_link(env: &Env, asset: &Address, ledger: u32) -> bool {
    match get_record(env, asset, ledger) {
        Some(r) => {
            if !verify_record(env, &r) {
                return false;
            }
            let (expected, found) = head_before(env, asset, ledger);
            if found {
                // The predecessor is still on chain, so the link must match.
                r.previous_hash == expected
            } else {
                // No predecessor is retained. Either this is the asset's first
                // record, or the chain was truncated by pruning; both leave
                // nothing to check beyond the commitment itself.
                true
            }
        }
        None => false,
    }
}

/// Drops the provenance record of a pruned ledger. Called from the history
/// pruning loop so the two never diverge.
///
/// When the pruned ledger is the chain head, the head pointer is rewound to
/// the newest surviving record, so verification of that record still resolves
/// its predecessor instead of dangling.
pub fn prune(env: &Env, asset: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .remove(&DataKey::Provenance(asset.clone(), ledger));
    if head(env, asset).map(|h| h.ledger) == Some(ledger) {
        let (prev, has_prev) = head_before(env, asset, ledger);
        let hk = head_key(asset);
        if has_prev {
            let hd = ProvenanceHead {
                hash: prev,
                ledger: newest_record_ledger_before(env, asset, ledger),
            };
            env.storage().persistent().set(&hk, &hd);
            env.storage()
                .persistent()
                .extend_ttl(&hk, LEDGER_THRESHOLD, LEDGER_BUMP);
        } else {
            env.storage().persistent().remove(&hk);
        }
    }
}

/// Ledger of the newest surviving record strictly older than `ledger`.
fn newest_record_ledger_before(env: &Env, asset: &Address, ledger: u32) -> u32 {
    let history: Vec<u32> = env
        .storage()
        .persistent()
        .get(&DataKey::PriceHistoryLedgers(asset.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let mut newest = 0u32;
    for i in 0..history.len() {
        let l = history.get_unchecked(i);
        if l < ledger && l > newest {
            newest = l;
        }
    }
    newest
}
