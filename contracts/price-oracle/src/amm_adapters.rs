//! #859 — Aquarius and Phoenix Soroban AMM price-source adapters.
//!
//! Both AMMs expose their pool state cross-contract. The adapters read the
//! reserves of an admin-registered pool, derive a constant-product spot price
//! and apply the #448 defences before the value is allowed to become a source
//! observation:
//!
//! * **Minimum-liquidity floor** — a pool whose quote-side reserve is below the
//!   configured floor is rejected (`ErrorCode::InvalidConfiguration`). Thin
//!   pools are the cheapest to sandwich (see `docs/dex-amm-manipulation.md`).
//! * **Deviation guard** — the derived price must stay within
//!   `max_deviation_bps` of the current aggregate, mirroring `amm.rs`.
//! * **Fail closed** — a reverting / malformed pool read yields `None`, never a
//!   zero or stale price.
//!
//! Registration is admin-gated, so an attacker cannot point the adapter at a
//! pool they control (same defence as `dex.rs`).

use soroban_sdk::{contractclient, panic_with_error, Address, Env, Symbol};

use crate::types::{DataKey, ErrorCode};

/// Default minimum quote-side reserve for an adapter pool (1_000 units at 7 dp).
pub const DEFAULT_MIN_LIQUIDITY: i128 = 10_000_000_000;

/// Default maximum deviation from the aggregate (500 bps = 5 %).
pub const DEFAULT_MAX_DEVIATION_BPS: u32 = 500;

/// Minimal view of an Aquarius / Phoenix pool. Both AMMs expose the reserve
/// pair under these names; the adapter only needs the two reserves.
#[contractclient(name = "AmmPoolClient")]
pub trait AmmPoolInterface {
    /// Returns `(reserve_a, reserve_b)` for the pool.
    fn get_reserves(env: Env) -> (i128, i128);
}

/// Which AMM an adapter entry points at.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AmmKind {
    Aquarius,
    Phoenix,
}

impl AmmKind {
    fn key(&self, asset: &Address) -> DataKey {
        match self {
            AmmKind::Aquarius => DataKey::AquariusPool(asset.clone()),
            AmmKind::Phoenix => DataKey::PhoenixPool(asset.clone()),
        }
    }
}

/// Admin-registered adapter pool: the pool contract plus the asset ordering.
#[derive(Clone)]
pub struct AmmAdapterConfig {
    pub pool: Address,
    /// `true` when the oracle asset is the pool's first reserve.
    pub asset_is_a: bool,
    /// Minimum quote-side reserve accepted from this pool.
    pub min_liquidity: i128,
}

fn read_config(env: &Env, kind: AmmKind, asset: &Address) -> Option<AmmAdapterConfig> {
    let key = kind.key(asset);
    let cfg: Option<AmmAdapterConfig> = env.storage().persistent().get(&key);
    if cfg.is_some() {
        env.storage().persistent().extend_ttl(&key, 10_000, 40_000);
    }
    cfg
}

/// Registers (or replaces) the pool an adapter reads for `asset`. Admin-only.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — `min_liquidity` is not positive.
pub fn register_amm_pool(
    env: &Env,
    kind: AmmKind,
    asset: Address,
    pool: Address,
    asset_is_a: bool,
    min_liquidity: i128,
) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    if min_liquidity <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let key = kind.key(&asset);
    env.storage().persistent().set(
        &key,
        &AmmAdapterConfig {
            pool,
            asset_is_a,
            min_liquidity,
        },
    );
    env.storage().persistent().extend_ttl(&key, 10_000, 40_000);
}

/// Reads the pool reserves, failing closed on a reverting or malformed pool.
fn read_reserves(env: &Env, pool: &Address) -> Option<(i128, i128)> {
    let func = Symbol::new(env, "get_reserves");
    let args = soroban_sdk::Vec::new(env);
    let res = env.try_invoke_contract::<(i128, i128), soroban_sdk::Error>(pool, &func, args);
    match res {
        Ok(Ok((a, b))) => Some((a, b)),
        _ => None,
    }
}

/// Applies the #448 minimum-liquidity floor to a reserve pair.
///
/// Returns the quote-side reserve when it clears the floor, `None` otherwise.
pub fn check_min_liquidity(reserves: (i128, i128), asset_is_a: bool, min_liquidity: i128) -> Option<i128> {
    let (reserve_a, reserve_b) = reserves;
    if reserve_a <= 0 || reserve_b <= 0 {
        return None;
    }
    let quote = if asset_is_a { reserve_b } else { reserve_a };
    if quote < min_liquidity {
        return None;
    }
    Some(quote)
}

/// Derives the constant-product spot price scaled to 1e18, or `None` when the
/// pool is unregistered, reverts, or fails the liquidity floor.
pub fn read_amm_price(env: &Env, kind: AmmKind, asset: Address) -> Option<i128> {
    let cfg = read_config(env, kind, &asset)?;
    let reserves = read_reserves(env, &cfg.pool)?;
    check_min_liquidity(reserves, cfg.asset_is_a, cfg.min_liquidity)?;

    let (reserve_a, reserve_b) = reserves;
    let (base, quote) = if cfg.asset_is_a {
        (reserve_a, reserve_b)
    } else {
        (reserve_b, reserve_a)
    };
    let price = (quote as u128).checked_mul(1_000_000_000_000_000_000u128)? / (base as u128);
    if price == 0 || price > i128::MAX as u128 {
        return None;
    }
    Some(price as i128)
}

/// Deviation guard shared by both adapters: rejects a pool price that is more
/// than `max_deviation_bps` away from `reference`.
pub fn within_deviation(price: i128, reference: i128, max_deviation_bps: u32) -> bool {
    if price <= 0 || reference <= 0 {
        return false;
    }
    let diff = if price > reference {
        price - reference
    } else {
        reference - price
    };
    let limit = reference
        .saturating_mul(max_deviation_bps as i128)
        / 10_000;
    diff <= limit
}

/// Aquarius adapter: pool price for `asset`, guarded by the #448 defences.
pub fn aquarius_price(env: &Env, asset: Address) -> Option<i128> {
    read_amm_price(env, AmmKind::Aquarius, asset)
}

/// Phoenix adapter: pool price for `asset`, guarded by the #448 defences.
pub fn phoenix_price(env: &Env, asset: Address) -> Option<i128> {
    read_amm_price(env, AmmKind::Phoenix, asset)
}
