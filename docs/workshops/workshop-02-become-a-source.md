# Workshop 02 — Becoming an Oracle Source

> **Incident classes covered:** single-source trust · unfinalized price trust · timestamp manipulation  
> **Contract revision:** pin to `main` at the time of delivery.

---

## Learning Objectives

1. Register an address as an oracle source via `add_source()`.
2. Submit prices correctly using `submit_price()`.
3. Understand the `min_sources` threshold and why consumers must respect it.
4. Identify attacks that are possible when consumers read before the threshold is met.

---

## Setup

```bash
cargo test -p price-oracle --lib workshop_02
```

---

## Background: How Source Submission Works

1. Admin registers a source address via `add_source(source_address, name)`.
2. The source calls `submit_price(source, asset, price, timestamp)` with its own auth.
3. The oracle stores the per-source price and recomputes the median when queried.
4. `get_price()` returns the median **only if** the number of submissions ≥ `min_sources`.

```rust
// Correct source submission pseudocode
oracle.submit_price(
    &source_address,   // must match registered source; requires auth
    &asset,
    &price_value,      // must be > 0
    &current_timestamp // must not be more than threshold seconds in the future
);
```

---

## Lab A — Single-Source Trust

**Real-world incident class:** Attacker registers as the only active source (or the admin
removes competing sources), submits a manipulated price, and the consumer reads it as the
"median" of one.

### Vulnerable Consumer

```rust
// VULNERABLE: reads price without checking how many sources contributed
pub fn liquidate_if_undercollateralised(
    env: Env, oracle: Address, asset: Address, collateral: u128, debt: u128
) -> bool {
    let oracle_client = OracleClient::new(&env, &oracle);
    let price = oracle_client.lastprice(&asset).unwrap().price;
    let value = collateral * price / 1_000_000;
    value < debt // liquidate if true
}
```

### Scripted Attack

```rust
#[test]
fn lab_a_single_source_exploit() {
    // Setup: min_sources = 1 (or attacker is the only active source)
    // Attacker submits price = 1 (near-zero), making all collateral appear worthless
    // ... register attacker as only source, submit price = 1 ...
    let should_liquidate = vulnerable_consumer::liquidate_if_undercollateralised(
        env.clone(), oracle.clone(), asset.clone(), 1_000_000, 500_000
    );
    // collateral * 1 / 1_000_000 = 1 < 500_000 → liquidation triggered on valid collateral
    assert!(should_liquidate, "exploit: single manipulated source triggers liquidation");
}
```

### Fixed Version

```rust
// FIXED: check that the oracle's source count meets a minimum before trusting the price
const REQUIRED_SOURCES: u32 = 3;

pub fn liquidate_if_undercollateralised(
    env: Env, oracle: Address, asset: Address, collateral: u128, debt: u128
) -> bool {
    let oracle_client = OracleClient::new(&env, &oracle);

    // Verify that the aggregated price comes from enough sources
    let sources = oracle_client.get_oracle_sources();
    assert!(
        sources.sources.len() >= REQUIRED_SOURCES as u32,
        "insufficient sources for safe liquidation"
    );

    let price = oracle_client.lastprice(&asset).unwrap().price;
    let value = collateral * price / 1_000_000;
    value < debt
}

#[test]
fn lab_a_single_source_fixed() {
    // With only 1 source, fixed version must refuse to liquidate
    let result = std::panic::catch_unwind(|| {
        fixed_consumer::liquidate_if_undercollateralised(/* 1 source only ... */)
    });
    assert!(result.is_err(), "fix verified: single source rejected");
}
```

---

## Lab B — Unfinalized Price Trust

**Real-world incident class:** Consumer reads from the oracle between two source submissions.
The first source submitted an outlier; the second (correcting) submission has not arrived.
The median of one is the outlier.

### Vulnerable Consumer

```rust
// VULNERABLE: reads immediately after each source submission without waiting for full round
pub fn read_price_immediately(env: Env, oracle: Address, asset: Address) -> u128 {
    let oracle_client = OracleClient::new(&env, &oracle);
    oracle_client.lastprice(&asset).unwrap().price
}
```

### Scripted Attack

```rust
#[test]
fn lab_b_unfinalized_price_exploit() {
    // Setup: min_sources = 2. Source A submits manipulated price. Source B has not submitted yet.
    // Consumer reads between submissions — gets median of [manipulated_price] = manipulated_price.
    // ... submit only source A's price ...
    let price = vulnerable_consumer::read_price_immediately(
        env.clone(), oracle.clone(), asset.clone()
    );
    // In this state the oracle returns InsufficientSources — but the vulnerable consumer
    // calls .unwrap() and panics, or if min_sources=1, returns the single outlier.
    // Either way, the consumer does not handle the partial state correctly.
    // (Demonstrate whichever behaviour applies to the configured min_sources.)
    println!("unfinalized price: {}", price);
}
```

### Fixed Version

```rust
// FIXED: handle InsufficientSources gracefully; never unwrap without checking
pub fn read_price_safely(env: Env, oracle: Address, asset: Address) -> Option<u128> {
    let oracle_client = OracleClient::new(&env, &oracle);
    // lastprice returns None when insufficient sources; propagate None instead of panicking
    oracle_client.lastprice(&asset).map(|pd| pd.price)
}

#[test]
fn lab_b_unfinalized_price_fixed() {
    // With only one source submitted against min_sources=2, fixed version returns None
    let price = fixed_consumer::read_price_safely(env.clone(), oracle.clone(), asset.clone());
    assert!(price.is_none(), "fix verified: unfinalized price returns None");
}
```

---

## Summary

| Lab | Exploit | Fix |
|---|---|---|
| A | Attacker controls only source; consumer trusts single-source median | Consumer enforces minimum source count independently |
| B | Consumer reads mid-round before threshold met | Consumer handles `None` / `InsufficientSources` without panicking or using a default |

## Next Workshop

[Workshop 03 — Oracle Governance](workshop-03-govern.md)
