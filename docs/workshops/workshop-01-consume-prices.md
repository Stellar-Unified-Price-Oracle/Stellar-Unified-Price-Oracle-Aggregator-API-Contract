# Workshop 01 — Consuming Oracle Prices Safely

> **Incident classes covered:** stale-price consumption · decimal mismatch · asset identity confusion  
> **Contract revision:** pin to `main` at the time of delivery.

---

## Learning Objectives

By the end of this workshop you will be able to:

1. Call `lastprice()` and `get_price()` correctly from a consumer contract.
2. Detect and reject stale prices.
3. Apply `decimals()` correctly before any arithmetic.
4. Validate that the returned asset matches the expected asset.

---

## Setup

```bash
# From repo root — no network needed
cargo test -p price-oracle --lib workshop_01
```

All three labs run inside a single test module using `soroban-sdk testutils`.

---

## Background: Safe Consumption Pattern

A correct consumer checks three things after every oracle call:

1. **Presence** — the call returned `Some(price)`, not `None`.
2. **Staleness** — `price.timestamp` is within the consumer's acceptable freshness window.
3. **Decimals** — the raw integer is divided by `10^decimals()` before use in arithmetic.

```rust
// Correct consumer pseudocode
let price_data = oracle.lastprice(&env, &asset)?; // returns Option<PriceData>
let price = price_data?; // reject None

// 1. Staleness check (example: 60-second window)
let age = env.ledger().timestamp() - price.price; // use price.price field per PriceData
assert!(env.ledger().timestamp() - price.price <= 60, "price is stale");

// 2. Apply decimals before arithmetic
let decimals = oracle.decimals(&env);
let scaled_price = price.price / 10u128.pow(decimals);
```

---

## Lab A — Stale Price Consumption

**Real-world incident class:** Lending protocol liquidates positions using a price that is hours
old, because the consumer never checks the timestamp.

### Vulnerable Version

```rust
// VULNERABLE: no staleness check
pub fn get_collateral_value(env: Env, oracle: Address, asset: Address, amount: u128) -> u128 {
    let oracle_client = OracleClient::new(&env, &oracle);
    let price_data = oracle_client.lastprice(&asset).unwrap(); // panics on None, ignores staleness
    price_data.price * amount
}
```

### Scripted Attack

```rust
#[test]
fn lab_a_stale_price_exploit() {
    // Setup: submit a price, then advance time past the staleness window
    let env = Env::default();
    // ... initialize oracle, submit price at ledger T ...
    // advance ledger timestamp by 3600 seconds (1 hour)
    env.ledger().with_mut(|l| l.timestamp += 3600);

    // The vulnerable consumer still returns the stale price without error
    let value = vulnerable_consumer::get_collateral_value(
        env.clone(), oracle.clone(), asset.clone(), 1_000_000
    );
    // value is non-zero even though the price is 1 hour old — exploit demonstrated
    assert!(value > 0, "exploit: stale price accepted");
}
```

### Fixed Version

```rust
// FIXED: staleness check before use
const MAX_PRICE_AGE_SECS: u64 = 60;

pub fn get_collateral_value(env: Env, oracle: Address, asset: Address, amount: u128) -> u128 {
    let oracle_client = OracleClient::new(&env, &oracle);
    let price_data = oracle_client.lastprice(&asset)
        .expect("no price available");

    let age = env.ledger().timestamp()
        .checked_sub(price_data.price) // price field carries the timestamp in PriceData
        .expect("timestamp underflow");
    assert!(age <= MAX_PRICE_AGE_SECS, "price is stale");

    price_data.price * amount
}

#[test]
fn lab_a_stale_price_fixed() {
    // Same setup as the attack — fixed version must panic on stale price
    // ... advance time by 3600 secs ...
    let result = std::panic::catch_unwind(|| {
        fixed_consumer::get_collateral_value(/* ... */)
    });
    assert!(result.is_err(), "fix verified: stale price rejected");
}
```

---

## Lab B — Decimal Mismatch

**Real-world incident class:** Consumer treats a price with 18 decimals as if it has 6 decimals,
producing a value 10^12 times too large, leading to wildly wrong collateral calculations.

### Vulnerable Version

```rust
// VULNERABLE: ignores decimals, treats raw integer as if it has 6 decimal places
pub fn usd_value(price_raw: u128, amount: u128) -> u128 {
    (price_raw * amount) / 1_000_000 // hardcoded 6-decimal assumption
}
```

### Scripted Attack

```rust
#[test]
fn lab_b_decimal_mismatch_exploit() {
    // Oracle is configured with 18 decimals; price for 1 XLM = 0.12 USD
    // Raw value: 0.12 * 10^18 = 120_000_000_000_000_000
    let price_raw = 120_000_000_000_000_000u128;
    let amount = 1u128;

    let result = vulnerable_consumer::usd_value(price_raw, amount);
    // Expected: 0.12 USD. Actual: 120_000_000_000 USD — exploit demonstrated
    assert_ne!(result, 0, "exploit: decimal mismatch produces nonsense value");
    assert!(result > 1_000_000_000, "exploit: value wildly inflated");
}
```

### Fixed Version

```rust
// FIXED: reads decimals from oracle and applies them
pub fn usd_value(env: &Env, oracle: &Address, price_raw: u128, amount: u128) -> u128 {
    let oracle_client = OracleClient::new(env, oracle);
    let decimals = oracle_client.decimals();
    let divisor = 10u128.pow(decimals);
    (price_raw * amount) / divisor
}

#[test]
fn lab_b_decimal_mismatch_fixed() {
    // Same raw price; fixed version produces the correct 0.12 USD result
    // ... setup ...
    let result = fixed_consumer::usd_value(&env, &oracle, 120_000_000_000_000_000u128, 1u128);
    // Result should be 0 (integer division of 0.12) or scaled correctly depending on amount
    // Key: no 10^12 inflation
    assert!(result < 1_000_000, "fix verified: decimal applied correctly");
}
```

---

## Lab C — Asset Identity Confusion

**Real-world incident class:** Consumer passes the wrong asset address and receives a price for
a different asset. No error is raised; the wrong price is silently used.

### Vulnerable Version

```rust
// VULNERABLE: asset address not validated against expected asset
pub fn get_xlm_price(env: Env, oracle: Address, any_asset: Address) -> u128 {
    let oracle_client = OracleClient::new(&env, &oracle);
    oracle_client.lastprice(&any_asset).unwrap().price // no assertion that this is XLM
}
```

### Scripted Attack

```rust
#[test]
fn lab_c_asset_confusion_exploit() {
    // Register two assets: XLM and BTC with very different prices
    // Call vulnerable consumer with BTC address instead of XLM
    let btc_price = fixed_consumer::get_xlm_price(env.clone(), oracle.clone(), btc_asset.clone());
    // Returns BTC price (e.g. 50_000 USD) instead of XLM price (e.g. 0.12 USD) — no error
    assert!(btc_price > 1000, "exploit: wrong asset price silently accepted");
}
```

### Fixed Version

```rust
// FIXED: assert the returned asset matches the expected asset
const EXPECTED_XLM_ASSET: &str = "XLM_CONTRACT_ADDRESS";

pub fn get_xlm_price(env: Env, oracle: Address, asset: Address) -> u128 {
    // Assert the caller-supplied asset is the known XLM contract
    assert_eq!(asset, Address::from_str(&env, EXPECTED_XLM_ASSET), "wrong asset");

    let oracle_client = OracleClient::new(&env, &oracle);
    oracle_client.lastprice(&asset).unwrap().price
}

#[test]
fn lab_c_asset_confusion_fixed() {
    // Passing BTC asset must panic
    let result = std::panic::catch_unwind(|| {
        fixed_consumer::get_xlm_price(env.clone(), oracle.clone(), btc_asset.clone())
    });
    assert!(result.is_err(), "fix verified: wrong asset rejected");
}
```

---

## Summary

| Lab | Exploit | Fix |
|---|---|---|
| A | Stale price accepted silently | Check `price.timestamp` against `env.ledger().timestamp()` |
| B | Decimal mismatch inflates value by 10^12 | Read `oracle.decimals()` and divide raw price |
| C | Wrong asset silently used | Assert asset address against a known-good constant |

## Next Workshop

[Workshop 02 — Becoming an Oracle Source](workshop-02-become-a-source.md)
