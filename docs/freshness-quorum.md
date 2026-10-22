# Freshness-aware quorum (#489)

Quorum computed over *all* historical submissions can be satisfied by stale data:
two sources reported an hour ago and a third reported now, and a naive count
says "3 sources, quorum met" when in truth only one source is currently
participating. This module makes the exclusion explicit.

## Only fresh values count

A submission inside the asset's freshness window counts toward quorum. One that
exists but has aged out is counted as **stale** and excluded — never silently
served, never quietly counted.

```rust
let status = client.get_freshness_status(&asset).unwrap();
assert_eq!(status.fresh, 1);   // counts toward quorum
assert_eq!(status.stale, 1);   // excluded, but visible
```

## Window resolution

```
per-asset window  >  tier window (#487)  >  global default
```

`status.overridden` reports which of those supplied the value, so a long-window
illiquid asset is distinguishable from a globally-configured one.

```rust
client.set_default_freshness_window(&60);
client.set_asset_freshness_window(&illiquid, &86_400);  // slow natural cadence
```

## Measured against ledger time

Freshness is always

```
env.ledger().timestamp() - entry.ledger_timestamp
```

never wall-clock and never a source-supplied value. A source cannot make its own
submission fresh by reporting a later timestamp: the ledger timestamp the
submission was *recorded* at is what counts.

## Fail closed

When the fresh count is below quorum the asset is explicitly `FreshnessState::Stale`
— visible through `get_freshness_status` and through the
`FreshnessFilteredEvent` emitted on every gated aggregation — rather than being
served as if it were current. The last genuinely-fresh aggregate stays live
rather than being advanced on the strength of a value that aged out.

## Events

`FreshnessFilteredEvent` carries `fresh`, `stale`, `quorum`, `window_secs`,
`overridden`, `quorum_met` and the ledger time the ages were measured against, so
the decision is auditable without re-deriving it.

## Bounds

Windows are `0..=604800` seconds (one week). `0` clears a configured window and
returns the asset to the next level of the precedence chain. Anything above the
maximum is refused with `ErrorCode::InvalidFreshnessWindow` (188).

## Errors

| Code | Name | Meaning |
|---|---|---|
| 188 | `InvalidFreshnessWindow` | Window above `604800` seconds |
