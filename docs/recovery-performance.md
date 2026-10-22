# Cold-Start and Recovery Performance Baseline (#703)

Derived state (sources, price submissions, aggregated quotes, and oracle metadata) must be rebuildable deterministically and within bounded recovery budgets. Without explicit baselines and regression gates, cold-starts on disaster recovery or fresh node deployments can lead to prolonged outages and SLA violations.

The recovery baseline harness is implemented in [`services/capacity/recovery_baseline.py`](../services/capacity/recovery_baseline.py).

---

## 1. Recovery Modes

The baseline evaluates two distinct operational recovery modes:

### Fresh Deployment (`--mode fresh`)
* **Scope**: Complete state reconstruction from the beginning of history (genesis or initial tracked ledger).
* **Workload**: Creates new database schema, verifies all historical source configurations, records submissions, and computes all historical aggregates sequentially.
* **Evaluation Metrics**:
  * Wall-clock reconstruction duration ($T_{\text{wall}}$)
  * CPU time consumed ($T_{\text{cpu}}$)
  * Peak memory footprint (RSS)
  * Derived state disk footprint growth
  * Rebuild throughput (events/second and ledgers/second)

### Snapshot Recovery (`--mode snapshot`)
* **Scope**: Resuming state from a validated point-in-time snapshot, replaying only recent uncommitted ledger delta events up to the ledger tip.
* **Workload**: Restores base state directly from snapshot metadata, bypasses bulk ingestion, and replays only the recent window.
* **Evaluation Metrics**:
  * Snapshot load duration + delta replay duration
  * Incremental memory and storage overhead
  * Time to live tip parity

---

## 2. Performance Budgets & Regression Thresholds

The recovery framework enforces strict performance budgets via `RecoveryBudget`:

| Metric | Fresh Deployment Target | Snapshot Target | Regression Action |
|---|---|---|---|
| **Recovery Rate** | $\le 5.0\text{ s}$ per 1,000 events | $\le 1.0\text{ s}$ per 1,000 events | Fail CI / Alert Ops |
| **Max Peak Memory** | $\le 128\text{ MB}$ | $\le 64\text{ MB}$ | Fail CI / Alert Ops |
| **Max Total Duration** | $\le 300\text{ s}$ (full backfill) | $\le 30\text{ s}$ (delta replay) | Fail CI / Block Deploy |

If any measured metric violates these bounds, the baseline marks `passed_threshold = false` and exits with a non-zero status code when `--fail-on-regression` is enabled.

---

## 3. CLI Usage

Run a fresh recovery benchmark over 2,000 synthetic events:

```bash
python -m services.capacity.recovery_baseline --mode fresh --events 2000
```

Run a snapshot-based replay recovery test:

```bash
python -m services.capacity.recovery_baseline --mode snapshot --events 1000 --json
```

Export Prometheus metrics for scraper ingestion:

```bash
python -m services.capacity.recovery_baseline --mode fresh --events 500 --metrics
```

---

## 4. Machine-Readable Metrics

### Prometheus Export Format
```text
# HELP oracle_recovery_wall_time_seconds Time taken to rebuild derived state
# TYPE oracle_recovery_wall_time_seconds gauge
oracle_recovery_wall_time_seconds{mode="fresh"} 0.045
# HELP oracle_recovery_cpu_time_seconds CPU time consumed during rebuild
# TYPE oracle_recovery_cpu_time_seconds gauge
oracle_recovery_cpu_time_seconds{mode="fresh"} 0.031
# HELP oracle_recovery_events_per_second Event processing throughput
# TYPE oracle_recovery_events_per_second gauge
oracle_recovery_events_per_second{mode="fresh"} 22222.2
# HELP oracle_recovery_peak_memory_mb Peak memory during rebuild
# TYPE oracle_recovery_peak_memory_mb gauge
oracle_recovery_peak_memory_mb{mode="fresh"} 1.25
# HELP oracle_recovery_derived_state_bytes Resulting database file size in bytes
# TYPE oracle_recovery_derived_state_bytes gauge
oracle_recovery_derived_state_bytes{mode="fresh"} 98304
# HELP oracle_recovery_budget_pass 1 if within performance threshold, 0 if violated
# TYPE oracle_recovery_budget_pass gauge
oracle_recovery_budget_pass{mode="fresh"} 1
```

### JSON Output
```json
{
  "mode": "fresh",
  "events_processed": 1000,
  "ledgers_processed": 800,
  "wall_time_seconds": 0.045,
  "cpu_time_seconds": 0.031,
  "peak_memory_mb": 1.25,
  "derived_state_bytes": 98304,
  "events_per_second": 22222.2,
  "passed_threshold": true,
  "violations": []
}
```
