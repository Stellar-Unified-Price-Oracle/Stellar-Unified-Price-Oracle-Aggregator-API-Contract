# Event Census and Indexing Integrity (#469)

Tests: `contracts/price-oracle/src/event_integrity_tests.rs`.

## Trust rule for monitors

Soroban attaches the **emitting contract id** to every event; a caller cannot set it.
Monitors MUST filter by the oracle's contract id. An event with a matching topic from
any other contract id is untrusted — relying on topic/payload alone is **unsafe**
(`untrusted_contract_cannot_emit_privileged_looking_event_under_oracle_id`).

## Census (state-modifying events)

| Event | Emitter | Actor field | Attacker-influenceable fields | Impersonation risk |
|---|---|---|---|---|
| `SourceAddedEvent` | admin (`add_source`) | `admin` (topic) | none | None under contract-id filter |
| `SourceRemovedEvent` | admin (`remove_source`, removal cooldown) | `admin` (topic) | none | None |
| `AssetRegisteredEvent` / `AssetUnregisteredEvent` | admin | `admin` (topic) | none | None |
| `AdminChangedEvent` | admin (`set_admin`) | `old_admin` (topic) | none | None |
| `ContractUpgradedEvent` | admin | **none** (only `new_wasm_hash`) | none | Low: not impersonable under the contract-id filter, but monitors must attribute it via the transaction invoker / admin audit log |
| `PriceSubmittedEvent` | source (`submit_price`) | `source` (topic) | `price`, `timestamp` (bounded by validation) | None; values are the source's own claim |
| `PriceUpdatedEvent` / `PriceAggregatedEvent` | contract (aggregation) | contract | derived from submissions (median) | None |
| `SourceReputationUpdatedEvent` | contract | `source` | derived | None |
| `admin_action` (`emit_admin_action`) | admin | `admin` | `data` bytes | Payload is admin-supplied; do not parse as authorization |
| Configuration events (`MinSourcesChanged`, `MaxHistoryChanged`, …) | admin | `admin` where present | new value | None |

The remaining ~200 feature-specific events (fee market, relayer, cross-chain, etc.) are
emitted only after the corresponding `require_auth` succeeds; the same contract-id rule
applies.

## Guarantees verified by test

| Criterion | Test |
|---|---|
| No untrusted caller can emit a privileged-looking event under the oracle id | `untrusted_contract_cannot_emit_privileged_looking_event_under_oracle_id` |
| State-modifying events carry the actor | `state_modifying_events_carry_actor` |
| Rejected operations emit no event that could read as success | `rejected_operations_emit_no_success_events`, `rejected_unauthorized_admin_call_emits_nothing` |
| Event-log reconstruction matches state across a lifecycle | `event_log_reconstruction_matches_state` (source set rebuilt from `SourceAdded`/`SourceRemoved` after every step) |

Failed invocations roll back their events, so a rejection is signalled by the
transaction result, never by a success-shaped event.
