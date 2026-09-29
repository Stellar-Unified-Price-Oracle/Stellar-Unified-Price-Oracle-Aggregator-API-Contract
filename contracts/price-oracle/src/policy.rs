//! # Per-asset aggregation policy engine
//!
//! Each asset resolves to exactly one [`EffectivePolicy`]. Every field is
//! resolved independently with the precedence **asset override → asset class →
//! global default**, and the layer that supplied it is reported alongside.
//!
//! Bounds are validated on write, so a stored policy is always valid; an
//! invalid write panics with `InvalidConfiguration` and never falls back to a
//! laxer layer. Policies are read at aggregation time only, so a change never
//! reinterprets an aggregate that was already computed.

use soroban_sdk::{contractevent, panic_with_error, Address, Env};

use crate::admin::{get_aggregation_method, get_min_sources_required, MAX_MIN_SOURCES};
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, EffectivePolicy, ErrorCode, PolicyOverride};

pub const MAX_METHOD: u32 = 4;
pub const MAX_FRESHNESS_SECS: u64 = 604_800;
pub const MAX_DEVIATION_BPS: u32 = 10_000;

pub const LAYER_GLOBAL: u32 = 0;
pub const LAYER_CLASS: u32 = 1;
pub const LAYER_ASSET: u32 = 2;

/// Emitted on every policy or class-assignment change.
///
/// `scope` is 1 for a class policy, 2 for an asset policy and 3 for an asset
/// class assignment (then `old_class`/`new_class` carry the change).
#[contractevent]
#[derive(Clone)]
pub struct AggregationPolicyChangedEvent {
    #[topic]
    pub scope: u32,
    pub asset: Option<Address>,
    pub class: u32,
    pub old: Option<PolicyOverride>,
    pub new: Option<PolicyOverride>,
    pub old_class: Option<u32>,
    pub new_class: Option<u32>,
}

fn validate(env: &Env, p: &PolicyOverride) {
    let ok = p.method.is_none_or(|m| m <= MAX_METHOD)
        && p.min_sources
            .is_none_or(|q| (1..=MAX_MIN_SOURCES).contains(&q))
        && p.freshness_secs
            .is_none_or(|f| (1..=MAX_FRESHNESS_SECS).contains(&f))
        && p.max_deviation_bps
            .is_none_or(|d| (1..=MAX_DEVIATION_BPS).contains(&d));
    if !ok {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
}

fn read(env: &Env, key: &DataKey) -> Option<PolicyOverride> {
    let v = env.storage().persistent().get(key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

fn write(env: &Env, key: &DataKey, value: &Option<PolicyOverride>) {
    match value {
        Some(p) => {
            env.storage().persistent().set(key, p);
            env.storage()
                .persistent()
                .extend_ttl(key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        None => env.storage().persistent().remove(key),
    }
}

/// Returns the class an asset belongs to, if one was assigned.
pub fn get_asset_class(env: &Env, asset: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::AssetClassId(asset.clone()))
}

/// Resolves the effective policy for `asset`.
pub fn effective_policy(env: &Env, asset: &Address) -> EffectivePolicy {
    let asset_p = read(env, &DataKey::AssetPolicy(asset.clone()));
    let class_p = get_asset_class(env, asset).and_then(|c| read(env, &DataKey::ClassPolicy(c)));

    let mut out = EffectivePolicy {
        method: get_aggregation_method(env),
        min_sources: get_min_sources_required(env),
        freshness_secs: 0,
        max_deviation_bps: 0,
        method_layer: LAYER_GLOBAL,
        min_sources_layer: LAYER_GLOBAL,
        freshness_layer: LAYER_GLOBAL,
        max_deviation_layer: LAYER_GLOBAL,
    };
    for (layer, p) in [(LAYER_CLASS, class_p), (LAYER_ASSET, asset_p)] {
        if let Some(p) = p {
            if let Some(v) = p.method {
                out.method = v;
                out.method_layer = layer;
            }
            if let Some(v) = p.min_sources {
                out.min_sources = v;
                out.min_sources_layer = layer;
            }
            if let Some(v) = p.freshness_secs {
                out.freshness_secs = v;
                out.freshness_layer = layer;
            }
            if let Some(v) = p.max_deviation_bps {
                out.max_deviation_bps = v;
                out.max_deviation_layer = layer;
            }
        }
    }
    out
}

/// Sets (or, with `None`, clears) the policy override of one asset. Admin only.
pub fn set_asset_policy(env: &Env, asset: Address, policy: Option<PolicyOverride>) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);
    if let Some(p) = &policy {
        validate(env, p);
    }
    let key = DataKey::AssetPolicy(asset.clone());
    let old = read(env, &key);
    write(env, &key, &policy);
    AggregationPolicyChangedEvent {
        scope: 2,
        asset: Some(asset),
        class: 0,
        old,
        new: policy,
        old_class: None,
        new_class: None,
    }
    .publish(env);
}

/// Sets (or, with `None`, clears) the policy override of an asset class. Admin only.
pub fn set_class_policy(env: &Env, class: u32, policy: Option<PolicyOverride>) {
    let admin = get_admin(env);
    admin.require_auth();
    if let Some(p) = &policy {
        validate(env, p);
    }
    let key = DataKey::ClassPolicy(class);
    let old = read(env, &key);
    write(env, &key, &policy);
    AggregationPolicyChangedEvent {
        scope: 1,
        asset: None,
        class,
        old,
        new: policy,
        old_class: None,
        new_class: None,
    }
    .publish(env);
}

/// Assigns an asset to a class (or, with `None`, removes the assignment). Admin only.
pub fn set_asset_class(env: &Env, asset: Address, class: Option<u32>) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);
    let old_class = get_asset_class(env, &asset);
    let key = DataKey::AssetClassId(asset.clone());
    match class {
        Some(c) => env.storage().persistent().set(&key, &c),
        None => env.storage().persistent().remove(&key),
    }
    AggregationPolicyChangedEvent {
        scope: 3,
        asset: Some(asset),
        class: 0,
        old: None,
        new: None,
        old_class,
        new_class: class,
    }
    .publish(env);
}

/// Returns the raw override stored for an asset, if any.
pub fn get_asset_policy(env: &Env, asset: &Address) -> Option<PolicyOverride> {
    read(env, &DataKey::AssetPolicy(asset.clone()))
}

/// Marks the prices that deviate from the plain median by more than
/// `max_bps`.
///
/// Exposed separately from [`filter_deviation`] so callers that must keep
/// other per-submission data aligned (sources, submission ledgers,
/// provenance entries — see #491/#493) can apply the same rule through an
/// index mask instead of three parallel vectors.
pub fn deviation_mask(prices: &soroban_sdk::Vec<i128>, max_bps: u32) -> soroban_sdk::Vec<bool> {
    let mut mask = soroban_sdk::Vec::new(prices.env());
    if prices.is_empty() {
        return mask;
    }
    let median = crate::storage::compute_median(prices);
    let bound = median.abs().saturating_mul(max_bps as i128);
    for i in 0..prices.len() {
        let price = prices.get_unchecked(i);
        mask.push_back((price - median).abs().saturating_mul(10_000) <= bound);
    }
    mask
}

/// Drops entries deviating from the plain median by more than `max_bps`.
///
/// Fails closed: the caller re-checks quorum on what remains.
pub fn filter_deviation(
    env: &Env,
    prices: &soroban_sdk::Vec<i128>,
    volumes: &soroban_sdk::Vec<i128>,
    weights: &soroban_sdk::Vec<u32>,
    max_bps: u32,
) -> (
    soroban_sdk::Vec<i128>,
    soroban_sdk::Vec<i128>,
    soroban_sdk::Vec<u32>,
) {
    let mut p = soroban_sdk::Vec::new(env);
    let mut v = soroban_sdk::Vec::new(env);
    let mut w = soroban_sdk::Vec::new(env);
    if prices.is_empty() {
        return (p, v, w);
    }
    let mask = deviation_mask(prices, max_bps);
    for i in 0..prices.len() {
        if mask.get_unchecked(i) {
            p.push_back(prices.get_unchecked(i));
            v.push_back(volumes.get(i).unwrap_or(0));
            w.push_back(weights.get(i).unwrap_or(1));
        }
    }
    (p, v, w)
}
