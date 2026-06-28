# Disaster Recovery Plan

This document covers failure scenarios for the Stellar Unified Price Oracle contract and
describes detection, immediate response, recovery steps, and prevention measures for each.

---

## Scenario 1 — Contract Upgrade Goes Wrong

**Description:** A new WASM is deployed via `upgrade()` but contains a bug or introduces
incompatible storage schema changes that break the contract.

### Detection
- On-chain calls to any endpoint start returning unexpected errors.
- CI/CD smoke tests against the deployed contract fail.
- Monitoring dashboards show a sudden spike in contract errors.

### Immediate Actions
1. **Pause the contract** via `pause()` to stop price submissions and prevent further damage.
2. Announce an incident to oracle consumers.

### Recovery Steps
1. Identify the previous working WASM hash from deployment records or the upgrade event log.
2. Propose a rollback via `propose_operation(0, <old_wasm_hash_bytes>)` and wait for the
   timelock delay to elapse.
3. Execute the rollback with `execute_operation(<op_id>)`.
4. Run the integration test suite against the restored contract.
5. Resume normal operations via `unpause()`.

### Prevention
- Always deploy to testnet and run the full test suite before upgrading mainnet.
- Use the timelock so there is a window to cancel a bad upgrade before it executes.
- Keep a record of all deployed WASM hashes and their Git tags.

---

## Scenario 2 — Admin Key Compromised

**Description:** The admin private key is leaked or stolen, giving an attacker control over
governance functions (`set_admin`, `add_source`, `upgrade`, etc.).

### Detection
- Unexpected governance transactions appear on-chain.
- Alerting fires on `AdminChangedEvent`, `SourceAddedEvent`, or `ContractUpgradedEvent`
  from an unknown origin.

### Immediate Actions
1. **Do not panic.** Assess whether the attacker has already called `set_admin`.
2. If the original admin key still controls the contract, immediately call `set_admin` to
   transfer control to a freshly generated, secure keypair.
3. Revoke all API access and rotation of any secrets that shared infrastructure with the
   compromised key.

### Recovery Steps
1. Audit all changes made by the attacker (source list, WASM hash, configuration).
2. Reverse any malicious changes using the new admin key.
3. Notify oracle consumers of the incident and any transient data integrity risk.

### Prevention
- Store the admin key in a hardware wallet or a cloud HSM (e.g., AWS KMS).
- Use a multi-sig scheme for admin operations if the platform supports it.
- Rotate admin keys periodically and after any suspected exposure.
- Never store the admin secret key in environment variables on shared servers.

---

## Scenario 3 — Source Collusion Detected

**Description:** Two or more registered oracle sources submit coordinated, manipulated prices
to move the median aggregate in their favor.

### Detection
- Per-source price data shows multiple sources clustering around an extreme value that
  diverges significantly from independent market data.
- The `SubmissionDeviant` flag is set on multiple sources simultaneously.
- External market feeds (CoinGecko, Binance) show a material discrepancy versus the oracle.

### Immediate Actions
1. **Pause the contract** to halt further manipulated aggregation.
2. Identify the colluding sources by comparing their recent submissions against reference prices.
3. Remove the colluding sources via `remove_source(<address>)`.

### Recovery Steps
1. Manually inspect the on-chain history to determine the ledger range affected.
2. If possible, issue a price override via `override_price()` to correct the aggregate for
   the duration of the incident, then remove it once legitimate sources resume.
3. Onboard replacement oracle sources.
4. Resume via `unpause()`.

### Prevention
- Require a minimum of 3 independent, geographically and organizationally diverse sources.
- Set a tight `max_price_deviation` (e.g., 5 %) to flag outlier submissions automatically.
- Monitor per-source submission patterns for clustering anomalies.

---

## Scenario 4 — Incorrect Price Aggregation

**Description:** A bug in the aggregation logic (median, mean, or trimmed mean) produces an
incorrect aggregate price without contract-level errors.

### Detection
- The aggregate price diverges from all individual source submissions.
- Consumer contracts report unexpected pricing behavior.
- Automated sanity checks in the monitoring dashboard flag an impossible price movement.

### Immediate Actions
1. Pause the contract.
2. Capture the current aggregate and per-source prices from the chain for forensic analysis.

### Recovery Steps
1. Reproduce the bug in a local test environment using the captured state.
2. Fix the logic bug in the contract source code and write a regression test.
3. Deploy the fixed WASM via the upgrade + timelock flow.
4. After the fix is live, allow sources to resubmit to re-establish a correct aggregate.
5. Unpause and communicate to consumers.

### Prevention
- Comprehensive property-based tests (see `prop_tests.rs`) covering edge cases in aggregation.
- Fuzz test the median and mean computations with large and extreme price sets.

---

## Scenario 5 — Contract Storage Corruption

**Description:** A bug or an edge case in storage writes leaves the contract in an
inconsistent state (e.g., `PriceHistoryLedgers` out of sync with actual `PriceHistory` entries).

### Detection
- `get_historical_prices` panics or returns empty results when history is expected.
- `get_all_prices` returns fewer entries than the number of registered sources.
- Contract-level errors appear for operations that should succeed.

### Immediate Actions
1. Pause the contract to prevent further state corruption.
2. Log all observable on-chain state before attempting any fixes.

### Recovery Steps
1. If the corruption is isolated to temporary storage (history entries), it will self-heal
   after entries expire. Sources can resubmit to rebuild the aggregate.
2. If persistent storage is corrupted, an admin-callable repair function may need to be
   added via a contract upgrade. Implement and deploy with thorough tests.
3. Document the corrupted ledger range and inform consumers.

### Prevention
- Invariant checks in `submit_price` to assert consistency of history index vs. entries.
- Integration tests that simulate high-frequency submissions and history pruning.
- Staged rollout of upgrades (testnet → canary → mainnet).

---

## Emergency Contacts and Runbook Reference

| Role | Action |
|------|--------|
| On-call engineer | Pause contract, triage alert |
| Security lead | Admin key compromise response |
| Consumer relations | Notify dependent contracts / dApps |

For deployment commands and RPC endpoint configuration, see `scripts/verify-deployment.sh`.
