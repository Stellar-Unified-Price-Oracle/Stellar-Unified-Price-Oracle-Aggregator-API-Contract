# Grant Program — Stellar Unified Price Oracle

> Security-first grant framework with milestone escrow, independent security review gates, and clawback.

## Purpose

Fund sustained oracle development beyond issue bounties while ensuring that every deliverable
strengthens — and never weakens — the contract's security guarantees. Grants assume the applicant
is optimising for payout; the framework makes unsafe shortcuts unprofitable.

---

## Grant Categories and Sizes

| Category | Description | Maximum Size |
|---|---|---|
| **Protocol Extension** | New contract features (new aggregation modes, feed types, SEP extensions) | 5,000 USDC |
| **Tooling & Integrations** | Off-chain bots, adapters, dashboards, SDKs | 3,000 USDC |
| **Security Research** | Audit contributions, invariant proofs, adversarial test suites | 4,000 USDC |
| **Documentation & Education** | Guides, workshop materials, case studies | 1,500 USDC |

Amounts above the category maximum require Core Maintainer approval and an expanded review process.

---

## Milestone Structure

Every grant is broken into at most **three milestones**. Each milestone:

1. Has a written deliverable description and acceptance criteria.
2. Names the independent security reviewer (see below).
3. Defines **disqualifying outcomes** — conditions that block payout regardless of technical completion.
4. Specifies the percentage of grant funds held in escrow until the milestone passes review.

### Standard Milestone Template

```
Milestone N — <title>

Deliverable:
  <Precise description of what will be produced>

Acceptance Criteria:
  - [ ] <Criterion 1>
  - [ ] <Criterion 2>

Security Review Gate:
  Reviewer: <GitHub handle> (no conflict of interest; see §Reviewer Eligibility)
  Must pass: <list of checks>

Disqualifying Outcomes:
  - Weakened or removed authentication check
  - Removed or relaxed input validation
  - Unreviewed upgrade or admin path change
  - Scope expansion into admin paths without prior approval
  - Deliverable tested only against mocks, never against testnet

Escrow: <X>% of grant total released on milestone approval
```

---

## Disqualifying Outcomes (global)

Any of the following **immediately block payout** for the affected milestone and trigger a
clawback review for any previously released funds:

- Any authentication check weakened, removed, or bypassed.
- Any input-validation bound lowered or removed.
- Any unreviewed change to upgrade paths, admin paths, or signing key rotation logic.
- Any milestone validated exclusively against mocks without a testnet run.
- Scope quietly expanded into admin or privileged paths without written approval.
- A change that lowers gas cost by removing a safety bound.

---

## Independent Security Review

### Reviewer Eligibility

A reviewer is eligible if they:

- Are not a co-author of any code being reviewed in the milestone.
- Have no financial interest in the milestone's approval (not the applicant, not their employer).
- Hold at least **Maintainer** status in the contributor ladder (see `CONTRIBUTING.md`).
- Have reviewed at least one prior security-relevant PR in this repository.

A reviewer who is ineligible must be replaced before the milestone is submitted. The reviewer
is named at grant acceptance and cannot be changed without Core Maintainer approval.

### Review Checklist

The reviewer signs off against the following (all must pass):

- [ ] All authentication paths unchanged or demonstrably strengthened.
- [ ] All input validation bounds unchanged or demonstrably tightened.
- [ ] No new upgrade, admin, or key-rotation paths introduced without prior design review.
- [ ] Deliverable tested against a real testnet run (not mocks only); run log attached.
- [ ] No scope expansion beyond the approved milestone description.
- [ ] Every technical claim in documentation cites a code path, test, or existing doc.

---

## Escrow and Payout

Funds are held in the project treasury module. Release is triggered only when:

1. The applicant submits a milestone completion report (PR or issue).
2. The named independent reviewer signs off (comment with explicit approval on the PR/issue).
3. A Core Maintainer confirms no disqualifying outcomes are present.

The treasury release transaction is linked to the merged PR in the milestone completion report.

---

## Clawback

Clawback is available under the following conditions:

| Condition | Window | Mechanism |
|---|---|---|
| Delivered work later shown to introduce a vulnerability | 12 months from payout | Core Maintainer resolution + treasury reversal |
| Milestone accepted by a reviewer with an undisclosed conflict of interest | 6 months from payout | Same as above |
| Scope expansion approved retroactively found to weaken guarantees | 6 months from payout | Same as above |

Clawback requires a documented incident report, a majority Core Maintainer vote, and a 72-hour
dispute window for the recipient. Clawback proceeds to the project treasury (not to individuals).

---

## Application Process

1. Open a GitHub issue using the **Grant Application** template.
2. Complete: project description, category, milestone breakdown, proposed reviewer.
3. Core Maintainer reviews eligibility and approves or requests changes within 14 days.
4. On approval: funds are moved to escrow and the milestone tracking issue is pinned.

---

## Worked Specimen — Example Milestone

**Grant:** Implement signed CEX adapter for Binance price feed  
**Category:** Tooling & Integrations  
**Total:** 2,000 USDC  

**Milestone 1 — Adapter implementation (50% = 1,000 USDC)**

- Deliverable: `docs/signed-price-adapters.md` updated; adapter code merged to `main`.
- Reviewer: `@reviewer-handle`
- Disqualifying outcomes: adapter bypasses source authentication; testnet run not attached.
- Checklist: auth unchanged ✓, validation bounds unchanged ✓, testnet log attached ✓.

**Milestone 2 — Adversarial test suite (50% = 1,000 USDC)**

- Deliverable: ≥5 new tests covering staleness, signature forgery, and feed manipulation scenarios.
- Reviewer: same reviewer (no new conflict introduced).
- Disqualifying outcomes: tests pass only against mocks; any auth path weakened to make tests pass.
- Checklist: tests run against testnet ✓, all prior tests still pass ✓.

---

## References

- Contributor ladder and access controls: [`CONTRIBUTING.md`](../CONTRIBUTING.md)
- Ambassador program (content grants): [`docs/ambassador-program.md`](ambassador-program.md)
- Workshop materials (education grants): [`docs/workshops/README.md`](workshops/README.md)
- Security audit checklist: [`docs/security-audit-checklist.md`](security-audit-checklist.md)
