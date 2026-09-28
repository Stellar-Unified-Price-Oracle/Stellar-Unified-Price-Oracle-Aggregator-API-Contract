# Runbook — Stellar Unified Price Oracle (#527)

An alert is only useful if the person woken at 3 a.m. already knows what to do.
This runbook maps **every paging alert** in
[`docs/monitoring/alerts-v2.yml`](monitoring/alerts-v2.yml) and
[`docs/monitoring/alerts.yml`](monitoring/alerts.yml) to one entry with a fixed
shape:

| Field | Meaning |
|---|---|
| **Meaning** | What the alert says about the system, in one sentence |
| **First check** | The single fastest check to run before touching anything |
| **Mitigation** | The action that stops the bleeding, with its blast radius |
| **Escalation** | Who is paged next, and after how long without progress |
| **Resolution** | The objective condition that closes the incident |
| **Owner** | The rotation accountable for the entry |
| **Review cadence** | How often the entry is re-validated against reality |

**Paging definition.** An alert pages when its `severity` label is `critical`
or its SLA class is `P0`/`P1` ([`docs/SLA.md`](SLA.md) §5). Warning-severity
alerts do not page and therefore have no entry of their own; they are handled
in the normal review loop.

**Enforcement.** [`services/runbook/check_runbook.py`](../services/runbook/check_runbook.py)
parses both alert files and this document and fails CI when a paging alert has
no entry, an entry has a missing or empty required field, an entry is orphaned,
or the `Alert -> entry` table disagrees with the rules. Run it with:

```bash
python -m services.runbook.check_runbook          # coverage check (CI gate)
python -m services.runbook.check_runbook --routing  # JSON alert -> entry map
```

`--routing` emits the machine-readable `{alert: {entry, url, owner,
review_cadence}}` map that an Alertmanager `runbook_url` templating step links
from, so the responder's page carries a direct link to the entry below.

Escalation targets are the roles below; the roster behind each role lives in
the on-call config and is deliberately not in this repository.

| Role | Responsibility |
|---|---|
| `oracle-oncall` | First responder for every P0/P1 |
| `oracle-secondary` | Second responder, paged when the first does not acknowledge |
| `security` | Key compromise, unauthorised governance, adversarial submission |
| `governance` | Admin, pause, timelock and upgrade operations |
| `source-onboarding` | Source/asset registration and source-operator contact |
| `core-contracts` | Contract code, gas and storage behaviour |
| `risk` | Accuracy, deviation and reference-price disputes |

Response-time commitments referenced below are those in
[`docs/SLA.md`](SLA.md) §5; credit tiers are in §6.

---

## RB-01 — All sources down (no registered sources)

**Alerts:** `OracleAllSourcesDown` (`alerts-v2.yml`, `alerts.yml`) — severity `critical`, SLA class `P0`
**Meaning:** The contract reports zero registered oracle sources, so no aggregate can be produced at all. Consumers reading the oracle get no price.
**First check:** `get_sources` on the contract, then the `SourceRemoved` events that preceded it: `rate(oracle_config_change_events_total{event_type="SourceRemoved"}[15m])`.
**Mitigation:** Re-register the removed sources with `add_source(<address>)` if the removal was not authorised; if it was a legitimate governance operation, page `source-onboarding` to restore an equivalent set. Do **not** re-add a source whose key may be compromised — escalate first.
**Escalation:** `oracle-oncall` immediately (SLA §5 P0: 30 min first response). If unacknowledged after 10 min, page `oracle-secondary`; if the cause is a key or governance event, page `security` in parallel.
**Resolution:** `get_sources()` returns at least `min_sources_required` active sources and a fresh aggregate is published for every registered asset.
**Owner:** `source-onboarding`
**Review cadence:** Quarterly, and after every source onboarding or removal.

## RB-02 — Active sources below `min_sources_required`

**Alerts:** `OracleBelowMinSourcesRequired` (`alerts-v2.yml`) — severity `critical`, SLA class `P1`; `OracleInsufficientSources` (`alerts.yml`) — severity `critical`
**Meaning:** Fewer sources are submitting than aggregation requires, so aggregates stop being published. Availability, not accuracy, is impaired.
**First check:** Per-source submission rate `rate(oracle_price_submissions_total[15m])` and which source went quiet, cross-referenced with `oracle_source_last_submission_timestamp_seconds`.
**Mitigation:** Contact the silent source's operator. If the source is down but its key is healthy, no on-chain action is needed — the aggregate resumes when it returns. If the source is permanently gone, add a replacement source via governance so the count is restored.
**Escalation:** `oracle-oncall` (SLA §5 P1: 1 h first response). Escalate to `source-onboarding` after 2 h without a submitting source; to `oracle-secondary` if the primary is unacknowledged for 15 min.
**Resolution:** Active source count ≥ `min_sources_required` and an aggregate published within the 60 s freshness target (SLA §1.2).
**Owner:** `source-onboarding`
**Review cadence:** Quarterly.

## RB-03 — Aggregate stale for over 1 hour

**Alerts:** `OracleFreshnessBreach1h` (`alerts-v2.yml`) — severity `critical`, SLA class `P1`; `OracleStalePriceData` (`alerts.yml`) — warning, the non-paging precursor of this entry
**Meaning:** No aggregate has been republished for over an hour, engaging the SLA §6.2 credit tier (1–4 h). Consumers relying on freshness are being served stale prices.
**First check:** `time() - oracle_last_price_timestamp_seconds` per asset, and whether submissions are arriving but aggregation is not firing (`rate(oracle_price_updated_events_total[15m]) == 0`).
**Mitigation:** If submissions are arriving but no aggregate is published, the contract is likely paused or a source is systematically deviant — check `oracle_paused`, then the per-source deviation before any unpause. Never override the price to "clear" the alert; that hides the cause.
**Escalation:** `oracle-oncall` (SLA §5 P1: 1 h first response), then `core-contracts` after 2 h, since this is usually a contract or source-state problem rather than a single source outage.
**Resolution:** An aggregate has been published for every affected asset and `oracle_last_price_timestamp_seconds` is within the 60 s target.
**Owner:** `oracle-oncall`
**Review cadence:** Quarterly.

## RB-04 — Aggregate stale for over 24 hours

**Alerts:** `OracleFreshnessBreach24h` (`alerts-v2.yml`) — severity `critical`, SLA class `P0`
**Meaning:** The top SLA §6.2 credit tier is engaged (>24 h). The oracle is effectively down for consumers; assume SLA credits are owed.
**First check:** Establish whether the chain itself is progressing (`oracle_registered_sources_total` still populated?) and whether this is a declared pause — check the announced maintenance schedule in SLA §4.2.
**Mitigation:** Treat it as a consumer-communication incident first: notify consumer relations so dependent contracts can fail closed. Then apply the RB-01 to RB-03 mitigation for the underlying cause. Do not resume publication from a source whose data cannot be verified against an independent reference.
**Escalation:** `oracle-oncall` immediately (SLA §5 P0: 30 min first response, 2 h resolution target); `oracle-secondary` at 10 min; `security` if an unauthorised pause or admin event appears in the timeline.
**Resolution:** Fresh aggregates flowing again, consumers notified, and the incident handed to the postmortem process in [`docs/incident-management/README.md`](incident-management/README.md) — a P0 of this length always requires one.
**Owner:** `oracle-oncall`
**Review cadence:** Quarterly.

## RB-05 — Data accuracy breach (deviation above 2x `max_price_deviation`)

**Alerts:** `OracleDataAccuracyBreach` (`alerts-v2.yml`) — severity `critical`, SLA class `P1`
**Meaning:** A source's price deviates from the aggregate by more than twice `max_price_deviation` (default above 10 %), engaging the SLA §6.3 accuracy credit tier. The aggregate may still be correct, but one input is not trustworthy.
**First check:** Compare the deviant source against an independent reference venue. A deviation that is real market movement (a genuine flash crash) is not an incident; a deviation only visible inside the oracle is.
**Mitigation:** If the source is wrong, pause the contract and remove the source with `remove_source(<address>)`; the median keeps working from the remaining sources. If the source is right and the aggregate is wrong, the aggregate is being dragged by other sources — inspect the full per-source set before changing anything. See scenario 3 in [`docs/disaster-recovery.md`](disaster-recovery.md).
**Escalation:** `risk` owns this class and is paged with `oracle-oncall` (SLA §5 P1). Escalate to `security` if the pattern looks coordinated across more than one source.
**Resolution:** The deviant source is removed or corrected, the aggregate agrees with the independent reference set, and the affected ledger range is recorded for the postmortem.
**Owner:** `risk`
**Review cadence:** Quarterly.

## RB-06 — Contract paused unexpectedly

**Alerts:** `OracleContractPausedUnexpectedly` (`alerts-v2.yml`) — severity `critical`, SLA class `P0`
**Meaning:** `oracle_paused == 1`. Planned pauses are announced 24 h ahead (SLA §4.2); an unannounced pause stops all publication and may indicate a compromised admin.
**First check:** Compare the pause time against the announced maintenance schedule, then look for `AdminChanged` / `ContractUpgraded` events around the same ledger.
**Mitigation:** If planned, no action beyond confirming the window. If unplanned, assume admin compromise and follow scenario 2 in [`docs/disaster-recovery.md`](disaster-recovery.md): rotate the admin key first, then investigate. Do **not** unpause until the key is secured — unpausing under a live attacker republishes whatever they want.
**Escalation:** `oracle-oncall` and `security` in parallel (SLA §5 P0, 30 min first response); `governance` for any timelock or admin rotation.
**Resolution:** The pause is explained (planned or attacker-driven), the admin key is trusted, and the contract is unpaused with fresh aggregates flowing.
**Owner:** `governance`
**Review cadence:** Quarterly.

## RB-07 — Admin address changed

**Alerts:** `OracleAdminChanged` (`alerts-v2.yml`, `alerts.yml`) — severity `critical`, SLA class `governance`
**Meaning:** Control of the contract moved. Legitimate rotations are rare and announced; anything else is the first observable sign of key compromise.
**First check:** Identify the ledger and the new admin, and compare against the change log. One `AdminChanged` is an incident even if prices look fine.
**Mitigation:** If unauthorised, rotate to a fresh key immediately with `set_admin` while the current admin is still usable, then audit every governance action the address performed (`SourceAdded`, `AssetUnregistered`, `ContractUpgraded`, pause).
**Escalation:** `security` immediately, with `oracle-oncall` as the second responder. Escalate to `governance` only for the legitimate-rotation path.
**Resolution:** The admin set is verified correct, all unauthorised changes are reversed, and a postmortem is filed in [`docs/incident-management/`](incident-management/README.md).
**Owner:** `security`
**Review cadence:** Quarterly.

## RB-08 — Contract upgraded

**Alerts:** `OracleContractUpgraded` (`alerts-v2.yml`, `alerts.yml`) — severity `critical`, SLA class `governance`
**Meaning:** The executing WASM changed. Storage-layout or aggregation changes can break consumers even though the transaction succeeded.
**First check:** Compare the new WASM hash against the released tag and the `ContractUpgraded` proposal that authorised it.
**Mitigation:** If the hash is not the expected release, initiate the rollback in scenario 1 of [`docs/disaster-recovery.md`](disaster-recovery.md) via `propose_operation` — the timelock is the safety net, so do not skip it. If it is expected, run the smoke checks in `scripts/verify-deployment.sh` and watch for storage errors.
**Escalation:** `core-contracts` owns this class; `oracle-oncall` is paged as second responder. If the upgrade was not authorised, page `security`.
**Resolution:** The running WASM matches an intended release and aggregation, history reads and SEP-40 queries behave as before.
**Owner:** `core-contracts`
**Review cadence:** Quarterly, and after every mainnet upgrade.

## RB-09 — Low effective source diversity

**Alerts:** `LowEffectiveDiversity` (`alerts.yml`) — severity `critical`
**Meaning:** The raw source count may look healthy while the sources collapse onto one failure domain (one cloud, one upstream, one owner). The Sybil / nominal-diversity trap: the aggregate is one point of failure, not many.
**First check:** `get_source_diversity` for the effective count and the largest-domain share, plus `oracle_diversity_max_hhi` per axis.
**Mitigation:** Treat it as an availability incident with a slow clock. Onboard a source from an independent failure domain via governance; do not add a nominal duplicate, which raises the raw count and not the effective one.
**Escalation:** `source-onboarding` owns the fix; `oracle-oncall` handles the paging. Escalate to `risk` if diversity fell while the source list was unchanged, which is a sign of silent exclusion rather than an outage.
**Resolution:** Effective independent source count and per-axis HHI are back inside their configured thresholds.
**Owner:** `source-onboarding`
**Review cadence:** Quarterly.

---

## RB-11 — Capacity exhausted or lag target breached

**Alerts:** `OracleCapacityExhausted` (`alerts-v2.yml`) — severity `critical`, SLA class `P1`; `OracleCapacityLagTargetBreached` (`alerts-v2.yml`) — severity `critical`, SLA class `P1`. The warning-level `OracleCapacityHeadroomLow`, `OracleCapacityHeadroomCritical` and `OracleCapacityLagApproachingTarget` alerts are the pre-exhaustion signals and are handled by the same entry before they page.
**Meaning:** A modelled resource (ingest, storage or ledger budget) is exhausted, or publication lag has passed the SLA §1.2 60 s freshness target. Work is being dropped or is failing; this is the failure mode that capacity planning exists to prevent.
**First check:** `oracle_capacity_headroom_ratio` and `oracle_capacity_lag_seconds` per resource, and which resource is named in the alert — `ledger_budget` is shared with unrelated network traffic and can be squeezed by someone else's transactions, while `ingest` and `storage` are ours alone.
**Mitigation:** Apply the load-shedding policy in [`docs/capacity-planning.md`](capacity-planning.md) §3 in order, cheapest consumer-visible cost first: drop historical backfill, then non-critical assets outside the settlement window, then reduce frequency on the least-traded assets, then disable optional analytics. The critical path is never dropped. Degrading signature verification (the last step) is an incident in itself and requires an explicit decision by `security`, not an automatic toggle.
**Escalation:** `oracle-oncall` (SLA §5 P1: 1 h first response). Escalate to `core-contracts` after 1 h, since sustained pressure is a capacity or driver problem rather than a transient burst. Page `security` before, not after, any decision to degrade signature verification.
**Resolution:** Every resource is back above its headroom target, lag is inside 60 s, and no critical-path work was shed. If the pressure is structural rather than transient, the growth assumptions in `docs/capacity-planning.md` §1 are re-projected before the next quarter.
**Owner:** `core-contracts`
**Review cadence:** Quarterly, alongside the capacity model review.

---

## Alert -> entry map

| Alert | Rule file(s) | Entry |
|---|---|---|
| `OracleAllSourcesDown` | `alerts-v2.yml`, `alerts.yml` | RB-01 |
| `OracleBelowMinSourcesRequired` | `alerts-v2.yml` | RB-02 |
| `OracleInsufficientSources` | `alerts.yml` | RB-02 |
| `OracleFreshnessBreach1h` | `alerts-v2.yml` | RB-03 |
| `OracleFreshnessBreach24h` | `alerts-v2.yml` | RB-04 |
| `OracleDataAccuracyBreach` | `alerts-v2.yml` | RB-05 |
| `OracleContractPausedUnexpectedly` | `alerts-v2.yml` | RB-06 |
| `OracleAdminChanged` | `alerts-v2.yml`, `alerts.yml` | RB-07 |
| `OracleContractUpgraded` | `alerts-v2.yml`, `alerts.yml` | RB-08 |
| `LowEffectiveDiversity` | `alerts.yml` | RB-09 |
| `OracleCapacityExhausted` | `alerts-v2.yml` | RB-11 |
| `OracleCapacityLagTargetBreached` | `alerts-v2.yml` | RB-11 |
| 
`alerts-v2.yml` also carries `OracleSourceDeviationFlagged` (warning, P3) and `OracleSingleSourceOffline` (warning, P2). They are covered operationally by RB-05 and RB-02 respectively but do not page, so they have no entry of their own.

## Entry ownership and review

| Role | Entries | Review |
|---|---|---|
| `oracle-oncall` | RB-03, RB-04 | Quarterly |
| `source-onboarding` | RB-01, RB-02, RB-09 | Quarterly |
| `security` | RB-07 | Quarterly |
| `governance` | RB-06 | Quarterly |
| `core-contracts` | RB-08, RB-11 | Quarterly |
| `risk` | RB-05 | Quarterly |

Quarterly review is a calendar task, not a suggestion: each owner re-runs the entry against a real or drilled alert and updates the first check if the command no longer returns what the entry claims. A review that finds a stale step raises a tracked issue; an entry not reviewed in two cycles is escalated to `oracle-secondary`.

## Game-day drill

The runbook is exercised, not just written. The drill runs a [disaster-recovery scenario](./disaster-recovery.md) against a testnet deployment with a simulated source outage, and its record lives next to the postmortems it produces in [`docs/incident-management/`](incident-management/README.md). A drill must:

1. Page at least one P0 and one P1 entry from this runbook.
2. Follow the *first check* and *mitigation* verbatim and record every step that did not work as written.
3. Exercise one escalation hop to `oracle-secondary`.
4. Produce a postmortem using the template, with action items filed as issues.
5. Update the entries whose steps failed; a drill that finds no stale step is a sign the drill was not hard enough.

The first drill, covering the P0 in RB-01 and the P1 in RB-02, is recorded as `2026-08-12-source-outage-drill.md` in the postmortem index.
