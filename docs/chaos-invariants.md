# Chaos Invariants

Tests: `contracts/price-oracle/src/chaos_tests.rs`
(`cargo test -p price-oracle --lib chaos_`).

Every fault is paired with a hostile variant: "survives chaos" means
"survives an attacker during chaos". The attacker's advantage column states
what the hostile test asserts the adversary gains.

| ID | Invariant | Fault survived | Hostile variant (test) | Attacker advantage |
|---|---|---|---|---|
| INV-1 | A price is finalized at most once, and a reorg-retracted price never finalizes. | Ledger reorg around `finality.rs` | Re-calling the permissionless `mark_price_pending` to resurrect a retracted entry (`chaos_reorg_hostile_resurrection_of_retracted_price_rejected`); a non-admin retracting (`chaos_reorg_hostile_non_admin_cannot_retract_finalized_or_pending`) | Zero: rejected with `PriceRetracted` / `AlreadyFinalized` / auth failure |
| INV-2 | Finalizing an older committed ledger after a newer one never rolls the finalized price back. | Out-of-order finalization after a reorg | `chaos_out_of_order_finalization_keeps_newest_price` | Zero: newest committed ledger wins |
| INV-3 | A delayed submission never silently overwrites a newer one from the same source. | Delayed / out-of-order delivery | Replaying an older signed price with a fresh nonce (`chaos_delayed_submission_hostile_replay_with_explicit_nonce_rejected`) | Zero: rejected with `InvalidTimestamp` |
| INV-4 | A partitioned minority of sources cannot produce an aggregate or move the median. | Source partition | Hostile minority submitting extreme prices (`chaos_partition_hostile_minority_cannot_move_median`) | Bounded: median stays within the honest range |
| INV-5 | Staleness checks are conservative in both directions of ledger-time skew. | Clock/ledger-time skew | Source timestamp ahead of ledger (`chaos_skew_source_clock_ahead_bounded_by_threshold`); ledger jumping forward (`chaos_skew_ledger_clock_jumps_forward_fails_closed`) | Bounded by the timestamp threshold; stale reads fail closed |
| INV-6 | Storage growth is bounded and TTL eviction fails loudly, never silently. | Storage exhaustion, TTL eviction | History flood (`chaos_storage_flood_history_stays_bounded`); re-writing evicted history (`chaos_ttl_lapse_fails_closed_and_blocks_rewrite`) | Zero: history capped; reads after lapse error instead of returning stale data |

## Resolution rules

- **Delayed submissions:** a submission whose timestamp is older than the
  source's last stored submission is rejected with `InvalidTimestamp`
  (`prices.rs::submit_price`). Equal timestamps are accepted.
- **Finality:** `mark_price_pending` refuses to overwrite a `Finalized` or
  `Retracted` entry; `try_finalize_price` only replaces the finalized price
  when the committed ledger is newer (`finality.rs`).
