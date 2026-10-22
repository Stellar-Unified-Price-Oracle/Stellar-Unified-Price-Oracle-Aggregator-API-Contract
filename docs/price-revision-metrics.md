# Price Revision Frequency and Correction-Rate Metrics

**Issue:** #499 — track how often published prices are corrected or revised, per
asset and per source, so a rising correction rate is visible as a data-quality
trend rather than a series of isolated incidents.

| Claim | Enforced by |
|---|---|
| Revision rates are computed and exposed per asset | `services/revision_metrics/metrics.py::compute_asset_metrics`, asserted in `test_revision_rate_is_exposed_per_asset` |
| Causes are separated | `RevisionCause` + `classify`, asserted in `test_every_revision_is_counted_exactly_once_in_one_cause_bucket` |
| A sustained increase triggers an alert | `_sustained` / `evaluate_alerts`, asserted in `test_sustained_increase_triggers_an_alert` |
| Volume normalization is applied | `normalized_rate` + the `min_publications` gate in `_sustained`, asserted in `test_rate_is_volume_normalized_on_a_low_volume_asset` and `test_low_volume_recent_window_cannot_alert_even_with_many_corrections` |
| The metric cannot be reduced by suppressing corrections | `reconcile` against the append-only on-chain revision chain, asserted in `test_suppressed_corrections_show_up_as_a_reconciliation_gap` |

```bash
python -m services.revision_metrics.metrics events.jsonl \
    --authorized-actor GADMIN... \
    --on-chain-revisions '{"CASSET...": 5}' \
    --fail-on-alert
```

---

## 1. Revision events and their causes

The only on-chain path that revises an already-published aggregate is
`correct_price` (`contracts/price-oracle/src/corrections.rs`), which appends to
an immutable revision chain and emits `PriceCorrectedEvent`. The service reads
that event from the shared stream
(`services.common.events.TOPIC_PRICE_CORRECTED`) together with
`price_aggregated` (publications — the rate denominator) and
`price_submitted` (per-source volume).

Each revision is classified into **exactly one** cause bucket:

| Cause | Meaning | Why it is separated |
|---|---|---|
| `authorized_correction` | Filed by the admin or a `PriceUpdater` delegate through `correct_price` | Expected, bounded (`MAX_CORRECTIONS_PER_ASSET`, correction window) and already audited. Mixing it into the same rate as an incident would make a healthy operator look like a failing pipeline. |
| `source_correction` | Not filed by an authorized actor, but attributable to a named oracle source | Points the incident at *that source's* upstream, not at the aggregation pipeline — the input to `services/reliability_score`. |
| `unattributed` | Neither an authorized filer nor a named source | This **is** the data-quality incident. It is counted, never dropped, and raises its own alert. |

Precedence is deliberate (`classify`): an authorized actor wins even when a
source is also named, because the *filing* path is the part an operator
controls. Classifying is total and single-valued — every revision lands in
exactly one bucket (`test_cause_classification_is_total_and_exclusive`).

## 2. Rate per asset and per source, over rolling windows

Two windows are computed for every scope and compared:

* **recent** — the last `window_secs` (default 24 h);
* **baseline** — the `baseline_window_secs` immediately before it (default 24 h,
  so like is compared with like).

Per asset the denominator is the asset's **publications**; per source it is
**that source's own submissions** over the same window, so a source is not
penalised for an asset it does not cover. `compute_source_metrics` shares the
windowing code with `compute_asset_metrics`, so the two cannot drift apart.

Outputs: `AssetRevisionMetrics` / `SourceRevisionMetrics`, both with lifetime
counts, per-cause counts, both window rates, the trend multiplier and a
`sustained_increase` flag. Everything is a pure function of the events
(`test_metrics_are_pure_functions_of_the_event_stream`) and `now` defaults to
the newest event, so a report is reproducible from the same export.

## 3. Volume normalization

A raw `revisions / publications` ratio is unusable on a low-volume asset: one
correction on an asset that publishes twice a day is a 50 % "correction rate"
and says nothing about quality. Two mechanisms fix this, and they are separate
on purpose:

1. **The reported rate is floored.** `normalized_rate` scores the window
   against `max(publications, min_publications)`, and the report carries
   `low_volume: true` so a consumer can refuse to act on it. One correction on
   a 2-publication window reads as 50 ‰ (5 %), not 1000 ‰ (100 %).
2. **Alerting requires volume.** `_sustained` additionally requires
   `recent.publications >= min_publications`, so *no* number of corrections on a
   nearly-idle asset can page
   (`test_low_volume_recent_window_cannot_alert_even_with_many_corrections`).

The baseline rate used in the comparison is also floored
(`BASELINE_RATE_FLOOR_PERMILLE`) so a single correction against a zero baseline
reads as a large multiple rather than infinity
(`test_baseline_floor_stops_a_zero_baseline_reading_as_infinite`).

## 4. Trends and alerts

A **sustained increase** requires all three of:

1. `recent.revisions >= min_revisions` (default 3) — one correction is an
   incident, not a trend;
2. `recent.publications >= min_publications` (default 20) — the volume gate;
3. `trend_multiplier >= sustained_multiplier` (default 2.0).

`evaluate_alerts` then emits a `page` alert per offending scope (`asset:…` /
`source:…`) and a `warning` alert for any asset with unattributed revisions —
including an asset with *no* rate increase, because an un-attributable revision
is a finding on its own.

## 5. The metric is observational, and cannot be gamed by suppression

This is a property of the design, not a disclaimer:

* **It is read-only.** The service consumes the event stream. It has no path
  that writes to the contract, and none that suppresses, delays or discards a
  correction. Adding corrections can only raise the rate
  (`test_metric_reads_only_events_and_never_suppresses_a_correction`).
* **The ground truth is public and append-only.** The revision chain lives in
  contract storage: `get_price_revisions(asset)` returns it whole and
  `get_original_price(asset)` returns revision 0. `reconcile` compares the
  observed corrections against `len(chain) - 1` and reports a gap for any asset
  where the two disagree. A correction that is filed on chain but never indexed
  is therefore *visible* as a gap
  (`test_suppressed_corrections_show_up_as_a_reconciliation_gap`) — hiding it
  makes the report fail, not look better.
* **Un-attributable is not a free option.** Leaving a revision with no cause and
  no source raises a warning alert, so declining to attribute cannot buy
  silence.
* **Correcting prices automatically is out of scope.** Nothing in this service
  mutates a price, a source set or a source's reputation. An automated response
  to a possibly-wrong correction is how a bad correction becomes a price attack;
  remediation stays with the operator and the existing runbooks.

## 6. Operational use

Wire the report into the existing alerting surface (see
`docs/monitoring/README.md`): emit one time series per asset and per source
(`recent_rate_permille`), one per cause, and page on `page` alerts. Suggested
SLO: `recent_rate_permille` per asset should stay under
`sustained_multiplier × baseline_rate_permille`; a breach is a data-quality
incident even when every individual correction was authorized and correctly
reasoned — a rising rate is a degradation signal in its own right.

## 7. Out of scope

Automatically correcting prices, revoking sources, or changing any on-chain
state. The metric observes; the operator (and the existing runbooks) act.
