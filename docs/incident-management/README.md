# Incident management

Postmortems for production incidents and game-day drills. Each one follows
[`TEMPLATE.md`](TEMPLATE.md) and is checked in CI by
`python -m services.incident_review.check_postmortems`: required sections and
header fields, an owner, due date, tracking issue and status on every action
item, and this index listing every postmortem.

The [runbook](../runbook.md) says when an incident needs a postmortem.

## Postmortems

| Date | Postmortem | Severity |
|---|---|---|
| 2026-08-12 | [Game-day drill: point-in-time restore of off-chain state](2026-08-12-restore-drill.md) | P1 |
| 2026-08-12 | [Game-day drill: total source outage (RB-01, RB-02)](2026-08-12-source-outage-drill.md) | P0 |

## Adding a postmortem

1. Copy `TEMPLATE.md` to `YYYY-MM-DD-<slug>.md` and fill in every placeholder.
2. Add a row to the table above.
3. Run `python -m services.incident_review.check_postmortems --report` to see
   open and overdue action items.
