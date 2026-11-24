//! Quadratic voting credit accounting and integer square-root weighting (#780).
use soroban_sdk::{panic_with_error, Address, Env};

use crate::{GovernanceError, GovernanceKey};

/// Integer floor square root (`floor(sqrt(n))`) without floating-point arithmetic.
pub fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x0 = n / 2;
    let mut x1 = (x0 + n / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + n / x0) / 2;
    }
    x0
}

pub fn record_credits(env: &Env, voter: Address, proposal_id: u64, credits: u64) -> u64 {
    voter.require_auth();
    if credits == 0 {
        panic_with_error!(env, GovernanceError::InvalidWeight);
    }
    let effective = isqrt(credits);
    if effective == 0 {
        panic_with_error!(env, GovernanceError::InvalidWeight);
    }
    env.storage().persistent().set(
        &GovernanceKey::QuadraticCredits(voter, proposal_id),
        &credits,
    );
    effective
}
