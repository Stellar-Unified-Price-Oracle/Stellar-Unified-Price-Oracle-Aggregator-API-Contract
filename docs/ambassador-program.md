# Ambassador and Educator Program — Stellar Unified Price Oracle

> Security-aware educator program with mandatory vetting, claim traceability, and a correction process.

## Purpose

Extend oracle adoption across communities — especially non-English-speaking regions — while
preventing the propagation of misinformation. Authority amplifies both good and bad content equally;
a confidently wrong tutorial is harder to correct than to prevent.

Every ambassador is vetted before their first publication and every technical claim must be
traceable to a code path, test, or existing document.

---

## Ambassador Tiers

| Tier | Role | Security Responsibilities |
|---|---|---|
| **Community Ambassador** | Shares official content, answers questions, translates existing docs | Must not author new technical claims; shares only versioned, reviewed material |
| **Educator Ambassador** | Produces tutorials, guides, workshop adaptations | Must pass the security vetting step; every technical claim must be cited |
| **Lead Educator** | Reviews other ambassadors' content, coordinates regional cohorts | Must have at least one documented correction exercise; approves content before publication |

---

## Vetting Process

No ambassador may **author or publish new technical content** until they have completed vetting.

### Vetting Step

The applicant must demonstrate the ability to identify at least **one unsafe integration pattern**
from the list below. Identification must be written (a GitHub issue, a PR comment, or a
submitted vetting form):

**Unsafe integration patterns to recognise:**

1. Consuming a price without checking staleness (timestamp vs. current block).
2. Treating an unfinalized price as final before the minimum-sources threshold is met.
3. Ignoring the `decimals` field and using the raw price integer directly.
4. Asset identity mismatch — consuming a price for asset A when asset B was intended.
5. Trusting a single source without checking the aggregated median.
6. Consuming a price mid-manipulation window (before the median settles).

The applicant must: name the pattern, explain why it is dangerous, and cite the relevant contract
code path or test that demonstrates the correct behaviour.

Vetting is evaluated by a **Lead Educator** or a **Maintainer** (per `CONTRIBUTING.md`). Outcome
is recorded in the ambassador's onboarding issue (pass / fail with feedback).

---

## Content Guidelines

All Educator Ambassador content must follow these rules:

### Citation Requirement

Every technical claim must end with a citation in one of these forms:

- `[code: <file>#L<line>]` — a link to the relevant contract source line.
- `[test: <file>#<test_name>]` — a link to the test that proves the behaviour.
- `[doc: <path>]` — a link to an existing reviewed document.

Uncited claims are rejected at review. A claim like "prices expire after X seconds" must cite
the storage TTL or the `resolution()` return value in the contract.

### Versioning Requirement

All published content must specify the contract revision it was written against:

```
Verified against: contract revision <git-sha or tag>
```

Content becomes **stale** when the contract advances by one major revision. Stale content must
be annotated or retracted within 30 days of the new revision being tagged.

### Review SLA

| Content Type | Review Turnaround |
|---|---|
| Short post / social content | 3 business days |
| Tutorial or guide | 7 business days |
| Workshop adaptation | 14 business days |

Reviews are conducted by a Lead Educator or a Maintainer. The reviewer uses the checklist below.

### Content Review Checklist

- [ ] Every technical claim is cited (code path, test, or doc).
- [ ] Contract revision is specified.
- [ ] No unsupported guarantee is asserted (finalisation, staleness-immunity, single-source safety).
- [ ] Unsafe integration patterns are not presented as safe.
- [ ] The content does not contradict existing documentation without explanation.

---

## Starter Kits

Onboarding materials provided to new ambassadors:

- Link to `docs/workshops/README.md` — runnable labs covering safe and unsafe consumption patterns.
- Link to `docs/security-audit-checklist.md` — the contract's own security review checklist.
- Link to `docs/case-studies.md` — real integration patterns with threat models.
- The unsafe-pattern list from this document (vetting step above).

---

## Coordination Channel

Ambassadors coordinate in the project's dedicated channel (linked from the GitHub README).
The channel is moderated by a Lead Educator. Questions about whether a claim is supported
by the contract are escalated to a Maintainer within 48 hours.

---

## Correction Process

When published content is found to assert an incorrect or unsupported claim:

1. A Maintainer or Lead Educator opens a **correction issue** referencing the content URL.
2. The author is notified and has **72 hours** to acknowledge.
3. The corrected version is published within **7 days** of the correction issue.
4. The original is annotated with a link to the corrected version and the date of correction.
5. The correction is recorded in the ambassador's history and counts as one iteration of their
   content improvement record.

**Exercise requirement:** Every Educator Ambassador must have at least one correction exercise
documented before being promoted to Lead Educator. The exercise may be self-initiated (finding
and correcting one's own error).

---

## Compensation

Educator Ambassador compensation (where applicable) is governed by the grant program.  
See [`docs/grant-program.md`](grant-program.md) — Documentation & Education category.

---

## References

- Contributor ladder and access: [`CONTRIBUTING.md`](../CONTRIBUTING.md)
- Grant program: [`docs/grant-program.md`](grant-program.md)
- Workshop labs (exploit + fix): [`docs/workshops/README.md`](workshops/README.md)
- Security audit checklist: [`docs/security-audit-checklist.md`](security-audit-checklist.md)
- Integration case studies: [`docs/case-studies.md`](case-studies.md)
