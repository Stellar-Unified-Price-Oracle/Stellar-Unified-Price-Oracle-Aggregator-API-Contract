# Postmortem: <short title>

> Copy this file to `docs/incident-management/YYYY-MM-DD-<slug>.md` and fill in
> every section. The checker (`services/incident_review/check_postmortems.py`)
> rejects a postmortem that is missing a section or has an action item without
> an owner, a due date, or a tracking issue.

| Field | Value |
|---|---|
| **Incident ID** | `INC-YYYYMMDD-NN` |
| **Date** | YYYY-MM-DD |
| **Severity** | P0 / P1 / P2 / P3 (docs/SLA.md §5) |
| **Status** | Draft / Final |
| **Incident commander** | role, not a person |
| **Duration** | e.g. 2 h 14 min (detection to resolution) |
| **Runbook entries used** | e.g. RB-01, RB-02 (docs/runbook.md) |
| **Postmortem owner** | role accountable for the follow-ups |

## 1. Summary

Two or three sentences: what broke, who was affected, for how long. Written
for someone who was not on call and has no prior context.

## 2. Impact

* **Consumer impact** — which consumers, which assets, what they would have
  read, and whether any could have failed closed.
* **SLA impact** — the clause and credit tier engaged (docs/SLA.md §6), and
  whether a claim window is open.
* **Financial impact** — if quantifiable, with the method; otherwise "none
  quantified".
* **On-chain footprint** — ledger range affected, transactions submitted or
  dropped, history entries written or lost.

## 3. Timeline

All times UTC, from the first signal to the last action. Include detection
time separately from occurrence time: the gap is itself a finding.

| Time (UTC) | Event | Source of truth |
|---|---|---|
| HH:MM | What happened, and who noticed | alert / on-chain event / manual |

## 4. Root cause

* **Trigger** — the specific condition that started it.
* **Contributing factors** — what made the impact larger or the recovery
  slower. More than one is normal; a single cause is a sign of a shallow
  analysis.
* **What went well** — keep this. It is what a future responder will rely on.

**Blamelessness norms.** We describe systems and decisions, never people.
"He did not check the key" becomes "the runbook had no step for verifying the
key, so the rotation went unnoticed". A postmortem that names a culprit is
rejected at review, and the reviewer's rejection is itself an incident-review
action item. The purpose is a system that fails safely and a team that reports
early, which both require that nobody fears writing this document.

## 5. What went wrong

The concrete gaps between what the runbook says and what actually happened.
Quote the runbook step that was wrong, missing, or misleading — this is what
gets the entry fixed.

## 6. Action items

Every row must have an owner, a due date, and a tracking issue. An action item
without all three is a wish, and the checker fails on it.

| # | Action | Owner | Due | Issue | Status |
|---|---|---|---|---|---|
| 1 | What will change, stated so it can be verified | role | YYYY-MM-DD | #NNN | Open |
| 2 | Runbook step that was wrong, and its correction | role | YYYY-MM-DD | #NNN | Open |

Status is one of `Open`, `In progress`, `Done`, `Won't fix (rationale)`. A
`Done` row needs a closing comment on its issue; a `Won't fix` row needs the
rationale in the same table.

## 7. Detection and response assessment

* How was this detected — alert, consumer report, or luck?
* Did the page carry enough context to start work (see `docs/runbook.md`)?
* Was the escalation path unambiguous?
* Time to first response vs. the SLA §5 target.

## 8. Review and sign-off

| Reviewer role | What they checked | Date |
|---|---|---|
| `oracle-secondary` | Timeline accuracy, action items are actionable | YYYY-MM-DD |
| security / risk / core-contracts | Applicable domain, whichever is relevant | YYYY-MM-DD |

The postmortem moves from Draft to Final only when every action item has an
owner and a due date and the review above is complete. Overdue action items
are surfaced automatically by the checker until they close.
