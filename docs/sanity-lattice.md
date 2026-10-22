# Cross-asset sanity lattice (#488)

A per-asset deviation bound only asks *"is this price near the other prices for
this asset?"*. That is blind to a manipulation that is internally consistent for
one asset but impossible given its relatives — a stablecoin quoting at 1.30
while its sibling still says 1.00, or a triangular FX cross that does not close.

This module adds a family-level check: every candidate aggregate is validated
against the **published** aggregates of the assets it is declared related to.

## Relation kinds

| Kind | Holds when | Fields used |
|---|---|---|
| `Peg` | `price(asset) ~= price(peer) * ratio_num / ratio_den` | `peer`, `ratio_num`, `ratio_den` |
| `Triangle` | `price(asset) * price(peer) ~= price(peer2)` | `peer`, `peer2` |
| `Spread` | `\|price(asset) - price(peer)\| / price(asset) <= tolerance` | `peer` |

```rust
client.add_sanity_relation(&asset, &SanityRelation {
    asset: asset.clone(),
    peer: peer.clone(),
    peer2: None,
    kind: SanityRelationKind::Peg,
    tolerance_bps: 100,        // 1 %
    ratio_num: 1,
    ratio_den: 1,
    action: SanityAction::Reject,
});
```

Relations are checked against the **stored** aggregate of the peer, not against
the peer's sources, so a check never re-runs a whole aggregation.

## Actions

| Action | Effect |
|---|---|
| `Flag` | Publishes, and records + events the violation |
| `Reject` | Does not publish; the previous aggregate stays live |
| `Quarantine` | Does not publish, and the asset stays blocked until an admin clears it |

## Cycles terminate

The relation graph is allowed to contain cycles — A pegs B and B pegs A is a
legitimate configuration. Evaluation is **iterative with a visited set and a
hard cap** (`MAX_EXPANSIONS = 64`), never recursive. A cycle terminates because
a visited asset is never expanded twice, and a pathological graph is
additionally cut off by the cap.

```rust
let peers = client.get_sanity_peers(&a);  // terminates on A <-> B <-> C
```

## De-pegs suspend, they do not disable

A legitimate de-peg breaks the assumed relation. `declare_depeg` suspends every
relation touching an asset until a given ledger **without disabling the asset
itself** — it keeps aggregating, publishing and serving. Only the family checks
stop applying, which is exactly what a de-peg needs.

```rust
client.declare_depeg(&asset, &until_ledger, &String::from_str(&e, "issuer failure"));
assert!(client.is_depeg_suspended(&asset));
// ... the asset still publishes the off-peg value ...
client.clear_depeg(&asset);
```

## An unpriced peer is not a violation

If a peer has never published, the family is simply not observable yet. Treating
that as a violation would let an unpriced relative block a healthy feed, so the
relation is skipped and the aggregate publishes.

## Violations are reproducible

`SanityRelationViolatedEvent` carries everything needed to redo the check
off-chain: both assets, the candidate value, the reference value, the observed
`magnitude_bps`, the `tolerance_bps` it exceeded, and the action taken.
`get_sanity_status(asset)` returns the same record.

## Bounds

Tolerances are `1..=10000` bps (`MAX_TOLERANCE_BPS`). Wider than that is not a
relation but an absence of one. A peg ratio needs a positive numerator and a
non-zero denominator, and a triangle must name three distinct registered assets
— a degenerate "triangle" is a self-reference with no information in it.

## Errors

| Code | Name | Meaning |
|---|---|---|
| 186 | `InvalidSanityRelation` | A self-relation, a degenerate triangle, a duplicate pair, or an unregistered peer |
| 187 | `InvalidSanityTolerance` | Tolerance is `0` or above `10000` |
| 190 | `InvalidSanityRatio` | Peg ratio has a non-positive numerator or a zero denominator |
