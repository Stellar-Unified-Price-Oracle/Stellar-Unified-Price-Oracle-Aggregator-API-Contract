---
name: Bug report
about: Report a bug in the price oracle contract
labels: bug
---

**Describe the bug**
A clear description of what the bug is.

**To Reproduce**
Steps to reproduce, including the function called and input values.

**Expected behavior**
What you expected to happen.

**Error code**
If the contract panicked, include the error code (e.g., `Error(Contract, #2)`).

**Environment**
- Rust version: `rustc --version`
- Soroban SDK version (from Cargo.toml):
- Network / test setup:

**Additional context**
Anything else relevant.

---

## Integration reports (production consumers)

If you are a production consumer integrating with the oracle, file your report
through the public path below so it can be triaged and routed consistently.

**Public filing path**
- Open a GitHub issue in this repository using this template and prefix the
  title with `[Integration]` (e.g. `[Integration] Stale price on testnet`).
- Do not use private channels for initial reports; keep the thread public so
  other consumers can follow the resolution.

**Required reproduction data**
- Oracle contract ID and network (testnet/mainnet) you are calling.
- Function called and exact input values (asset pairs, amounts, args).
- Expected vs. actual result, including any error code
  (e.g. `Error(Contract, #2)`).
- Timestamp (UTC) and ledger sequence of the failing call.
- Minimal reproduction: a snippet, script, or steps a maintainer can run.
- SDK/consumer version and environment details.

**Response ownership**
- The maintainers triage every `[Integration]` report; the on-call maintainer
  for the current cycle owns the first response and status updates.
- Expect an initial acknowledgement and a triage category within the
  documented response window; the owner stays assigned until resolution.

**Severity routing**
Reports are routed to one of these documented triage categories:
- `sev:critical` — incorrect prices or outages affecting production consumers.
- `sev:high` — degraded behavior with a viable workaround.
- `sev:medium` — incorrect behavior without production impact.
- `sev:low` — questions, docs, and non-blocking requests.

**Sample report (no secrets)**
```
Title: [Integration] Stale price returned for XLM/USD on testnet
Contract ID: <public testnet contract id>
Network: testnet
Function: get_price(asset_pair = "XLM/USD")
Expected: current price; Actual: price from ledger N-50
Error code: none
Timestamp: 2024-01-01T00:00:00Z, ledger: 123456
Repro: call get_price("XLM/USD") after a price update; see attached script
Environment: soroban-sdk <version>, rustc <version>
```
This sample traces to the `sev:high` category (degraded behavior with a
workaround) and contains no secrets or production credentials.

**Do not include** private keys, seed phrases, API tokens, or production
credentials in any report. Redact sensitive values before filing.
