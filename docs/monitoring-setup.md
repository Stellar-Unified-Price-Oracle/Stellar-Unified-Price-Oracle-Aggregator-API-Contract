# Monitoring and Alerting Setup Guide

This guide covers how to set up production monitoring for the deployed Stellar Unified Price Oracle Aggregator contract — including event indexing, metric collection, alert thresholds, notification channels, and dashboard setup.

---

## Architecture Overview

```
Stellar Network
    │
    ▼
Horizon / RPC Node  ──►  Event Indexer  ──►  Prometheus  ──►  Grafana
                                                  │
                                                  ▼
                                          Alertmanager
                                          (Email / Slack / PagerDuty)
```

---

## 1. Event Monitoring Setup

The contract emits the following on-chain events. Your indexer must stream and parse these.

| Event | Key fields | What to watch |
|---|---|---|
| `PriceSubmittedEvent` | asset, source, price, timestamp | Submission rate per source |
| `PriceUpdatedEvent` | asset, new_price, old_price, timestamp | Price delta, update rate |
| `SourceAddedEvent` | source, name | Source roster changes |
| `SourceRemovedEvent` | source | Source roster changes |
| `AssetRegisteredEvent` | asset | Asset roster changes |
| `AssetUnregisteredEvent` | asset | Asset roster changes |
| `AdminChangedEvent` | old_admin, new_admin | Unexpected admin transfers |
| `ContractUpgradedEvent` | new_wasm_hash | Unplanned upgrades |

### Setting up a Horizon event indexer

Use the Horizon `/accounts/{id}/effects` or the RPC `getEvents` endpoint to stream contract events:

```bash
# Stream events for your contract via RPC
curl -X POST https://soroban-testnet.stellar.org \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "getEvents",
    "params": {
      "startLedger": 0,
      "filters": [{"type": "contract", "contractIds": ["<CONTRACT_ID>"]}],
      "pagination": {"limit": 100}
    }
  }'
```

A minimal Node.js indexer polling loop:

```js
import { SorobanRpc, scValToNative } from "@stellar/stellar-sdk";
import { register, Gauge, Counter } from "prom-client";

const rpc = new SorobanRpc.Server("https://soroban-testnet.stellar.org");
const CONTRACT_ID = process.env.CONTRACT_ID;

const submissionsTotal = new Counter({
  name: "oracle_price_submissions_total",
  help: "Cumulative price submissions",
  labelNames: ["asset", "source_name"],
});

// Poll getEvents, parse topics, increment metrics
async function poll(startLedger) {
  const res = await rpc.getEvents({ startLedger, filters: [{ type: "contract", contractIds: [CONTRACT_ID] }] });
  for (const event of res.events) {
    const topics = event.topic.map(scValToNative);
    if (topics[0] === "PriceSubmitted") {
      submissionsTotal.inc({ asset: topics[1], source_name: topics[2] });
    }
    // handle other event types similarly
  }
  return res.latestLedger;
}
```

---

## 2. Key Metrics to Track

| Metric | Labels | Alert if |
|---|---|---|
| `oracle_price_submissions_total` | `asset`, `source_name` | Rate = 0 for any source > 15 min |
| `oracle_latest_price` | `asset` | Unchanged > staleness window |
| `oracle_price_delta` | `asset` | Sudden jump > 10% in one interval |
| `oracle_registered_sources_total` | — | Drops unexpectedly |
| `oracle_errors_total` | `error_code`, `error_name` | `InsufficientSources` rate > 0 for > 5 min |
| `oracle_config_change_events_total` | `event_type` | Any `AdminChanged` or `ContractUpgraded` |

Full metrics reference: [`docs/monitoring/README.md`](./monitoring/README.md)

---

## 3. Alert Thresholds and Severity Levels

| Alert | Condition | Severity | Action |
|---|---|---|---|
| **SourceDown** | `rate(oracle_price_submissions_total[15m]) == 0` for a source | Warning | Notify source operator |
| **AllSourcesDown** | `oracle_registered_sources_total == 0` | Critical | Page on-call immediately |
| **InsufficientSources** | `rate(oracle_errors_total{error_name="InsufficientSources"}[5m]) > 0` | Critical | Page on-call |
| **StalePriceData** | `oracle_latest_price` unchanged > 600s | Warning | Investigate source submissions |
| **PriceSpike** | Price delta > 10% vs last reading | Warning | Verify against external feeds |
| **UnauthorizedAttempt** | `rate(oracle_errors_total{error_name="NotAuthorized"}[5m]) > 5` | Warning | Investigate caller |
| **AdminChanged** | `increase(oracle_config_change_events_total{event_type="AdminChanged"}[1m]) > 0` | Critical | Verify intentional |
| **ContractUpgraded** | `increase(oracle_config_change_events_total{event_type="ContractUpgraded"}[1m]) > 0` | Critical | Verify intentional |

### Prometheus alerting rules (`alerts.yml`)

```yaml
groups:
  - name: oracle
    rules:
      - alert: OracleSourceDown
        expr: rate(oracle_price_submissions_total[15m]) == 0
        for: 15m
        labels:
          severity: warning
        annotations:
          summary: "Oracle source {{ $labels.source_name }} is not submitting"

      - alert: OracleInsufficientSources
        expr: rate(oracle_errors_total{error_name="InsufficientSources"}[5m]) > 0
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Oracle aggregation failing — not enough active sources"

      - alert: OracleAdminChanged
        expr: increase(oracle_config_change_events_total{event_type="AdminChanged"}[1m]) > 0
        labels:
          severity: critical
        annotations:
          summary: "Oracle admin address changed — verify this was intentional"

      - alert: OracleContractUpgraded
        expr: increase(oracle_config_change_events_total{event_type="ContractUpgraded"}[1m]) > 0
        labels:
          severity: critical
        annotations:
          summary: "Oracle contract upgraded — verify wasm hash"
```

---

## 4. Notification Channels

### Email (Alertmanager)

```yaml
# alertmanager.yml
route:
  receiver: email-ops
receivers:
  - name: email-ops
    email_configs:
      - to: ops@yourorg.com
        from: alerts@yourorg.com
        smarthost: smtp.yourorg.com:587
        auth_username: alerts@yourorg.com
        auth_password: "<smtp-password>"
```

### Slack

```yaml
receivers:
  - name: slack-ops
    slack_configs:
      - api_url: "https://hooks.slack.com/services/T.../B.../..."
        channel: "#oracle-alerts"
        text: "{{ .CommonAnnotations.summary }}"
```

### PagerDuty

```yaml
receivers:
  - name: pagerduty-critical
    pagerduty_configs:
      - routing_key: "<PAGERDUTY_INTEGRATION_KEY>"
        description: "{{ .CommonAnnotations.summary }}"
```

Route critical alerts to PagerDuty, warnings to Slack:

```yaml
route:
  routes:
    - match:
        severity: critical
      receiver: pagerduty-critical
    - match:
        severity: warning
      receiver: slack-ops
```

---

## 5. Dashboard Setup (Grafana)

A ready-to-import Grafana dashboard is in [`docs/monitoring/grafana-dashboard.json`](./monitoring/grafana-dashboard.json).

### Quick setup

1. **Add Prometheus datasource**
   - Grafana → Connections → Data sources → Add → Prometheus
   - URL: `http://prometheus:9090`
   - Save & test

2. **Import the dashboard**
   - Grafana → Dashboards → Import
   - Upload `docs/monitoring/grafana-dashboard.json`
   - Select your Prometheus datasource

3. **Filter by contract**
   - Use the `Contract ID` variable dropdown at the top of the dashboard

### Panel groups

| Panel group | What it shows |
|---|---|
| Contract Configuration | Source count, asset count, decimals, min sources, max history |
| Latest Prices | Aggregated price per asset; per-source price comparison |
| Submission Activity | Submission rate by source and asset |
| Aggregation Events | Price-update rate; price delta per asset |
| Errors | Error rate by code; top error table |
| Config Changes | Admin, source, asset, upgrade change counts (24 h) |

---

## 6. Runbook for Common Alerts

### OracleSourceDown

1. Check the source's submission service logs for errors.
2. Verify the source account is funded (transaction fees).
3. Confirm the RPC endpoint the source uses is reachable.
4. If the source is unrecoverable, notify the contract admin to call `remove_source()` and onboard a replacement.

### OracleInsufficientSources

1. Run `get_oracle_sources()` to see which sources are registered.
2. For each source, check `oracle_price_submissions_total` — identify which are down.
3. Contact down sources. If multiple are down simultaneously, escalate to on-call.
4. Temporary mitigation: admin can lower `min_sources_required` if one source is permanently unavailable.

### OracleAdminChanged / OracleContractUpgraded

1. Check the transaction hash from the event.
2. Verify the signing key matches the expected admin.
3. If unauthorized, treat as a security incident — revoke access and audit.

### StalePriceData

1. Check submission activity: `rate(oracle_price_submissions_total[5m])`.
2. If submissions are happening but price isn't updating, check `min_sources_required` — it may not be met.
3. If no submissions, treat as OracleSourceDown.

---

## Further Reading

- [Source Onboarding Guide](./source-onboarding.md)
- [Gas Usage Reference](./gas-usage.md)
- [Grafana Dashboard Reference](./monitoring/README.md)
