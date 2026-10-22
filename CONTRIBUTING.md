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

## Integration Support Intake and Triage

Consumers integrating with the oracle (wallets, dApps, indexers, downstream services) file
integration reports through the **public path** below. This is the single supported channel for
production consumer support; do not send integration reports through private channels.

### Public Path for Filing Reports

- **Where:** open a GitHub issue using the `integration-report` label on this repository.
  This is the public, traceable intake path.
- **Who may file:** any consumer, including external integrators without write access.
- **Do not** include secrets, private keys, signing material, API tokens, or production
  credentials in a report. Redact them before filing; if a secret may have leaked, report only
  that a leak occurred and rotate it out-of-band — never paste the value.

### Required Reproduction Data

Every integration report must include the following fields so it can be reproduced and triaged:

| Field | Description |
|---|---|
| `network` | Target network (`testnet`, `futurenet`, or `mainnet`) |
| `contract_id` | Deployed oracle contract ID (public; never a secret) |
| `sdk_version` | Client SDK / library name and version used to call the contract |
| `function` | Contract entry point invoked (e.g. `get_price`, `get_prices`) |
| `inputs` | Sanitized arguments passed to the call (asset pairs, amounts) |
| `expected` | Expected result or behavior |
| `actual` | Observed result, including error code if any |
| `reproduction` | Minimal steps or snippet to reproduce (secrets redacted) |
| `first_seen` | Timestamp (UTC) when the issue was first observed |

Reports missing required fields are labeled `needs-info` and returned to the reporter for
completion before triage proceeds.

### Response Ownership

- **Triagers** own first response: acknowledge within 2 business days, apply the triage label,
  and request missing reproduction data.
- **Maintainers** own resolution of `sev-1`/`sev-2` reports and coordinate fixes.
- **Core Maintainers** own any report touching upgrade/admin paths or suspected secret exposure.

### Severity Routing

Each report is routed to exactly one triage category via label:

| Label | Category | Routing |
|---|---|---|
| `sev-1` | Contract correctness / funds at risk / suspected secret exposure | Core Maintainers, immediate |
| `sev-2` | Incorrect prices or broken core calls on `mainnet` | Maintainers, same day |
| `sev-3` | Degraded behavior or workaround available | Triagers, within 2 business days |
| `sev-4` | Documentation, ergonomics, or feature requests | Triagers, backlog |

### Sample Report

A minimal, secret-free report that traces to a documented category:

```
Title: get_price returns stale timestamp on testnet
network: testnet
contract_id: C...PUBLIC_CONTRACT_ID...
sdk_version: @stellar/stellar-sdk 12.0.0
function: get_price
inputs: { asset: "XLM/USD" }
expected: timestamp within 60s of ledger close
actual: timestamp ~10m old; no error code
reproduction: call get_price("XLM/USD") on testnet, compare timestamp to latest ledger
first_seen: 2025-01-01T00:00:00Z
```

This sample routes to **`sev-3`** (degraded behavior, workaround available) and is owned by a
**Triager** for first response. It contains no secrets — only a public contract ID and public
inputs.

## Governance Proposals

### Repeatable check

👉 [Governance Proposal Template](docs/governance-proposal-template.md)

Submit completed proposals as a GitHub issue or pull request for community review before any on-chain action is taken.

## Mainnet Release Go/No-Go

Before any mainnet release, complete the [mainnet release checklist](docs/mainnet-release-checklist.md).
Each production gate must record an owner, an evidence link, and an explicit pass/fail result.
The release workflow runs `scripts/check-release-evidence.sh`, which blocks the release when any
mandatory evidence is absent. Never commit secrets or production credentials as evidence.

## Contributor Ladder

The contributor ladder is a security control as much as a recognition scheme. Access escalates
only when a contributor demonstrates the judgment required to exercise that access safely.
Contribution volume alone does not justify promotion.

### Levels

| Level | Permissions | Entry Criteria |
|---|---|---|
| **Contributor** | Fork, open PRs, file issues | First merged PR |
| **Triager** | Label issues, request reviews, close stale issues | ≥3 merged PRs *and* at least one documented security-review action (see below) |
| **Maintainer** | Merge PRs (non-upgrade paths), manage releases | ≥2 months as Triager, second Maintainer approval, documented security-judgment evidence |
| **Core Maintainer** | Merge upgrade/admin-path PRs, rotate signing keys | Separate nomination; requires unanimous current Core Maintainer approval |

### Least-Privilege Permission Map

Each rung is limited to the minimum permissions required:

- **Contributor** — no repository write access; no CI secrets access.
- **Triager** — GitHub Triage role only; no merge rights; no secrets.
- **Maintainer** — GitHub Write role; can merge to non-protected branches; no key/deploy access.
- **Core Maintainer** — GitHub Admin role (scoped); upgrade-path merge rights; signing-key rotation.

### Security Judgment Requirement

Promotion to **Triager or above** requires at least one documented instance of security judgment. Examples:

- Caught a validation gap or missing auth check in a PR review.
- Rejected a plausible-but-unsafe change with a written rationale.
- Identified a contract invariant violation in a proposed change.

Document the instance in the promotion PR (reference the PR/issue number where the judgment was exercised).

### Inactivity Expiry

Privileged access (Triager and above) expires automatically after **90 days of inactivity**
(no merged PR, no substantive review, no triaging action). On expiry:

1. Access is downgraded one level automatically.
2. The contributor is notified and may request reinstatement by demonstrating current activity.

A dry run of the expiry process is performed quarterly by a Core Maintainer, with results documented as an issue.
The same quarterly review covers deployment, monitoring, source and treasury access for humans and
automation; see [docs/security/access-review.md](docs/security/access-review.md).

### Removal and Demotion

Any maintainer may propose removal or demotion by opening an issue with:

- The rung being revoked.
- The specific trigger (inactivity, conduct, security incident, role change).
- Evidence supporting the action.

Removal is effective after 48 hours unless disputed. Disputed removals require a majority vote
of Core Maintainers. Removed access is logged in the project's access-change history.

### Security-Relevant Promotions

Promotions to Maintainer or Core Maintainer require a **second Maintainer's approval** documented
in the promotion PR. The approving Maintainer must not be the nominee's primary collaborator and
must independently verify the security-judgment evidence.

### Automated Recognition

- GitHub Achievements and profile badges are assigned automatically on promotion.
- The README contributor table is regenerated on each merge to `main`.
- All access changes are logged as GitHub issues under the `access-change` label.

---

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
