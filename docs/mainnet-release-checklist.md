# Mainnet release checklist

Complete this before any mainnet release. Every gate needs an **owner**, an
**evidence** link (CI run, report artifact, or signed-off document) and an
explicit **result** of `PASS` or `FAIL`. A blank or `FAIL` row is a no-go.

Never paste secrets, keys or production credentials as evidence; link to where
they are held instead.

Release: `vX.Y.Z` · Commit: `<sha>` · Release manager: `<name>`

## Build and supply chain

| Gate | How to check | Owner | Evidence | Result |
|---|---|---|---|---|
| Reproducible build | `make reproducible-build`, digests match | | | |
| Dependencies pinned | `make check-pins` | | | |
| No malware / secrets | `make malware-scan`, `make secret-scan` | | | |
| Advisories and licenses | CI `deny` and `Static analysis` jobs | | | |
| Release provenance | [release-provenance.md](release-provenance.md) | | | |

## Correctness and security

| Gate | How to check | Owner | Evidence | Result |
|---|---|---|---|---|
| Full test suite | `make test` | | | |
| Attack-regression corpus | `make attack-gate` | | | |
| Fuzz corpus replay | `make fuzz-gate` | | | |
| Bounded proofs | `make kani-gate` | | | |
| SEP-40 conformance, upgrade matrix | `make conformance` | | | |
| Security audit items closed | [security-audit-checklist.md](security-audit-checklist.md) | | | |

## Operations

| Gate | How to check | Owner | Evidence | Result |
|---|---|---|---|---|
| Gas and endpoint budgets | `make gas-gate`, `make endpoint-budgets` | | | |
| Testnet lifecycle run | `testnet-lifecycle` workflow on the release commit | | | |
| Runbook covers every paging alert | `python -m services.runbook.check_runbook` | | | |
| No overdue postmortem actions | `python -m services.incident_review.check_postmortems --strict-overdue` | | | |

## Sign-off

| Role | Name | Date |
|---|---|---|
| Release manager | | |
| Security reviewer | | |
