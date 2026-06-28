# Governance Proposal Template

Use this template for all governance proposals related to the Stellar Unified Price Oracle Aggregator contract. Fill in every section before submitting.

---

## Title

<!-- A short, descriptive title. Example: "Add Chainlink as oracle source" -->

## Author

<!-- Your name / GitHub handle and contact. -->

## Date

<!-- YYYY-MM-DD -->

## Summary

<!-- 2–3 sentences describing what this proposal does and why it matters. -->

---

## Motivation

<!-- Why is this change needed? What problem does it solve or what improvement does it enable?
     Include any relevant data, incidents, or community feedback. -->

## Specification

<!-- Precise technical description of the proposed change.
     - For upgrades: include the new WASM hash and a diff/changelog of behavioural changes.
     - For parameter changes: list the parameter name, current value, and proposed value.
     - For source additions/removals: name, address, and reputation/credentials of the source.
     - For asset management: asset symbol, contract address, and decimal precision. -->

## Rationale

<!-- Why this approach over alternatives?
     Discuss trade-offs, rejected alternatives, and why this option was selected. -->

---

## Implementation Plan

<!-- Step-by-step breakdown of what needs to happen on-chain and off-chain.
     Example:
     1. Propose operation via `propose_operation()` — op_type and encoded data.
     2. Wait for timelock delay to elapse.
     3. Execute operation via `execute_operation(op_id)`.
     4. Verify state change on-chain. -->

## Timeline

| Milestone | Target Date |
|-----------|-------------|
| Proposal posted | <!-- YYYY-MM-DD --> |
| Community review period ends | <!-- YYYY-MM-DD --> |
| On-chain proposal submitted | <!-- YYYY-MM-DD --> |
| Timelock expires / execution ready | <!-- YYYY-MM-DD --> |
| Execution | <!-- YYYY-MM-DD --> |

---

## Risk Assessment

<!-- Identify potential risks and mitigations.
     Consider: smart contract risk, market impact, source reliability, irreversibility. -->

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| <!-- Risk description --> | Low/Med/High | Low/Med/High | <!-- How it is mitigated --> |

---

## Vote Options

- **Yes** — approve and proceed with implementation.
- **No** — reject this proposal.
- **Abstain** — no preference; counted toward quorum but not for/against.

---

## Example Proposals

### Example 1 — Contract Upgrade

```
Title:    Upgrade oracle contract to v2.1.0
Author:   alice.stellar / alice@example.com
Date:     2025-01-15
Summary:  Upgrades the on-chain WASM to v2.1.0, which adds the trimmed-mean
          aggregation method and fixes a rounding edge case in compute_median.

Specification:
  - New WASM hash: abc123...def456 (32 bytes)
  - Changelog: https://github.com/org/repo/releases/tag/v2.1.0
  - No storage schema changes; no migration required.

Implementation Plan:
  1. propose_operation(op_type=0, data=<new_wasm_hash_bytes>)
  2. Wait 10 ledgers (timelock_duration default).
  3. execute_operation(op_id)
  4. Verify get_description() and decimals() return expected values.
```

### Example 2 — Add Oracle Source

```
Title:    Register Band Protocol as oracle source
Author:   bob.stellar
Date:     2025-02-01
Summary:  Adds Band Protocol's Stellar address as a permissioned price source,
          increasing source count from 2 to 3 and improving aggregation robustness.

Specification:
  - Source address: GBAND...XXXX
  - Display name: "Band Protocol"
  - Call: add_source(source=GBAND...XXXX, name="Band Protocol")

Risk Assessment:
  - If Band goes offline, min_sources_required (currently 2) is still met.
```

### Example 3 — Parameter Change

```
Title:    Increase max_history_length from 100 to 500
Author:   carol.stellar
Date:     2025-03-10
Summary:  Increases the per-asset history retention from 100 to 500 entries to
          support longer backtesting windows for consumer contracts.

Specification:
  - Parameter: MaxHistoryLength
  - Current value: 100
  - Proposed value: 500
  - Call: set_max_history_length(500) via timelock (op_type=3).
```

### Example 4 — Asset Registration

```
Title:    Register USDC as a tracked asset
Author:   dave.stellar
Date:     2025-04-05
Summary:  Registers USDC (Circle's Stellar token) so oracle sources can begin
          submitting USD/USDC prices.

Specification:
  - Asset contract address: GUSDC...YYYY
  - Call: register_asset(asset=GUSDC...YYYY)
  - Sources will begin submitting within 24 hours of registration.
```
