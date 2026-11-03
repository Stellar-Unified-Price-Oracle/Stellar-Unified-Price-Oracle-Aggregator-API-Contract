# Incident Severity and Decision Authority Matrix

Oracle impact levels map to response roles, pause authority, communications, and
escalation deadlines. Use the highest severity that applies.

| Severity | Oracle impact | Incident commander | Pause authority | Communications | Escalation deadline |
| --- | --- | --- | --- | --- | --- |
| SEV1 | Stale or manipulated prices served on-chain; funds at risk | On-call lead + protocol maintainer | On-call lead may pause immediately; maintainer confirms within 15 min | Public status page + consumer notice within 30 min | Acknowledge in 5 min; escalate to maintainers in 15 min |
| SEV2 | Degraded feeds, partial asset coverage, or delayed updates | On-call lead | On-call lead may pause affected assets | Consumer notice within 1 hour | Acknowledge in 15 min; escalate in 1 hour |
| SEV3 | Single asset anomaly with fallback intact | On-call engineer | On-call engineer may pause the single asset | Internal channel notice within 4 hours | Acknowledge in 1 hour; escalate in 4 hours |
| SEV4 | Cosmetic or non-impacting issue | On-call engineer | No pause authority required | Logged in incident tracker | Triage within 1 business day |

### Decision authority rules

- Only the roles listed for a severity may pause the oracle or an asset.
- Pausing above the listed authority requires incident-commander approval.
- Every pause must be recorded with severity, role, timestamp, and rationale.
- Resuming requires the same authority that paused, plus a post-incident review.

## Tabletop verification

The severity matrix is verified by a repeatable tabletop check. For each severity,
walk the scenario and confirm:

1. The correct incident commander and pause authority are identified.
2. The communications requirement and channel are met within the stated window.
3. The escalation deadline is respected.

Run the tabletop before each release and after any incident. Record the outcome in
the incident tracker; a severity with no correct decision-maker or response path
fails the check and blocks release.
