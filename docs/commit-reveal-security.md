# Commit-Reveal Adversarial Analysis (#447)

Commit hash (as of this change):

```
sha256(price_i128_le || salt || round_ledger_u32_le || source_address_xdr)
```

> **Breaking change for off-chain committers:** the committer's address (XDR)
> is now appended to the preimage. Bots must include it or reveals fail with
> `CommitHashMismatch`.

## 1. Copy-commit

*Attack:* copy a victim's commitment to invalidate or de-duplicate their reveal,
or replay the victim's disclosed `(price, salt)` in the copier's own reveal.

*Defence:* commits are stored per `(asset, source, round)`, so a copy never
touches the victim's entry; and the preimage binds the committer's address, so
the copied hash can never be opened by anyone else. Tested by
`test_copy_commit_cannot_neutralise_victim`.

*Forensics:* `PriceCommittedEvent` now carries `hash`; two sources publishing
the same hash in the same round is the copy-commit signature. The copier's
commit then goes unrevealed and is visible via `CommitWithheldEvent`.

## 2. Nonce grinding

A committer already chooses `price` freely; grinding the salt changes nothing
about the revealed value. The only way grinding helps is to open one hash to
a *different* price than committed, which requires breaking SHA-256:

| Goal | Work | At 6×10²⁰ H/s (whole Bitcoin network, 2026) |
|---|---|---|
| Second preimage (open victim-independent H to a new price) | 2²⁵⁶ ≈ 1.2×10⁷⁷ | ≈ 1.9×10⁵⁶ s |
| Birthday collision (pre-build H that opens to two prices) | 2¹²⁸ ≈ 3.4×10³⁸ | ≈ 5.7×10¹⁷ s ≈ 18 billion years |

Expected gain is bounded by moving the median within the honest range: 36 bps
at 9 sources in the load test (`docs/gas-usage.md`). **Conclusion: infeasible**
— cost is astronomically above any gain.

## 3. Withhold-and-shrink

*Attack:* commit, then reveal only when the value is favourable, shrinking the
round's source set.

*Defence:*

* Aggregation only fires once `min_sources_required` reveals exist, so a
  withholder can never produce an aggregate from fewer sources than the
  protocol minimum — the round simply does not aggregate.
* **Penalty:** `slash_expired_commits` (permissionless, callable once the
  reveal window closes) slashes `commit_reveal_slash_amount` from the
  withholder's bond to the treasury and consumes the commit so it cannot be
  slashed twice. Operators should set the slash above the value of a one-rank
  median shift for the largest asset.

Tested by `test_withholding_cannot_shrink_quorum_and_is_slashed`.

*Forensics:* `CommitWithheldEvent { asset, source, round_ledger, slashed_amount }`
plus the `PriceCommittedEvent` with no matching `PriceRevealedEvent`.

## 4. Deadline sniping

*Attack:* reveal in the last ledger of the window to deny others a response.

*Analysis:* every value is fixed at commit time, before any reveal is
visible, so there is nothing to respond to — the last-ledger revealer can only
reveal its committed value or withhold (covered by §3). Revealing at
`round + commit + reveal - 1` with a changed value fails, and at `+0` the
window is closed. Tested by `test_last_ledger_reveal_grants_no_advantage`.
No window widening is needed.

*Forensics:* `PriceRevealedEvent.revealed_at_ledger` vs `round_ledger` shows
exactly how late each reveal landed.

## Reconstructing each attack from events

| Attack | Events |
|---|---|
| Copy-commit | ≥ 2 `PriceCommittedEvent` with equal `hash` and `round_ledger` |
| Grinding | `PriceCommittedEvent.hash` / `PriceRevealedEvent.price` per source across rounds |
| Withholding | `PriceCommittedEvent` without `PriceRevealedEvent`; `CommitWithheldEvent` |
| Sniping | `PriceRevealedEvent.revealed_at_ledger` near the reveal-window end |
