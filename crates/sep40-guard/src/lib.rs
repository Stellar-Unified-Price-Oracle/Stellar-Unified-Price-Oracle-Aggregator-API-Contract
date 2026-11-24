#![no_std]
//! `sep40-guard` — a small, dependency-light helper for contracts that consume
//! [SEP-40](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0040.md)
//! price oracles.
//!
//! [`read_with_fallback`] reads a price from a *primary* oracle and falls back to a
//! *secondary* oracle when the primary is stale, paused, or returns no data. The
//! result is rejected when the two feeds diverge by more than a caller-supplied
//! deviation limit, so a consumer never silently accepts a price that the two
//! oracles disagree on.
//!
//! The crate is `no_std` and only depends on `soroban-sdk`, so it can be used from
//! any Soroban contract.

use soroban_sdk::{contractclient, contracterror, contracttype, panic_with_error, Address, Env};

/// SEP-40 `Asset` type, mirrored here so consumers do not need to depend on a
/// specific oracle implementation crate.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum Asset {
    /// A Stellar token identified by its contract address.
    Stellar(Address),
    /// A non-Stellar asset identified by a short symbol (e.g. `"USD"`, `"BTC"`).
    Other(soroban_sdk::Symbol),
}

/// SEP-40 `PriceData` returned by `lastprice` / `price`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceData {
    /// Price scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp (seconds) of the observation.
    pub timestamp: u64,
    /// Ledger sequence of the last update.
    pub last_updated: u32,
}

/// The subset of the SEP-40 oracle interface used by this crate.
#[contractclient(name = "Sep40Client")]
pub trait Sep40Oracle {
    /// Latest price for `asset`, or `None` when the oracle has no data.
    fn lastprice(env: Env, asset: Asset) -> Option<PriceData>;
    /// Decimal precision of the prices returned by this oracle.
    fn decimals(env: Env) -> u32;
}

/// Errors returned by [`read_with_fallback`].
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GuardError {
    /// Neither oracle returned a usable price.
    NoPrice = 1,
    /// The primary price is older than `max_age` and the secondary is unusable.
    StalePrice = 2,
    /// The primary and secondary prices diverge beyond `max_deviation_bps`.
    DeviationExceeded = 3,
    /// The two oracles report different decimal precision.
    DecimalsMismatch = 4,
    /// A returned price was zero or negative.
    InvalidPrice = 5,
}

/// Which oracle produced the returned price.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum PriceSource {
    /// The primary oracle answered and passed every guard.
    Primary = 0,
    /// The primary was stale/paused/empty and the secondary was used.
    Secondary = 1,
}

/// Result of a guarded read.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct GuardedPrice {
    /// The accepted price data.
    pub data: PriceData,
    /// Which oracle the price came from.
    pub source: PriceSource,
}

/// Read a price from `primary`, falling back to `secondary`.
///
/// # Behaviour
///
/// 1. The primary is queried. Its price is accepted when it is present, positive,
///    and no older than `max_age` seconds relative to the current ledger timestamp.
///    A primary that panics (e.g. because the oracle is paused) is treated as
///    unusable and the secondary is consulted instead.
/// 2. When the primary is unusable, the secondary is queried under the same
///    freshness and positivity rules.
/// 3. When both oracles answer, their prices must agree within
///    `max_deviation_bps` basis points of the primary price, and both must report
///    the same `decimals`. Otherwise the read is rejected.
///
/// # Arguments
///
/// * `primary` — address of the primary SEP-40 oracle.
/// * `secondary` — address of the secondary SEP-40 oracle.
/// * `asset` — asset to price.
/// * `max_age` — maximum accepted age, in seconds, of a price observation.
/// * `max_deviation_bps` — maximum allowed divergence between the two feeds, in
///   basis points (1 bps = 0.01 %). `0` requires an exact match.
///
/// # Errors
///
/// Panics with [`GuardError`] when no acceptable price can be produced.
pub fn read_with_fallback(
    env: &Env,
    primary: &Address,
    secondary: &Address,
    asset: &Asset,
    max_age: u64,
    max_deviation_bps: u32,
) -> GuardedPrice {
    let now = env.ledger().timestamp();
    let primary_data = read_fresh(env, primary, asset, now, max_age);
    let secondary_data = read_fresh(env, secondary, asset, now, max_age);

    match (primary_data, secondary_data) {
        (Some(p), Some(s)) => {
            let p_decimals = Sep40Client::new(env, primary).decimals();
            let s_decimals = Sep40Client::new(env, secondary).decimals();
            if p_decimals != s_decimals {
                panic_with_error!(env, GuardError::DecimalsMismatch);
            }
            if !within_deviation(p.price, s.price, max_deviation_bps) {
                panic_with_error!(env, GuardError::DeviationExceeded);
            }
            GuardedPrice {
                data: p,
                source: PriceSource::Primary,
            }
        }
        (Some(p), None) => GuardedPrice {
            data: p,
            source: PriceSource::Primary,
        },
        (None, Some(s)) => GuardedPrice {
            data: s,
            source: PriceSource::Secondary,
        },
        (None, None) => panic_with_error!(env, GuardError::NoPrice),
    }
}

/// Read a single oracle, returning `None` when it is empty, stale, non-positive,
/// or panics (paused / misconfigured).
fn read_fresh(
    env: &Env,
    oracle: &Address,
    asset: &Asset,
    now: u64,
    max_age: u64,
) -> Option<PriceData> {
    let data = Sep40Client::new(env, oracle).try_lastprice(asset).ok()??;
    if data.price <= 0 {
        return None;
    }
    if now.saturating_sub(data.timestamp) > max_age {
        return None;
    }
    Some(data)
}

/// Returns `true` when `candidate` is within `max_deviation_bps` of `reference`.
///
/// Uses saturating arithmetic so extreme values cannot overflow.
pub fn within_deviation(reference: i128, candidate: i128, max_deviation_bps: u32) -> bool {
    if reference <= 0 || candidate <= 0 {
        return false;
    }
    let diff = reference.abs_diff(candidate);
    let limit = reference
        .saturating_mul(max_deviation_bps as i128)
        .saturating_div(10_000);
    diff <= limit
}
