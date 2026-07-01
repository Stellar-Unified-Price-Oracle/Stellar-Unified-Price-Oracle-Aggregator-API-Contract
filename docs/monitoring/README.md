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
