# SEP-40 Conformance Suite (#515)

Conformance suite: `contracts/price-oracle/src/sep40_conformance_tests.rs`.
Run it with `make sep40-conformance` (or `cargo test -p price-oracle --lib sep40_conformance`).
CI runs the same target on every pull request (`.github/workflows/ci.yml`, job `conformance`).

The point of this suite is to check the contract against the **published SEP**, not
against our own expectations. Every requirement below is transcribed from the SEP
text, given an ID, and mapped to a test. Anything we do differently is recorded as
a deviation (`D-*`), and anything the SEP leaves open is recorded as an ambiguity
(`A-*`) together with the interpretation this contract implements.

The suite fails if the table below and the `CHECKLIST` constant in the test module
drift apart (`every_requirement_has_a_test`), if a mapped test function does not
exist (`requirement_test_functions_exist`), or if a documented deviation or
ambiguity has no rationale (`deviations_and_ambiguities_are_documented`).

## Pinned spec version

| Field | Value |
|---|---|
| SEP | [SEP-40 — Oracle Consumer Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0040.md) |
| Spec version | `0.1.0` (front-matter `Version:`) |
| Status | Draft |
| Front-matter `Updated:` | `2023-05-13` |
| Upstream path | `ecosystem/sep-0040.md` |
| Last upstream commit touching the file | `d4d17e6` (2025-06-17) — a SEP *process* change; the technical content is unchanged since 2023-05-13 |
| Pinned on | 2026-09-29 |

Everything below is stated against that pinned text. If the SEP changes, bump the
table above, re-read the diff, and update the affected rows in the same PR — the
"Version skew" section lists what to re-check on every bump.

## Normative requirements

`MUST`-class rows are requirements the SEP states normatively; the rest are
properties the SEP states descriptively and the contract implements anyway.

| ID | SEP clause | Requirement | Test | Status |
|---|---|---|---|---|
| SEP40-R01 | Interface / `base` | `base()` returns the `Asset` all prices are quoted in | `r01_base_returns_the_denomination_asset` | conforms |
| SEP40-R02 | Interface / `assets` | `assets()` returns every asset quoted by the feed | `r02_assets_lists_every_quoted_asset_as_stellar_variant` | conforms |
| SEP40-R03 | Interface / `decimals` | `decimals()` returns the number of decimals used by all quoted assets | `r03_decimals_reports_price_precision` | conforms |
| SEP40-R04 | Interface / `resolution` | `resolution()` returns the default tick period in seconds | `r04_resolution_reports_tick_seconds` | conforms |
| SEP40-R05 | Interface / `lastprice` | `lastprice(asset)` returns the most recent price for an asset | `r05_lastprice_returns_the_latest_aggregate` | conforms |
| SEP40-R06 | Interface / `price` | `price(asset, timestamp)` returns the price at a specific timestamp | `r06_price_returns_newest_record_at_or_before_timestamp` | conforms (interpretation A-02) |
| SEP40-R07 | Interface / `prices` | `prices(asset, records)` returns the last N records | `r07_prices_returns_at_most_records_entries_newest_first` | conforms |
| SEP40-R08 | Interface / `PriceData` | `PriceData` carries `price: i128` and `timestamp: u64` | `r08_price_data_exposes_the_spec_fields` | conforms + deviation D-01 |
| SEP40-R09 | Interface / `Asset` | `Asset` is `Stellar(Address)` or `Other(Symbol)` | `r09_asset_has_exactly_the_two_spec_variants` | conforms |
| SEP40-R10 | Design Rationale (precision) | `price` is scaled by `10^decimals`; real price is `price / 10^decimals` | `r10_price_is_scaled_by_ten_to_the_decimals` | conforms |
| SEP40-R11 | Design Rationale (precision) | Every read entrypoint reports the same scaled value | `r11_every_read_entrypoint_reports_the_same_scaled_value` | conforms |
| SEP40-R12 | Design Rationale (errors) | Invalid input (e.g. unknown asset) returns `None`, it does not throw | `r12_unknown_asset_returns_none_instead_of_error` | conforms |
| SEP40-R13 | Design Rationale (errors) | Non-quoted `Asset::Other` input returns `None`, it does not throw | `r13_other_asset_variant_returns_none_instead_of_error` | conforms |
| SEP40-R14 | Design Rationale (errors) | A timestamp before the first retained record returns `None` | `r14_timestamp_outside_history_returns_none` | conforms |
| SEP40-R15 | Interface / `prices` | `records` is the maximum number of records to return | `r15_zero_records_returns_an_empty_vec` | conforms (interpretation A-03) |
| SEP40-R16 | Interface (symbol names) | The exported function names are exactly the spec's | `r16_exported_symbols_match_the_spec_names` | conforms (ambiguity A-01) |
| SEP40-R17 | Interface (authorization) | Consumer read entrypoints need no authorization | `r17_read_entrypoints_require_no_authorization` | conforms |
| SEP40-R18 | Design Rationale (staleness) | A price older than the resolution window is not served as current | `r18_lastprice_is_none_once_the_resolution_window_lapses` | conforms |
| SEP40-R19 | Design Rationale (timestamps) | `PriceData.timestamp` is trimmed to the oracle resolution | `r19_timestamps_are_trimmed_to_the_resolution` | deviation D-02 |
| SEP40-R20 | Timeframe Resolution and Price Feed Precision | `decimals()`/`resolution()` never change after deployment | `r20_decimals_and_resolution_are_admin_changeable` | deviation D-03 |

## Deviations

| ID | SEP requirement | What we do | Rationale | Affected spec versions |
|---|---|---|---|---|
| D-01 | `PriceData { price: i128, timestamp: u64 }` | `PriceData` is `{ price: i128, timestamp: u64, last_updated: u32 }` | Consumers need to know which ledger wrote a datapoint, e.g. to detect an aggregate that has not moved since the last poll. **Impact, measured by `r08_price_data_exposes_the_spec_fields`:** Soroban decodes a struct strictly, so a value carrying the third field does *not* decode into the two-field struct the SEP prints. A consumer must therefore use the contract's own `PriceData` — which the generated client does — rather than hand-rolling the SEP's struct. | 0.1.0 |
| D-02 | "`timestamp` … is calculated as `floor(unix_now()/resolution)*resolution`" | The published timestamp is the source-supplied observation timestamp; it is not floored to the resolution grid | A resolution-floored timestamp misstates when the price was observed, and this oracle accepts off-chain-signed submissions whose observation time is known exactly. `last_updated` carries the ledger write time instead. Consumers that need a uniform grid must floor the value themselves. | 0.1.0 |
| D-03 | "Once the price feed aggregation contract is deployed, these values should never change" | `decimals()` and `resolution()` stay admin-configurable after deployment | Aggregation policy changes (source onboarding, precision migration) are normal operations for a permissioned aggregator. Every change is audited (`admin_action` plus `DecimalsChanged`/`ResolutionChanged` events) and must be coordinated off-chain, so consumers must re-read `decimals()`/`resolution()` at start-up rather than hard-code them. | 0.1.0 |
| D-04 | "a contract should return `None` on such function calls" for invalid input | `prices(asset, records)` **throws** `RecordsLimitExceeded` when `records` exceeds the configured retention length | A `records` value above the retention limit is a consumer bug (an unbounded read), not a missing datapoint, and the `Option` return has nowhere to report "your request is impossible". `lastprice` and `price` follow the SEP exactly. | 0.1.0 |

## Ambiguities

| ID | Ambiguity | Interpretation chosen | Rationale |
|---|---|---|---|
| A-01 | The interface block names the method `lastprice`; the Design Rationale prose refers to `last_price(env, asset)` | The exported symbol is `lastprice` (the interface block wins); `last_price` is **not** exported | The prose uses a snake_case spelling the Rust interface block never defines, so a consumer reading only the prose would call a non-existent function. `r16_exported_symbols_match_the_spec_names` pins the choice and asserts `last_price` is absent. |
| A-02 | "`price(asset, timestamp)` … under condition that the historical data for a requested timestamp is available, or throws an error otherwise" — exact match or nearest-earlier record? | Nearest-earlier: the newest retained record with `record.timestamp <= timestamp`; `None` when the requested timestamp predates all retained records, and the newest record when it is later than the last observation | The price at time *t* is by definition the most recent observation at or before *t*; exact-match-only would make the endpoint useless for any consumer that samples off-grid, which D-02 forces consumers to do. Consumers needing strict exactness must compare `PriceData.timestamp` themselves. |
| A-03 | `prices(asset, records)` with `records = 0` | `Some(empty vec)` — a well-formed "no records requested" answer, not `None` | The SEP reserves `None` for "no such asset / no data". Zero is a legal in-range `u32` and asking for zero records is not an error. |
| A-04 | May `Some(vec)` be shorter than `records`? | Yes: up to `records` newest entries are returned, and when history is empty but an aggregate exists the current aggregate is returned as a single entry | Retention is provider-configured and the SEP explicitly leaves it to the provider, so a short vector is the only truthful answer. |

## Version skew

Re-check these rows whenever the pinned SEP revision is bumped:

* The `PriceData` field set (D-01) — a spec-side `PriceData` change is breaking for every consumer.
* The `lastprice` vs `last_price` spelling (A-01).
* The "never change `decimals()`/`resolution()`" sentence (D-03) — if it becomes a `MUST`, the admin setters must be removed or gated behind a migration.
* The "return `None`, don't throw" rule (D-04) — if it is extended to over-large `records`, the `prices` guard has to move to `Option` semantics.
* Timestamp trimming (D-02).

A pinned-version bump is a deliberate act: update the table, update the deviations
and ambiguities, and re-run `make sep40-conformance`. The suite is intentionally
not auto-updated, so a spec change can never silently pass CI.
