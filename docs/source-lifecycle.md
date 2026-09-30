# Source Onboarding / Offboarding Lifecycle (#402)

Module: `contracts/price-oracle/src/source_lifecycle.rs`.
Script: `scripts/source-lifecycle.sh`.

## Onboarding checklist

| Step | On-chain call | Pass/fail evidence (`get_onboarding_checklist`) |
|---|---|---|
| 1. Identity | `onboard_source(source, name, identity)` (admin) | `identity_verified`. The fingerprint must be unused and never revoked, and the address must never have been offboarded. |
| 2. Bond | `deposit_source_bond(source)` (source) | `bond_posted`, `bond_amount` (≥ `set_source_bond` amount) |
| 3. Probation | automatic, `probation_secs` (default 7 days, `set_lifecycle_config`) | `probation_complete` |
| 4. Graduate | `graduate_source(source)` (permissionless; checks steps 1–3) | `graduated` |

`identity` is a 32-byte fingerprint of the source's off-chain verification
record (#211), for example the hash of its DID document. Legal identity
verification itself is out of scope.

### Probation cap

While any source is on probation, **at most `PROBATION_MAX_COUNTED = 1`
probation value is counted per aggregation round**, across all probation
sources combined. Probation sources also cannot cast volatility-blackout
signals (#400). Joining several sources with minimal bonds therefore buys at
most one counted value, which cannot form a quorum majority. Asserted by
`probation_caps_quorum_influence`: three agreeing probation sources contribute
exactly one value.

## Offboarding: one atomic step

`offboard_source(source, evidence_asset)` (admin) does everything in one
transaction:

1. **Slash** (only when `evidence_asset` is given, see below).
2. **Revoke derived state** for every registered asset:
   * submissions, submission ledgers, last-submission ledgers and
     non-compliance flags are removed;
   * reputation is pinned to 0, not deleted, because deletion would reset it
     to the default score;
   * the lifecycle record is removed.
3. **Tombstone** the address and the identity fingerprint.
4. **De-register** the source. Its bond is returned unless it was slashed.
5. **Recompute** every asset from the surviving sources and emit
   `SourceOffboardedEvent`.

There is no window in which the source is nominally removed but still counted.
Covered by `offboarding_is_atomic_and_leaves_zero_influence`, which fails if
any derived key survives or if a later aggregate still reflects the removed
source.

## Slashing

Slashing requires evidence that the contract verifies itself: the source's
**stored** submission for `evidence_asset` must deviate from that asset's
current published aggregate by more than `slash_deviation_bps` (default
2 000 = 20 %). The admin cannot simply assert malfeasance. Without qualifying
evidence the call fails with `InvalidConfiguration` and nothing is revoked
(`slashing_requires_evidence`). On success the full bond is forfeited to the
treasury, and `SourceSlashedEvent` records the price, aggregate, deviation and
threshold.

## Re-onboarding and rotated identities

An offboarded address is rejected with `IdentityRevoked` by all of these:

* `add_source`
* `add_source_with_assets`
* `onboard_source`
* `rotate_source_key` when used as the new key

An offboarded identity fingerprint is rejected by `onboard_source`. A key
rotation carries the lifecycle record and identity binding to the new address,
so rotating a key cannot shed probation or the identity record. Covered by
`rotated_identity_cannot_launder_removal`.

**Limitation:** a completely new address together with a new identity
fingerprint is not linkable on-chain. Preventing that relies on the off-chain
identity verification (#211) refusing to issue a second fingerprint to the
same operator.
