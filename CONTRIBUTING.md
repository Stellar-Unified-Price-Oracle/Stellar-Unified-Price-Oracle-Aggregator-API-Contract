# Contributing

Thanks for your interest in improving this project. This document covers the
basics of getting a change merged and the operational expectations for
maintainers and on-call operators.

## Development workflow

1. Fork the repository and create a topic branch from `main`.
2. Make focused changes that address a single issue.
3. Add or update tests that cover the change, including boundary and failure
   paths where relevant.
4. Update any directly related documentation.
5. Open a pull request that references the issue it resolves.

## Code style

- Follow the existing module structure, naming, and types.
- Keep edits surgical; avoid unrelated refactors in the same pull request.
- Do not commit secrets, tokens, or production credentials.

## Operator handover and on-call readiness drill

Operators must be able to hand off a shift, reach the escalation contacts, and
find the current runbooks under time pressure. Run the readiness drill below at
least once per on-call rotation and after any change to the escalation policy or
runbook layout.

### Running the drill

The drill is a timed exercise. Record the start time, then complete each step in
order and record the elapsed time for that step. A step is **missed** if it
cannot be completed within its target time or the required artifact is not
available.

| # | Step | Target |
|---|------|--------|
| 1 | Incoming operator receives shift handover notes and confirms open incidents | 10 min |
| 2 | Escalation contact list is reachable (primary and secondary acknowledged) | 5 min |
| 3 | Current runbooks are located and the relevant one is opened | 5 min |
| 4 | Handover summary is posted to the on-call channel | 5 min |

### Recording results

For each step record:

- the step number and description,
- the response time (elapsed time to complete the step),
- whether the step was completed or missed.

### Corrective actions

Every missed step must produce a corrective action with a named owner and a due
date. Corrective actions are tracked until closed and reviewed at the next
drill. Do not close a drill with an unowned missed step.

### Repeatable check

Keep the drill repeatable by running it on the schedule above and recording the
results in the on-call log. The recorded response times and owned corrective
actions are the evidence that the drill was completed.
