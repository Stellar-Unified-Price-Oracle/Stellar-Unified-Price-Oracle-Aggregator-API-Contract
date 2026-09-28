# Capacity Planning — Ingest, Storage and Ledger Budget (#530)

Growth in assets and sources is the main driver of cost and of failure.
Without a model, capacity is discovered by outage and the overload response is
improvised. This document projects the three resources the pipeline competes
for, states the targets it is held to, and defines what gets given up first
when a target is at risk.

The model is [`services/capacity/model.py`](../services/capacity/model.py).

```bash
python -m services.capacity.model --assets 20 --sources 5 --submission-rate-hz 8
```

---

## 1. Cost drivers

Three resources, each with an explicit per-unit cost. The driver values live in
`Drivers` in the model and are re-fitted from measurement every quarter (§6).

| Resource | Unit | Driver | Default cost | Source of the number |
|---|---|---|---|---|
| **ingest** | CPU-seconds of pipeline work per wall second | `sources x assets x submission rate` | 0.004 CPU-s per submission | measured on the pipeline host |
| **storage** | retained bytes | `submission rate x retention + assets` | 512 B per record, 32 KiB per asset, 24 h retention | measured; retention is the `storage_retention_hours` driver |
| **ledger_budget** | CPU instructions per hour | `submission rate` + `assets x history depth` | 1 287 000 per submission, 76 000 per history entry | `docs/gas-budget.md` budgets |

The ledger figures are the published gas budgets, so the model and the gas gate
cannot drift apart silently; `test_ledger_cpu_driver_matches_the_published_gas_budget`
asserts it.

### Why the ledger budget gets the largest margin

The ledger is shared with unrelated network activity. A busy hour caused by
someone else's transactions consumes the same instruction budget we compete
for, and we cannot shed it. That is why `ledger_budget` carries a 50 % headroom
target while ingest carries 30 %: it is the resource with the least control, so
it gets the most margin. A margin on a resource you control buys safety; a
margin on one you do not is the only thing that does.

### Growth projections

| Driver | Growth assumption | Effect at 4x |
|---|---|---|
| Assets | +25 % per quarter in the current onboarding pace | storage and history cost scale linearly; ingest does not |
| Sources per asset | capped at **10** | at 11 sources `submit_price` needs 102 footprint entries and becomes uncallable on-network (`docs/gas-budget.md`, Ceilings) — this is a hard ceiling, not a target |
| Submission rate | +15 % per quarter | the dominant driver of ingest and ledger cost |

The source ceiling is the one growth assumption that cannot be tuned: it is a
network limit. Onboarding a source past it is not a capacity problem to be
solved, it is an uncallable endpoint.

## 2. Headroom and lag targets

| Target | Value | Rationale |
|---|---|---|
| ingest headroom | 30 % | absorbs a source burst and a re-fetch after a network partition |
| storage headroom | 25 % | compaction and restore both need room to work |
| ledger_budget headroom | 50 % | shared, uncontrollable resource (§1) |
| **lag target** | **60 s** | the SLA §1.2 freshness target; the aggregate is never usefully "a bit late" |
| lag pre-alert | 45 s | fires before the target is missed, not after |

Headroom is checked continuously; lag is checked per asset. Both are exported as
metrics so the alerts below have data to fire on:

```
oracle_capacity_headroom_ratio{resource="ingest|storage|ledger_budget"}
oracle_capacity_lag_seconds{resource="lag"}
```

## 3. Load-shedding policy

Shedding is **progressive and ordered**, cheapest consumer-visible cost first.
The policy degrades gracefully: it reduces work we chose to do before work
consumers depend on, and it never increases load.

| Order | Priority | Step | Consumer-visible cost |
|---|---|---|---|
| 1 | best-effort | drop historical backfill and replay of old ticks | none |
| 2 | best-effort | drop non-critical assets outside the settlement window | none for settlement assets |
| 3 | standard | reduce submission frequency on the least-traded assets | longer staleness on low-value pairs |
| 4 | standard | disable optional analytics (anomaly scoring, forecasting) | none for prices |
| 5 | **critical** | degrade signature verification to the cached-key fast path | reduced defence in depth — last resort, and an incident in itself |

Rules the policy obeys, each asserted by a test:

* **The critical path is never dropped.** Settlement-window assets keep
  publishing under every step. A correct price for the assets that matter is
  worth more than a fresh price for all of them.
* **Shedding lowers offered load; it never raises it.** No step increases
  submission volume.
* **Steps are applied only as far as pressure requires.** A mildly degraded
  plan sheds one step, not all five. The number of steps is derived from how
  far headroom is below target, so shedding is proportional.
* **Step 5 is an incident.** Degrading signature verification reduces a
  security control; it requires a page and a postmortem, not a silent toggle.

```bash
python -m services.capacity.model --assets 200 --sources 9 --submission-rate-hz 90
```

prints the ordered shedding plan for that profile.

## 4. Alerting before headroom is exhausted

The point of a headroom target is that it is defended *before* the resource is
gone. The model emits pre-exhaustion alerts, and the rules for them live with
the other capacity signals:

| Alert | Condition | Severity | Meaning |
|---|---|---|---|
| `OracleCapacityHeadroomLow` | headroom below the target for that resource | warning | we are approaching the limit; shed or scale now |
| `OracleCapacityExhausted` | used ≥ capacity | critical | the resource is gone; load is being dropped or failing |
| `OracleCapacityLagApproachingTarget` | lag above 45 s | warning | the freshness target is at risk |
| `OracleCapacityLagApproachingTarget` | lag above 60 s | critical | the SLA §1.2 target is being missed |

`test_headroom_alert_fires_before_exhaustion` asserts that every resource
flagged `at_risk` is not yet exhausted — a warning that can only fire after the
fact is not a pre-exhaustion alert.

These alerts are handled through the standard path: each is mapped to a
runbook entry in [`docs/runbook.md`](runbook.md) and its mitigation is a step
from the shedding table above.

## 5. Validation against measured usage

A model nobody checks against reality is a story. `compare_to_measured()`
compares a projection with a recorded measurement and reports the worst
relative error against a **stated tolerance of 20 %**
(`TOLERANCE` in the model).

The validation is a test, not a ritual:
`test_model_reproduces_measured_ingest_within_tolerance` and
`test_model_reproduces_measured_ledger_budget_within_tolerance` project the
reference deployment (20 assets, 5 sources, 8 submissions/second) and compare
it against a month of recorded measurements. If a driver, a gas budget or a
retention setting changes materially, those tests fail and the model is
re-fitted before it is trusted.

Outside tolerance is not a suggestion to ignore: it means either a driver is
wrong or the workload changed shape. Both are findings for the quarterly review
(§6), and both are recorded there.

## 6. Quarterly review

Capacity is re-fitted against measurement once a quarter, on a calendar task
owned by `core-contracts`. The review is short and its output is written down:

1. **Export the measurements** — ingest CPU, retained bytes, ledger instructions
   per hour, and p95 lag, for the last quarter at the reference profile.
2. **Re-run the validation** (§5). Record the worst relative error per
   resource. Anything above tolerance is investigated before anything else.
3. **Re-fit the drivers.** Update `Drivers` and `DEFAULT_CAPACITIES` to match
   measurement, and state the old → new value and why.
4. **Re-project the next four quarters** (§1) and check that the headroom
   targets still hold at the projected growth. If they do not, either the
   target or the growth assumption is wrong, and the change goes through
   governance rather than being quietly absorbed.
5. **Update the record** in this section with the date, the error figures and
   what changed, so the model has a visible history rather than a silent one.

Reviewing the projections against measured usage is the step that keeps this
document honest. A model that is only consulted during an incident is not a
capacity model.

## 7. Out of scope

Procuring or resizing infrastructure is explicitly out of scope for this
issue. The model states what is needed and when; the decision to buy or resize
lives elsewhere. Stellar network data availability is likewise out of scope.

## 8. Related

* [`docs/gas-budget.md`](gas-budget.md) — the per-endpoint CPU budgets used as drivers
* [`docs/runbook.md`](runbook.md) — what to do when a capacity alert pages
* [`docs/SLA.md`](SLA.md) — the 60 s freshness target behind the lag budget
* [`docs/monitoring/`](monitoring/README.md) — the metrics pipeline these gauges join
