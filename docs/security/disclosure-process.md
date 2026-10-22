# Security Contact and Disclosure Process (#690)

## Overview

The public policy for reporters is [`SECURITY.md`](../../SECURITY.md). The same
commitments are recorded as data in
[`config/security-disclosure.json`](../../config/security-disclosure.json), so
they can be checked:

| Commitment | Value |
|---|---|
| Private channel | GitHub private vulnerability reporting (`/security/advisories/new`) |
| Response owner | **Security Lead**, `@Stellar-Unified-Price-Oracle/maintainers` |
| Acknowledgement target | **72 hours** |
| Embargo | Until the fix is released, at most **90 days** unless the reporter agrees otherwise |

---

## What the validator checks

`python -m services.disclosure.policy` fails when:

- the channel is not a private channel, or its URL is not the repository's
  private advisory form;
- the response owner has no role, or its team is not a GitHub team handle;
- the acknowledgement target is outside 1 to 72 hours, or the embargo is
  outside 1 to 90 days;
- `SECURITY.md` does not state the channel URL, owner role and team,
  acknowledgement target and embargo from the policy, or links a public issue,
  discussion or pull request form.

## Synthetic disclosure drill

Run the drill each quarter, and whenever the responder team changes. It goes
through the real channel without publishing any report contents.

1. Generate a synthetic report:
   ```bash
   python -m services.disclosure.policy --new-synthetic
   ```
   It prints a `report_id` (`SYNTHETIC-…`), a random filler `body` with no
   vulnerability content, and the body's `content_sha256`.
2. A drill coordinator submits the body through the private advisory form,
   recording the submission time.
3. The response owner acknowledges it in the advisory as they would a real
   report.
4. Probe access: each responder opens the advisory, and an account outside the
   responder team tries to open it. Record each result.
5. Record the observation. It holds only IDs, times, handles and the digest:
   ```json
   {
     "report_id": "SYNTHETIC-20261020-1a2b3c4d",
     "content_sha256": "<64 hex chars>",
     "submitted_at": "2026-10-20T09:00:00Z",
     "acknowledged_at": "2026-10-20T14:00:00Z",
     "acknowledged_by": "<responder handle>",
     "responders": ["<responder handle>", "..."],
     "access_checks": [
       {"principal": "<responder handle>", "can_read": true},
       {"principal": "<outside handle>", "can_read": false}
     ]
   }
   ```
6. Score it, then close the synthetic advisory without publishing it:
   ```bash
   python -m services.disclosure.policy --drill drill.json
   ```

The drill fails when the report ID is not synthetic, the observation contains
report contents (`body`, `content`, `description`, `details`, `report`,
`steps`), the acknowledgement is missing, late or made by a non-responder, a
responder cannot read the report, someone outside the responders can, or no
access check was made from outside the responders.

## Tests

```bash
python -m pytest services/disclosure -q
```

`test_committed_policy_and_security_md_agree` validates the real policy and
`SECURITY.md`, so changing one without the other fails the test suite.
