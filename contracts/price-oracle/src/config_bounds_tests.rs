#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{test_helpers::setup_contract, PriceOracleContractClient};

fn setup_config_case<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address) {
    let (client, admin) = setup_contract(e);
    (client, admin)
}

#[test]
fn validates_min_sources_lower_bound() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test minimum valid value (1)
    let result = std::panic::catch_unwind(|| {
        client.set_min_sources_required(&1u32);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_min_sources_too_small() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test zero (invalid)
    let result = std::panic::catch_unwind(|| {
        client.set_min_sources_required(&0u32);
    });
    assert!(result.is_err());
}

#[test]
fn validates_min_sources_upper_bound() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test reasonable upper bound
    let result = std::panic::catch_unwind(|| {
        client.set_min_sources_required(&100u32);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_min_sources_too_large() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test unreasonably large value
    let result = std::panic::catch_unwind(|| {
        client.set_min_sources_required(&1_000_000u32);
    });
    assert!(result.is_err());
}

#[test]
fn validates_max_history_lower_bound() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test minimum valid value (1)
    let result = std::panic::catch_unwind(|| {
        client.set_max_history(&1u32);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_max_history_too_small() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test zero (invalid)
    let result = std::panic::catch_unwind(|| {
        client.set_max_history(&0u32);
    });
    assert!(result.is_err());
}

#[test]
fn validates_max_history_upper_bound() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test reasonable upper bound (10000)
    let result = std::panic::catch_unwind(|| {
        client.set_max_history(&10_000u32);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_max_history_too_large() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test unreasonably large value that could exhaust storage
    let result = std::panic::catch_unwind(|| {
        client.set_max_history(&u32::MAX);
    });
    assert!(result.is_err());
}

#[test]
fn validates_decimals_bounds() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test valid range [0, 18]
    let result = std::panic::catch_unwind(|| {
        client.set_decimals(&18u32);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_decimals_too_large() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test invalid value > 18
    let result = std::panic::catch_unwind(|| {
        client.set_decimals(&19u32);
    });
    assert!(result.is_err());
}

#[test]
fn accepts_decimals_zero() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test zero decimals (valid)
    let result = std::panic::catch_unwind(|| {
        client.set_decimals(&0u32);
    });
    assert!(result.is_ok());
}

#[test]
fn validates_resolution_bounds() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test valid resolution
    let result = std::panic::catch_unwind(|| {
        client.set_resolution(&3600u32);
    });
    assert!(result.is_ok());
}

#[test]
fn accepts_resolution_zero() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test zero resolution (valid)
    let result = std::panic::catch_unwind(|| {
        client.set_resolution(&0u32);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_resolution_too_large() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test unreasonably large resolution
    let result = std::panic::catch_unwind(|| {
        client.set_resolution(&u32::MAX);
    });
    assert!(result.is_err());
}

#[test]
fn validates_timestamp_threshold_bounds() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test valid threshold (5 minutes = 300 seconds)
    let result = std::panic::catch_unwind(|| {
        client.set_timestamp_threshold(&300u64);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_timestamp_threshold_too_small() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test zero (invalid)
    let result = std::panic::catch_unwind(|| {
        client.set_timestamp_threshold(&0u64);
    });
    assert!(result.is_err());
}

#[test]
fn rejects_timestamp_threshold_too_large() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test unreasonably large value
    let result = std::panic::catch_unwind(|| {
        client.set_timestamp_threshold(&u64::MAX);
    });
    assert!(result.is_err());
}

#[test]
fn validates_multiple_bounds_together() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Set multiple config values with bounds
    let r1 = std::panic::catch_unwind(|| {
        client.set_min_sources_required(&3u32);
    });
    assert!(r1.is_ok());

    let r2 = std::panic::catch_unwind(|| {
        client.set_max_history(&1000u32);
    });
    assert!(r2.is_ok());

    let r3 = std::panic::catch_unwind(|| {
        client.set_decimals(&8u32);
    });
    assert!(r3.is_ok());

    let r4 = std::panic::catch_unwind(|| {
        client.set_resolution(&60u32);
    });
    assert!(r4.is_ok());
}

#[test]
fn rejects_heartbeat_interval_too_small() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test heartbeat interval below minimum
    let result = std::panic::catch_unwind(|| {
        client.set_heartbeat_interval(&0u64);
    });
    assert!(result.is_err());
}

#[test]
fn validates_heartbeat_interval_upper_bound() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test reasonable heartbeat interval (1 hour = 3600 seconds)
    let result = std::panic::catch_unwind(|| {
        client.set_heartbeat_interval(&3600u64);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_heartbeat_interval_too_large() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test unreasonably large heartbeat
    let result = std::panic::catch_unwind(|| {
        client.set_heartbeat_interval(&u64::MAX);
    });
    assert!(result.is_err());
}

#[test]
fn boundary_test_min_sources_at_limit() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test just below max
    let result1 = std::panic::catch_unwind(|| {
        client.set_min_sources_required(&99_999u32);
    });
    assert!(result1.is_ok());

    // Test just above max
    let result2 = std::panic::catch_unwind(|| {
        client.set_min_sources_required(&100_001u32);
    });
    assert!(result2.is_err());
}

#[test]
fn boundary_test_max_history_at_limit() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test just below max
    let result1 = std::panic::catch_unwind(|| {
        client.set_max_history(&999_999u32);
    });
    assert!(result1.is_ok());

    // Test just above max
    let result2 = std::panic::catch_unwind(|| {
        client.set_max_history(&1_000_001u32);
    });
    assert!(result2.is_err());
}

#[test]
fn boundary_test_decimals_at_limit() {
    let e = Env::default();
    let (client, _admin) = setup_config_case(&e);

    // Test at limit (18)
    let result1 = std::panic::catch_unwind(|| {
        client.set_decimals(&18u32);
    });
    assert!(result1.is_ok());

    // Test above limit (19)
    let result2 = std::panic::catch_unwind(|| {
        client.set_decimals(&19u32);
    });
    assert!(result2.is_err());
}
