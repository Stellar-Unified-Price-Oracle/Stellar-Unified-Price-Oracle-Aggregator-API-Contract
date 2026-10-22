# Admin Key Compromise: Blast Radius & Recovery Drill (#454)

Tests: `contracts/price-oracle/src/admin_compromise_tests.rs`.

## Threat model

The attacker holds the admin key and can sign any admin-gated call. The
guardian quorum (`recovery_*`) is the revocation authority.

## Capability classes

The full endpoint-by-endpoint list is in the appendix: 208 admin-gated functions
across 59 modules. It was generated from every non-test function whose body
authorises against the admin. Each function falls into one of these classes.

| Class | Examples | Reversible by new admin? | Status |
|---|---|---|---|
| **Code replacement** | `upgrade` (instant `update_current_contract_wasm`) | **No.** The new WASM can delete the recovery module. | **Accepted risk (critical).** Only guardian monitoring of `ContractUpgradedEvent` catches it. Gating `upgrade` behind the timelock would break the current upgrade flow and needs a governance decision. See finding AC-1. |
| **Authority transfer** | `set_admin`, `recovery_set_guardians`, `recovery_set_delay`, `recovery_cancel` | Yes, after recovery | **Mitigated in this change.** See below. |
| **Source / asset admission** | `add_source`, `add_source_asset`, `register_asset`, bridge trust config (`set_axelar_*`, `set_lz_*`, `register_foreign_asset_mapping`) | Yes: `remove_source` etc. Rogue prices already published remain in history. | Accepted. Detection is by admin events. |
| **Parameter changes** | `set_min_sources_required`, `set_max_price_deviation`, `set_timestamp_threshold`, tier delays | Yes (`config_history` snapshots allow a rollback) | Accepted, bounded by `config_bounds` checks. |
| **Availability** | `pause`, emergency pause, freezes | Yes | Accepted. Recovery runs while paused (`recovery_*` does not call `check_not_paused`). |
| **Treasury / fees** | fee market, treasury withdrawals | **No** (funds leave) | Accepted risk. Bounded by `admin_op_limits` daily caps where configured. |

## Revocation cannot be blocked (fixed in this change)

Before this change, the compromised admin could block its own removal in three
ways. Each is now closed:

| Attack | Fix | Test |
|---|---|---|
| Replace guardians with puppets while a recovery is pending | `recovery_set_guardians` / `recovery_set_delay` are rejected while a recovery is pending or a veto cooldown is active (`RecoveryAlreadyPending`) | `compromised_admin_cannot_swap_guardians_mid_recovery`, `compromised_admin_cannot_stretch_delay_mid_recovery`, `veto_cooldown_blocks_guardian_swap` |
| Stretch the delay to `u32::MAX` (never executes, or overflows `ready + delay`) | Delay capped at `MAX_RECOVERY_DELAY` (172 800 ledgers); `saturating_add` in execute | `recovery_delay_is_bounded` |
| Cancel every recovery forever | One veto per candidate. Once guardians re-reach quorum for a vetoed candidate, `recovery_cancel` fails with `NotAuthorized`. The veto also opens a cooldown (one recovery delay) in which guardians cannot be swapped. | `compromised_admin_gets_only_one_veto_per_candidate` |
| Hand the admin to a second attacker key | A pending recovery still executes against the new admin | `admin_rotation_does_not_evade_recovery` |

## Recovery drill

Scripted and executable as `recovery_drill_end_to_end`
(`cargo test -p price-oracle --lib recovery_drill`):

1. **Detect.** The attacker admits a rogue source (`SourceAdded` / admin-action events).
2. **Revoke.** Guardians reach quorum. The attacker spends its single veto, and
   guardians re-approve the same candidate.
3. **Rotate.** After the cancellation window, `recovery_execute` installs the
   rescue admin.
4. **Verify state.** The new admin is `get_admin()`, and no recovery is pending.
5. **Reinstate.** The rogue source is removed and an honest source is re-admitted.

### Rehearsal result (2026-09-26, soroban-sdk 26 test host)

| Metric | Result | SLA (`docs/SLA.md` §5, P0) |
|---|---|---|
| Time-to-detect | Event-driven. Bounded by the monitoring poll interval, which is not measured on-chain. | 30 min response |
| Time-to-contain (quorum → admin rotated) | `recovery_delay` ledgers: 100 in the drill (~8 min). **Default 17 280 ledgers ≈ 24 h.** | 2 h resolution |

**The default recovery delay does not meet the P0 2-hour resolution SLA.**
Operators should set `recovery_set_delay` to at most ~1 400 ledgers (~2 h) once
guardians are trusted, or the SLA should carve out admin compromise.

## Residual risk

- **AC-1 (Critical).** `upgrade` is instant and unilateral. A compromised admin can
  replace the code, including the recovery path, before guardians react. Recommended
  fix: route `upgrade` only through the LongTerm timelock, or require guardian
  co-signature.
- **AC-2 (High).** Treasury withdrawals and published rogue prices are irreversible.
  Containment is limited to the detection latency.
- **AC-3 (Medium).** A malicious guardian quorum outranks an honest admin: the admin
  gets only one veto per candidate. This is the intended trust ordering. Guardians
  must be held to a higher custody standard than the admin key.
- **AC-4 (Low).** An honest admin cannot rotate guardians while a recovery is pending
  or during a veto cooldown (one recovery delay).

## Appendix: admin-gated functions by module

| Module | Functions |
|---|---|
| `admin.rs` | `initialize`, `upgrade`, `set_admin`, `get_admin_address`, `set_min_sources_required`, `set_max_history_length`, `set_resolution`, `set_decimals`, `set_description`, `set_aggregation_method`, `set_timestamp_threshold`, `set_max_price_deviation`, `set_circuit_breaker_threshold`, `set_heartbeat_interval`, `set_max_history_per_asset`, `set_max_events_per_call`, `set_max_aggregation_sources`, `set_asset_resolution`, `set_aggregation_cooldown`, `set_optimistic_dispute_window`, `set_optimistic_min_bond`, `set_min_submission_interval`, `set_interpolation_enabled`, `set_max_sources`, `set_query_rate_limit`, `set_max_assets`, `set_subscription_price`, `set_compaction_threshold_bps` |
| `admin_op_limits.rs` | `set_admin_op_daily_limit`, `validate_admin_op_allowed` |
| `alert_severity.rs` | `set_severity_thresholds`, `set_asset_severity_thresholds` |
| `alerts.rs` | `set_max_subscriptions`, `set_subscription_ttl` |
| `amm.rs` | `init_amm`, `set_amm_status`, `set_amm_max_deviation_bps`, `set_amm_weight`, `register_soroswap_pool`, `set_soroswap_pool_status` |
| `asset_inactivity.rs` | `set_inactivity_timeout`, `set_asset_inactivity_timeout`, `check_and_deregister_if_inactive` |
| `asset_registry.rs` | `register_foreign_asset_mapping`, `update_foreign_asset_mapping`, `remove_foreign_asset_mapping` |
| `assets.rs` | `register_asset`, `unregister_asset`, `set_asset_metadata`, `batch_set_asset_metadata`, `set_min_price`, `set_price_bounds`, `pause_asset`, `unpause_asset` |
| `axelar_gmp.rs` | `set_axelar_gateway`, `set_axelar_trusted_source`, `remove_axelar_trusted_source` |
| `bridge_oracle.rs` | `register_bridge_oracle` |
| `calibration.rs` | `set_calibration_config`, `set_calibration_benchmark`, `record_calibration_sample` |
| `challenger.rs` | `resolve_challenge` |
| `config_history.rs` | `rollback_config` |
| `consumer_auth.rs` | `add_authorized_consumer`, `remove_authorized_consumer`, `set_consumer_access_mode`, `block_consumer`, `unblock_consumer` |
| `contribution_quality.rs` | `set_scoring_window` |
| `correlation.rs` | `set_correlation_pair`, `remove_correlation_pair`, `clear_correlation_flag` |
| `cross_chain_relay.rs` | `set_relay_config` |
| `cross_chain_verify.rs` | `set_cross_chain_verification_enabled`, `set_cross_chain_deviation_threshold`, `store_cross_chain_price`, `submit_cross_chain_price` |
| `cross_reference.rs` | `add_reference_oracle`, `remove_reference_oracle`, `set_cross_ref_deviation_bps` |
| `dex.rs` | `register_dex_pool` |
| `did.rs` | `register_did`, `link_source_did` |
| `ecosystem_metadata.rs` | `register_ecosystem_metadata`, `update_ecosystem_metadata`, `register_feed_metadata` |
| `emergency_pause.rs` | `emergency_pause`, `extend_emergency_pause`, `cancel_emergency_pause` |
| `eth_bridge.rs` | `set_eth_bridge_config`, `map_erc20_asset` |
| `exotic_pricing.rs` | `set_exotic_asset_config` |
| `external_governance.rs` | `set_external_governor`, `get_external_governor`, `clear_external_governor`, `allow_governor_op`, `disallow_governor_op` |
| `fee_market.rs` | `set_min_priority_fee`, `set_fee_distribution_ratio`, `set_treasury_address` |
| `finality.rs` | `set_finality_ledgers`, `retract_price` |
| `freeze.rs` | `freeze_price`, `unfreeze_price` |
| `ibc_oracle.rs` | `update_light_client`, `submit_consensus_state`, `register_ibc_asset_mapping` |
| `layerzero.rs` | `set_layerzero_endpoint`, `set_lz_chain_name`, `set_trusted_remote`, `remove_trusted_remote` |
| `migration.rs` | `migrate_storage` |
| `multisig.rs` | `set_governors`, `execute_ms_operation`, `cancel_ms_operation` |
| `notifications.rs` | `set_notification_preference`, `clear_notification_preferences` |
| `optimistic.rs` | `resolve_dispute`, `resolve_via_external_data` |
| `pause.rs` | `pause`, `unpause` |
| `per_asset_decimals.rs` | `set_asset_decimals`, `clear_asset_decimals` |
| `price_proof.rs` | `set_asset_proof_requirement` |
| `prices.rs` | `set_bft_parameters`, `override_price`, `remove_price_override`, `set_commit_window`, `set_reveal_window`, `set_commit_reveal_enabled`, `set_commit_reveal_slash_amount` |
| `pruning.rs` | `set_asset_retention_window`, `remove_asset_retention_window`, `prune_history` |
| `rate_limiting.rs` | `grant_enterprise_tier`, `revoke_enterprise_tier` |
| `rbac.rs` | `has_role`, `delegate_role`, `revoke_role`, `get_address_roles`, `get_roles_for_holder` |
| `recovery.rs` | `set_guardians`, `set_recovery_delay`, `cancel_recovery`, `execute_recovery` |
| `relayer.rs` | `add_relayer`, `remove_relayer`, `set_relayer_fee_per_submission` |
| `relayer_bonds.rs` | `set_relayer_bond_amount`, `record_relayer_failure`, `slash_relayer`, `set_relayer_slash_percent`, `set_relayer_failure_threshold`, `set_relayer_reward_rate` |
| `reputation.rs` | `slash_source`, `sweep_treasury`, `set_stake_token_contract`, `set_decay_factor`, `set_slash_percent`, `set_slash_threshold` |
| `rotation.rs` | `set_source_schedule`, `disable_rotation` |
| `scheduling.rs` | `admin_remove_schedule` |
| `source_deviation.rs` | `set_source_deviation_tolerance` |
| `sources.rs` | `add_source`, `add_source_with_assets`, `remove_source`, `add_source_asset`, `remove_source_asset`, `set_source_verification`, `set_demerit_config`, `reset_source_demerits`, `set_removal_cooldown`, `mark_source_for_removal`, `cancel_source_removal`, `finalize_source_removal`, `set_reputation_decay_factor`, `set_max_inactive_ledgers`, `set_heartbeat_window`, `set_source_governance`, `set_source_geo`, `set_source_bond` |
| `state_introspection.rs` | `build_state_dump`, `build_state_analysis` |
| `submission_deadline.rs` | `start_aggregation_round`, `clear_current_round` |
| `subscription.rs` | `set_subscription_token`, `distribute_subscription_fees`, `refund_subscription_payment` |
| `timelock.rs` | `set_priority_delay`, `propose_operation_with_priority`, `execute_operation`, `cancel_operation`, `set_timelock_duration`, `propose_batch`, `execute_batch`, `cancel_batch` |
| `triggers.rs` | `set_time_trigger`, `set_submission_threshold_trigger`, `set_deviation_trigger` |
| `vdf_sampler.rs` | `set_sampling_size` |
| `whitelisting.rs` | `set_tier_pricing`, `set_xlm_token_contract`, `set_whitelist_treasury`, `sweep_fees` |
| `wormhole_relay.rs` | `set_guardian_set`, `set_chain_mapping` |
| `zk_verify.rs` | `set_verification_key` |
