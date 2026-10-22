# Integration Case Studies (#408)

Most oracle incidents are integration bugs rather than contract bugs. Each case
study below gives the trust model, one worked attack on a naive integration and
the corrected integration. Every guarantee cites the code that enforces it and
the executable snippet in
[`case_study_tests.rs`](../contracts/price-oracle/src/case_study_tests.rs), which
CI runs with the full test suite (`cargo test -p price-oracle --lib case_`):

```
test case_study_tests::case_lending_stale_price_accepted_when_max_age_is_zero ... ok
test case_study_tests::case_dex_single_source_cannot_move_median ... ok
test case_study_tests::case_payments_frozen_price_bypasses_max_age ... ok
test result: ok. 3 passed; 0 failed
```

## What the oracle guarantees, and what it does not

| Guaranteed | Enforced by |
|---|---|
| No aggregate is published until `min_sources_required` sources have submitted | `prices.rs` aggregation; `case_dex_single_source_cannot_move_median` |
| Aggregate is the median of counted submissions | `core_pricing::median_core`; `prop_tests.rs` |
| Submissions are rejected while paused (`ErrorCode::ContractPaused` = 12) | `pause::check_not_paused`; `seam_pause_submission__*` |
| `get_price(asset, max_age)` returns `None` when the aggregate is older than `max_age` seconds | `prices::get_price` (`max_age > 0 && timestamp + max_age < now`) |
| Prices are scaled by `10^decimals()` | `AggregatePrice.decimals`, `decimals()` |

| **Not** guaranteed | Consequence |
|---|---|
| Staleness when `max_age = 0` | The check is skipped entirely |
| Freshness of a **frozen** price | `get_price` returns the frozen value with its original timestamp and `num_sources = 0`, even past `max_age` |
| That the median equals a fair market price | A majority of colluding sources can move it |
| Finality of the latest aggregate | Use `get_finalized_price(asset, min_finality)` when a value must not change |

---

## 1. Lending protocol — liquidation pricing

```
Borrower ──► Lending pool ──get_price(collateral, max_age)──► Oracle
                  │                                            ▲
                  └── liquidate if collateral * price < debt   │ sources submit
```

**Trust model.** Relies on the quorum and median guarantees and on the
staleness bound. Does *not* rely on the price being fresh unless it passes a
non-zero `max_age`.

**Attack on the naive integration.** The pool calls `get_price(asset, 0)`.
Sources stop submitting during a market crash; the last aggregate (100) is
still returned an hour later while the real price is 60. A borrower deposits
collateral valued at the stale 100 and borrows against it; the pool absorbs a
40 % loss on the position when the price updates.

**Countermeasure.** Always pass a bounded `max_age` and treat `None` as "halt
borrowing". Snippet: `case_lending_stale_price_accepted_when_max_age_is_zero`.

```rust
let p = oracle.get_price(&asset, &60u64).ok_or(Error::OracleStale)?;
```

## 2. DEX — pricing an AMM rebalance

**Trust model.** Relies on median aggregation so a single manipulated source
cannot move the price. Does *not* rely on a fair price if a majority of sources
collude, and does not treat a spot price as a TWAP.

**Attack on the naive integration.** The DEX consumes a single source's price
(or configures `min_sources_required = 1`). An attacker who controls that
source reports 1,000,000 against a fair 100 and drains the pool at the
manipulated rate.

**Countermeasure.** Keep `min_sources_required ≥ 3`, check `num_sources` on the
returned `AggregatePrice`, and cap per-block deviation against `get_twap`.
Snippet: `case_dex_single_source_cannot_move_median` (one source at
1,000,000 leaves the median at 100–102).

```rust
let p = oracle.get_price(&asset, &60u64).ok_or(Error::OracleStale)?;
require(p.num_sources >= 3 && !p.is_override, Error::WeakPrice)?;
```

## 3. Payments — fiat-denominated invoices

**Trust model.** Relies on the decimal scaling and the staleness bound. Does
*not* rely on a frozen price being current.

**Attack on the naive integration.** Admin freezes XLM/USD during an incident.
The payments contract calls `get_price(asset, 60)` and trusts that a non-`None`
result is fresh. The frozen value is returned a day later (`num_sources = 0`,
original timestamp), and invoices are settled at a price that no longer
reflects the market. Separately, ignoring `decimals` mis-scales amounts by
`10^decimals`.

**Countermeasure.** Reject `num_sources == 0` or check
`is_price_frozen(asset)`; re-check the timestamp yourself; always divide by
`10^p.decimals`. Snippet: `case_payments_frozen_price_bypasses_max_age`.

```rust
let p = oracle.get_price(&asset, &60u64).ok_or(Error::OracleStale)?;
require(p.num_sources > 0 && now - p.timestamp <= 60, Error::FrozenOrStale)?;
let amount = fiat * 10i128.pow(p.decimals) / p.price;
```

---

## Integrator checklist

- [ ] **Staleness:** never pass `max_age = 0`; handle `None` as a halt.
- [ ] **Freeze/override:** reject `num_sources == 0` or `is_override == true`
      unless the protocol explicitly accepts admin-set prices.
- [ ] **Quorum:** require `num_sources` ≥ your own minimum.
- [ ] **Finality:** for irreversible actions use `get_finalized_price`
      (`ErrorCode::InsufficientFinality` when too new).
- [ ] **Decimals:** scale with `p.decimals`, never a hard-coded constant.
- [ ] **Deviation:** bound per-action change against `get_twap` or the previous
      accepted price.
- [ ] **Pause:** handle `ErrorCode::ContractPaused` (12) / `PriceFrozen` (116)
      and listen for `ContractPausedEvent`.

> Trust models still need review by someone outside the author's team
> before this document is final.
