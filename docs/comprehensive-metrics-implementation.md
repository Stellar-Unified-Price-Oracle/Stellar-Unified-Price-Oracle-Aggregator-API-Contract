# Comprehensive Price-Quality Metrics Implementation

**Issue:** #520 — Unified quality metrics dashboard covering correction rate, freshness, source disagreement, and missing-data trends.

## Overview

This document summarizes the implementation of a comprehensive four-category metrics system for the Stellar Unified Price Oracle, extending the existing correction-rate metrics (#499) to include freshness, source disagreement, and missing-data visibility.

## Implementation Status

### ✅ Completed

#### 1. Schema Design (docs/quality-metrics-schema.md)
- Comprehensive metrics schema defining all four metric categories
- Calculation methods and aggregation windows for each category
- Reconciliation rules to detect indexing gaps and clock skew
- Prometheus metric names and alert thresholds
- Operational use cases and interpretation guidelines

#### 2. Service Extension (services/revision_metrics/quality_metrics.py)
- **Freshness Metrics:** Time since last aggregate publication; per-source submission compliance
  - StalenessClass: Fresh | Stale | Critical
  - ComplianceClass: Compliant | Overdue | Missing
  - FreshnessMetrics dataclass with asset-level and per-source breakdown
  
- **Disagreement Metrics:** Source deviations from published aggregate
  - Calculated in basis points (1 bps = 0.01%)
  - Max, mean, and p95 deviation statistics
  - Incident level classification: Normal | Warning | Critical
  - DisagreementMetrics dataclass with per-source analysis
  
- **Missing Data Metrics:** Submission compliance and health tracking
  - HealthStatus: Healthy | Degraded | Offline
  - Expected vs. actual submission counts
  - Consecutive miss streak tracking
  - MissingDataMetrics dataclass with recovery tracking
  
- **Configuration Classes:**
  - FreshnessThresholds: resolution_secs, max_staleness_secs, submission_interval_secs
  - DisagreementThresholds: max_deviation_bps, warning/critical thresholds
  - MissingDataThresholds: submission_interval_secs, streak/rate-based classification

- **Core Function:** `compute_quality_metrics()` 
  - Pure function of event stream + configuration
  - Returns QualityMetricsBatch with all three categories
  - JSON-serializable output for Prometheus export

#### 3. Test Coverage (services/revision_metrics/test/test_quality_metrics.py)
- 18 comprehensive test cases covering:
  - Freshness classification (Fresh, Stale, Critical)
  - Per-source submission compliance
  - Disagreement detection and severity classification
  - Missing-data health status determination
  - Multi-asset and multi-source scenarios
  - Threshold customization
  - JSON serialization and batch completeness
  - Empty stream handling

### 🔄 In Progress / Next Steps

#### 3. Metrics Exporter Integration (scripts/metrics_exporter.py)
**Required changes:**
- Add FreshessMetrics event handling (PriceAggregated timestamps)
- Add DisagreementMetrics calculation (per-submission deviation)
- Add MissingDataMetrics tracking (submission intervals)
- Emit Prometheus metrics:
  ```
  oracle_asset_freshness_secs{asset}
  oracle_asset_freshness_class{asset}
  oracle_source_compliance_pct{asset, source}
  oracle_source_disagreement_mean_bps{asset, source}
  oracle_source_disagreement_p95_bps{asset, source}
  oracle_source_health_status{asset, source}
  oracle_source_missing_rate_pct{asset, source}
  ```

#### 4. Metrics Documentation (docs/monitoring/quality-metrics-guide.md)
**To be created:**
- Consumer guide for each metric category
- Interpretation examples (what each alert means)
- Remediation playbooks
- Dashboard panel descriptions
- Alert configuration examples

#### 5. Dashboard and Reconciliation Testing
**To be implemented:**
- Grafana dashboard with four metric panels
- Fixture-based reconciliation tests
- Known-good dashboard values for regression testing
- Incomplete source window detection

#### 6. Automated Coverage Tests
**To be added:**
- Unit tests for each metric calculation
- Property-based tests for consistency
- Reconciliation gap detection tests
- Edge cases (zero submissions, clock skew, reorgs)

#### 7. Operator Documentation (docs/runbook.md extensions)
**Alert entries to add:**
- Freshness critical: Check source heartbeats, RPC connectivity
- Disagreement incident: Review source upstream, audit aggregate
- Missing data offline: Verify source RPC, check auth, restart if needed
- Reconciliation gap: Re-index from scratch, verify event stream integrity

#### 8. Build and Test Verification
**To run:**
```bash
# Build
make build

# Run all tests (including quality_metrics tests once fixtures are fixed)
make test

# Lint
cargo clippy
cargo fmt

# Security gates
make attack-gate
make fuzz-gate
```

---

## Data Flow

```
On-chain Events
  ↓
Event Stream (JSONL)
  ↓
metrics_exporter.py
  ├─ EventIndex (in-memory processing)
  ├─ quality_metrics.py compute_quality_metrics()
  │  ├─ Freshness calculation
  │  ├─ Disagreement calculation
  │  └─ Missing data calculation
  └─ Prometheus Exporter (/metrics)
     ↓
  Prometheus Scrape
     ↓
  Grafana Dashboard
     ↓
  Alerts (page, warning)
     ↓
  Operator Runbook
```

---

## Key Design Decisions

### 1. Pure Functions
All metric calculations are pure functions of the event stream + configuration + `now`. This ensures:
- Reproducibility (same input → same output)
- Verifiability (can recompute offline)
- No side effects (safe to re-run)

### 2. Normalized Windows
All three categories use the same rolling 24-hour window and 5-minute interval granularity, ensuring consistency and reducing operator confusion.

### 3. Reconciliation Signals
Each category includes reconciliation checks that detect:
- Missing events (lost from indexer)
- Out-of-order events (reorgs)
- Clock skew (exporter clock vs. ledger time)

### 4. Configurable Thresholds
Operators can tune thresholds per deployment:
- resolution_secs (contract-wide SLA)
- submission_interval_secs (per-source or global)
- max_deviation_bps (per-asset or per-source overrides)

### 5. Composable Metrics
Each metric category is independent but shares:
- Event parsing infrastructure (services.common.events)
- Window calculation logic
- Serialization format (dict → JSON)
- Alert thresholds and runbook linkage

---

## Acceptance Criteria

### ✅ Schema & Documentation
- [x] Four metric categories defined with calculation methods
- [x] Aggregation windows and reconciliation rules specified
- [x] Prometheus metric names documented
- [x] Alert thresholds and operational use cases described
- [ ] Consumer guide and interpretation examples (in progress)

### ✅ Service Implementation
- [x] Freshness metrics computed (asset + per-source)
- [x] Disagreement metrics computed (per-source deviation analysis)
- [x] Missing data metrics computed (compliance & health tracking)
- [x] QualityMetricsBatch with JSON serialization
- [ ] Integrated into metrics_exporter.py (in progress)

### 🔄 Testing & Verification
- [x] Unit test framework with fixtures
- [ ] All fixtures using correct event format (in progress)
- [ ] Reconciliation gap detection tests
- [ ] Dashboard fixture testing (pending)

### 🔄 Documentation & Alerts
- [ ] Quality metrics consumer guide (pending)
- [ ] Runbook entries for each alert (pending)
- [ ] Dashboard panel descriptions (pending)

### 🔄 Build & Security
- [ ] No secrets committed (to verify)
- [ ] All tests passing (fixtures need fixing)
- [ ] Lint/format clean (to verify)

---

## Files Modified/Created

| File | Status | Changes |
|------|--------|---------|
| `docs/quality-metrics-schema.md` | ✅ Created | Comprehensive schema definition |
| `services/revision_metrics/quality_metrics.py` | ✅ Created | Core metrics calculations |
| `services/revision_metrics/test/test_quality_metrics.py` | 🔄 Created | 18 test cases (fixtures need fixing) |
| `docs/comprehensive-metrics-implementation.md` | ✅ Created | This document |
| `scripts/metrics_exporter.py` | 🔄 Pending | Add freshness, disagreement, missing-data handling |
| `docs/monitoring/quality-metrics-guide.md` | 🔄 Pending | Consumer guide and examples |
| `docs/runbook.md` | 🔄 Pending | Alert handling entries |

---

## Next Steps (Priority Order)

1. **Fix Test Fixtures** (High Priority)
   - Ensure all test fixtures use correct event envelope format
   - Add contract_id and topic fields
   - Run full test suite to verify

2. **Integrate with Metrics Exporter** (High Priority)
   - Wire quality_metrics.py into metrics_exporter.py
   - Emit Prometheus metrics for all three categories
   - Test with live Soroban event stream

3. **Create Consumer Guide** (Medium Priority)
   - Document what each metric means
   - Provide interpretation examples
   - Include remediation playbooks

4. **Build Grafana Dashboard** (Medium Priority)
   - Create dashboard panels for each metric
   - Add alert rules
   - Link to runbook entries

5. **Update Runbook** (Medium Priority)
   - Add entries for each alert type
   - Describe first checks and remediation steps
   - Link from Grafana alert annotations

6. **Verify Security** (High Priority)
   - Ensure no secrets committed
   - Run security gates
   - Verify lint/format compliance

---

## References

- **Existing Correction-Rate Metrics:** `docs/price-revision-metrics.md` (#499)
- **Monitoring Setup:** `docs/monitoring/README.md`
- **Event Schema:** `docs/event-streaming/README.md`
- **Runbook:** `docs/runbook.md`
- **SLA:** `docs/SLA.md`
