# Off-Chain Anomaly Detection Service

Flags anomalous source submissions **before** they can affect on-chain aggregation. The
on-chain median in `contracts/price-oracle/src/prices.rs` already resists outliers
structurally, but this service runs earlier — as submissions stream in — and gives
operators source + deviation context to act on a misbehaving source faster than waiting
for it to show up in aggregate data.

Code: `services/anomaly_detection/` (`detector.py` — detection logic, `server.py` — CLI +
HTTP metrics/alerts endpoint). Shared event-reading code lives in `services/common/`.

## Architecture

```
oracle_events (Postgres/ClickHouse, see docs/event-streaming/)
        │  PriceSubmittedEvent rows, per docs/event-streaming/README.md envelope
        ▼
StreamingDetector.run()  ──buffers submissions per (asset, ledger) round──▶ score_round()
        │                                                                        │
        │                                                              MAD-based modified
        │                                                              z-score per source
        ▼                                                                        │
   AlertSink(s)  ◀───────────────────────────────────────────────────────────────┘
   (log / webhook / in-memory for the HTTP API)
```

## Detection method

For every round (all sources' submissions for one asset at one ledger), the service computes
a **modified z-score** using the median absolute deviation (MAD) — Iglewicz & Hoaglin's
robust outlier test:

```
z_i = (price_i − median) / (1.4826 × MAD)
```

MAD-based scoring is used instead of a mean/stdev z-score because it is itself robust to the
outliers it's trying to detect (a single wildly wrong submission can't drag the median or MAD
far, the way it would a mean/stdev).

A submission is flagged when **both**:

- `|z_i| >= mad_threshold` (default `3.5`, the standard Iglewicz–Hoaglin threshold), and
- `|deviation_bps| >= min_deviation_bps` (default `50` bps).

The second gate exists because MAD can collapse toward zero when peers happen to cluster very
tightly in a given round; without a floor, ordinary sub-basis-point rounding noise can produce
enormous but economically meaningless z-scores. Rounds with fewer than `min_peers` (default
`3`) submissions are skipped — there isn't enough peer data to judge one submission against.

### Optional secondary detector: Isolation Forest

`isolation_forest_outliers()` runs scikit-learn's Isolation Forest over a single source's own
recent price history to catch temporal anomalies a cross-sectional check can miss. It's fully
optional — `pip install scikit-learn` to enable it; without it, the function returns `None`
and the service runs unaffected using MAD detection alone.

## Running

```bash
pip install -r services/anomaly_detection/requirements.txt

# Seeded-anomaly demo — generates synthetic submissions with known injected
# outliers and prints every alert found (this is what CI/reviewers can run
# to confirm the service works end-to-end):
python -m services.anomaly_detection.server --demo

# Replay an exported JSONL of the oracle_events envelope (see
# docs/event-streaming/README.md):
python -m services.anomaly_detection.server --events-file events.jsonl

# Stream continuously from the Postgres event-streaming sink, alerting to a webhook:
python -m services.anomaly_detection.server \
  --postgres-dsn postgresql://user:pass@host/db \
  --webhook-url https://alerts.example.com/oracle-anomalies
```

## HTTP endpoints

The service listens on `--port` (default `9101`):

| Path | Description |
|------|-------------|
| `/health` | Liveness check |
| `/metrics` | Prometheus-format counters: `oracle_anomaly_alerts_total`, `oracle_anomaly_rounds_scored_total` |
| `/alerts` | JSON array of the most recent alerts (source, asset, price, z-score, deviation, ledger) |

## Integrating with the existing monitoring stack

Add `/metrics` as another Prometheus scrape target alongside
`scripts/price-submission-bot.py`'s health server (see `docs/monitoring/README.md`), and pair
it with the `OracleAnomalousSubmission` rule added to `docs/monitoring/alerts.yml`.

## Testing

```bash
python -m pytest services/anomaly_detection/tests/
```

Tests cover: the MAD scoring math on synthetic rounds, that a batch of seeded anomalies is
found with no false positives on the surrounding clean data, that the streaming detector's
output matches the batch detector's, and that the final buffered round is flushed correctly.
