# Aggregate recomputation on source-set change (#483)

When a source is removed, loses its claim on an asset, is disqualified by the
demerit system, or is suspended for missing heartbeats, its **last submission
used to linger inside the published median** until the next submission arrived.
A disqualified or malicious source therefore kept influencing the price after
removal.

The aggregator now recomputes synchronously, in the same transaction as the
source-set change, from the surviving source set only.

## What triggers a recomputation

| Path | `RecomputeReason` |
|---|---|
| `remove_source` / `remove_sources` | `SourceRemoved` |
| `remove_source_asset` | `SourceAssetRemoved` |
| demerit threshold crossed → `Disqualified` | `SourceDisqualified` |
| heartbeat threshold crossed → inactive | `SourceSuspended` |

`recompute_asset_price(asset)` triggers the same recomputation on demand. It
is permissionless by design: it can only ever reproduce what the next
submission would publish, so it grants no authority over the value.

## Guarantees

* **No stale contribution.** The source is removed from the registry *before*
  recomputation runs, so its prior submission is skipped by the aggregation
  loop exactly like a source that had never submitted.
* **Atomic.** The recomputed aggregate, its history entry, the removal event
  (`SourceRemovedEvent`) and the `AggregateRecomputedEvent` are all written in
  one transaction. An observer never sees a removal without the matching
  recomputation.
* **No partial or double-counted state.** Recomputation is a pure function of
  the surviving registry — it never merges with a partially-cleared one.
* **Batch == sequential.** `remove_sources` removes each source and recomputes
  once at the end; because recomputation reads only the final registry, the
  batch converges on the same aggregate as removing them one at a time.
* **Bounded cost.** At most [`MAX_RECOMPUTE_ASSETS`] (64) assets are
  re-derived per change, and history is never walked — the cost is one
  aggregation pass per affected asset, cheap enough to run inside the removal
  transaction.

## Event

`AggregateRecomputedEvent` — topics `asset`, `reason`; fields `price`,
`num_sources`, `previous_num_sources`, `ledger`.

`previous_num_sources` vs `num_sources` makes the effect of the change legible
off-chain: a removal that drops the count below quorum leaves the previously
published aggregate in place and the asset becomes quorum-deficient.

## Inspecting the blast radius

* `is_source_excluded(source)` — whether the source is currently ineligible
  (inactive or disqualified).
* `get_recompute_affected_assets(source)` — the assets a change to this source
  would re-derive.

## Out of scope

Historical price revision for previously published values. A recomputation
appends a *new* aggregate; it does not rewrite past history entries. See
[price-corrections.md](price-corrections.md) for the audited path to correct a
value that was already published.
