# Degraded-Mode Serving Analytics (#495)

> Every path by which a consumer receives a degraded value is instrumented, and
> the counts are queryable per asset, per state and per rolling window.

## Problem

A consumer cannot tell, from a price alone, whether the value it read was the
live median or something the contract substituted under stress. "How many
consumer reads last month were degraded?" — the number that matters for trust
and SLA — had no answer, because degradation was invisible in aggregate.

## Degradation states

`degradation::classify` walks these in order and returns the **first** match, so
every degraded read is attributed to exactly one reason.

| State | Discriminant | Predicate on the read path |
|---|---|---|
| `Fresh` | 0 | No predicate held. |
| `Stale` | 1 | The value is older than the caller's `max_age`, or older than the asset resolution. |
| `Clamped` | 2 | The value came from a freeze or an admin override, not from the live median. |
| `LowConfidence` | 3 | Fewer than `min_sources_required` sources contributed. |
| `Deferred` | 4 | The live aggregation path was unavailable (circuit breaker tripped) and a TWAP / last-raw fallback was served. |

Precedence is consumer-harm-first: a value that is both stale and overridden is
reported as `Stale`, because staleness is what the caller must act on. Because
each state is *also* counted separately, the counts stay reconstructible from
the events either way.

A read that is refused because the value is stale counts as `Stale` too: the
consumer did not get a usable value, which is exactly what a degradation rate is
supposed to measure.

## Sampling never hides severe states

A high-frequency degradation (a stale read on every poll) would otherwise flood
the event stream, so `sample_every` emits one `DegradedReadEvent` per N counted
degradations.

**Severe states — `Clamped` and `Deferred` — are never sampled.** They are rare,
and rare-and-severe is precisely what sampling must not lose. Window *counts*
are always exact, independently of sampling, and every emitted event carries its
own `window`, so the per-window rate is reconstructible off-chain either way.

## Querying

| Endpoint | Returns |
|---|---|
| `get_degradation_stats(asset)` | Counts for the asset's current rolling window. |
| `get_degradation_window_stats(asset, window)` | Counts for an explicit (possibly older) window. |
| `get_degradation_config()` / `set_degradation_config(config)` | The instrumentation configuration. |

`DegradationStats`:

| Field | Meaning |
|---|---|
| `window` | Window index (`sequence / window_ledgers`). |
| `total_degraded` | Degraded reads counted, all states. |
| `by_state` | Per-state counts, indexed by the `DegradationState` discriminant. |
| `severe` | `Clamped + Deferred`; never sampled. |
| `emitted_events` | Individual events emitted, including sampled-out reads. |

`degradation::total_from_states(&stats) == stats.total_degraded` is the
invariant behind "exactly one reason per read", and is asserted by
`degradation_states_partition_the_degraded_reads`.

## Read gas

A fresh read is effectively free: the only extra work is resolving the quorum it
is classified against. A degraded read pays one counter read and one counter
write, plus one event when sampled.

`degradation_read_gas_overhead_is_bounded` measures both and prints a
`GAS_SAMPLE,degradation_read,...` line. Representative figures:

| Read | Instrumentation off | On | Overhead |
|---|---|---|---|
| Fresh | 439 227 | 439 314 | 0 % |
| Degraded (clamped) | 391 288 | 509 604 | 30 % |

Setting `enabled: false` removes counting entirely; `count_windows: false`
reduces it to the event alone.

## Bounds

Only [`RETAINED_WINDOWS`] (4) window buckets are kept per asset, pruned when a
new window opens — so a long-running deployment has a fixed per-asset footprint
regardless of how many reads it serves.

## Out of scope

Automatically remediating a degraded state. This module reports; it never
changes a price, a source set, or a configuration.

See `docs/anomaly-explanations.md` for the flag-reporting counterpart and
`docs/drift-detection.md` for long-horizon bias.
