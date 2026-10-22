//! #496 — Anomaly explanation reports for flagged submissions and aggregates
//!
//! A flag with no explanation is unactionable and erodes trust with sources.
//! Every flagging path in this contract therefore produces an
//! [`AnomalyExplanation`]: the rule that fired, the inputs it compared, and
//! the threshold it applied — machine-readable (stable numeric `rule_id` plus
//! `observed` / `reference` / `threshold` fields) and human-readable (`rule`
//! symbol).
//!
//! ## Rejections are explained by a view, not by a stored record
//!
//! A rejecting path ends in `panic_with_error!`, which reverts the whole
//! transaction — a record written just before the revert is rolled back with
//! it. Explaining rejections by writing state would therefore explain nothing.
//! So [`explain_submission`] evaluates the same rules, in the same order, on
//! the same inputs and returns the explanation the submission *would* get: a
//! pure, storage-free function the source calls after its transaction reverts.
//! It is stable by construction — the same `(rule, subject, asset, ledger,
//! observed, reference, threshold)` always yields the same explanation.
//!
//! Flags that do **not** revert (a correlation-band exclusion, an aggregate
//! served below quorum, a collapsed confidence band) are recorded in the
//! bounded ring and emitted as events, so they are visible without a second
//! call and remain reconstructible from the event stream after the ring rolls.
//!
//! ## Stability
//!
//! An explanation is a pure function of the rule inputs. Identical inputs
//! always produce an identical explanation, so an explanation recorded at flag
//! time can be re-derived and audited later.
//!
//! ## Retention is bounded
//!
//! Explanations live in a fixed-size ring ([`DEFAULT_RETENTION`], admin-tunable
//! up to [`MAX_RETENTION`]) per (asset, source) and per aggregate. When full,
//! the oldest entry is dropped. Retention is by count, not by age, so a burst
//! of flags cannot evict the recent record and an idle asset cannot grow.
//!
//! ## Information leakage
//!
//! An explanation reveals only what the flagger already knew: the flagged
//! party's own submitted price, the reference value the rule compared against,
//! and a threshold the flagger can already observe by probing. It contains no
//! keys, no admin state, no other sources' identities, and no information about
//! sources that were not flagged. See `docs/anomaly-explanations.md` §
//! "Information-leakage review".
//!
//! Every explanation is also emitted as an `AnomalyExplainedEvent`, so the
//! record is reconstructible from events alone even after the ring has rolled.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::AnomalyExplainedEvent;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AnomalyExplanation, AnomalyRule, DataKey, ErrorCode};

/// Explanations retained per log when unset.
pub const DEFAULT_RETENTION: u32 = 16;
/// Hard ceiling on the ring size, so admin configuration cannot grow a log
/// without bound.
pub const MAX_RETENTION: u32 = 64;

/// Current retention (ring size) for every explanation log.
pub fn get_retention(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AnomalyRetention)
        .unwrap_or(DEFAULT_RETENTION)
}

/// Sets the ring size for every explanation log. Admin-only.
///
/// Values above [`MAX_RETENTION`] are rejected so a configuration change can
/// never turn a bounded log into an unbounded one.
pub fn set_retention(env: &Env, retention: u32) {
    get_admin(env).require_auth();
    if retention == 0 || retention > MAX_RETENTION {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::AnomalyRetention;
    env.storage().persistent().set(&key, &retention);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Builds the explanation for a rule, without touching storage.
///
/// Pure: the same `(rule, subject, asset, ledger, observed, reference,
/// threshold, rejected)` always yields the same explanation.
pub fn build(
    rule: AnomalyRule,
    subject: &Address,
    asset: &Address,
    ledger: u32,
    observed: i128,
    reference: i128,
    threshold: i128,
    rejected: bool,
) -> AnomalyExplanation {
    AnomalyExplanation {
        rule_id: rule as u32,
        rule: rule.name(),
        subject: subject.clone(),
        asset: asset.clone(),
        ledger,
        observed,
        reference,
        threshold,
        rejected,
    }
}

/// Appends `explanation` to the bounded ring at `key`, dropping the oldest
/// entry when the ring is full. Emits `AnomalyExplainedEvent` unconditionally
/// so the record stays reconstructible from events after the ring has rolled.
fn push_bounded(env: &Env, key: &DataKey, explanation: &AnomalyExplanation) {
    let retention = get_retention(env);
    let mut log: Vec<AnomalyExplanation> = env
        .storage()
        .persistent()
        .get(key)
        .unwrap_or_else(|| Vec::new(env));
    // Ring semantics: always append, then drop from the front until it fits.
    log.push_back(explanation.clone());
    while log.len() > retention {
        log.remove(0);
    }
    env.storage().persistent().set(key, &log);
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_THRESHOLD, LEDGER_BUMP);

    AnomalyExplainedEvent {
        asset: explanation.asset.clone(),
        subject: explanation.subject.clone(),
        rule_id: explanation.rule_id,
        rule: explanation.rule.clone(),
        observed: explanation.observed,
        reference: explanation.reference,
        threshold: explanation.threshold,
        rejected: explanation.rejected,
    }
    .publish(env);
}

/// Records a submission-level flag (the source is the subject) and returns the
/// explanation that was stored.
#[allow(clippy::too_many_arguments)]
pub fn record_submission(
    env: &Env,
    rule: AnomalyRule,
    source: &Address,
    asset: &Address,
    observed: i128,
    reference: i128,
    threshold: i128,
) -> AnomalyExplanation {
    let explanation = build(
        rule,
        source,
        asset,
        env.ledger().sequence(),
        observed,
        reference,
        threshold,
        rule.is_rejecting(),
    );
    let key = DataKey::AnomalyLog(asset.clone(), source.clone());
    push_bounded(env, &key, &explanation);
    explanation
}

/// Records an aggregate-level flag (the asset is the subject).
pub fn record_aggregate(
    env: &Env,
    rule: AnomalyRule,
    asset: &Address,
    observed: i128,
    reference: i128,
    threshold: i128,
) -> AnomalyExplanation {
    let explanation = build(
        rule,
        asset,
        asset,
        env.ledger().sequence(),
        observed,
        reference,
        threshold,
        false,
    );
    let key = DataKey::AggregateAnomalyLog(asset.clone());
    push_bounded(env, &key, &explanation);
    explanation
}

/// Explanations addressed to `source` for `asset` (oldest first).
///
/// Read-only and permissionless: an explanation contains nothing the source
/// does not already know about its own submission.
pub fn get_for_source(env: &Env, asset: &Address, source: &Address) -> Vec<AnomalyExplanation> {
    env.storage()
        .persistent()
        .get(&DataKey::AnomalyLog(asset.clone(), source.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// Aggregate-level explanations for `asset` (oldest first) — the operator view.
pub fn get_for_asset(env: &Env, asset: &Address) -> Vec<AnomalyExplanation> {
    env.storage()
        .persistent()
        .get(&DataKey::AggregateAnomalyLog(asset.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// The most recent submission-level explanation for `(asset, source)`.
pub fn latest_for_source(
    env: &Env,
    asset: &Address,
    source: &Address,
) -> Option<AnomalyExplanation> {
    let log = get_for_source(env, asset, source);
    if log.is_empty() {
        None
    } else {
        Some(log.get_unchecked(log.len() - 1))
    }
}

/// Whether `explanation` was raised by `rule`. Used by the per-path tests to
/// assert that the *right* rule was reported, not merely that one was.
pub fn matches_rule(explanation: &AnomalyExplanation, rule: AnomalyRule) -> bool {
    explanation.rule_id == rule as u32 && explanation.rule == rule.name()
}

/// Explains what would happen to a submission, without changing any state.
///
/// Walks the rejecting rules in the same order `prices::submit_price` applies
/// them and returns the first one that would fire, or `None` when the
/// submission would be accepted. Pure: it reads configuration and the source's
/// own last submission, and writes nothing.
///
/// This is the explanation a source reads after its transaction reverted,
/// because a reverting path cannot leave a durable record behind.
pub fn explain_submission(
    env: &Env,
    source: &Address,
    asset: &Address,
    price: i128,
    timestamp: u64,
) -> Option<AnomalyExplanation> {
    let ledger = env.ledger().sequence();
    let ledger_time = env.ledger().timestamp();

    if price <= 0 {
        return Some(build(
            AnomalyRule::NonPositivePrice,
            source,
            asset,
            ledger,
            price,
            0,
            0,
            true,
        ));
    }

    let bounds = crate::assets::get_price_bounds(env, asset.clone());
    if price < bounds.min_price || price > bounds.max_price {
        return Some(build(
            AnomalyRule::PriceOutOfBounds,
            source,
            asset,
            ledger,
            price,
            bounds.min_price,
            bounds.max_price,
            true,
        ));
    }

    let threshold = crate::admin::get_timestamp_threshold(env);
    if timestamp > ledger_time.saturating_add(threshold) {
        return Some(build(
            AnomalyRule::FutureTimestamp,
            source,
            asset,
            ledger,
            timestamp as i128,
            ledger_time as i128,
            ledger_time.saturating_add(threshold) as i128,
            true,
        ));
    }

    // Out-of-order relative to this source's own last submission.
    let last: Option<crate::types::PriceEntry> = env
        .storage()
        .persistent()
        .get(&DataKey::Submission(asset.clone(), source.clone()));
    if let Some(prev) = last {
        if timestamp < prev.timestamp {
            return Some(build(
                AnomalyRule::StaleSubmission,
                source,
                asset,
                ledger,
                timestamp as i128,
                prev.timestamp as i128,
                prev.timestamp as i128,
                true,
            ));
        }
    }

    // Move-rate breach, evaluated against the stored aggregate.
    if bounds.max_change_bps_per_ledger > 0 {
        let prev: Option<crate::types::AggregatePrice> = env
            .storage()
            .persistent()
            .get(&DataKey::Aggregate(asset.clone()));
        if let Some(prev) = prev {
            if prev.price > 0 {
                let diff = if price > prev.price {
                    price - prev.price
                } else {
                    prev.price - price
                };
                let change_bps = diff.saturating_mul(10_000) / prev.price;
                if change_bps > bounds.max_change_bps_per_ledger as i128 {
                    return Some(build(
                        AnomalyRule::ChangeRateBreach,
                        source,
                        asset,
                        ledger,
                        price,
                        prev.price,
                        bounds.max_change_bps_per_ledger as i128,
                        true,
                    ));
                }
            }
        }
    }

    None
}
