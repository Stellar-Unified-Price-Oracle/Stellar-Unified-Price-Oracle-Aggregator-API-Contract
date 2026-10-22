# Incremental Aggregate Maintenance (#473)

`contracts/price-oracle/src/incremental_aggregate.rs` provides `IncrementalAggregate`,
a sorted price index that keeps the median readable in O(1) and updates it with one
binary search plus one O(n) shift per mutation, instead of re-selecting over the full
source set on every submission.

## Design

| Concern | Rule |
|---|---|
| Formula | Identical to `storage::compute_median` (lower + (upper − lower) / 2 for even n). |
| Source-set change | Every admission, removal, or TTL eviction bumps the source-set **epoch**. A structure with a stale epoch never answers (`median_at` → `None`); it is rebuilt from ground truth, never patched in place. |
| Per-source update | `replace(old, new)` removes the source's previous price and inserts the new one — no rebuild. |
| Fallback | `median_or_rebuild` performs a full recompute when the epoch differs, the length drifts from ground truth (e.g. a failed submission), or the sortedness invariant fails. |
| Order | Arrival order never changes the result (sorted multiset). |

## Evidence (tests in `incremental_aggregate.rs`)

| Acceptance criterion | Test |
|---|---|
| Incremental == full recompute over randomized sequences | `incremental_matches_full_recompute` (proptest, 300 cases × up to 60 ops, asserted after every mutation) |
| Order independence | `order_independent` |
| Removal without full rebuild | `removal_updates_without_rebuild` |
| No stale aggregate after source-set change | `stale_aggregate_never_served_after_source_set_change` |
| Rebuild fallback exercised | `rebuild_fallback_on_uncertain_invariant` |
| Benchmark on a max-size (64) source set | `incremental_update_cheaper_than_full_recompute` — one incremental update + median read costs fewer CPU instructions than a full `compute_median`. |

## Gas

The structure is not yet wired into `prices.rs::aggregate_asset`, so gas for a
max-size submission batch is **unchanged** by this change. Wiring it in requires
persisting the index per asset (one extra storage entry) and bumping the epoch in
`sources::add_source` / `remove_source` and on TTL eviction; the benchmark above
shows the per-update saving that integration would realise.
