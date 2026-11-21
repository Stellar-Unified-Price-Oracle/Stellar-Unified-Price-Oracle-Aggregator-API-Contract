# Endpoint Surface Audit (`interface/interface.json`)

Generated from `interface/interface.json` (640 public endpoints), cross-referenced with `contracts/price-oracle/src/lib.rs` and [docs/security/endpoint-authority-matrix.md](security/endpoint-authority-matrix.md).

| Endpoint | Module | Mutates state? | Requires auth? | Original issue |
|---|---|---|---|---|
| `abandon_stalled_round` | `consensus_rounds` | yes (delegated) | delegated / review | #397 |
| `activate_source` | `reentrancy` | yes (delegated) | delegated / review | — |
| `add_authorized_consumer` | `consumer_auth` | yes (delegated) | yes (`admin`) | #304 |
| `add_reference_oracle` | `cross_reference` | yes (delegated) | yes (`admin`) | — |
| `add_relayer` | `relayer` | yes (delegated) | yes (`admin`) | — |
| `add_sanity_relation` | `sanity_lattice` | yes (delegated) | delegated / review | #488 |
| `add_source` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `add_source_asset` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `add_source_with_assets` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `allow_governor_op` | `external_governance` | yes (delegated) | yes (`admin`) | — |
| `amm_add_liquidity` | `reentrancy` | yes (delegated) | yes (`caller:caller`) | — |
| `amm_get_max_deviation_bps` | `amm` | no | no (`public-read`) | #180 |
| `amm_get_pool` | `amm` | no | no (`public-read`) | #180 |
| `amm_get_weight` | `amm` | no | no (`public-read`) | #180 |
| `amm_init` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `amm_set_max_deviation_bps` | `amm` | yes (delegated) | yes (`admin`) | #180 |
| `amm_set_status` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `amm_set_weight` | `amm` | yes (delegated) | yes (`admin`) | #180 |
| `amm_swap` | `reentrancy` | yes (delegated) | yes (`caller:caller`) | — |
| `apply_retention_policy` | `retention_policy` | yes (delegated) | yes | #714 |
| `approve_source` | `reentrancy` | yes (delegated) | yes (`caller:approver`) | — |
| `approve_storage_tier_downgrade` | `storage_tier` | yes (delegated) | delegated / review | #246 |
| `assets` | `lib` | no | no (`public-read`) | #171, #172, #173, #174 |
| `assets_paginated` | `assets` | no | no (public-read) | — |
| `audit_governance_events` | `audit_log` | yes (delegated) | delegated / review | #659 |
| `authorize_renewal` | `auto_renewal` | yes (delegated) | delegated / review | #289 |
| `base` | `lib` | no | no (`public-read`) | #171, #172, #173, #174 |
| `bridge_get_oracle` | `bridge_oracle` | no | no (`public-read`) | — |
| `bridge_get_price` | `bridge_oracle` | no | no (`public-read`) | — |
| `bridge_normalize_price` | `bridge_oracle` | no | no (`public-read`) | — |
| `bridge_register_oracle` | `bridge_oracle` | yes (delegated) | yes (`admin`) | — |
| `bridge_submit_price` | `bridge_oracle` | yes (delegated) | yes (`contract-self`) | — |
| `cancel_batch` | `timelock` | yes (delegated) | yes (`admin`) | — |
| `cancel_blackout` | `reentrancy` | yes (delegated) | delegated / review | — |
| `cancel_dependent_operation` | `operations` | yes (delegated) | yes (`admin`) | — |
| `cancel_emergency_pause` | `emergency_pause` | yes (delegated) | yes (`admin`) | #240 |
| `cancel_operation` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `cancel_source_removal` | `sources` | yes (delegated) | yes (`admin`) | — |
| `cancel_subscription` | `subscription` | yes (delegated) | yes (`caller:consumer`) | — |
| `challenge_price` | `challenger` | yes (delegated) | yes (`caller:challenger`) | #235, #461 |
| `challenge_relayed_submission` | `relayer` | yes (delegated) | yes (`admin`) | — |
| `check_and_deregister_if_inactive` | `asset_inactivity` | no | yes (`admin`) | — |
| `check_and_prune_inactive_sources` | `sources` | no | no (`public-read`) | — |
| `check_diversity_alert` | `reentrancy` | no | no (`public-read`) | — |
| `check_reorg` | `finality` | no | no (`public-read`) | #188 |
| `check_source_liveness` | `sources` | no | no (`public-read`) | — |
| `claim_rewards` | `challenger` | yes (delegated) | yes (`caller:claimer`) | #235, #461 |
| `clear_aggregation_round` | `submission_deadline` | yes (delegated) | yes (`admin`) | #225 |
| `clear_asset_decimals` | `per_asset_decimals` | yes (delegated) | yes (`admin`) | #227 |
| `clear_breaker` | `breaker` | yes (delegated) | delegated / review | #481 |
| `clear_correlation_flag` | `correlation` | yes (delegated) | yes (`admin`) | #172 |
| `clear_deferral_policy` | `deferral` | yes (delegated) | yes (`admin`) | #485 |
| `clear_depeg` | `sanity_lattice` | yes (delegated) | delegated / review | #488 |
| `clear_dq_config` | `reentrancy` | yes (delegated) | delegated / review | — |
| `clear_external_governor` | `external_governance` | yes (delegated) | yes (`admin`) | — |
| `clear_notification_preferences` | `notifications` | yes (delegated) | yes (`admin`) | #243 |
| `clear_price_bounds_tier` | `price_bounds` | yes (delegated) | yes (`admin`) | #484 |
| `commit_price` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `compact_history` | `history` | yes (delegated) | yes (`admin`) | #247 |
| `compute_adaptive_interval` | `admin` | no | no (`public-read`) | — |
| `compute_derived_feed` | `derived_feeds` | no | no (public-read) | #478 |
| `correct_price` | `reentrancy` | yes (delegated) | yes (`admin`) | #486 |
| `create_operation` | `operations` | yes (delegated) | yes (`admin`) | — |
| `current_round_ledger` | `prices` | no | no (`public-read`) | — |
| `dead_man_configure` | `input_validation` | yes (delegated) | delegated / review | #506 |
| `dead_man_evaluate` | `dead_man` | yes (delegated) | delegated / review | #510 |
| `dead_man_get_config` | `dead_man` | yes (delegated) | delegated / review | #510 |
| `dead_man_heartbeat` | `dead_man` | yes (delegated) | delegated / review | #510 |
| `dead_man_is_degraded` | `dead_man` | yes (delegated) | delegated / review | #510 |
| `dead_man_recover` | `dead_man` | yes (delegated) | delegated / review | #510 |
| `decimals` | `admin` | no | no (`public-read`) | — |
| `declare_depeg` | `input_validation` | yes (delegated) | delegated / review | #488 |
| `delegate_relayer` | `relayer` | yes (delegated) | yes (`signature`) | — |
| `delegate_role` | `rbac` | yes (delegated) | yes (`admin`) | #241 |
| `deposit_relayer_bond` | `relayer_bonds` | yes (delegated) | yes (`caller:relayer`) | #265 |
| `deposit_source_bond` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `dex_register_pool` | `dex` | yes (delegated) | yes (`admin`) | #280 |
| `did_get_all_source_links` | `did` | no | no (`public-read`) | — |
| `did_get_document` | `did` | no | no (`public-read`) | — |
| `did_get_source_link` | `did` | no | no (`public-read`) | — |
| `did_link_source` | `did` | yes (delegated) | yes (`admin`) | — |
| `did_register` | `did` | yes (delegated) | yes (`admin`) | — |
| `did_verify` | `did` | no | no (`public-read`) | — |
| `disable_asset` | `admin` | yes (delegated) | delegated / review | — |
| `disable_auto_renewal` | `auto_renewal` | yes (delegated) | delegated / review | #289 |
| `disable_scorecards` | `scorecards` | yes (delegated) | delegated / review | #490 |
| `disallow_governor_op` | `external_governance` | yes (delegated) | yes (`admin`) | — |
| `dispute_proposal` | `reentrancy` | yes (delegated) | yes (`caller:disputer`) | — |
| `distribute_subscription_fees` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `emergency_pause` | `emergency_pause` | yes (delegated) | yes (`admin`) | #240 |
| `enable_all_feature_flags` | `reentrancy` | yes (delegated) | yes | — |
| `enable_asset` | `admin` | yes (delegated) | delegated / review | — |
| `enable_auto_renewal` | `auto_renewal` | yes (delegated) | delegated / review | #289 |
| `enable_scorecards` | `scorecards` | yes (delegated) | delegated / review | #490 |
| `enroll_source` | `input_validation` | yes (delegated) | delegated / review | #506 |
| `evaluate_breaker_rearm` | `breaker` | yes (delegated) | delegated / review | #481 |
| `execute_axelar_message` | `axelar_gmp` | yes (delegated) | yes (`caller:gateway`) | #182 |
| `execute_batch` | `timelock` | yes (delegated) | yes (`admin`) | — |
| `execute_dependent_operation` | `operations` | yes (delegated) | yes (`admin`) | — |
| `execute_operation` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `execute_storage_tier_downgrade` | `storage_tier` | yes (delegated) | delegated / review | #246 |
| `exotic_get_config` | `exotic_pricing` | no | no (`public-read`) | #177 |
| `exotic_get_price` | `exotic_pricing` | no | no (`public-read`) | #177 |
| `exotic_set_config` | `exotic_pricing` | yes (delegated) | yes (`admin`) | #177 |
| `explain_submission` | `explanation` | no | no (`public-read`) | #496 |
| `export_history` | `export_history` | no | no (`public-read`) | — |
| `extend_asset_ttl` | `ttl_batching` | yes (delegated) | no (`public`) | #203 |
| `extend_blackout` | `reentrancy` | yes (delegated) | delegated / review | — |
| `extend_emergency_pause` | `emergency_pause` | yes (delegated) | yes (`admin`) | #240 |
| `finalize_confirmation` | `consensus_rounds` | yes (delegated) | delegated / review | #397 |
| `finalize_source_removal` | `sources` | yes (delegated) | yes (`admin`) | — |
| `fm_enqueue_submission` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `fm_get_fee_distribution_ratio` | `fee_market` | no | no (`public-read`) | #176 |
| `fm_get_min_priority_fee` | `fee_market` | no | no (`public-read`) | #176 |
| `fm_get_pending_submissions` | `fee_market` | no | no (`public-read`) | #176 |
| `fm_get_source_fee_balance` | `fee_market` | no | no (`public-read`) | #176 |
| `fm_get_treasury_fee_balance` | `fee_market` | no | no (`public-read`) | #176 |
| `fm_process_fee_market` | `reentrancy` | yes (delegated) | no (`public`) | — |
| `fm_set_fee_distribution_ratio` | `fee_market` | yes (delegated) | yes (`admin`) | #176 |
| `fm_set_min_priority_fee` | `fee_market` | yes (delegated) | yes (`admin`) | #176 |
| `fm_set_treasury_address` | `fee_market` | yes (delegated) | yes (`admin`) | #176 |
| `freeze_price` | `freeze` | yes (delegated) | yes (`admin`) | #223 |
| `get_active_proposals` | `optimistic` | no | no (`public-read`) | #462 |
| `get_adaptive_quorum_config` | `adaptive_quorum` | no | no (public-read) | #482 |
| `get_address_roles` | `rbac` | no | no (`public-read`) | #241 |
| `get_admin` | `storage` | no | no (`public-read`) | — |
| `get_admin_address` | `admin` | no | no (`public-read`) | — |
| `get_admin_audit_log` | `audit_log` | no | no (`public-read`) | #239, #674 |
| `get_admin_op_daily_count` | `admin_op_limits` | no | no (`public-read`) | #238, #654 |
| `get_admin_op_daily_limit` | `admin_op_limits` | no | no (`public-read`) | #238, #654 |
| `get_aggregate_flag_explanations` | `explanation` | no | no (`public-read`) | #496 |
| `get_aggregate_with_version` | `prices` | no | no (`public-read`) | #252 |
| `get_aggregation_cooldown` | `admin` | no | no (`public-read`) | — |
| `get_aggregation_method` | `admin` | no | no (`public-read`) | — |
| `get_all_prices` | `prices` | no | no (`public-read`) | — |
| `get_anomaly_retention` | `explanation` | no | no (`public-read`) | #496 |
| `get_asset_decimals` | `per_asset_decimals` | no | no (`public-read`) | #227 |
| `get_asset_disabled_at` | `admin` | no | no (public-read) | — |
| `get_asset_inactivity_timeout` | `asset_inactivity` | no | no (`public-read`) | — |
| `get_asset_policy` | `policy` | no | no (`public-read`) | — |
| `get_asset_proof_requirement` | `price_proof` | no | no (`public-read`) | #296 |
| `get_asset_resolution` | `admin` | no | no (`public-read`) | — |
| `get_asset_risk_tier` | `risk_tier` | no | no (public-read) | #487 |
| `get_asset_storage_tier` | `storage_tier` | no | no (public-read) | #246 |
| `get_audit_log_count` | `audit_log` | no | no (`public-read`) | #239, #674 |
| `get_audit_log_head` | `audit_log` | no | no (`public-read`) | #239, #674 |
| `get_auto_renewal_record` | `auto_renewal` | no | no (public-read) | #289 |
| `get_axelar_gateway` | `axelar_gmp` | no | no (`public-read`) | #182 |
| `get_basket` | `basket` | no | no (public-read) | #479 |
| `get_basket_composition` | `basket` | no | no (public-read) | #479 |
| `get_basket_contribution` | `basket` | no | no (public-read) | #479 |
| `get_basket_value` | `basket` | no | no (public-read) | #479 |
| `get_bft_aggregation_method` | `prices` | no | no (`public-read`) | — |
| `get_bft_fault_tolerance` | `prices` | no | no (`public-read`) | — |
| `get_blackout` | `blackout` | no | no (public-read) | #400 |
| `get_breaker_policy` | `breaker` | no | no (public-read) | #481 |
| `get_breaker_status` | `breaker` | no | no (public-read) | #481 |
| `get_bucket_entries` | `history` | no | no (`public-read`) | #251 |
| `get_cc_deviation_threshold` | `cross_chain_verify` | no | no (`public-read`) | #226 |
| `get_challenge_history` | `challenger` | no | no (`public-read`) | #235, #461 |
| `get_challenger_rewards` | `challenger` | no | no (`public-read`) | #235, #461 |
| `get_commit_reveal_enabled` | `prices` | no | no (`public-read`) | — |
| `get_commit_reveal_slash_amount` | `prices` | no | no (`public-read`) | — |
| `get_commit_window` | `prices` | no | no (`public-read`) | — |
| `get_compaction_metadata` | `history` | no | no (`public-read`) | #251 |
| `get_compaction_threshold_bps` | `admin` | no | no (`public-read`) | — |
| `get_compliant_sources` | `prices` | no | no (`public-read`) | — |
| `get_confidence_band` | `confidence_band` | no | no (`public-read`) | #476 |
| `get_config_history` | `config_history` | no | no (`public-read`) | — |
| `get_confirmed_price` | `consensus_rounds` | no | no (public-read) | #397 |
| `get_consumer_access_mode` | `consumer_auth` | no | no (`public-read`) | #304 |
| `get_contract_metadata` | `metadata` | no | no (`public-read`) | — |
| `get_contribution_quality` | `contribution_quality` | no | no (`public-read`) | #298 |
| `get_correction_count` | `corrections` | no | no (`public-read`) | #486 |
| `get_correction_scope` | `corrections` | no | no (`public-read`) | #486 |
| `get_coverage_gap_list` | `coverage` | no | no (`public-read`) | #399, #498 |
| `get_coverage_report` | `coverage` | no | no (`public-read`) | #399, #498 |
| `get_coverage_thresholds` | `coverage` | no | no (`public-read`) | #399, #498 |
| `get_cross_chain_price` | `cross_chain_verify` | no | no (`public-read`) | #226 |
| `get_cross_ref_deviation_bps` | `cross_reference` | no | no (`public-read`) | — |
| `get_cross_reference` | `cross_reference` | no | no (`public-read`) | — |
| `get_current_aggregation_round` | `submission_deadline` | no | no (`public-read`) | #225 |
| `get_decentralization_report` | `sources` | no | no (`public-read`) | — |
| `get_decimals` | `admin` | no | no (`public-read`) | — |
| `get_default_freshness_window` | `freshness_quorum` | no | no (public-read) | #489 |
| `get_deferral_policy` | `deferral` | no | no (`public-read`) | #485 |
| `get_degradation_config` | `degradation` | no | no (`public-read`) | #495 |
| `get_degradation_stats` | `degradation` | no | no (`public-read`) | #495 |
| `get_degradation_window_stats` | `degradation` | no | no (`public-read`) | #495 |
| `get_demerit_config` | `sources` | no | no (`public-read`) | — |
| `get_description` | `admin` | no | no (`public-read`) | — |
| `get_deviation_trigger` | `triggers` | no | no (`public-read`) | #218 |
| `get_dex_price` | `dex` | no | no (`public-read`) | #280 |
| `get_disagreement_index` | `disagreement` | no | no (public-read) | #494 |
| `get_diversity_thresholds` | `source_diversity` | no | no (`public-read`) | #399 |
| `get_dq_anchor` | `dq_pipeline` | no | no (public-read) | #398 |
| `get_dq_config` | `dq_pipeline` | no | no (public-read) | #398 |
| `get_drift_report` | `drift` | no | no (`public-read`) | #497 |
| `get_drift_thresholds` | `drift` | no | no (`public-read`) | #497 |
| `get_effective_policy` | `policy` | no | no (`public-read`) | — |
| `get_effective_quorum` | `adaptive_quorum` | no | no (public-read) | #482 |
| `get_emergency_pause` | `emergency_pause` | no | no (`public-read`) | #240 |
| `get_equivocation_count` | `consensus_rounds` | no | no (public-read) | #397 |
| `get_external_governor` | `external_governance` | no | no (`public-read`) | — |
| `get_finality_ledgers` | `finality` | no | no (`public-read`) | #188 |
| `get_finality_status` | `finality` | no | no (`public-read`) | #188 |
| `get_finalized_price` | `finality` | no | no (`public-read`) | #188 |
| `get_flag_explanations` | `explanation` | no | no (`public-read`) | #496 |
| `get_foreign_asset_mapping` | `asset_registry` | no | no (`public-read`) | — |
| `get_foreign_mappings_for_asset` | `asset_registry` | no | no (`public-read`) | — |
| `get_freshness_curve` | `freshness_weight` | no | no (`public-read`) | — |
| `get_freshness_status` | `freshness_quorum` | no | no (public-read) | #489 |
| `get_gas_stats` | `gas_metering` | no | no (`public-read`) | — |
| `get_governor_epoch` | `external_governance` | no | no (`public-read`) | — |
| `get_heartbeat_interval` | `admin` | no | no (`public-read`) | — |
| `get_heartbeat_window` | `sources` | no | no (`public-read`) | — |
| `get_historical_price` | `history` | no | no (`public-read`) | #251 |
| `get_historical_prices` | `history` | no | no (`public-read`) | #251 |
| `get_historical_prices_paginated` | `history` | no | no (`public-read`) | #229 |
| `get_history_watermark` | `watermark` | no | no (public-read) | #712 |
| `get_inactive_sources` | `sources` | no | no (`public-read`) | — |
| `get_inactivity_timeout` | `asset_inactivity` | no | no (`public-read`) | — |
| `get_influence_cap` | `influence_cap` | no | no (`public-read`) | #475 |
| `get_interpolation_enabled` | `admin` | no | no (`public-read`) | — |
| `get_inverse_feed` | `derived_feeds` | no | no (public-read) | #478 |
| `get_last_diversity_breach_ledger` | `source_diversity` | no | no (`public-read`) | #399 |
| `get_last_price_ledger` | `sources` | no | no (`public-read`) | — |
| `get_latency_report` | `latency` | no | no (public-read) | #492 |
| `get_latency_samples` | `latency` | no | no (public-read) | #492 |
| `get_latest_flag_explanation` | `explanation` | no | no (`public-read`) | #496 |
| `get_latest_ledger` | `lib` | no | no (`public-read`) | #171, #172, #173, #174 |
| `get_layerzero_endpoint` | `layerzero` | no | no (`public-read`) | — |
| `get_lz_inbound_nonce` | `layerzero` | no | no (`public-read`) | — |
| `get_max_aggregation_sources` | `admin` | no | no (`public-read`) | — |
| `get_max_assets` | `admin` | no | no (`public-read`) | — |
| `get_max_events_per_call` | `admin` | no | no (`public-read`) | — |
| `get_max_history_length` | `admin` | no | no (`public-read`) | — |
| `get_max_history_per_asset` | `admin` | no | no (`public-read`) | — |
| `get_max_inactive_ledgers` | `sources` | no | no (`public-read`) | — |
| `get_max_price_deviation` | `admin` | no | no (`public-read`) | — |
| `get_max_sources` | `admin` | no | no (`public-read`) | — |
| `get_migration_state` | `migration` | no | no (`public-read`) | #112 |
| `get_min_sources` | `admin` | no | no (`public-read`) | — |
| `get_min_sources_required` | `admin` | no | no (`public-read`) | — |
| `get_min_submission_interval` | `admin` | no | no (`public-read`) | — |
| `get_missed_heartbeats` | `sources` | no | no (`public-read`) | — |
| `get_onboarding_checklist` | `source_lifecycle` | no | no (public-read) | #211, #400, #402 |
| `get_open_challenge_count` | `challenger` | no | no (public-read) | #235, #461 |
| `get_operation_dependencies` | `operations` | no | no (`public-read`) | — |
| `get_operation_status` | `operations` | no | no (`public-read`) | — |
| `get_optimistic_dispute_window` | `admin` | no | no (`public-read`) | — |
| `get_optimistic_min_bond` | `admin` | no | no (`public-read`) | — |
| `get_oracle_sources` | `sources` | no | no (`public-read`) | — |
| `get_original_price` | `corrections` | no | no (`public-read`) | #486 |
| `get_outlier_config` | `outlier_filter` | no | no (public-read) | #491 |
| `get_outlier_exclusions` | `outlier_filter` | no | no (public-read) | #491 |
| `get_price` | `prices` | no | no (`public-read`) | — |
| `get_price_authorized` | `consumer_auth` | no | yes (`caller:consumer`) | #304 |
| `get_price_bound_status` | `price_bounds` | no | no (`public-read`) | #484 |
| `get_price_bounds` | `assets` | no | no (`public-read`) | — |
| `get_price_bounds_tier` | `price_bounds` | no | no (`public-read`) | #484 |
| `get_price_callbacks` | `price_callback` | no | no (`public-read`) | #297 |
| `get_price_history` | `history` | no | no (`public-read`) | #251 |
| `get_price_override` | `prices` | no | no (`public-read`) | — |
| `get_price_revision` | `corrections` | no | no (`public-read`) | #486 |
| `get_price_revisions` | `corrections` | no | no (`public-read`) | #486 |
| `get_price_with_confidence` | `prices` | no | no (`public-read`) | — |
| `get_priority_delay` | `timelock` | no | no (`public-read`) | — |
| `get_proposal` | `optimistic` | no | no (`public-read`) | #462 |
| `get_provenance` | `provenance` | no | no (public-read) | #493 |
| `get_publication_status` | `deferral` | no | no (`public-read`) | #485 |
| `get_query_rate_limit` | `admin` | no | no (`public-read`) | — |
| `get_quorum_regime` | `adaptive_quorum` | no | no (public-read) | #482 |
| `get_ratio_feed` | `derived_feeds` | no | no (public-read) | #478 |
| `get_recompute_affected_assets` | `recompute` | no | no (`public-read`) | #483 |
| `get_reference_oracles` | `cross_reference` | no | no (`public-read`) | — |
| `get_relayer_bond_amount` | `relayer_bonds` | no | no (`public-read`) | #265 |
| `get_relayer_bond_balance` | `relayer_bonds` | no | no (`public-read`) | #265 |
| `get_relayer_dashboard` | `relayer_dashboard` | no | no (`public-read`) | #267 |
| `get_relayer_delegation` | `relayer` | no | no (`public-read`) | — |
| `get_relayer_failure_count` | `relayer_bonds` | no | no (`public-read`) | #265 |
| `get_relayer_failure_threshold` | `relayer_bonds` | no | no (`public-read`) | #265 |
| `get_relayer_fee_balance` | `relayer` | no | no (`public-read`) | — |
| `get_relayer_fee_per_submission` | `relayer` | no | no (`public-read`) | — |
| `get_relayer_info` | `relayer` | no | no (`public-read`) | — |
| `get_relayer_reward_balance` | `relayer_bonds` | no | no (`public-read`) | #265 |
| `get_relayer_reward_rate` | `relayer_bonds` | no | no (`public-read`) | #265 |
| `get_relayer_slash_percent` | `relayer_bonds` | no | no (`public-read`) | #265 |
| `get_relayer_submission_count` | `relayer` | no | no (`public-read`) | — |
| `get_removal_cooldown` | `sources` | no | no (`public-read`) | — |
| `get_renewal_authorization` | `auto_renewal` | no | no (public-read) | #289 |
| `get_reputation_decay_factor` | `sources` | no | no (`public-read`) | — |
| `get_resolution` | `admin` | no | no (`public-read`) | — |
| `get_resolved_asset_tier` | `risk_tier` | no | no (public-read) | #487 |
| `get_retention_policy` | `retention_policy` | no | no (public-read) | #714 |
| `get_reveal_window` | `prices` | no | no (`public-read`) | — |
| `get_risk_tier_params` | `lib` | no | no (public-read) | #487 |
| `get_role_holders` | `rbac` | no | no (`public-read`) | #241 |
| `get_roles_for_holder` | `rbac` | no | no (`public-read`) | #241 |
| `get_round_config` | `consensus_rounds` | no | no (public-read) | #397 |
| `get_round_quorum` | `adaptive_quorum` | no | no (public-read) | #482 |
| `get_round_status` | `consensus_rounds` | no | no (public-read) | #397 |
| `get_round_tally` | `consensus_rounds` | no | no (public-read) | #397 |
| `get_sanity_peers` | `sanity_lattice` | no | no (public-read) | #488 |
| `get_sanity_relations` | `sanity_lattice` | no | no (public-read) | #488 |
| `get_sanity_status` | `sanity_lattice` | no | no (public-read) | #488 |
| `get_scorecard_config` | `scorecards` | no | no (public-read) | #490 |
| `get_scoring_window` | `contribution_quality` | no | no (`public-read`) | #298 |
| `get_soroswap_price` | `amm` | no | no (`public-read`) | #180 |
| `get_source_accuracy_score` | `scorecards` | no | no (public-read) | #490 |
| `get_source_assets` | `sources` | no | no (`public-read`) | — |
| `get_source_bond` | `sources` | no | no (`public-read`) | — |
| `get_source_comparison` | `source_comparison` | no | no (public-read) | #401 |
| `get_source_demerits` | `sources` | no | no (`public-read`) | — |
| `get_source_deposited_bond` | `sources` | no | no (`public-read`) | — |
| `get_source_deviation_report` | `source_deviation` | no | no (`public-read`) | — |
| `get_source_deviation_tolerance` | `source_deviation` | no | no (`public-read`) | — |
| `get_source_diversity` | `source_diversity` | no | no (`public-read`) | #399 |
| `get_source_enrollment` | `sources` | no | no (public-read) | — |
| `get_source_geo` | `sources` | no | no (`public-read`) | — |
| `get_source_governance` | `sources` | no | no (`public-read`) | — |
| `get_source_health` | `sources` | no | no (`public-read`) | — |
| `get_source_last_heartbeat` | `sources` | no | no (`public-read`) | — |
| `get_source_price` | `prices` | no | no (`public-read`) | — |
| `get_source_proposal` | `sources` | no | no (`public-read`) | — |
| `get_source_reputation` | `sources` | no | no (`public-read`) | — |
| `get_source_scorecard` | `scorecards` | no | no (public-read) | #490 |
| `get_source_scorecard_samples` | `scorecards` | no | no (public-read) | #490 |
| `get_source_verification` | `sources` | no | no (`public-read`) | — |
| `get_spread_indicator` | `spread_indicator` | no | no (public-read) | #713 |
| `get_stake_token_contract` | `reputation` | no | no (`public-read`) | #171 |
| `get_storage_batch` | `input_validation` | no | no (`public-read`) | #506 |
| `get_storage_budget` | `history` | no | no (`public-read`) | #253 |
| `get_storage_tier_info` | `storage_tier` | no | no (public-read) | #246 |
| `get_storage_ttl_status` | `storage` | no | no (`public-read`) | — |
| `get_storage_version` | `migration` | no | no (`public-read`) | #112 |
| `get_submission_proof` | `price_proof` | no | no (`public-read`) | #296 |
| `get_submission_threshold_trigger` | `triggers` | no | no (`public-read`) | #218 |
| `get_subscribed_consumers` | `price_update_subscription` | no | no (`public-read`) | #305 |
| `get_subscription_expiry` | `subscription` | no | no (`public-read`) | — |
| `get_subscription_payment` | `subscription` | no | no (`public-read`) | — |
| `get_subscription_plans` | `subscription` | no | no (`public-read`) | — |
| `get_subscription_token` | `subscription` | no | no (`public-read`) | — |
| `get_tiered_historical_price` | `storage_tier` | no | no (public-read) | #246 |
| `get_time_trigger` | `triggers` | no | no (`public-read`) | #218 |
| `get_timelock_duration` | `timelock` | no | no (`public-read`) | — |
| `get_timestamp_threshold` | `admin` | no | no (`public-read`) | — |
| `get_total_storage_budget` | `history` | no | no (`public-read`) | #253 |
| `get_triangulated_feed` | `derived_feeds` | no | no (public-read) | #478 |
| `get_twap` | `prices` | no | no (`public-read`) | — |
| `get_twap_ex` | `prices` | no | no (`public-read`) | — |
| `get_twap_min_cardinality` | `prices` | no | no (`public-read`) | — |
| `get_weighted_aggregate` | `freshness_weight` | no | no (`public-read`) | — |
| `graduate_source` | `source_lifecycle` | yes (delegated) | delegated / review | #211, #400, #402 |
| `has_asset_risk_tier` | `risk_tier` | no | no (public-read) | #487 |
| `has_historical_price` | `history` | no | no (`public-read`) | #251 |
| `has_role` | `rbac` | no | no (`public-read`) | #241 |
| `has_tiered_historical_price` | `storage_tier` | no | no (public-read) | #246 |
| `health_check` | `health` | no | no (`public-read`) | — |
| `initialize` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `is_asset_disabled` | `admin` | no | no (public-read) | — |
| `is_asset_inactive` | `asset_inactivity` | no | no (`public-read`) | — |
| `is_asset_paused` | `assets` | no | no (`public-read`) | — |
| `is_asset_registered` | `assets` | no | no (`public-read`) | — |
| `is_barred_from_round` | `consensus_rounds` | no | no (public-read) | #397 |
| `is_circuit_breaker_tripped` | `assets` | no | no (`public-read`) | — |
| `is_consumer_authorized` | `consumer_auth` | no | no (`public-read`) | #304 |
| `is_correlation_flagged` | `correlation` | no | no (`public-read`) | #172 |
| `is_cross_verify_enabled` | `cross_chain_verify` | no | no (`public-read`) | #226 |
| `is_depeg_suspended` | `sanity_lattice` | no | no (public-read) | #488 |
| `is_emergency_pause_active` | `emergency_pause` | no | no (`public-read`) | #240 |
| `is_governor_op_allowed` | `external_governance` | no | no (`public-read`) | — |
| `is_paused` | `pause` | no | no (`public-read`) | — |
| `is_price_frozen` | `freeze` | no | no (`public-read`) | #223 |
| `is_relayer` | `relayer` | no | no (`public-read`) | — |
| `is_source` | `sources` | no | no (`public-read`) | — |
| `is_source_excluded` | `recompute` | no | no (`public-read`) | #483 |
| `is_source_inactive` | `sources` | no | no (`public-read`) | — |
| `is_source_pending_removal` | `sources` | no | no (`public-read`) | — |
| `lastprice` | `prices` | no | no (`public-read`) | — |
| `lastprice_authorized` | `consumer_auth` | no | yes (`caller:consumer`) | #304 |
| `list_assets` | `lib` | no | no (`public-read`) | #171, #172, #173, #174 |
| `list_notification_event_types` | `notifications` | no | no (`public-read`) | #243 |
| `list_notification_preferences` | `notifications` | no | no (`public-read`) | #243 |
| `list_sources` | `storage` | no | no (`public-read`) | — |
| `lz_receive` | `layerzero` | yes (delegated) | yes (`caller:endpoint`) | — |
| `mark_price_pending` | `reentrancy` | yes (delegated) | no (`public`) | — |
| `mark_source_for_removal` | `sources` | yes (delegated) | yes (`admin`) | — |
| `metadata_get` | `ecosystem_metadata` | yes (delegated) | no (`public`) | — |
| `metadata_get_feed` | `ecosystem_metadata` | no | no (`public-read`) | — |
| `metadata_list_feeds` | `ecosystem_metadata` | no | no (`public-read`) | — |
| `metadata_register` | `ecosystem_metadata` | yes (delegated) | yes (`admin`) | — |
| `metadata_register_feed` | `ecosystem_metadata` | yes (delegated) | yes (`admin`) | — |
| `metadata_update` | `ecosystem_metadata` | yes (delegated) | yes (`admin`) | — |
| `migrate_history_to_shards` | `history` | yes (delegated) | yes (`admin`) | #251 |
| `migrate_history_to_tier` | `storage_tier` | yes (delegated) | delegated / review | #246 |
| `migrate_storage` | `migration` | yes (delegated) | yes (`admin`) | #112 |
| `ms_approve_operation` | `reentrancy` | yes (delegated) | yes (`caller:governor`) | — |
| `ms_cancel_operation` | `reentrancy` | yes (delegated) | yes (`caller:canceller`) | — |
| `ms_execute_operation` | `reentrancy` | yes (delegated) | yes (`caller:executor`) | — |
| `ms_get_governors` | `multisig` | no | no (`public-read`) | #178 |
| `ms_get_operation` | `multisig` | no | no (`public-read`) | #178 |
| `ms_get_queue_head` | `multisig` | no | no (`public-read`) | #178 |
| `ms_get_required_approvals` | `multisig` | no | no (`public-read`) | #178 |
| `ms_propose_operation` | `reentrancy` | yes (delegated) | yes (`caller:proposer`) | — |
| `ms_retract_approval` | `reentrancy` | yes (delegated) | yes (`caller:governor`) | — |
| `ms_set_governors` | `input_validation` | yes (delegated) | yes (`admin`) | #506 |
| `observe_volatility` | `adaptive_quorum` | yes (delegated) | delegated / review | #482 |
| `offboard_source` | `reentrancy` | yes (delegated) | delegated / review | — |
| `onboard_source` | `input_validation` | yes (delegated) | delegated / review | #506 |
| `oracle_state_analyze` | `state_introspection` | no | no (`public-read`) | #278 |
| `oracle_state_dump` | `state_introspection` | no | no (`public-read`) | #278 |
| `override_price` | `prices` | yes (delegated) | yes (`admin`) | — |
| `pause` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `pause_asset` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `pin_round_quorum` | `adaptive_quorum` | yes (delegated) | delegated / review | #482 |
| `poke_time_trigger` | `triggers` | yes (delegated) | no (`public`) | #218 |
| `price` | `prices` | no | no (`public-read`) | — |
| `price_authorized` | `consumer_auth` | no | yes (`caller:consumer`) | #304 |
| `prices` | `prices` | no | no (`public-read`) | — |
| `prices_authorized` | `consumer_auth` | no | yes (`caller:consumer`) | #304 |
| `propose_batch` | `input_validation` | yes (delegated) | yes (`admin`) | #506 |
| `propose_operation` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `propose_operation_with_priority` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `propose_price` | `reentrancy` | yes (delegated) | yes (`caller:proposer`) | — |
| `propose_source` | `reentrancy` | yes (delegated) | yes (`caller:proposer`) | — |
| `propose_storage_tier_downgrade` | `storage_tier` | yes (delegated) | delegated / review | #246 |
| `prune_history` | `pruning` | yes (delegated) | yes (`admin`) | #288 |
| `reauthorize_governor` | `external_governance` | yes (delegated) | yes (`admin`) | — |
| `rebalance_basket` | `input_validation` | yes (delegated) | delegated / review | #506 |
| `recompute_asset_price` | `reentrancy` | yes (delegated) | no (`public`) | #483 |
| `record_breaker_source_recovery` | `breaker` | yes (delegated) | yes | #623 |
| `record_drift_sample` | `reentrancy` | yes (delegated) | no (`public`) | — |
| `record_relayer_failure` | `relayer_bonds` | yes (delegated) | yes (`admin`) | #265 |
| `recovery_approve` | `reentrancy` | yes (delegated) | yes (`caller:guardian`) | — |
| `recovery_cancel` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `recovery_execute` | `reentrancy` | yes (delegated) | no (`public`) | — |
| `recovery_get_delay` | `recovery` | no | no (`public-read`) | #245 |
| `recovery_get_guardians` | `recovery` | no | no (`public-read`) | #245 |
| `recovery_get_pending` | `recovery` | no | no (`public-read`) | #245 |
| `recovery_get_threshold` | `recovery` | no | no (`public-read`) | #245 |
| `recovery_set_delay` | `recovery` | yes (delegated) | yes (`admin`) | #245 |
| `recovery_set_guardians` | `input_validation` | yes (delegated) | yes (`admin`) | #506 |
| `refresh_history_watermark` | `watermark` | yes (delegated) | delegated / review | #712 |
| `refund_subscription` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `register_asset` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `register_foreign_asset_mapping` | `asset_registry` | yes (delegated) | yes (`admin`) | — |
| `register_price_callback` | `price_callback` | yes (delegated) | yes (`caller:consumer`) | #297 |
| `register_submission_key` | `signed_submission` | yes (delegated) | yes (`caller:source`) | #216 |
| `relay_emit_price_update` | `cross_chain_relay` | yes (delegated) | yes (`admin`) | #182 |
| `relay_get_config` | `cross_chain_relay` | no | no (`public-read`) | #182 |
| `relay_set_config` | `cross_chain_relay` | yes (delegated) | yes (`admin`) | #182 |
| `relay_verify_event_proof` | `input_validation` | no | no (`public-read`) | #506 |
| `relay_verify_header` | `cross_chain_relay` | no | no (`public-read`) | #182 |
| `relay_verify_validator_set` | `input_validation` | yes (delegated) | yes (`signature`) | #506 |
| `remove_authorized_consumer` | `consumer_auth` | yes (delegated) | yes (`admin`) | #304 |
| `remove_axelar_trusted_source` | `axelar_gmp` | yes (delegated) | yes (`admin`) | #182 |
| `remove_foreign_asset_mapping` | `asset_registry` | yes (delegated) | yes (`admin`) | — |
| `remove_lz_trusted_remote` | `layerzero` | yes (delegated) | yes (`admin`) | — |
| `remove_price_override` | `prices` | yes (delegated) | yes (`admin`) | — |
| `remove_reference_oracle` | `cross_reference` | yes (delegated) | yes (`admin`) | — |
| `remove_relayer` | `relayer` | yes (delegated) | yes (`admin`) | — |
| `remove_sanity_relation` | `sanity_lattice` | yes (delegated) | delegated / review | #488 |
| `remove_source` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `remove_source_asset` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `remove_sources` | `reentrancy` | yes (delegated) | yes (`admin`) | #483 |
| `renew_subscription` | `subscription` | yes (delegated) | yes (`caller:consumer`) | — |
| `report_equivocation` | `consensus_rounds` | yes (delegated) | delegated / review | #397 |
| `reset_drift_window` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `reset_source_demerits` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `resolution` | `admin` | no | no (`public-read`) | — |
| `resolve_challenge` | `challenger` | yes (delegated) | yes (`admin`) | #235, #461 |
| `resolve_dispute` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `resolve_via_external_data` | `reentrancy` | yes (delegated) | yes (`admin`) | #291 |
| `retract_price` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `reveal_price` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `reveal_prices_batch` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `revoke_role` | `rbac` | yes (delegated) | yes (`admin`) | #241 |
| `rollback_config` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `rotate_source_key` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `sc_close_channel` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `sc_dispute_channel` | `input_validation` | yes (delegated) | yes (`signature`) | #506 |
| `sc_get_channel` | `state_channel` | no | no (`public-read`) | #179 |
| `sc_open_channel` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `sc_submit_batch` | `input_validation` | yes (delegated) | yes (`caller:source`) | #506 |
| `schedule_blackout` | `reentrancy` | yes (delegated) | delegated / review | — |
| `set_adaptive_quorum_config` | `adaptive_quorum` | yes (delegated) | delegated / review | #482 |
| `set_admin` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_admin_op_daily_limit` | `admin_op_limits` | yes (delegated) | yes (`admin`) | #238, #654 |
| `set_aggregation_cooldown` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_aggregation_method` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_anomaly_retention` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_asset_class` | `policy` | yes (delegated) | yes (`admin`) | — |
| `set_asset_decimals` | `per_asset_decimals` | yes (delegated) | yes (`admin`) | #227 |
| `set_asset_freshness_window` | `freshness_quorum` | yes (delegated) | delegated / review | #489 |
| `set_asset_inactivity_timeout` | `asset_inactivity` | yes (delegated) | yes (`admin`) | — |
| `set_asset_policy` | `policy` | yes (delegated) | yes (`admin`) | — |
| `set_asset_proof_requirement` | `price_proof` | yes (delegated) | yes (`admin`) | #296 |
| `set_asset_resolution` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_asset_risk_tier` | `input_validation` | yes (delegated) | delegated / review | #487 |
| `set_asset_storage_tier` | `storage_tier` | yes (delegated) | delegated / review | #246 |
| `set_axelar_gateway` | `axelar_gmp` | yes (delegated) | yes (`admin`) | #182 |
| `set_axelar_trusted_source` | `axelar_gmp` | yes (delegated) | yes (`admin`) | #182 |
| `set_basket` | `basket` | yes (delegated) | delegated / review | #479 |
| `set_basket_max_staleness` | `basket` | yes (delegated) | delegated / review | #479 |
| `set_bft_parameters` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_breaker_policy` | `breaker` | yes (delegated) | delegated / review | #481 |
| `set_cc_deviation_threshold` | `cross_chain_verify` | yes (delegated) | yes (`admin`) | #226 |
| `set_class_policy` | `policy` | yes (delegated) | yes (`admin`) | — |
| `set_commit_reveal_enabled` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_commit_reveal_slash_amount` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_commit_window` | `prices` | yes (delegated) | yes (`admin`) | — |
| `set_compaction_threshold_bps` | `admin` | yes (delegated) | yes (`admin`) | #247 |
| `set_consumer_access_mode` | `consumer_auth` | yes (delegated) | yes (`admin`) | #304 |
| `set_correction_scope` | `corrections` | yes (delegated) | yes (`admin`) | #486 |
| `set_correlation_pair` | `correlation` | yes (delegated) | yes (`admin`) | #172 |
| `set_coverage_thresholds` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_cross_ref_deviation_bps` | `cross_reference` | yes (delegated) | yes (`admin`) | — |
| `set_cross_verify_enabled` | `cross_chain_verify` | yes (delegated) | yes (`admin`) | #226 |
| `set_decimals` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_default_freshness_window` | `freshness_quorum` | yes (delegated) | delegated / review | #489 |
| `set_deferral_policy` | `deferral` | yes (delegated) | yes (`admin`) | #485 |
| `set_degradation_config` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_demerit_config` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_derived_feed_base` | `derived_feeds` | yes (delegated) | delegated / review | #478 |
| `set_description` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_deviation_trigger` | `triggers` | yes (delegated) | yes (`admin`) | #218 |
| `set_diversity_thresholds` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_dq_config` | `reentrancy` | yes (delegated) | delegated / review | — |
| `set_drift_thresholds` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_external_governor` | `external_governance` | yes (delegated) | yes (`admin`) | — |
| `set_finality_ledgers` | `finality` | yes (delegated) | yes (`admin`) | #188 |
| `set_freshness_curve` | `freshness_weight` | yes (delegated) | yes (`admin`) | — |
| `set_heartbeat_interval` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_heartbeat_window` | `sources` | yes (delegated) | yes (`admin`) | — |
| `set_inactivity_timeout` | `asset_inactivity` | yes (delegated) | yes (`admin`) | — |
| `set_influence_cap` | `influence_cap` | yes (delegated) | yes (`admin`) | #475 |
| `set_interpolation_enabled` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_layerzero_endpoint` | `layerzero` | yes (delegated) | yes (`admin`) | — |
| `set_lifecycle_config` | `source_lifecycle` | yes (delegated) | delegated / review | #211, #400, #402 |
| `set_lz_chain_name` | `layerzero` | yes (delegated) | yes (`admin`) | — |
| `set_lz_trusted_remote` | `layerzero` | yes (delegated) | yes (`admin`) | — |
| `set_max_aggregation_sources` | `admin` | yes (delegated) | yes (`admin`) | #93 |
| `set_max_assets` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_max_events_per_call` | `admin` | yes (delegated) | yes (`admin`) | #92 |
| `set_max_history` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_max_history_length` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_max_history_per_asset` | `admin` | yes (delegated) | yes (`admin`) | #94 |
| `set_max_inactive_ledgers` | `sources` | yes (delegated) | yes (`admin`) | — |
| `set_max_price_deviation` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_max_sources` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_min_sources_required` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_min_submission_interval` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_notification_preference` | `notifications` | yes (delegated) | yes (`admin`) | #243 |
| `set_optimistic_dispute_window` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_optimistic_min_bond` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_outlier_config` | `outlier_filter` | yes (delegated) | delegated / review | #491 |
| `set_price_bounds` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_price_bounds_tier` | `price_bounds` | yes (delegated) | yes (`admin`) | #484 |
| `set_priority_delay` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_query_rate_limit` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_relayer_bond_amount` | `relayer_bonds` | yes (delegated) | yes (`admin`) | #265 |
| `set_relayer_failure_threshold` | `relayer_bonds` | yes (delegated) | yes (`admin`) | #265 |
| `set_relayer_fee_per_submission` | `relayer` | yes (delegated) | yes (`admin`) | — |
| `set_relayer_reward_rate` | `relayer_bonds` | yes (delegated) | yes (`admin`) | #265 |
| `set_relayer_slash_percent` | `relayer_bonds` | yes (delegated) | yes (`admin`) | #265 |
| `set_removal_cooldown` | `sources` | yes (delegated) | yes (`admin`) | — |
| `set_reputation_decay_factor` | `sources` | yes (delegated) | yes (`admin`) | — |
| `set_resolution` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_retention_policy` | `retention_policy` | yes (delegated) | yes | #714 |
| `set_reveal_window` | `prices` | yes (delegated) | yes (`admin`) | — |
| `set_risk_tier_enforcement` | `risk_tier` | yes (delegated) | delegated / review | #487 |
| `set_round_config` | `consensus_rounds` | yes (delegated) | delegated / review | #397 |
| `set_scorecard_config` | `scorecards` | yes (delegated) | delegated / review | #490 |
| `set_scoring_window` | `contribution_quality` | yes (delegated) | yes (`admin`) | #298 |
| `set_source_bond` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_source_comparison` | `source_comparison` | yes (delegated) | delegated / review | #401 |
| `set_source_deviation_tolerance` | `source_deviation` | yes (delegated) | yes (`admin`) | — |
| `set_source_diversity` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_source_geo` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_source_governance` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_source_verification` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_stake_token_contract` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_submission_threshold_trigger` | `triggers` | yes (delegated) | yes (`admin`) | #218 |
| `set_subscription_price` | `admin` | yes (delegated) | yes (`admin`) | — |
| `set_subscription_token` | `subscription` | yes (delegated) | yes (`admin`) | — |
| `set_time_trigger` | `triggers` | yes (delegated) | yes (`admin`) | #218 |
| `set_timelock_duration` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_timestamp_threshold` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `set_twap_min_cardinality` | `prices` | yes (delegated) | yes (`admin`) | — |
| `set_volatility_quorum` | `blackout` | yes (delegated) | delegated / review | #400 |
| `signal_volatility` | `reentrancy` | yes (delegated) | delegated / review | — |
| `simulate_aggregation` | `input_validation` | no | no (`public-read`) | #506 |
| `simulate_batch` | `input_validation` | no | no (`public-read`) | #506 |
| `slash_expired_commits` | `prices` | yes (delegated) | no (`public`) | — |
| `slash_relayer` | `relayer_bonds` | yes (delegated) | yes (`admin`) | #265 |
| `soroswap_get_pool` | `amm` | no | no (`public-read`) | #180 |
| `soroswap_register_pool` | `amm` | yes (delegated) | yes (`admin`) | #180 |
| `soroswap_set_pool_status` | `amm` | yes (delegated) | yes (`admin`) | #180 |
| `start_aggregation_round` | `submission_deadline` | yes (delegated) | yes (`admin`) | #225 |
| `start_round` | `consensus_rounds` | yes (delegated) | delegated / review | #397 |
| `submit_cross_chain_price` | `cross_chain_verify` | yes (delegated) | yes (`admin`) | #226 |
| `submit_heartbeat` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `submit_price` | `lib` | yes (delegated) | yes (`caller:source`) | #171, #172, #173, #174 |
| `submit_price_merkle` | `input_validation` | yes (delegated) | yes (`caller:source`) | #506 |
| `submit_price_relayed` | `relayer` | yes (delegated) | yes (`caller:relayer`) | — |
| `submit_price_with_external_proof` | `price_proof` | yes (delegated) | yes (`caller:source`) | #296 |
| `submit_price_with_nonce` | `lib` | yes (delegated) | yes (`caller:source`) | #171, #172, #173, #174 |
| `submit_price_with_proof` | `signed_submission` | yes (delegated) | yes (`signature`) | #216 |
| `submit_price_with_volume` | `reentrancy` | yes (delegated) | yes (`caller:source`) | — |
| `submit_prices` | `input_validation` | yes (delegated) | yes (`caller:source`) | #506 |
| `submit_prices_relayed` | `input_validation` | yes (delegated) | yes (`caller:relayer`) | #264, #266 |
| `submit_round_vote` | `consensus_rounds` | yes (delegated) | delegated / review | #397 |
| `submit_source_evidence` | `input_validation` | yes (delegated) | delegated / review | #506 |
| `subscribe` | `subscription` | yes (delegated) | yes (`caller:consumer`) | — |
| `subscribe_price_updates` | `price_update_subscription` | yes (delegated) | yes (`caller:consumer`) | #305 |
| `supports_interface` | `metadata` | no | no (`public-read`) | — |
| `trigger_aggregation` | `prices` | yes (delegated) | no (`public`) | — |
| `try_auto_renew` | `auto_renewal` | yes (delegated) | delegated / review | #289 |
| `try_finalize_price` | `reentrancy` | yes (delegated) | no (`public`) | — |
| `unfreeze_price` | `freeze` | yes (delegated) | yes (`admin`) | #223 |
| `unpause` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `unpause_asset` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `unregister_asset` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `unregister_price_callback` | `price_callback` | yes (delegated) | yes (`caller:consumer`) | #297 |
| `unsubscribe_price_updates` | `price_update_subscription` | yes (delegated) | yes (`caller:consumer`) | #305 |
| `update_foreign_asset_mapping` | `asset_registry` | yes (delegated) | yes (`admin`) | — |
| `upgrade` | `reentrancy` | yes (delegated) | yes (`admin`) | — |
| `vdf_get_current_seed` | `vdf_sampler` | no | no (`public-read`) | #181 |
| `vdf_get_sampling_size` | `vdf_sampler` | no | no (`public-read`) | #181 |
| `vdf_sample_sources` | `vdf_sampler` | no | no (`public-read`) | #181 |
| `vdf_set_sampling_size` | `vdf_sampler` | yes (delegated) | yes (`admin`) | #181 |
| `vdf_verify_proof` | `vdf_sampler` | no | no (`public-read`) | #181 |
| `verify_audit_chain` | `audit_log` | no | no (`public-read`) | #239, #674 |
| `verify_export` | `export_history` | no | no (`public-read`) | — |
| `verify_provenance` | `provenance` | no | no (public-read) | #493 |
| `withdraw_relayer_bond` | `relayer_bonds` | yes (delegated) | yes (`caller:relayer`) | #265 |
| `zk_get_verification_key` | `zk_verify` | no | no (`public-read`) | #175 |
| `zk_set_verification_key` | `zk_verify` | yes (delegated) | yes (`admin`) | #175 |
| `zk_submit_price` | `input_validation` | yes (delegated) | yes (`caller:source`) | #506 |
