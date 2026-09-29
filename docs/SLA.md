# Service Level Agreement — Stellar Unified Price Oracle Aggregator

**Version:** 1.0  
**Effective Date:** 2025-01-01  
**Last Updated:** 2025-01-01  

This Service Level Agreement (SLA) defines the guarantees, commitments, and expectations for the Stellar Unified Price Oracle Aggregator smart contract deployed on the Stellar network.

---

## 1. Price Freshness Guarantees

### 1.1 Resolution Window

The contract exposes a configurable `resolution` parameter (seconds). When set:

- Any price older than `resolution` seconds is treated as **stale** and will not be returned by freshness-filtered queries.
- Default: `0` (no staleness filtering — consumers are responsible for their own freshness checks).

### 1.2 Target Update Frequency

| Condition | Target |
|-----------|--------|
| Normal operation (≥ `min_sources_required` active) | Price updated within **60 seconds** of any source submission |
| Single active source (below minimum) | No aggregate published until minimum is met |
| Contract paused | No new aggregates until unpaused |

### 1.3 Timestamp Validity

- Submitted prices must carry a timestamp within **±5 minutes** (`timestamp_threshold`, default 300 s) of the ledger clock.
- Submissions outside this window are rejected with `ErrorCode::InvalidTimestamp`.

---

## 2. Minimum Source Count Commitments

| Parameter | Default | Governance-adjustable |
|-----------|---------|----------------------|
| `min_sources_required` | 1 | Yes — via timelock proposal |

**Commitments:**

- The oracle operator commits to maintaining **at least 2 active oracle sources** in production deployments at all times.
- If the active source count falls below `min_sources_required`, aggregate prices are **not updated** and the `health_check()` endpoint will reflect degraded status.
- Emergency source additions are targeted within **4 hours** of a source going offline.

---

## 3. Maximum Price Deviation Thresholds

| Parameter | Default | Range |
|-----------|---------|-------|
| `max_price_deviation` | 500 bp (5 %) | 0–100,000 bp |

**Behaviour:**

- A source submission that deviates from the current aggregate by more than `max_price_deviation` is flagged (`SubmissionDeviant` flag set).
- Flagged submissions are still included in aggregation (they are not silently dropped) to preserve liveness; however, they generate an on-chain event for monitoring.
- The aggregation method (default: **median**) provides inherent outlier resistance — a single manipulated source cannot move the aggregate by more than one rank in the sorted price list.

---

## 4. Expected Uptime and Availability

### 4.1 On-Chain Contract Availability

The contract is a Soroban smart contract on the Stellar network. Its availability is tied to Stellar network availability:

| Layer | Target Uptime |
|-------|--------------|
| Stellar network | 99.9 % (per Stellar's own SLA) |
| Contract logic (once deployed) | 100 % (deterministic, no off-chain dependency) |
| Oracle price freshness | 99.5 % of 5-minute windows have a non-stale price |

### 4.2 Pause Window

- The contract may be **paused** by the admin for scheduled maintenance or emergency response.
- Planned pause windows will be announced at least **24 hours** in advance.
- Emergency pauses may occur without notice; the target duration is **< 2 hours**.

---

## 5. Incident Response Times

| Severity | Definition | First Response | Resolution Target |
|----------|-----------|---------------|-------------------|
| **P0 — Critical** | All price aggregation stopped; contract paused unexpectedly | 30 minutes | 2 hours |
| **P1 — High** | Active source count below `min_sources_required`; stale prices for ≥ 1 asset | 1 hour | 4 hours |
| **P2 — Medium** | Single source offline; aggregation still functional | 4 hours | 24 hours |
| **P3 — Low** | Monitoring alert; no user impact | 24 hours | 72 hours |

Incident reports and post-mortems will be published in the GitHub repository under `.github/ISSUE_TEMPLATE/`.

---

## 6. Compensation and Penalties for SLA Breaches

### 6.1 Scope

This oracle is an open-source, on-chain smart contract. Compensation terms apply only to **formally integrated consumers** who have executed a service agreement with the oracle operator.

### 6.2 Price Freshness Breach

If the oracle fails to publish a fresh aggregate price within the committed window for a registered asset:

| Breach Duration | Credit |
|-----------------|--------|
| 1–4 hours | 10 % of monthly integration fee |
| 4–24 hours | 25 % of monthly integration fee |
| > 24 hours | 50 % of monthly integration fee |

### 6.3 Data Accuracy Breach

If a verifiably incorrect aggregate price (deviating > 2× the configured `max_price_deviation` from all independent reference prices) is published and remains uncorrected:

| Breach Duration | Credit |
|-----------------|--------|
| < 1 hour | 5 % of monthly integration fee |
| 1–4 hours | 20 % of monthly integration fee |
| > 4 hours | 50 % of monthly integration fee |

### 6.4 Exclusions

Credits are not issued for:

- Stellar network outages outside the operator's control.
- Force majeure events (network forks, protocol upgrades).
- Breaches caused by consumer misconfiguration.
- Periods during which the contract was paused for a declared emergency.

### 6.5 Claims Process

SLA breach claims must be submitted as a GitHub issue within **7 days** of the incident with:

- Timestamp range of the breach.
- Asset(s) affected.
- Evidence (on-chain transaction history or RPC query results).

---

## 7. Governance and SLA Amendments

Changes to this SLA are subject to the governance proposal process described in [docs/governance-proposal-template.md](governance-proposal-template.md). All changes require:

1. A governance proposal posted for community review (minimum 72-hour review period).
2. On-chain timelock execution for any parameter changes.
3. Updated SLA document committed to this repository.

---

## 8. Contact

For SLA inquiries, incident reports, or integration agreements, open a GitHub issue or contact the maintainers via the repository's discussion board.

---

## 9. Machine-Enforceable Clause Map (Monitoring v2)

Every clause below is checked by [`services/sla_monitor/monitor.py`](../services/sla_monitor/monitor.py).
This table is parsed by `check_sla_consistency()` and CI fails if it drifts from
`SLA_CLAUSES` in the monitor (`python -m services.sla_monitor.monitor --check-sla`).
Violations are exported as `oracle_sla_violations_total{clause="…"}` and written to an
automatic violation report that names the breached clause.

| Clause | Metric | Owner | Threshold | Meaning |
|--------|--------|-------|-----------|---------|
| §1.2 | `oracle_sla_price_age_seconds` | oracle-ops | 60 | Aggregate refreshed within 60 s of source submissions |
| §1.3 | `oracle_sla_timestamp_skew_seconds` | oracle-ops | 300 | Submission timestamps within ±300 s of the ledger clock |
| §2 | `oracle_sla_counted_sources` | source-onboarding | 2 | At least 2 active sources counted in every aggregate |
| §3 | `oracle_sla_reference_deviation_bps` | risk | 500 | Aggregate within `max_price_deviation` of an independent reference set |
| §4.1 | `oracle_sla_freshness_burn_rate` | oracle-ops | 14.4 | 99.5 % freshness SLO; fast-burn alert when both 5-round and 60-round windows exceed 14.4× |
| §4.2 | `oracle_sla_pause_duration_seconds` | governance | 7200 | Emergency pauses shorter than 2 hours |
| §6.3 | `oracle_sla_accuracy_breach_bps` | risk | 1000 | Accuracy breach: deviation > 2× `max_price_deviation` |
| §9.1 | `oracle_sla_aggregate_divergence` | core-contracts | 0 | Published aggregate equals the independent recomputation from counted inputs (checked in the same ledger) |
| §9.2 | `oracle_sla_flatline_rounds` | oracle-ops | 10 | Aggregate not flatlined for 10 rounds while inputs move |
| §9.3 | `oracle_sla_relationship_error_bps` | risk | 200 | Inverse and cross price relationships remain consistent |
| §9.4 | `oracle_sla_uncounted_admitted_sources` | source-onboarding | 0 | No admitted source is silently excluded for a full 20-round window |
| §9.5 | `oracle_sla_unadmitted_influence` | security | 0 | No source is counted without an on-chain admission (`SourceAdded`) event |

### 9.1 Why agreement, not just uptime

A minority of degraded sources can shift the median while every availability and
freshness gauge stays green. Clauses §3 and §9.1 catch this: the aggregate is
recomputed from raw inputs (bit-exact with `core_pricing::median_core`) and compared
with an independent reference set. `test_minority_manipulation_fires_with_full_uptime`
demonstrates the monitor firing while uptime is 100 %.

---

## 10. Disaster-Recovery Targets (RTO / RPO)

Measured, not estimated: `services/dr_gameday` drills each scenario weekly
(`.github/workflows/dr-gameday.yml`) in an isolated sandbox and fails the run
when a measured value exceeds its target. Keep this table in sync with
`SLA_TARGETS` in `services/dr_gameday/gameday.py`.

| Scenario | RTO target | RPO target |
|----------|-----------|------------|
| `region_loss` | 900 s | 0 ledgers |
| `key_unavailability` | 3600 s | 0 ledgers |
| `migration_failure` | 1800 s | 0 ledgers |
| `index_corruption` | 7200 s | 120 ledgers |
