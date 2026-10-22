# Price Quality Metrics Schema — Comprehensive Dashboard

**Issue:** #520 (placeholder) — unified quality metrics framework covering correction rate, price freshness, source disagreement, and missing-data trends.

This document defines the four metric categories consumed by the dashboard, their calculation logic, aggregation windows, reconciliation rules, and interpretation guidelines.

## Overview: Four Metric Categories

| Category | What it measures | Why it matters | Scope |
|----------|------------------|----------------|-------|
| **Correction Rate** | How often published prices are revised | Leading indicator of data-quality degradation | Per-asset, per-source, per-cause |
| **Freshness** | How stale the published price is | Detects source outages and submission delays | Per-asset (all sources), per-source-per-asset |
| **Source Disagreement** | How much sources differ from the aggregate | Identifies sources that are systematically out-of-sync | Per-asset, per-source |
| **Missing Data** | What fraction of expected submissions are absent | Measures submission compliance and reliability | Per-source, per-asset, by aggregation window |

---

## 1. Correction Rate (Existing in #499)

**What:** How many times a published aggregate price is subsequently revised, normalized by publication volume.

**Why:** A rising correction rate is a leading indicator before a source fails or the aggregation logic breaks. Separating causes (authorized corrections vs. source-driven vs. unattributed) distinguishes a controlled operator response from an incident.

**Calculation:**
- **Denominator:** Ledgers where the asset published an aggregate (count distinct ledgers in window).
- **Numerator:** Revisions filed in that window, classified by cause.
- **Rate:** `revisions / max(publications, min_publications)` (volume-normalized).
- **Windows:** Recent (24 h) vs. baseline (previous 24 h).
- **Trend:** `trend_multiplier = recent_rate / max(baseline_rate, BASELINE_RATE_FLOOR_PERMILLE)`.
- **Alert:** `sustained_increase` if `trend_multiplier >= 2.0` AND `recent_revisions >= 3` AND `recent_publications >= 20`.

**Outputs:**

```python
@dataclass
class CorrectionRateMetrics:
    asset: str
    recent_revisions: int
    recent_publications: int
    recent_rate_permille: float
    baseline_rate_permille: float
    trend_multiplier: float
    sustained_increase: bool
    low_volume: bool
    
    # Per-cause breakdown (new)
    recent_authorized_revisions: int
    recent_source_revisions: int
    recent_unattributed_revisions: int
    
    # Reconciliation (new)
    on_chain_revision_count: int
    reconciliation_gap: int  # on_chain - observed
```

**Existing Coverage:** See `docs/price-revision-metrics.md`, `services/revision_metrics/metrics.py`.

---

## 2. Price Freshness

**What:** For each asset, how much time has elapsed since the most recent price publication.

**Why:** Detects source outages (all sources stale simultaneously) and submission delays (per-source latency).

**Calculation:**

### 2A. Asset Freshness
- **Metric:** `time_since_last_aggregate_ledger` (seconds).
- **Staleness Class:**
  - `Fresh`: age ≤ `resolution` (contract's price-resolution window).
  - `Stale`: age > `resolution` AND age ≤ `max_staleness` (SLA-defined).
  - `Critical`: age > `max_staleness`.
- **SLA Window:** Default 1 h; operator-configurable per asset.
- **Rate:** Count of time-windows (5-minute buckets) where asset was in each class.

### 2B. Per-Source Freshness (for asset)
- **Metric:** `time_since_last_submission` per (asset, source) pair (seconds).
- **Compliance Class:**
  - `Compliant`: source submitted within `submission_interval` (configured per source or global).
  - `Overdue`: submitted but older than interval; submission is aged out.
  - `Missing`: no submission in the window.
- **Rate:** Count of submission opportunities (intervals) where source was in each class.

**Outputs:**

```python
@dataclass
class FreshnessMetrics:
    asset: str
    
    # Asset-level freshness
    time_since_last_aggregate_secs: int
    staleness_class: str  # "Fresh" | "Stale" | "Critical"
    time_in_fresh_pct: float      # % of window in Fresh class
    time_in_stale_pct: float      # % of window in Stale class
    time_in_critical_pct: float   # % of window in Critical class
    freshness_breach_count: int   # # of times crossed into Critical
    
    # Per-source freshness (per source)
    per_source: Dict[str, PerSourceFreshness]

@dataclass
class PerSourceFreshness:
    source: str
    asset: str
    time_since_last_submission_secs: int
    submission_compliance_class: str  # "Compliant" | "Overdue" | "Missing"
    expected_intervals_in_window: int
    compliant_intervals: int
    overdue_intervals: int
    missing_intervals: int
```

**Alert Thresholds:**
- Asset critical: `time_in_critical_pct >= 10%` (10% of window in critical).
- Source missing: `missing_intervals >= 3` consecutive.

---

## 3. Source Disagreement

**What:** How much each source's submitted price deviates from the published aggregate (median).

**Why:** Identifies sources that are systematically out-of-sync (outlier in their own right) vs. occasional variance (normal market movement).

**Calculation:**

- **Metric:** For each submission in window, `deviation_bps = abs(source_price - aggregate_price) * 10_000 / aggregate_price`.
- **Aggregation:** Per-source statistics over a 24-hour window.
  - `max_deviation_bps` (worst single submission).
  - `mean_deviation_bps` (average across all submissions).
  - `p95_deviation_bps` (95th percentile).
  - `submission_count` (number of submissions counted).
  - `outlier_count` (submissions flagged as pre-filter outliers).
- **Comparison:** Against SLA deviation threshold (default 500 bps = 5%).
  - `above_threshold_count` (submissions exceeding threshold).
  - `above_threshold_pct` (% of submissions above threshold).

**Outputs:**

```python
@dataclass
class SourceDisagreementMetrics:
    asset: str
    source: str
    
    submission_count: int
    max_deviation_bps: int
    mean_deviation_bps: float
    p95_deviation_bps: int
    
    outlier_count: int
    above_threshold_count: int
    above_threshold_pct: float  # Fraction of submissions
    
    systematically_divergent: bool  # True if p95 > threshold
    incident_level: str  # "Normal" | "Warning" | "Critical"
```

**Alert Thresholds:**
- `incident_level = "Warning"` if `mean_deviation_bps > 300 bps` OR `above_threshold_pct >= 20%`.
- `incident_level = "Critical"` if `mean_deviation_bps > 500 bps` OR `above_threshold_pct >= 50%`.

---

## 4. Missing Data (per-source aggregation window coverage)

**What:** For a given (source, asset) pair in a time window, what fraction of expected submissions were received?

**Why:** Measures source compliance and identifies partial outages (source still submitting but less frequently than expected).

**Calculation:**

- **Expected Interval:** Configured per source or globally (e.g., "submit every 60 seconds").
- **Observation Window:** Rolling 24-hour window, divided into fixed-size intervals (e.g., 5-minute buckets).
- **Expected Submissions:** `floor(window_duration_secs / interval_secs)`.
- **Actual Submissions:** Count of (asset, source) submissions in window.
- **Missing Data Rate:** `(expected - actual) / expected`.
- **Consecutive Miss Streak:** Count of consecutive intervals with no submission (used to detect hard outages).

**Outputs:**

```python
@dataclass
class MissingDataMetrics:
    source: str
    asset: str
    
    expected_submissions: int
    actual_submissions: int
    missing_count: int
    missing_rate_pct: float
    
    consecutive_miss_streak: int  # Longest stretch without submission
    
    health_status: str  # "Healthy" | "Degraded" | "Offline"
    
    # Recovery stats (for long-offline sources)
    first_outage_started_ledger: int
    latest_recovery_attempt_ledger: int
```

**Alert Thresholds:**
- `health_status = "Degraded"` if `missing_rate_pct >= 20%` OR `consecutive_miss_streak >= 5`.
- `health_status = "Offline"` if `missing_rate_pct >= 80%` OR `consecutive_miss_streak >= 20`.

---

## Aggregation Windows and Reconciliation

### Window Definition
All four categories use the same time windows:
- **Observation Period:** Rolling 24-hour window ending at the current ledger close time.
- **Comparison Baseline:** The 24-hour period immediately before the observation period.
- **Granularity:** 5-minute fixed intervals for per-interval calculations (freshness, missing data).

### Reconciliation Rules

| Metric | Reconciliation Check | Gap Detection |
|--------|----------------------|----------------|
| **Correction Rate** | `on_chain_revision_count` vs. observed event count | Unindexed corrections; causes gap in `reconciliation_gap` |
| **Freshness** | Query ledger close times from RPC; verify staleness against `resolution` config | Time drift in exporter clock |
| **Disagreement** | Compare submission prices against `get_aggregate_price` RPC call | Out-of-order event indexing |
| **Missing Data** | Count expected intervals against actual submissions; reconstruct from `PriceSubmittedEvent` stream | Lost indexer events; partial reorg |

**Reconciliation Output:**
```python
@dataclass
class ReconciliationReport:
    asset: str
    report_timestamp: int  # Unix seconds
    
    # Per-category reconciliation status
    correction_rate_reconciled: bool
    correction_rate_gap: int  # Discrepancy in revision count
    
    freshness_reconciled: bool
    freshness_time_drift_secs: int  # Clock skew detected
    
    disagreement_reconciled: bool
    disagreement_price_mismatches: int  # Submissions vs. aggregate mismatch
    
    missing_data_reconciled: bool
    missing_data_events_lost: int  # Estimated lost submission events
    
    overall_health: str  # "Healthy" | "Degraded" | "UnderReconciliation"
```

---

## Dashboard Implementation

### Data Flow
1. **Event Indexer** (metrics_exporter.py) consumes Soroban event stream.
2. **Metrics Processor** (revision_metrics service + new modules) computes all four categories.
3. **Prometheus Exporter** emits time-series metrics.
4. **Grafana Dashboard** visualizes and alerts.

### Metric Names (Prometheus)

```
# Correction Rate
oracle_correction_rate_permille{asset, cause}
oracle_correction_trend_multiplier{asset}
oracle_corrections_sustained_increase{asset}

# Freshness
oracle_asset_freshness_secs{asset}
oracle_asset_freshness_class{asset}  # 0=Fresh, 1=Stale, 2=Critical
oracle_source_submission_latency_secs{asset, source}
oracle_source_compliance_class{asset, source}  # 0=Compliant, 1=Overdue, 2=Missing

# Disagreement
oracle_source_disagreement_mean_bps{asset, source}
oracle_source_disagreement_p95_bps{asset, source}
oracle_source_above_deviation_threshold_pct{asset, source}

# Missing Data
oracle_source_missing_data_rate_pct{asset, source}
oracle_source_consecutive_miss_streak{asset, source}
oracle_source_health_status{asset, source}  # 0=Healthy, 1=Degraded, 2=Offline

# Reconciliation
oracle_reconciliation_correction_rate_gap{asset}
oracle_reconciliation_freshness_drift_secs{asset}
oracle_reconciliation_status{asset}  # 0=Healthy, 1=Degraded, 2=UnderReconciliation
```

---

## Operational Use

### Alerting Rules
See `docs/monitoring/alerts-quality.yml` (new):
- Correction-rate sustained-increase: Page (severity critical).
- Freshness critical: Page (severity critical).
- Disagreement incident: Warning, then critical escalation.
- Missing data offline: Page (severity critical).
- Reconciliation gap detected: Warning.

### Runbook Coverage
Each paging alert has an entry in `docs/runbook.md`:
- **Correction rate spike:** Check on-chain revisions; identify filer; review correction chain.
- **Freshness critical:** Check source heartbeats; RPC connectivity; submission-interval config.
- **Disagreement incident:** Review source's upstream data; compare with published aggregate.
- **Missing data offline:** Verify source's RPC endpoint; check auth permissions; restart if needed.
- **Reconciliation gap:** Re-index from scratch; verify event stream integrity.

---

## Out of Scope

- Automatically correcting prices or removing sources.
- Changing submission intervals or deviation thresholds without operator review.
- Rewriting correction history or silencing an alert by suppressing events.

## See Also

- `docs/price-revision-metrics.md` — Detailed correction-rate design.
- `docs/monitoring/README.md` — Existing Prometheus/Grafana setup.
- `scripts/metrics_exporter.py` — Event indexer and Prometheus exporter.
- `services/revision_metrics/metrics.py` — Correction-rate calculation service.
