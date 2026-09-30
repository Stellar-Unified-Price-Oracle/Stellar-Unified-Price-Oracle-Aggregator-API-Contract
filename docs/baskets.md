# Basket and index price feeds (#479)

A **basket** is a named, weighted average of other assets' published
aggregates. It exists so an index or portfolio consumer reads one number over a
defined set of assets instead of assembling it themselves — and so every such
consumer gets the *same* number, with the same weights and the same staleness
handling.

## The value

```text
value = Σ (weight_i × price_i) / 1_000_000
```

The sum runs over the **configured** weight vector, in `u128`, with a single
truncation at the end. Because weights sum to exactly 1,000,000, the result
matches a reference weighted-sum computation to within one unit of the last
place. Each term is also reported individually, so the arithmetic can be
audited leg by leg.

| Leg | Price | Weight | Contribution |
|-----|-------|--------|--------------|
| A   | 100   | 250,000 | 25 |
| B   | 200   | 250,000 | 50 |
| C   | 300   | 250,000 | 75 |
| D   | *missing* | 250,000 | 0 |

Under the `Degrade` policy the value above is **150**, not 200. See below.

## Weights

Weights are integers in parts per million and must sum to **exactly**
1,000,000. Enforced on every write, in `set_basket` and in `rebalance_basket`.

The exact sum is the point. If weights summed to 0.999 of the scale, every
published value would be 0.1 % low — permanently, and invisibly. Floating-point
weights are not used because their sum is not exactly representable, which would
put the sum-check beyond what the contract can enforce.

Other write-time rejections:

| Condition | Error |
|-----------|-------|
| Empty list | `InvalidBasketComposition` |
| More than 32 legs | `InvalidBasketComposition` |
| The same asset twice | `InvalidBasketComposition` |
| A zero weight | `InvalidBasketWeights` |
| Weights not summing to 1,000,000 | `InvalidBasketWeights` |
| A leg that is itself a basket | `RecursiveBasket` |

A **zero weight is rejected** rather than accepted-and-ignored: a leg that is in
the list but contributes nothing is precisely the silent skip this feature
exists to prevent.

## Missing and stale constituents

This is the security property the feature is built around. A constituent with no
price — or one older than the staleness bound — must **never** be silently
dropped. A basket that quietly drops its worst-performing constituent reports a
*better* number precisely when it should report a worse one.

A constituent is *usable* when it has a published aggregate whose age is within
the staleness bound (`set_basket_max_staleness`, default 300 s). There are
exactly two policies and no third:

### `Reject` (default)

Panics with `BasketConstituentStale`. The index does not exist. A consumer
cannot mistake a partial index for the real one, because there is nothing to
read.

### `Degrade`

Computes over the usable legs, sets `is_degraded = true`, and emits
`BasketDegradedEvent`.

**The value is not rescaled to the live weights.** This is deliberate. With
three live legs out of four at equal weight, rescaling would report 200 — the
same number a complete basket would report — and call it exact. Instead the
value stays 150, the missing leg is reported with `present = false` and
`contribution = 0`, and the consumer decides whether a flagged lower bound is
acceptable. An index consumer that needs an exact index must check
`is_degraded` and refuse.

## Staleness propagation

`staleness_secs` is the **maximum** age across all constituents. A basket is
only as fresh as its stalest input; propagating anything else would let a stale
leg hide behind a fresh one.

## Recursion

Baskets may not contain baskets, directly or transitively. A leg that is itself
a configured basket is rejected with `RecursiveBasket`, as is a basket naming
itself.

Together with the 32-leg bound this makes the evaluation graph a strict one-level
DAG over registered assets. Compute cost is `O(constituents)` and **there is no
cycle to bound** — the property holds by construction rather than by detection,
so no depth limit or traversal budget is needed.

## Rebalancing

`rebalance_basket(basket, weights)` re-prices weights **positionally**: entry
`i` re-prices constituent `i` of the stored configuration. Passing a whole new
constituent list would let a rebalance silently change *which* assets are in the
index, which is a different and much larger operation.

The rebalance is **atomic**: the entire new vector is validated before the first
write, then lands in a single storage write. A rejected rebalance leaves the
basket exactly as it was — there is no state in which it holds a mix of old and
new weights.

It is also **evented**: `BasketRebalancedEvent` carries the whole new weight
vector, so replaying the event stream reconstructs the exact configuration
history without reading storage.

## API

| Endpoint | Purpose |
|----------|---------|
| `set_basket(basket, config)` | Create or replace a basket. Admin only. |
| `get_basket(basket)` | The stored configuration. |
| `get_basket_composition(basket)` | Ordered legs with weights. |
| `rebalance_basket(basket, weights)` | Re-price atomically. Admin only. |
| `get_basket_value(basket)` | Index value, degraded flag, per-leg breakdown. |
| `get_basket_contribution(basket, i)` | One leg's weight and share. |
| `get_basket_max_staleness()` / `set_basket_max_staleness(s)` | Staleness bound. Admin only. |

## Worked example

```rust
// Three assets at 1/3 each, exact sum.
let mut w = Vec::new(&env);
w.push_back(BASKET_WEIGHT_SCALE / 3 + 1); // 333_334
w.push_back(BASKET_WEIGHT_SCALE / 3);     // 333_333
w.push_back(BASKET_WEIGHT_SCALE / 3);     // 333_333

client.set_basket(&basket, &BasketConfig {
    constituents: /* (A, 333_334), (B, 333_333), (C, 333_333) */,
    total_weight: BASKET_WEIGHT_SCALE,
    staleness_policy: BasketStalenessPolicy::Reject,
    rebalance_policy: BasketRebalancePolicy::Manual,
});
```
