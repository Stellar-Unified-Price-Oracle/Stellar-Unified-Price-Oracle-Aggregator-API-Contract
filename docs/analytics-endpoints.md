# Read-only analytics endpoints (off-chain migration candidates)

Input to #783 "Move analytics endpoints off-chain into the indexer". This is the
**list** step only: it names the read-only endpoints whose output is derived
analytics rather than data a consumer contract needs to read atomically.

Source: the endpoint audit (every public endpoint in `interface/interface.json`
cross-referenced with `contracts/price-oracle/src/lib.rs`; 640 endpoints, of which
325 are classified read-only / no state mutation).

## Selection criteria

An endpoint is listed as an analytics endpoint when **all** of these hold:

1. The audit classifies it as read-only (`Mutates state? = no`).
2. It returns a report, score, statistic, sample window or diagnostic computed
   from stored events/history — not a price, a configuration value, or a
   safety flag.
3. No on-chain consumer needs to branch on it inside a transaction, so an
   indexer serving the same numbers from emitted events and ledger history is an
   acceptable replacement.

Endpoints that fail (3) — price reads, SEP-40 surface, circuit-breaker / finality /
freshness / price-bound *status* used to gate other contracts, and plain
configuration getters — are **not** listed and stay on-chain.

## Analytics endpoints (30)

| Endpoint | Module | Kind | Original issue |
|---|---|---|---|
| `get_coverage_report` | `coverage` | report | #399, #498 |
| `get_coverage_gap_list` | `coverage` | report | #399, #498 |
| `get_decentralization_report` | `sources` | report | — |
| `get_drift_report` | `drift` | report | #497 |
| `get_latency_report` | `latency` | report | #492 |
| `get_latency_samples` | `latency` | sample window | #492 |
| `get_source_deviation_report` | `source_deviation` | report | — |
| `get_source_comparison` | `source_comparison` | comparison | #401 |
| `get_source_diversity` | `source_diversity` | score | #399 |
| `get_last_diversity_breach_ledger` | `source_diversity` | statistic | #399 |
| `check_diversity_alert` | `reentrancy` | diagnostic | — |
| `get_source_scorecard` | `scorecards` | score | #490 |
| `get_source_scorecard_samples` | `scorecards` | sample window | #490 |
| `get_source_accuracy_score` | `scorecards` | score | #490 |
| `get_contribution_quality` | `contribution_quality` | score | #298 |
| `get_source_health` | `sources` | score | — |
| `get_degradation_stats` | `degradation` | statistic | #495 |
| `get_degradation_window_stats` | `degradation` | statistic | #495 |
| `get_disagreement_index` | `disagreement` | statistic | #494 |
| `get_spread_indicator` | `spread_indicator` | statistic | #713 |
| `explain_submission` | `explanation` | diagnostic | #496 |
| `get_anomaly_retention` | `explanation` | diagnostic | #496 |
| `get_challenge_history` | `challenger` | history | #235, #461 |
| `get_config_history` | `config_history` | history | — |
| `get_gas_stats` | `gas_metering` | statistic | — |
| `oracle_state_analyze` | `state_introspection` | diagnostic | #278 |
| `oracle_state_dump` | `state_introspection` | diagnostic | #278 |
| `export_history` | `export_history` | export | — |
| `verify_export` | `export_history` | export | — |
| `get_compaction_metadata` | `history` | diagnostic | #251 |

## Deliberately kept on-chain (read-only, not analytics)

| Group | Examples | Why it stays |
|---|---|---|
| Price reads | `get_twap`, `get_twap_ex`, `get_price_history`, `get_historical_price(s)`, `get_historical_prices_paginated`, `get_tiered_historical_price` | Consumers read these inside transactions; they are the product. |
| Safety status | `get_breaker_status`, `get_finality_status`, `get_freshness_status`, `get_price_bound_status`, `get_sanity_status`, `get_quorum_regime`, `get_publication_status`, `get_round_status`, `is_correlation_flagged` | Used to gate other contracts' behaviour atomically. |
| Configuration | `get_*_thresholds`, `get_scorecard_config`, `get_scoring_window`, `get_max_*`, `get_twap_min_cardinality`, `get_deviation_trigger`, `get_source_deviation_tolerance` | Parameters of on-chain logic; cheap and must match the executing code. |
| Storage / ops state | `get_storage_budget`, `get_total_storage_budget`, `get_storage_ttl_status`, `get_migration_state`, `get_operation_status`, `get_history_watermark`, `has_historical_price` | Operational invariants checked on-chain. |

## Caveats

- The audit's mutate/auth classification is heuristic (it follows delegating
  calls textually); every row above was selected from endpoints it marks
  `no` for mutation, but names such as `check_diversity_alert` (in
  `reentrancy`) are attributed to the module that owns the wrapper, not the
  module that owns the logic. Confirm each before deletion.
- Moving an endpoint requires the indexer to reconstruct it from emitted events;
  `get_gas_stats`, `get_latency_samples` and `get_source_scorecard_samples`
  depend on on-chain sample windows that may not be fully evented today.
