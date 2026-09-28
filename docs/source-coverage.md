# Source Coverage Gap Analysis (#498)

> Reports where the source set has coverage gaps: assets with too few
> independent sources, and time windows with systematically low participation.

## Problem

"The number of admitted sources is a misleading summary; what matters is the
minimum independent coverage per asset and whether it survives nights, weekends
and market stress." Ten sources that share one cloud, one upstream and one owner
are one failure domain with ten names.

## Independence (the definition)

Reused **verbatim** from [`source-diversity.md`](source-diversity.md) (#399), so
the two reports cannot disagree. Two sources are independent iff they differ on
**all three** failure axes:

| Axis | What it captures |
|---|---|
| `infra` | Hosting / infrastructure. |
| `upstream` | Upstream data origin. |
| `owner` | Operating entity / funding. |

Sharing *any one* axis puts both in a single failure domain, so

```text
independent_domains = |{ (infra, upstream, owner) : source admitted for asset }|
```

Missing metadata counts as `"unknown"` on every axis. That is conservative:
unknowns cluster into one domain and **lower** the count rather than inflating
it. `unattested_metadata_counts_as_one_domain` pins this behaviour.

A source is admitted for an asset when its `SourceAssets` list is absent (no
restriction) or contains the asset — the same rule `storage::check_source_asset`
enforces at submission time.

## Temporal participation

Participation is **derived from the price history the contract already stores**:
every aggregate records the `num_sources` that produced it, keyed by ledger.
Those ledgers are bucketed into fixed windows (`window_ledgers`, default
4 320 ≈ 6 h) and each window is reduced to the fewest contributing sources seen
in it, so a report is comparable across assets and across time regardless of
how long the contract has been running.

Deriving it rather than recording it is deliberate. A participation counter
written from `submit_price` would add a per-asset storage entry to the
submission hot path, and the network caps an invocation at **100 footprint
entries** — `submit_price` at 10 sources already sits at exactly 100 (see
`docs/gas-usage.md`: "11th submission needs 102 footprint entries (> 100)"). One
more entry is the difference between shipping and not. Reading history keeps the
hot path byte-for-byte unchanged.

At most `MAX_SAMPLED_LEDGERS` (32) history entries are inspected, so the cost of
an operator-facing report is bounded however long the asset has been running. A
window with no aggregate in the sampled range is simply not reported: absence of
history is "no data", not "zero participation", and conflating the two would
manufacture gaps out of thin air.

A window is a *gap* when its fewest contributing sources falls below
`min_independent_sources`. Note that participation is **not** admission: a
window can rest on a single source while the asset still has several admitted —
which is exactly the kind of quiet gap this analysis exists to surface.

## The report

`get_coverage_report(asset)`:

| Field | Meaning |
|---|---|
| `registered_sources` | Raw number of sources admitted for the asset. |
| `independent_domains` | Distinct `(infra, upstream, owner)` triples. |
| `min_independent_required` | The configured threshold. |
| `below_independence_threshold` | The asset cannot meet quorum even if every source agrees. |
| `windows_observed` | Windows with an aggregate in the sampled history. |
| `low_participation_windows` | Windows that fell below the threshold. |
| `min_window_participation` | Fewest distinct sources in any observed window. |
| `recommendations` | Advisory strings (see below). |

`get_coverage_gap_list()` names every asset below the threshold, derived from
the registered-asset set, so both are reproducible from stored data alone.

## Read-only by construction

**Coverage must never become a source-admission gate by accident.** The module
contains no call to `add_source`, `remove_source`, `add_source_asset`,
`remove_source_asset`, `set_source_diversity`, `set_source_geo` or
`pause_asset`; `coverage_is_read_only` asserts their absence from the source,
and `coverage_analysis_cannot_admit_or_remove_sources` asserts the source and
asset registries are unchanged after every read-only entry point is exercised.

The reason is adversarial, not stylistic: an automated admission decision driven
by self-reported metadata is a Sybil vector, and a hard gate would let a party
*suppress* its own coverage — by declining to submit — to avoid scrutiny.

`recommendations` is therefore advisory text for an operator review. Nothing in
the contract acts on it.

## Thresholds

`set_coverage_thresholds(min_independent_sources, window_ledgers)`, admin-only.
Defaults: `3` independent domains and `4 320` ledgers per window — the same
default independence bar as #399, so the two reports agree out of the box.

## Out of scope

Automated source recruitment.
