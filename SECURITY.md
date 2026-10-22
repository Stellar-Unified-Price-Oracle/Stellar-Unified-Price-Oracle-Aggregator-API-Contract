# Security Policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub private vulnerability reporting:

https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/security/advisories/new

**Do not open a public issue, pull request or discussion** for a suspected
vulnerability. A private report is visible only to you and the repository's
security responders.

Please include the affected component (contract function, service or script),
the commit or release, the impact, and the steps to reproduce. Do not include
real secret keys or production credentials in the report.

## What happens next

| Step | Commitment |
|---|---|
| Response owner | The **Security Lead**, from the `@Stellar-Unified-Price-Oracle/maintainers` team. |
| Acknowledgement | Within **72 hours** of your report. |
| Triage | The response owner confirms the issue, rates its severity and keeps you updated in the private advisory. |
| Fix | Developed in a private fork attached to the advisory, then released. |
| Disclosure | Coordinated: the advisory is published once the fix is released, or after **90 days**, whichever comes first, unless you agree to a different date. |

## Embargo

Until the advisory is published, details stay within the private advisory and
its private fork. Responders do not discuss the report in public issues, pull
requests, commit messages or chat, and they cite only the advisory ID.

## How this process is verified

`config/security-disclosure.json` records the channel, response owner,
acknowledgement target and embargo above, and
`python -m services.disclosure.policy` checks that this file matches it.
A synthetic disclosure drill checks access control and response time without
storing any report contents. See
[docs/security/disclosure-process.md](docs/security/disclosure-process.md).
