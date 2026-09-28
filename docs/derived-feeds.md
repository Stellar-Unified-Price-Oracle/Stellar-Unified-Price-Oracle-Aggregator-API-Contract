# On-Chain Derived Price Feeds (#478)

> Inverse prices, pairwise ratios and triangulated cross-rates, computed
> on-chain with a defined rounding direction, propagated staleness, and full
> provenance.

Consumers repeatedly recompute `A/B` ratios and inverses off-chain, each with
its own rounding and staleness handling. Canonical derived feeds remove that
ambiguity and a class of consumer bugs.

- Base pin (admin): `set_derived_feed_base(asset, price, timestamp)`.
- Queries: `get_inverse_feed(asset)`, `get_ratio_feed(base, quote)`,
  `get_triangulated_feed(base, pivot, quote)`, and the generic
  `compute_derived_feed(kind, base, quote, pivot)`.

```text
DerivedFeed {
  kind,             // 0 = inverse, 1 = ratio, 2 = triangulation
  base, quote,
  price,            // scaled by 10^decimals
  decimals,
  staleness_secs,   // worst case (maximum) age across all inputs
  oldest_timestamp, // minimum input timestamp
  inputs,           // provenance, in evaluation order
}

DerivedFeedInput { asset, price, timestamp, from_base }
```

## Resolution order

A derived feed is a pure function of **base prices**. A base price for an asset
resolves in exactly this order:

1. `DerivedFeedBase` — an admin-pinned canonical `(price, ts)`, reported with
   `from_base = true`;
2. `Aggregate` — the live aggregate written by `submit_price`, reported with
   `from_base = false`;
3. neither → `UnknownDerivedPair`.

The asset itself must be registered, or `AssetNotRegistered`.

## Rounding direction

**Every derivation truncates toward zero.** This is stated per kind because it
is a consumer-facing contract, and it is asserted per kind by test.

| Kind | Formula | Rounding |
|---|---|---|
| `Inverse` | `scale * scale / p` | truncate toward zero |
| `Ratio` | `p_base * scale / p_quote` | truncate toward zero |
| `Triangulation` | `r1 = p_base * scale / p_pivot`, then `price = r1 * p_pivot / p_quote` | truncate toward zero **at each step** |

Truncation biases every derived value **low**: a consumer of an inverse or a
ratio never over-pays. For a triangulation the intermediate truncation is
deliberate — it keeps every intermediate inside `u128` and makes the result
reproducible from the same inputs — and it costs at most one unit of the
pivot's own scale per step, so the result is never above the exact real-number
cross rate `p_base / p_quote`.

The second triangulation step multiplies by `p_pivot`, **not** by `scale`: the
product `r1 * p_pivot` is already dimensionally a scaled price, so scaling it
again would yield `base / (pivot * quote)` rather than the cross rate.

## Numeric range

All derivation is `i128` arithmetic on values scaled by `scale = 10^decimals`.
`decimals > 18` is rejected with `InvalidConfiguration`.

Every intermediate product is formed in **`u128`**, not `i128`. This is not
defensive decoration: `p * 10^decimals` overflows `i128` for any price above
roughly `186.0` at 18 decimals, which is an ordinary price for most assets, so
a narrow intermediate would reject perfectly valid derivations with
`InvalidConfiguration`. A quotient that genuinely does not fit `i128` is
refused rather than wrapped.

**Test.** `test_derivations_survive_prices_above_the_i128_scaled_product_limit`
fixes prices at `250.0` and `125.0` — inside the window where the scaled product
exceeds `i128::MAX` but still fits `u128` — and asserts each kind against the
exact rational reference.

## Staleness propagates

`staleness_secs` is the **worst case** (maximum) age across *all* inputs, and
`oldest_timestamp` is the minimum input timestamp. A derived feed is only ever
as fresh as its stalest input, so propagating the maximum is the only safe rule:
anything else would let a stale leg hide behind a fresh one.

**Test.** `test_staleness_is_worst_case_across_inputs`.

## Pair validation and rejection

| Condition | Error |
|---|---|
| any division input is `0` | `DerivedFeedZeroDenominator` (184) |
| a requested pair/triplet has no price | `UnknownDerivedPair` (185) |
| a repeated leg collapses the derivation (`a/a`, or any repeated triangulation leg) | `DerivedFeedCycle` (186) |
| depth bound would be exceeded | `DerivedFeedDepthExceeded` (187) |
| a `pivot` is supplied for a non-triangulation, or omitted for one | `InvalidConfiguration` |
| the asset is not registered | `AssetNotRegistered` |

Zero-denominator and unknown-pair are **distinct** errors, because they mean
different things to a consumer: a known market with no valid price right now
versus a pair that was never established.

**Tests.** `test_zero_denominator_and_unknown_pair_codes_are_distinct`,
`test_zero_inverse_base_is_zero_denominator`,
`test_zero_ratio_quote_is_zero_denominator`,
`test_zero_triangulation_pivot_is_zero_denominator`.

## Cycles are structurally impossible

A derived feed is *only ever* computed from a base price — never from another
derived feed — and a `DerivedFeed` value is never written back to
`DerivedFeedBase` or `Aggregate`. The derivation graph therefore has depth
`MAX_DERIVATION_DEPTH` (1), and a cycle is **structurally** impossible rather
than merely detected.

The `DerivedFeedCycle` check is a belt-and-braces guard on the degenerate
same-asset request (a self-pair is a known pair whose *graph* is degenerate, so
it is reported as a cycle rather than an unknown pair), and
`assert_depth` backs the depth bound in code so the invariant is testable rather
than only commented.

**Tests.** `test_derived_feed_can_never_be_used_as_an_input`,
`test_depth_bound_is_one_and_enforced`,
`test_degenerate_triangulations_are_rejected_as_cycles`.

## Provenance and events

`inputs` records every input in evaluation order (base, then quote, then pivot),
with the price, timestamp and whether it came from the pinned base. Every
computation emits exactly one `DerivedFeedComputedEvent` carrying the kind, the
price and the propagated staleness, so a consumer can audit the derivation
off-chain.

**Tests.** `test_provenance_inputs_are_in_evaluation_order`,
`test_provenance_input_counts_per_kind`,
`test_each_getter_emits_exactly_one_event`,
`test_computed_event_is_emitted_per_kind`.

## Agreement with an off-chain reference

Each kind is checked against a longhand rational reference written out in the
test suite and deliberately **not** calling the contract's helpers, so the
assertion is a real check rather than a tautology.

**Tests.** `test_inverse_matches_offchain_reference_within_one_unit`,
`test_ratio_matches_offchain_reference_within_one_unit`,
`test_triangulation_matches_offchain_reference_within_one_unit`.

## The pinned base is allowed to be zero

`set_derived_feed_base` accepts a `0` price on purpose: a zero canonical base
models "this market has no valid price right now". The derivation engine turns
it into `DerivedFeedZeroDenominator` at read time rather than inventing a value
or an infinite inverse. A negative price is rejected with `InvalidPrice`, and a
timestamp further than the configured threshold into the future is rejected with
`InvalidTimestamp`.
