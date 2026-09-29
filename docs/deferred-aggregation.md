# Deferred (quorum-within-window) aggregation (#485)

For illiquid assets, publishing as soon as one source reports is more dangerous
than waiting. An asset can opt into a **deferral policy**: publication is
withheld until a minimum number of sources submit inside a window, and if that
never happens the asset escalates to `Stale` rather than being starved silently
forever.

## States

```
Absent ──first submission──> Deferred ──quorum reached──> Published
                                 │                            │
                                 └──max_defer_secs elapsed───> Stale
```

| State | Meaning for a consumer |
|---|---|
| `Absent` | Nothing has ever been published. No price exists. |
| `Deferred` | Submissions exist, quorum has not been reached in-window. Wait. |
| `Published` | A quorum-backed aggregate is live. |
| `Stale` | Deferral outlived `max_defer_secs`. The feed is starved — treat as failure. |

The four are distinguishable purely from `get_publication_status(asset)`, which
returns a `PublicationStatus`:

| Field | Meaning |
|---|---|
| `state` | `Absent` / `Deferred` / `Published` / `Stale` |
| `received` | Submissions counted inside the current window |
| `missing` | Submissions still required (`quorum - received`) |
| `quorum` | Configured quorum |
| `window_secs` | Configured window |
| `deferred_since` | Unix timestamp the current deferral began |
| `max_defer_secs` | Configured deferral bound |
| `ledger` | Ledger of the last state write |

Consumers **must not** infer "deferred" from an old timestamp — `Deferred`,
`Stale` and `Absent` look identical through `get_price` alone. Query
`get_publication_status` for the distinction.

## Configuration

```rust
client.set_deferral_policy(&asset, &DeferralPolicy {
    quorum: 3,          // 1..=64
    window_secs: 900,   // 1..=86_400
    max_defer_secs: 3600, // window_secs..=604_800
});
```

Bounds are **validated on write**; a violation panics with
`ErrorCode::InvalidDeferralPolicy` (157). Read back with
`get_deferral_policy`, remove with `clear_deferral_policy` (restores the
default publish-immediately trigger).

## Transitions

* **Quorum completes → publish deterministically.** The aggregation path calls
  `should_publish`, which counts distinct sources with a submission whose
  `ledger_timestamp` is inside `window_secs`. Once that count reaches the
  quorum, publication proceeds in the same transaction as the completing
  submission — there is no extra keeper step and no ordering ambiguity.
* **Deferral bound exceeded → stale.** Once `max_defer_secs` have elapsed since
  `deferred_since`, the state becomes `Stale` and the asset stops publishing
  until quorum is met again. Staleness wins over a later-completed quorum, so
  an operator sees the starvation instead of a quietly recovered feed.
* **Non-participating assets are unaffected.** Only assets with a configured
  policy are deferred; the global aggregation trigger policy is unchanged.

## Event

`PublicationStateChangedEvent` — topics `asset`, `state`; fields
`previous_state`, `received`, `missing`, `quorum`, `ledger`. Emitted only on an
actual transition, so a feed can be monitored by watching for it.

## Out of scope

Changing the global aggregation trigger policy. Deferral is opt-in per asset.
