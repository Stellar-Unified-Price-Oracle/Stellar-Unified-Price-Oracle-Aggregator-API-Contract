# Monitoring Dashboard — Stellar Unified Price Oracle

This directory contains a [Grafana](https://grafana.com/) dashboard template for monitoring the on-chain activity of the price oracle contract.

## Files

| File | Description |
|------|-------------|
| `grafana-dashboard.json` | Grafana dashboard definition (import-ready) |

## Dashboard Panels

| Panel Group | What it tracks |
|-------------|----------------|
| Contract Configuration | Registered sources count, registered assets count, decimals, min sources required, max history length |
| Latest Prices per Asset | Aggregated price time-series per asset; per-source price comparison |
| Source Submission Frequency | Submission rate by source and by asset |
| Aggregation Events | Price-update event rate; price delta (new − old) per asset |
| Error Events | Error rate by error code; top errors table; `NotAuthorized` / `InsufficientSources` highlight |
| Contract Configuration Changes | Count of admin, source, asset, and upgrade change events over 24 h |

## Prerequisites

- **Grafana ≥ 10** (or Grafana Cloud)
- **Prometheus** (or a compatible backend such as VictoriaMetrics)
- A **Stellar/Horizon indexer** that exposes oracle contract events as Prometheus metrics

## Metrics Reference

The dashboard expects the following metric names scraped from your indexer:

| Metric | Labels | Description |
|--------|--------|-------------|
| `oracle_registered_sources_total` | `contract_id` | Current number of registered oracle sources |
| `oracle_registered_assets_total` | `contract_id` | Current number of registered assets |
| `oracle_config_decimals` | `contract_id` | Configured decimal precision |
| `oracle_config_min_sources_required` | `contract_id` | Minimum sources required for aggregation |
| `oracle_config_max_history_length` | `contract_id` | Maximum history entries per asset |
| `oracle_latest_price` | `contract_id`, `asset` | Latest aggregated price |
| `oracle_source_price` | `contract_id`, `asset`, `source_name` | Latest per-source price |
| `oracle_price_submissions_total` | `contract_id`, `asset`, `source_name` | Cumulative price submissions |
| `oracle_price_updated_events_total` | `contract_id`, `asset` | Cumulative aggregation-update events |
| `oracle_price_delta` | `contract_id`, `asset` | Difference between latest and previous aggregated price |
| `oracle_errors_total` | `contract_id`, `error_code`, `error_name` | Cumulative contract error events |
| `oracle_config_change_events_total` | `contract_id`, `event_type` | Cumulative configuration-change events |

### Event types for `oracle_config_change_events_total`

`AdminChanged`, `SourceAdded`, `SourceRemoved`, `AssetRegistered`, `AssetUnregistered`, `ContractUpgraded`, `ContractInitialized`

### Error codes for `oracle_errors_total`

| `error_code` | `error_name` |
|---|---|
| 0 | NotAuthorized |
| 1 | AlreadyInitialized |
| 2 | AssetNotRegistered |
| 3 | AssetAlreadyRegistered |
| 4 | SourceAlreadyExists |
| 5 | SourceNotFound |
| 6 | InsufficientSources |
| 7 | InvalidPrice |
| 8 | NoData |

## Setup

### 1. Configure your indexer

Point a Horizon event-streaming indexer (e.g., a custom Node.js or Python service) at your contract ID and publish the metrics listed above to a Prometheus `/metrics` endpoint.

The contract emits the following events that map directly to the metrics:

| Contract event | Metrics updated |
|---|---|
| `PriceSubmittedEvent` | `oracle_price_submissions_total`, `oracle_source_price`, `oracle_latest_price` |
| `PriceUpdatedEvent` | `oracle_price_updated_events_total`, `oracle_price_delta`, `oracle_latest_price` |
| `SourceAddedEvent` / `SourceRemovedEvent` | `oracle_registered_sources_total`, `oracle_config_change_events_total` |
| `AssetRegisteredEvent` / `AssetUnregisteredEvent` | `oracle_registered_assets_total`, `oracle_config_change_events_total` |
| `AdminChangedEvent` | `oracle_config_change_events_total` |
| `ContractUpgradedEvent` | `oracle_config_change_events_total` |
| Error panics / traps | `oracle_errors_total` |

### 2. Add the Prometheus datasource in Grafana

1. Go to **Connections → Data sources → Add data source**.
2. Select **Prometheus**.
3. Set the **URL** to your Prometheus endpoint (e.g., `http://prometheus:9090`).
4. Click **Save & test**.

### 3. Import the dashboard

1. In Grafana, go to **Dashboards → Import**.
2. Upload `grafana-dashboard.json` or paste its contents.
3. Select the **Prometheus** datasource when prompted.
4. Click **Import**.

### 4. Select your contract

Use the **Contract ID** dropdown at the top of the dashboard to filter all panels to a specific deployed contract.

## Alerting (recommended)

Consider adding Grafana alerts on:

- `oracle_errors_total{error_name="InsufficientSources"}` rate > 0 for > 5 min — sources may be offline
- `oracle_price_submissions_total` rate = 0 for > 15 min per source — source may be down
- `oracle_latest_price` unchanged for > staleness window — stale price data

For the SLO-backed alert rules (freshness/deviation/source-health) see the **v2** section below.

---

## v2: SLO Dashboards, Alerts & Exporter

v2 upgrades the monitoring stack with a proper Prometheus metrics exporter, an
SLO-driven alert rule set derived directly from [`docs/SLA.md`](../SLA.md),
and three purpose-built Grafana dashboards. It supersedes the single v1
dashboard above for day-to-day operations; v1 is kept for backward
compatibility.

### Files

| File | Description |
|------|-------------|
| `../../scripts/metrics_exporter.py` | Prometheus `/metrics` exporter — polls Soroban RPC `getEvents` per contract and derives all metrics below |
| `../../scripts/metrics_exporter_config.example.toml` | Example exporter configuration |
| `../../scripts/test_metrics_exporter.py` | Unit tests for the event → metric derivation logic (`python3 -m unittest scripts/test_metrics_exporter.py`) |
| `alerts-v2.yml` | SLO alert rules for freshness, deviation, and source health, plus governance rules carried forward from v1 |
| `alerts-v2_test.yml` | `promtool test rules` synthetic-violation tests for `alerts-v2.yml` |
| `grafana-dashboard-v2-overview.json` | SLA status, freshness, deviation, and source-health summary |
| `grafana-dashboard-v2-sources.json` | Per-source drilldown: submission cadence, freshness, price comparison, deviation table |
| `grafana-dashboard-v2-governance.json` | Admin/upgrade audit trail, unauthorized-call rate, config drift over time |

### v2 metrics reference

In addition to every metric listed in the [Metrics Reference](#metrics-reference) above, the v2 exporter adds:

| Metric | Labels | Description |
|--------|--------|-------------|
| `oracle_active_sources_total` | `contract_id` | Registered sources minus suspended/inactive ones (SLA §2) |
| `oracle_paused` | `contract_id` | `1` while the contract is paused, else `0` (SLA §4.2) |
| `oracle_last_price_timestamp_seconds` | `contract_id`, `asset` | Unix timestamp of the last aggregate update — freshness source of truth (SLA §1.2) |
| `oracle_source_last_submission_timestamp_seconds` | `contract_id`, `asset`, `source_name` | Unix timestamp of a source's last submission |
| `oracle_source_deviation_bps` | `contract_id`, `asset`, `source_name` | Basis-point deviation of a source's last submission from the aggregate at submission time (SLA §3) |

### Running the exporter

```bash
pip install requests
cp scripts/metrics_exporter_config.example.toml scripts/metrics_exporter_config.toml
# edit contract_id(s) in the config, then:
python3 scripts/metrics_exporter.py --config scripts/metrics_exporter_config.toml
# metrics now available at http://localhost:9464/metrics
```

The exporter's XDR event decoding (`_decode_scval`) is left as an integration
point — wire it to `stellar_sdk.scval.to_native()` for a live deployment. The
event → metric derivation itself (`EventIndex`) is pure and fully covered by
`scripts/test_metrics_exporter.py`, independent of that decoding step.

### Validating the alert rules

```bash
promtool check rules docs/monitoring/alerts-v2.yml
promtool test rules docs/monitoring/alerts-v2_test.yml
```

The test file simulates a frozen price feed, a source drifting to 15%
deviation, an active-source count dropping below `min_sources_required`, all
sources disappearing, and an unplanned pause — and asserts each alert fires
(or, for the "not yet past `for:`" checkpoints, does not fire) at the correct
evaluation time. This is the synthetic-violation proof for the monitoring v2
acceptance criteria.

### Importing the v2 dashboards

Same steps as [Setup](#setup) above — import each of the three
`grafana-dashboard-v2-*.json` files individually, pointing them at the
Prometheus instance scraping `metrics_exporter.py`.
