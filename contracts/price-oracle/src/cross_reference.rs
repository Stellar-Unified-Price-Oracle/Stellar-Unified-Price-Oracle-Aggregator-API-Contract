use soroban_sdk::{panic_with_error, Address, Env, IntoVal, Map, Symbol, Val, Vec};

use crate::events::CrossRefDeviationEvent;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregatePrice, CrossReferenceResult, DataKey, ErrorCode, ReferenceOracleEntry};

const DEFAULT_CROSS_REF_DEVIATION_BPS: u32 = 500; // 5%

/// Registers an external oracle contract as a cross-reference source.
///
/// The `asset_mapping` maps our asset `Address` values to the corresponding asset
/// `Address` values used by the reference oracle contract. On each
/// `get_cross_reference` call the contract will invoke `lastprice` on this oracle.
pub fn add_reference_oracle(
    env: &Env,
    contract_id: Address,
    asset_mapping: Map<Address, Address>,
) {
    let admin = get_admin(env);
    admin.require_auth();

    let entry = ReferenceOracleEntry {
        contract_id: contract_id.clone(),
        asset_mapping,
    };
    let entry_key = DataKey::ReferenceOracle(contract_id.clone());
    env.storage().persistent().set(&entry_key, &entry);
    env.storage()
        .persistent()
        .extend_ttl(&entry_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let list_key = DataKey::ReferenceOracleList;
    let mut list: Vec<Address> = env
        .storage()
        .persistent()
        .get(&list_key)
        .unwrap_or(Vec::new(env));
    if !list.contains(&contract_id) {
        list.push_back(contract_id);
        env.storage().persistent().set(&list_key, &list);
        env.storage()
            .persistent()
            .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

/// Removes a previously registered reference oracle.
pub fn remove_reference_oracle(env: &Env, contract_id: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .remove(&DataKey::ReferenceOracle(contract_id.clone()));

    let list_key = DataKey::ReferenceOracleList;
    let list: Vec<Address> = env
        .storage()
        .persistent()
        .get(&list_key)
        .unwrap_or(Vec::new(env));
    let mut new_list: Vec<Address> = Vec::new(env);
    for i in 0..list.len() {
        let addr = list.get_unchecked(i);
        if addr != contract_id {
            new_list.push_back(addr);
        }
    }
    env.storage().persistent().set(&list_key, &new_list);
    env.storage()
        .persistent()
        .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns the list of all registered reference oracle contract addresses.
pub fn get_reference_oracles(env: &Env) -> Vec<Address> {
    let key = DataKey::ReferenceOracleList;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

/// Compares our current aggregated price for `asset` against the first registered
/// reference oracle that has a mapping for this asset.
///
/// The reference oracle must expose a `lastprice(asset: Address) -> i128` function
/// that returns `0` when no price is available.
///
/// If the deviation exceeds the configured threshold a [`CrossRefDeviationEvent`] is
/// emitted. Returns `None` when no aggregate price exists locally or no reference
/// oracle has a mapping for this asset.
pub fn get_cross_reference(env: &Env, asset: Address) -> Option<CrossReferenceResult> {
    let aggregate: Option<AggregatePrice> = env
        .storage()
        .persistent()
        .get(&DataKey::Aggregate(asset.clone()));
    let our_price = aggregate?.price;

    let list_key = DataKey::ReferenceOracleList;
    let list: Vec<Address> = env
        .storage()
        .persistent()
        .get(&list_key)
        .unwrap_or(Vec::new(env));

    for i in 0..list.len() {
        let oracle_addr = list.get_unchecked(i);
        let entry_opt: Option<ReferenceOracleEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ReferenceOracle(oracle_addr));
        if let Some(entry) = entry_opt {
            if let Some(mapped_asset) = entry.asset_mapping.get(asset.clone()) {
                let func = Symbol::new(env, "lastprice");
                let mut args: Vec<Val> = Vec::new(env);
                args.push_back(mapped_asset.into_val(env));

                let ref_price: i128 =
                    env.invoke_contract(&entry.contract_id, &func, args);

                if ref_price > 0 {
                    let deviation_bps = compute_deviation_bps(our_price, ref_price);
                    let threshold_bps = get_cross_ref_deviation_threshold(env);

                    if deviation_bps > threshold_bps {
                        CrossRefDeviationEvent {
                            asset: asset.clone(),
                            ref_contract: entry.contract_id.clone(),
                            our_price,
                            ref_price,
                            deviation_bps,
                            threshold_bps,
                        }
                        .publish(env);
                    }

                    return Some(CrossReferenceResult {
                        our_price,
                        ref_price,
                        deviation_bps,
                        ref_contract: entry.contract_id,
                    });
                }
            }
        }
    }

    None
}

/// Sets the deviation threshold (in basis points) above which a [`CrossRefDeviationEvent`]
/// is emitted during a cross-reference check. Must be in the range `[0, 100_000]`.
pub fn set_cross_ref_deviation_threshold(env: &Env, threshold_bps: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if threshold_bps > 100_000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::CrossRefDeviationThreshold;
    env.storage().persistent().set(&key, &threshold_bps);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns the current cross-reference deviation threshold in basis points.
/// Defaults to `500` (5 %).
pub fn get_cross_ref_deviation_threshold(env: &Env) -> u32 {
    let key = DataKey::CrossRefDeviationThreshold;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_CROSS_REF_DEVIATION_BPS)
}

/// Computes the absolute price deviation between two prices as basis points.
///
/// Returns `(|our_price - ref_price| * 10_000) / ref_price`, saturated to `u32::MAX`.
fn compute_deviation_bps(our_price: i128, ref_price: i128) -> u32 {
    if ref_price == 0 || our_price == 0 {
        return 0;
    }
    let diff = if our_price > ref_price {
        our_price - ref_price
    } else {
        ref_price - our_price
    };
    let numerator = diff.saturating_mul(10_000);
    let deviation = numerator / ref_price.abs();
    deviation.min(u32::MAX as i128) as u32
}
