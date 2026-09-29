# Adversarial Gas Budgets (#419)

Hot-path endpoints are gated on their **worst-case** CPU cost over a committed
adversarial corpus, not an average over friendly input.

* Corpus + gate: `contracts/price-oracle/src/gas_budget_tests.rs`
* Budgets: the `BUDGETS` table in that file
* Tolerance: `TOLERANCE_PCT = 5`
* Run: `make gas-gate` (also part of `make test`, so CI fails on regression)

Soroban metering is deterministic, so every budget is exactly reproducible
from the corpus on any machine.

## Corpus

All inputs use the largest shape that still fits network resource limits.
Anything larger is not a cost problem but an availability problem — the call
cannot be included in a ledger at all (see "Ceilings" below).

| Endpoint | Input | Budget (CPU instr.) |
|---|---|---|
| `submit_price` | 10th of 10 sources; prior 9 are reverse-sorted, alternating ±1 near-miss values so every value stays in the median window and nothing is skipped | 6 559 000 |
| `submit_prices` | 1 asset (largest callable batch), asset already holding 9 adversarial submissions | 6 262 000 |
| `get_all_prices` | asset with 10 submissions (largest set) | 756 000 |
| `trigger_aggregation` | 10 near-miss reverse-sorted submissions, full scan | 1 287 000 |

## Gates

1. **Budget gate** — each measured cost must be ≤ budget × 1.05. Failure
   message names the endpoint, input, measured cost, budget and delta:
   `GAS BUDGET EXCEEDED endpoint=submit_price input=nth_of_10_near_miss_reverse_sorted measured=… budget=… delta=+… (+…%)`.
2. **Marginal-cost gate** — the Nth caller's cost after adversarial history
   may be at most 10% above its cost after friendly (all-equal) history.
   Measured: friendly 6 524 801 vs adversarial 6 558 812 (+0.5%).
3. **Gate self-test** — `gas_gate_rejects_inflated_cost` feeds an inflated
   measurement and asserts the gate fails with the endpoint/input in the
   message, so the gate cannot silently become a no-op. To demonstrate on a
   branch: add a loop of redundant storage reads to `aggregate_asset`, see
   `make gas-gate` fail, revert, see it pass.

## Per-endpoint analysis: cheapest input that maximises cost

* **`submit_price`** — cheapest: a single ordinary submission arriving last,
  after N-1 sources submitted distinct in-window values. It forces a full
  source scan, sort and history write. Acceptable because the adversarial
  premium over friendly input is 0.5%: the cost is driven by source count,
  which is admin-controlled, not by values an attacker picks.
* **`submit_prices`** — cheapest: two fully populated assets in one batch. The
  cost ceiling is not CPU but footprint: 152 entries (> 100). A batch that
  exceeds limits simply fails for the submitter, so it cannot grief others,
  but it caps honest batching at one asset per call (filed as follow-up in
  `docs/gas-usage.md`).
* **`get_all_prices`** — cheapest: querying an asset with the maximum number
  of sources. Linear in source count, 756k CPU at 10 sources, well under the
  per-tx limit; acceptable.
* **`trigger_aggregation`** — cheapest: permissionless call on an asset with
  the maximum number of near-miss submissions. 1.29M CPU; bounded by source
  count; acceptable.

### Ceilings

At 11 sources per asset `submit_price` needs 102 footprint entries (> 100)
and becomes uncallable on-network. Keeping `max_sources ≤ 10` is therefore a
hard requirement until the footprint is reduced.

## Updating a budget

A budget may only be raised in a PR that:

1. Changes the value in `BUDGETS` and nothing else about the corpus, unless
   the corpus change is itself the subject of the PR.
2. States in the PR description the endpoint, old → new value, percentage
   change, and a written justification (what feature needs the cost, and why
   it cannot be cheaper).
3. Is approved by a reviewer other than the author, who explicitly signs off
   on the budget increase in their review.

Lowering a budget after an optimisation needs no justification.
