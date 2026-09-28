# Postmortem: Game-day drill — total source outage (RB-01, RB-02)

| Field | Value |
|---|---|
| **Incident ID** | `INC-20260812-01` |
| **Date** | 2026-08-12 |
| **Severity** | P0 |
| **Status** | Final |
| **Incident commander** | `oracle-oncall` |
| **Duration** | 1 h 06 min (detection to resolution) |
| **Runbook entries used** | RB-01, RB-02 (docs/runbook.md) |
| **Postmortem owner** | `oracle-oncall` |

## 1. Summary

A scheduled game-day drill simulated every registered source going silent on a
testnet deployment, which drove the alert pipeline into the P0
`OracleAllSourcesDown` state and then into the P1
`OracleBelowMinSourcesRequired` state as sources were restored one at a time.
The runbook was followed verbatim for the first four minutes and then diverged:
the escalation timer in RB-01 assumes an acknowledgement channel that the drill
harness does not emit, and the "re-add the removed sources" mitigation in RB-01
is wrong for an outage, where the sources were never removed. The drill resolved
in 1 h 06 min against a 2 h P0 target and produced five action items, three of
which are corrections to the runbook itself.

## 2. Impact

* **Consumer impact** — none. The drill ran against a testnet deployment with
  no consumer contracts pointed at it. Had it been mainnet, aggregates would
  have stopped entirely once the last source was removed.
* **SLA impact** — P0 response targets were met (first response 3 min against a
  30 min target). No credit tier engaged because no real consumer was served a
  stale price.
* **Financial impact** — none quantified; testnet fees only.
* **On-chain footprint** — ledgers 4188200-4188261 on testnet. Two
  `SourceRemoved` operations and three `SourceAdded` operations were proposed
  and executed through the timelock. No price history was lost, because
  aggregation never ran during the outage.

## 3. Timeline

| Time (UTC) | Event | Source of truth |
|---|---|---|
| 09:00 | Drill start. All three registered sources stopped by the harness. | drill harness |
| 09:04 | `OracleAllSourcesDown` fired (P0) and paged `oracle-oncall`. | alert |
| 09:07 | First response acknowledged (3 min, SLA §5 P0 target 30 min). | paging system |
| 09:12 | RB-01 first check run: `get_sources` returned empty; the `SourceRemoved` rate was zero. | on-chain query |
| 09:20 | Discovered RB-01's mitigation is written for a *removal* incident and does not apply; the sources were never removed. Runbook defect. | responder notes |
| 09:31 | Escalation to `oracle-secondary` at the 10 min mark. | paging system |
| 09:48 | First source restored; `OracleBelowMinSourcesRequired` (P1) fired because 1 < `min_sources_required` of 2. | alert |
| 10:02 | Second source restored; aggregates resumed. | on-chain event |
| 10:06 | Drill declared resolved; 1 h 06 min total. | incident commander |
| 10:30 | Draft postmortem opened from TEMPLATE.md. | this document |

The 16-minute gap between occurrence (09:00) and detection (09:04) is the
`for: 1m` window on the alert plus scrape interval, and is within target.

## 4. Root cause

* **Trigger** — the drill's deliberate removal of all sources.
* **Contributing factors** — (a) RB-01's mitigation assumes the cause is a
  `SourceRemoved` event, so a pure outage leaves the responder with no
  documented action; (b) the escalation timer is written against an
  acknowledgement the harness never sends, so the 10-minute hop fired late and
  had to be triggered manually; (c) there was no single query that answers "are
  sources registered *and* submitting", so the responder ran two.
* **What went well** — the alert fired within the intended window, the page
  carried the asset and contract labels needed to start, and the P0 target was
  met with margin. Restoring a source and watching the P1 clear needed no
  runbook at all, which is the correct outcome for a mechanical step.

**Blamelessness.** The timeline names roles only. The two responder decisions
that "went around" the runbook — checking the `SourceRemoved` rate before
trusting the entry, and triggering the escalation by hand — are recorded as
evidence that the entry was unusable at that moment, not as deviations from it.

## 5. What went wrong

1. **RB-01 mitigation does not cover an outage.** It says "re-register the
   removed sources", which is wrong when nothing was removed. The entry needs an
   outage branch: confirm the sources are still registered
   (`get_sources` non-empty), then work the source operators.
2. **No combined liveness query.** RB-01's first check and RB-02's first check
   are different queries of the same underlying question.
3. **Escalation timing is not observable in the drill.** The 10-minute
   acknowledgement hop needs an explicit statement of what "not acknowledged"
   means in the paging system so the next drill does not have to improvise it.
4. **Restoring sources one at a time re-paged as P1.** The intermediate state
   (`OracleBelowMinSourcesRequired`) is expected during recovery, and the
   responder has no documented way to know it is expected. That is an alert-noise
   finding, not a responder failure.
5. **The drill has no pass/fail definition.** "Did the runbook work" was
   answered by opinion.

## 6. Action items

| # | Action | Owner | Due | Issue | Status |
|---|---|---|---|---|---|
| 1 | Add an outage branch to the RB-01 mitigation that starts from `get_sources` being non-empty, and state that neither `add_source` nor `remove_source` is the right action for a silent-but-registered source | `source-onboarding` | 2026-09-15 | #566 | Open |
| 2 | Add a single combined liveness query to the RB-01 and RB-02 first checks and reference it from both entries | `oracle-oncall` | 2026-09-15 | #567 | Open |
| 3 | Define explicitly what "unacknowledged" means for the 10-minute `oracle-secondary` hop, and have the drill harness emit an acknowledgement so the hop is testable | `oracle-oncall` | 2026-09-15 | #568 | Open |
| 4 | Suppress or annotate `OracleBelowMinSourcesRequired` while a recovery is in progress, so an expected recovery step does not page as an incident | `source-onboarding` | 2026-10-01 | #569 | Open |
| 5 | Write the drill pass/fail criteria into `docs/runbook.md` (first check and mitigation followed verbatim; every divergence recorded as a finding) | `oracle-oncall` | 2026-09-01 | #570 | Open |

## 7. Detection and response assessment

* Detected by the alert pipeline, not by a person — 4 minutes, inside the
  `for: 1m` plus scrape budget.
* The page carried contract, asset and severity labels; the responder reached
  the right entry from the alert without asking anyone where to look.
* Escalation was **not** unambiguous: the 10-minute hop had to be triggered by
  hand because the harness emits no acknowledgement. This is action item 3.
* First response 3 min against the SLA §5 P0 target of 30 min. Resolution
  1 h 06 min against the 2 h P0 target.

## 8. Review and sign-off

| Reviewer role | What they checked | Date |
|---|---|---|
| `oracle-secondary` | Timeline against the alert and paging logs; the five action items are each verifiable | 2026-08-14 |
| `source-onboarding` | Action items 1 and 4 reflect how sources are actually onboarded and recovered | 2026-08-15 |
