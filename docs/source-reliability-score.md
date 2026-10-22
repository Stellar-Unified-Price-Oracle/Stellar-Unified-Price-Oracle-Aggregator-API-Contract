# Off-Chain Source Reliability Score

Publishes a per-source reliability score combining submission **uptime**, **freshness**, and
**accuracy vs. the aggregate**, computed from indexed events. This is the transparency layer
underpinning source incentives (the on-chain stake/slash system in
`contracts/price-oracle/src/reputation.rs`, and the per-round contribution-quality scoring in
`contracts/price-oracle/src/contribution_quality.rs`) — a transparent score drives better
source behavior.

Code: `services/reliability_score/` (`scorer.py` — scoring logic, `server.py` — HTTP API +
dashboard).

## Scoring formula

For each `(asset, source)` pair, over every round the asset was aggregated in:

| Dimension | Weight | Definition |
|---|---|---|
| Uptime | 30% | `100 × (rounds the source submitted in / rounds the asset was aggregated in)` |
| Freshness | 20% | `100 × (1 − lag / max_lag_secs)` averaged over participated rounds, where `lag = round_timestamp − submission_timestamp`, clamped to `[0, 100]` (default `max_lag_secs = 300`) |
| Accuracy | 50% | `100 × (1 − deviation_bps / max_deviation_bps)` averaged over participated rounds, where `deviation_bps` is the submission's distance from that round's aggregate price, clamped to `[0, 100]` (default `max_deviation_bps = 5000`, i.e. 50%, mirroring `reputation.rs`) |

```
composite_score = 0.30 × uptime_score + 0.20 × freshness_score + 0.50 × accuracy_score
```

Weights are configurable via `ScoreWeights` (must sum to `1.0`).

## Reproducibility

`compute_scores(submissions, aggregations)` is a pure function of the indexed event stream: the
same `PriceSubmittedEvent` / `PriceAggregatedEvent` history always produces the same scores, so
any operator (or a source disputing its own score) can independently recompute and verify it
from the same `oracle_events` export. `derive_aggregations()` reconstructs the per-round
aggregate (median price, `PriceAggregatedEvent`-equivalent) directly from submissions for
deployments that only index the raw submission stream.

## Running

```bash
pip install -r services/reliability_score/requirements.txt

# Demo: serves scores for a synthetic source population (one source with
# scheduled downtime, one with a seeded accuracy problem) — see how the
# median-based aggregate mutes a single deviant source's own accuracy hit,
# the same robustness property the on-chain median relies on.
python -m services.reliability_score.server --demo
# then open http://localhost:9103/ or:
curl 'http://localhost:9103/scores?asset=<asset>'

# Against a real exported submission history:
python -m services.reliability_score.server --events-file events.jsonl
```

## HTTP endpoints

The service listens on `--port` (default `9103`):

| Path | Description |
|------|-------------|
| `/` | Dashboard — lists every source's scores for an asset, sorted by composite score |
| `/scores?asset=` | JSON array of every source's score for `asset` |
| `/score?asset=&source=` | JSON score for a single `(asset, source)` — the off-chain equivalent of an on-chain `get_source_score(asset, source)` view; an on-chain view could wrap this once `#[contractimpl]` wiring on the contract is fully restored (see `feat: add explicit prune_history` commit on this branch for the wiring issues found there) |
| `/metrics` | Prometheus counter: `oracle_reliability_scores_computed_total` |
| `/health` | Liveness check |

## Testing

```bash
python -m pytest services/reliability_score/test/
```

Covers each scoring dimension in isolation (a source with scheduled downtime gets a
proportionally lower uptime score; a slower submitter scores lower on freshness than a faster
one at a precisely controlled lag; an inaccurate source scores lower than an accurate one),
determinism (`compute_scores` on the same input twice returns identical results), and an
end-to-end HTTP integration test including the `get_source_score`-equivalent endpoint.
