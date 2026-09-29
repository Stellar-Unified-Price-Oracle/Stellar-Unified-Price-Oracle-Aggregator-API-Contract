# Reputation Gaming and Laundering (#465)

Tests: `contracts/price-oracle/src/reputation_gaming_tests.rs`.

## What reputation gates

| Gate | Effect | Worth of one point |
|---|---|---|
| Slash eligibility (`slash_source`) | Score `< slash_threshold` (default 20) makes the source slashable without `force`. | Protects `slash_percent` (20%) of stake. |
| `WeightedMedian` aggregation method | Score is a weight, only when the admin selects method 3. | Marginal: weight share in a median, never a veto. |

Reputation does **not** gate admission, submission, or the default median, so farmed
trust cannot by itself move a published price.

## Scoring model

`new = floor((old * (100 − d) + accuracy * d) / 100)`, `d` = decay factor (default 5),
accuracy 100 within 1% of median, 0 at ≥ 50% deviation.

## Findings

| Id | Severity | Finding | Test |
|---|---|---|---|
| R-1 | Low | Integer flooring makes **81** a fixed point: no behaviour, honest or farmed, can exceed 81 with the default decay. Farming from 50 to 80 costs exactly **24** accurate submissions; the ceiling bounds what farming can buy. | `farming_cost_is_quantified` |
| R-2 | Info | Records are keyed by address and survive `remove_source`/`add_source`; a fresh identity cannot submit until the admin admits it, so laundering needs admin collusion. A fresh identity starts at neutral 50 — the admin must treat re-admission of a known operator under a new key as a policy decision. | `identity_rotation_cannot_shed_record` |
| R-3 | Info | Idle decay pulls a farmed score back toward 50 (it stops within 20 points of neutral because of flooring); repeated outliers drive the source below the slash threshold. | `quiet_then_abuse_requires_sustained_performance` |
| R-4 | Info | Scores depend only on the source's own deviation from the median; a competitor cannot lower an honest source's score. | `third_party_cannot_suppress_honest_source` |
| R-5 | Info | Updates are applied in the same call as the behaviour; the largest single-step move is `d` points (5), so there is no exploitable gap. | `update_gap_is_bounded_to_one_step` |

## Cost vs gain

Reaching the practical maximum (80–81) costs ~24 honest submissions (plus fees); one
malicious submission then costs ≤ 5 points and, because reputation does not gate the
default median, yields no price influence beyond that of any single source. Margin:
farming is **not** cheaper than earning honestly — they are the same action.
