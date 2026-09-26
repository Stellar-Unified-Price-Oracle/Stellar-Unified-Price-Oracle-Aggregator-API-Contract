# Timelock Bypass & Queue Manipulation (#455)

Tests: `contracts/price-oracle/src/timelock_bypass_tests.rs`.

## Queue semantics after this change

- **Delay snapshot.** The tier delay is stored at proposal
  (`TlOpRequiredDelay`, `PendingBatchRequiredDelay`). Execution requires
  `max(snapshot, current delay)`. Lowering a delay therefore cannot speed up an
  operation that is already queued, and raising a delay still applies to it.
- **Per-element batch delay.** A batch waits for its slowest element.
  `Upgrade` and `SetAdmin` elements take the LongTerm tier (100 ledgers by
  default). All other elements take the Normal tier, and nothing waits less than
  the legacy 10-ledger batch delay.
- **Cancellation authority.** Only the admin (the only proposer role) can cancel.
  No third party can cancel an honest proposer's operation.
- **Re-queue.** The only way to re-queue is cancel plus propose. That yields a
  **new id** and a fresh clock, and emits `OperationCancelled` +
  `OperationProposed`, so the reset is detectable. The cancelled id can never
  execute.
- **Replay.** An executed id is removed and cannot run again.

## Route map

| Timelocked effect | Timelocked route | Alternative (non-timelocked) routes | Status |
|---|---|---|---|
| Upgrade WASM | `propose_batch` op 0 → `execute_batch` | `upgrade` (instant, admin) | **Bypass (TL-1)** |
| Set admin | `propose_operation` op 1 (effect not applied); batch op 1 (no-op) | `set_admin` (instant); guardian recovery | **Bypass (TL-1)** |
| Set min sources | batch op 2 | `set_min_sources_required` | **Bypass (TL-1)** |
| Set max history | batch op 3 | `set_max_history_length` | **Bypass (TL-1)** |
| Set resolution | batch op 4 | `set_resolution` | **Bypass (TL-1)** |
| Set decimals | batch op 5 | `set_decimals` | **Bypass (TL-1)** |
| Set description | batch op 6 (no-op) | `set_description` | **Bypass (TL-1)** |
| Set timestamp threshold | batch op 7 | `set_timestamp_threshold` | **Bypass (TL-1)** |
| Tier delays | — | `set_priority_delay`, `set_timelock_duration` (instant) | Mitigated for queued ops by the snapshot (`delay_shortening_does_not_affect_queued_*`) |

| Attack | Result | Test |
|---|---|---|
| (2) Shorten the delay after queueing | Blocked | `delay_shortening_does_not_affect_queued_op`, `delay_shortening_does_not_affect_queued_batch` |
| (3) Mixed-delay batch ships under the short window | Blocked | `mixed_batch_enforces_longest_element_delay`, `short_batch_executes_after_its_own_delay` |
| (4) Cancel another party's action | Blocked (admin-only) | `non_admin_cannot_cancel_queued_operation` |
| (5) Re-queue to reset the clock quietly | New id, fresh clock, evented | `requeue_resets_clock_under_new_id` |
| Execute twice | Blocked | `executed_operation_cannot_be_replayed` |
| Dependency before prerequisite | Batch elements run in array order inside one transaction, and any failure rolls back the whole batch. Single operations have no dependency model. | — |

## Findings requiring a design change

| ID | Severity | Finding |
|---|---|---|
| TL-1 | **High** | The timelock is advisory. `execute_operation` only deletes the queued entry and emits an event; it never applies the effect. Every effect it covers is also reachable instantly through a direct admin endpoint (table above). The acceptance criterion "no route without the delay" cannot hold until the direct setters are removed or routed through the queue. That is a breaking API change, which needs a governance decision. |
| TL-2 | Medium | Priority is chosen by the proposer. Any operation type may be proposed as `Urgent` (1 ledger). Fix: a minimum tier per operation type (the batch path now applies this rule). |
| TL-3 | Low | The batch base delay reads `DataKey::TimelockDuration`, which nothing writes. `set_timelock_duration` writes `CfgTimelockDuration`. The per-element rule now makes the Normal tier (which inherits `CfgTimelockDuration`) the effective floor. |
