# Asset risk tiers (#487)

Per-asset configuration is powerful but easy to get wrong and hard to audit: a
quorum of 2 written on a thin market looks exactly like a quorum of 2 written on
a blue chip, and nothing in the published aggregate says which. This module
replaces the free-for-all with a small, **closed set of reviewed presets**.

```
asset ──▶ tier ──▶ { method, quorum, freshness window, deviation bound }
```

## The tiers

The set is closed at four. Adding one later is a new discriminant, never a
reinterpretation of an existing one, so a stored assignment always resolves to
the parameters it had when it was made.

| Tier | `RiskTier` | Method | Quorum | Freshness | Deviation |
|---|---|---|---|---|---|
| Deep, highly liquid | `Tier1BlueChip` | median | 5 | 60 s | 100 bps |
| Ordinary majors | `Tier2Standard` | median | 3 | 300 s | 300 bps |
| Thin but real | `Tier3Thin` | trimmed mean | 2 | 1 800 s | 1 000 bps |
| Speculative / new | `Tier4Speculative` | trimmed mean | 2 | 900 s | 3 000 bps |

`get_risk_tier_params()` returns the whole table, so the presets are readable in
one place rather than being spread across per-asset configuration.

## Using it

```rust
client.set_asset_risk_tier(
    &asset,
    &Some(RiskTier::Tier3Thin),
    &String::from_str(&e, "thin book, trimmed mean"),
);

let resolved = client.get_resolved_asset_tier(&asset).unwrap();
assert_eq!(resolved.effective.min_sources, 2);
assert_eq!(resolved.base.freshness_secs, 1_800);
```

Tier parameters are applied **consistently**: the tier supplies the aggregation
method, the quorum, the deviation bound and the freshness window, and all four
are read from the same place at aggregation time.

## Overrides stay visible

A per-asset `PolicyOverride` still wins, field by field, over the preset — but
the result is reported as an *override*, never silently merged:

```rust
client.set_asset_policy(&asset, &Some(PolicyOverride {
    method: None,
    min_sources: Some(4),   // tightened from the tier's 2
    freshness_secs: None,
    max_deviation_bps: None,
}));

let r = client.get_resolved_asset_tier(&asset).unwrap();
assert!(r.overridden);           // a tuned asset, not a preset one
assert_eq!(r.effective.min_sources, 4);
assert_eq!(r.base.min_sources, 2);   // the preset is still reported
```

## Fail closed

An asset with **no** tier is not defaulted to the global settings.

* `has_asset_risk_tier(asset)` returns `false`.
* `get_resolved_asset_tier(asset)` returns `None`.
* A stored discriminant that is not one of the four defined tiers is reported as
  unassigned rather than coerced, so a corrupted assignment degrades to
  "unconfigured" rather than to some arbitrary tier.

Enforcement is opt-in, so an existing deployment keeps publishing while it rolls
tiers out:

```rust
// Refused while any registered asset is still unassigned.
client.set_risk_tier_enforcement(&true);
```

Once on, an asset that loses its tier publishes nothing. It is visibly
unconfigured rather than quietly running on defaults.

## A tier change never rewrites the past

Tiers are read at *aggregation* time only. Moving an asset between tiers changes
what the **next** aggregate looks like; every value already published under the
old tier keeps the meaning it had when it was computed. Price and `version` are
both unchanged by the tier move itself.

## Atomic and evented

A tier change is one storage write plus one event in the same invocation, and a
rejected write (for example an oversized `reason`, capped at 256 characters)
leaves the previous tier untouched. `AssetTierChangedEvent` carries the actor,
the old and new tier, the mandatory reason and the ledger.

## Errors

| Code | Name | Meaning |
|---|---|---|
| 185 | `InvalidRiskTier` | A registered asset has no valid tier and enforcement is being turned on |
| 10 | `InvalidConfiguration` | The reason exceeded 256 characters |
