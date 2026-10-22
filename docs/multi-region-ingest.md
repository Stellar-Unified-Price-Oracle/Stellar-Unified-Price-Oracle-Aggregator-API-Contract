# Multi-Region RPC and Ingest Redundancy

**Issue:** #526 — make the off-chain ingest and RPC path redundant across
regions with health-checked failover, and document the behaviour and alerting
when a region degrades.

A single-region ingest path is a single point of failure for the whole oracle:
if the region hosting the submission bot or its RPC endpoint goes away, prices
go stale and every downstream consumer is affected within one freshness window
(SLA §1.2, 60 s). Redundancy with automatic failover turns that from an incident
into a blip.

**Scope note:** this is the *off-chain* ingest/RPC path only. There are no
on-chain contract changes — the contract's own source-count and staleness rules
are unaffected by how many regions are polling.

Implementation: `services/ingest_failover/router.py`.
Alerts: `docs/monitoring/alerts-v2.yml`, group `oracle-ingest-regions`.

```bash
# Run the failure drill and print the failover report + cost model
python -m services.ingest_failover.router --victim us-east

python -m pytest services/ingest_failover -q   # 21 tests
```

---

## 1. Topology

```
                    ┌──────────────────────────┐
   source feeds ───▶│  regional ingest workers  │
                    │  us-east │ eu-west │ ap-south│
                    └────┬────────┬────────┬──────┘
                         │        │        │
                    ┌────▼────────▼────────▼──────┐
                    │  replicated dedupe ledger    │  at-most-once
                    │  (submission key → outcome)  │  intent log
                    └────┬────────┬────────┬──────┘
                         │        │        │
                    ┌────▼────────▼────────▼──────┐
                    │      Stellar RPC (Soroban)   │
                    └────────────────────────────┘
```

Three regions (`us-east`, `eu-west`, `ap-south`), each running a full ingest
worker with its own RPC endpoint. Exactly one is **active** at a time; the others
are warm standbys that are health-checked continuously. The dedupe ledger is
replicated to every region — deliberately, because the guard against a
double-submit cannot live in the one region that may be the one that died.

## 2. Idempotency: the split-brain guard

Each submission carries a deterministic key derived from its **logical**
identity, not from the region that happened to send it:

```python
key = sha256(f"{source}|{asset}|{ledger}")[:32]
```

Because the key excludes the region, the same logical submission retried in
another region produces the same key, and the replicated ledger suppresses it.
The key is written to the ledger **before** the transport call (a write-ahead
intent), so two regions can never race for the same submission.

### Two failure modes, handled differently

This distinction is the core of the design, and getting it wrong in either
direction is a real bug:

| Failure | Meaning | Handling |
|---|---|---|
| **Refused** (`Unreachable`) | The request provably never reached the region — DNS failure, TCP reject, load-balancer 503 before forwarding. The write did **not** happen. | Fail over to the next region and re-send. The submission is not lost. |
| **Ambiguous** (timeout, connection reset, dropped response) | The write may or may not have reached the chain, and the ingest path cannot tell. | **Do not re-send.** The key is *quarantined* for reconciliation. |

Re-sending an ambiguous submission is precisely the split-brain double-submit
the issue warns about. Preferring a *detectable gap* over a *silent
double-count* is the right trade for an oracle: the existing freshness alerts
(SLA §1.2 / §6.2) fire on a missed round, whereas a double-counted submission
silently corrupts the aggregate with no alarm at all.

`services/ingest_failover/test/test_router.py` asserts both branches:
`test_failover_does_not_double_submit_the_ambiguous_submission` (the write
lands once, the key is quarantined, the transport is never called twice) and
`test_region_failure_fails_over_automatically` (a refused region loses nothing).

## 3. Health checks, failover, and anti-flapping

- **Threshold** — a region is marked `unhealthy` only after
  `failure_threshold` (default 3) consecutive failures, so a single blip does
  not move traffic. One failure alone leaves the region `healthy`.
- **Cooldown** — a degraded region is not re-probed for `cooldown_secs`
  (default 60). This prevents both hammering a region that is still down and
  the router itself flapping between regions.
- **Recovery** — after the cooldown, the region is re-probed; on success it
  returns to `healthy` and rejoins the pool.
- **Failover is immediate on submit** — if the active region refuses a write,
  the router moves to the next healthy region within the same submission. No
  polling interval sits between the failure and the recovery.
- **Total outage is loud, not silent** — with every region down, submissions are
  rejected with `no healthy region available` and an `unavailable` event is
  recorded. Nothing is dropped quietly.

## 4. The failover drill

`run_drill()` fails one region mid-run and asserts both acceptance criteria at
once. Run it with:

```bash
python -m services.ingest_failover.router --victim eu-west
```

Observed with the three-region default and 20 submissions:

```json
{
  "region_failed": "us-east",
  "active_before": "us-east",
  "active_after": "eu-west",
  "submissions_before": 10,
  "submissions_after": 20,
  "duplicates_suppressed": 0,
  "double_submits": 0,
  "passed": true
}
```

The region fails after submission 10; the router fails over; all 20 logical
submissions land **exactly once**. The drill is parameterised over every region
(`test_drill_passes_for_every_region`), so no single region is privileged, and
it is part of the normal test suite rather than a runbook step someone has to
remember.

## 5. Recovering quarantined submissions

An ambiguous submission is quarantined, not retried. Reconciliation resolves it:

1. Read the quarantined keys (exported as `ingest_quarantined_submissions`, and
   available from the ledger's `PENDING` entries).
2. Query the chain for `(source, asset, ledger)` and check whether a matching
   `PriceSubmitted` event exists.
3. If it landed, mark the key resolved — nothing further to do.
4. If it did not, the submission is re-offered as a **new** logical submission
   (a new ledger), so the write-ahead intent cannot suppress it.

Reconciliation runs on a schedule and is alerted on while the quarantine is
non-empty (`IngestAmbiguousSubmissionsQuarantined`), so a lost round surfaces
within minutes rather than at the next audit.

## 6. Alerting and observability

Metrics emitted by `RegionRouter.prometheus()`:

| Metric | Type | Used by |
|---|---|---|
| `ingest_region_up{region}` | gauge | `IngestRegionDegraded` |
| `ingest_region_submissions_total{region}` | counter | capacity / traffic split |
| `ingest_failovers_total{from,to}` | counter | `IngestRegionFailover`, `IngestFailoverFlapping` |
| `ingest_region_degraded_total{region}` | counter | post-incident review |
| `ingest_duplicate_submissions_total` | counter | split-brain guard activity |
| `ingest_quarantined_submissions` | gauge | `IngestAmbiguousSubmissionsQuarantined` |

Alerts (`docs/monitoring/alerts-v2.yml`, group `oracle-ingest-regions`):

| Alert | Severity | Fires when |
|---|---|---|
| `IngestRegionDegraded` | warning / P2 | A region is out of rotation |
| `IngestRegionFailover` | warning / P2 | Traffic moved between regions |
| `IngestFailoverFlapping` | critical / P1 | >3 failovers in 15 m — regions oscillating |
| `IngestAllRegionsDown` | critical / P0 | No region can submit (30 m first response, SLA §5) |
| `IngestAmbiguousSubmissionsQuarantined` | warning / P2 | An ambiguous outcome needs reconciliation |

Flapping gets its own alert because oscillation is a distinct failure mode from
sustained degradation: it costs latency on every switch and can exhaust the
region pool even though no region is truly down.

Every failover and degradation is also recorded as a structured
`FailoverEvent` (kind, from, to, reason, sequence), so a drill and a real
incident produce the same auditable record.

## 7. Cost, measured and justified

`cost_summary()` puts both sides of the trade on the table with explicit inputs,
so the decision can be re-run against real numbers rather than defended with a
gesture.

| Input | Value |
|---|---|
| Per-region ingest worker | $38.00 / month |
| Per-region managed RPC | $25.00 / month |
| Egress | 120 GB @ $0.09 / GB = $10.80 |
| Logs/metrics storage | 20 GB @ $0.23 / GB = $4.60 |
| **Per region** | **$78.40 / month** |
| 3 regions | $235.20 / month |
| **Incremental cost of redundancy** (2 extra regions) | **$156.80 / month** |
| Assumed outage exposure avoided | 4 h × 0.5 stale-price fraction × $1,000/h = $2,000 |
| **Payback** | **0.078 months (~2.3 days)** |

The redundancy pays for itself in under a week against a single 4-hour regional
outage per month. The assumptions are parameters of `cost_summary()`, not
constants buried in prose — change them and re-run to re-decide.
`test_cost_model_refuses_redundancy_that_cannot_pay_back` asserts the model
correctly reports `justified: false` for an expensive configuration, so the
function is a check rather than a rubber stamp.

## 8. Recovery runbook

| Symptom | Action |
|---|---|
| `IngestRegionDegraded` | Check the region's RPC endpoint and worker. Traffic has already failed over; no action is needed for correctness. |
| `IngestFailoverFlapping` | Investigate regional network health; consider raising `cooldown_secs` to damp oscillation. |
| `IngestAllRegionsDown` | Page on-call (SLA §5, P0, 30 m first response). Prices will breach the 60 s freshness target; expect SLA §6.2 credits. |
| `IngestAmbiguousSubmissionsQuarantined` | Run reconciliation (§5). Do **not** manually re-send the quarantined submissions. |

Contract-level recovery (pause, rollback via timelock) remains in
`docs/disaster-recovery.md`.

## 9. Out of scope

On-chain contract changes. Capacity provisioning for production is #523's
neighbour (`docs/soak-rig.md` §7) and is not addressed here.
