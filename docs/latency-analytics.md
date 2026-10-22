# Submission-to-Aggregate Latency Analytics — #492

Consumers could see the timestamp of the last aggregate and nothing else. A source whose
submissions *consistently* arrive too late to be counted looks exactly like a healthy one, so a
silent participation loss was invisible. This feature records the delay between a submission and
its inclusion in a published aggregate and exposes it as a per-source, per-asset distribution.

Code: `contracts/price-oracle/src/latency.rs`. Types: `types::LatencySample`,
`types::LatencyReport`. Queries: `get_latency_report(source, asset)`,
`get_latency_samples(source, asset)`.

## Units: ledgers, not seconds

Every duration in this feature is in **ledgers**. Ledger timestamps have coarse resolution and
each source chooses the `timestamp` it submits, so wall-clock subtraction between two sources is
not meaningful; the ledger sequence is the only unambiguous clock. `LatencyReport` carries
`seconds_per_ledger = 5` (Stellar's target close time) purely as a conversion hint — a consumer
that needs real freshness should compare ledger sequences, not multiply.

The report also carries `max_samples`, the size of the rolling window, so the window a
percentile was computed over is always explicit.

## Deferral is reported separately from latency

Aggregation can be deferred past a submission — a policy min-interval, an aggregation trigger, an
exhausted event budget. That is time the **oracle** added, not time the **source** was late, and
reporting only the combined figure hides a source-side participation problem behind an
oracle-side one. Each sample therefore separates:

| Field | Meaning |
|---|---|
| `latency_ledgers` | `inclusion_ledger - submission_ledger` — how long the source's value waited to be counted |
| `deferral_ledgers` | `inclusion_ledger - previous_aggregate_ledger` — how long the publication as a whole was deferred |

Reading them together tells you which side is slow: a high `p90_ledgers` with a low
`avg_deferral_ledgers` is a source that keeps arriving just after its window closes; high on both
is an aggregator-side deferral problem; low on both with a rising `never_counted` is a source
being overwritten before it ever gets in.

## Never-counted vs. slow

A submission replaced by a newer one from the same source, before any aggregate counted it, is
recorded with `counted: false` and `inclusion_ledger: 0`, and emits
`SubmissionNeverCountedEvent`. It does **not** enter the latency percentiles — a pending sample
has no latency to rank — but it is counted in `never_counted`. A source with
`never_counted > 0` and a healthy `p50_ledgers` is being systematically overwritten, which no
amount of latency monitoring would otherwise reveal.

## Percentiles are reproducible

`get_latency_samples` returns the raw rolling window, oldest first, and `get_latency_report`
embeds the same window in `LatencyReport::window`. Every percentile in the report can therefore be
recomputed off-chain from stored samples:

- `p50_ledgers` / `p90_ledgers` — nearest-rank percentiles over the **counted** samples;
- `max_ledgers` — the largest counted latency;
- `avg_deferral_ledgers` — arithmetic mean of `deferral_ledgers` over counted samples.

`latency::percentile` uses nearest rank (the smallest value at or above which `p` % of the
samples fall), matching the quartile convention used elsewhere in this contract.

## Bounded storage

At most `MAX_SAMPLES = 16` samples are retained per `(source, asset)` pair in a ring buffer; the
oldest is dropped on overflow. Per-pair storage is therefore constant — it does not grow with the
source's history or with the length of the deployment. The rolling window is the unit of
analysis: a source's *recent* behaviour is what matters for freshness, and old samples would
describe a configuration that no longer exists.

Two further bounds keep the per-round entry count down, which matters because the network
caps an invocation at 100 ledger entries and a wide batch is close to that before this feature:

* A sample identical to the one already at the head of the ring is not rewritten. A source that
  is counted in the ledger it submitted, round after round, therefore holds one entry rather than
  gaining one per round.
* The ledger of the last published aggregate — the anchor the next round's deferral is measured
  from — is stored inside the asset's disagreement record (#494) rather than in a key of its own,
  so deferral tracking costs no additional entry.

## Sample lifecycle

1. A source submits; the submission ledger is stored in the `Submission` entry.
2. If a newer submission from the same source replaces it before an aggregate counts it, a
   never-counted sample is recorded and `SubmissionNeverCountedEvent` is emitted.
3. When an aggregate publishes, every counted submission gets a counted sample with its
   submission ledger, inclusion ledger, latency and the round's deferral, and
   `SubmissionLatencyEvent` is emitted per contributor.

Samples are recorded only for submissions that survived the robust pre-filter (#491) and the
deviation filter, so the latency population matches the population the aggregate was built from.

## Operator notes

- A rising `p90_ledgers` with a flat `p50_ledgers` is bimodal: most submissions are prompt and a
  tail is not. That is usually a batching relayer, not a misbehaving source.
- Compare `never_counted` across sources, not in absolute terms — a source submitting far more
  often will accumulate more overwritten samples.
- Nothing here penalises a source. Latency is reported, not enforced.
