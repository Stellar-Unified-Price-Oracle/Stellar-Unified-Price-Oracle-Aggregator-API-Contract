# Volatility-bucketed adaptive quorum (#482)

A fixed quorum is wrong in both directions. During a volatile regime it is too
permissive — precisely when manipulation is most profitable, a small quorum is
easiest to buy. During a calm regime it is needlessly expensive.

## The estimate

Volatility is the **mean absolute return** over a rolling window of
observations, in basis points:

```text
volatility_bps = mean(|price_i − price_{i−1}| / price_{i−1} × 10 000)
```

A mean absolute return rather than a standard deviation, because it needs one
pass, has no square-root or division step that could be tuned, and cannot be
driven to an arbitrary value by a single outlier the way a variance can.

It is **scale-invariant**: a 10 % move is 1 000 bps whether the asset trades at
1 or at 1 000 000. The window stores *returns*, not prices.

The first observation has no predecessor and only seeds the reference price.
Classification needs `min_samples` observations; below that,
`get_effective_quorum` fails with `InsufficientVolatilitySamples` rather than
guessing.

## Buckets

Boundaries ascend; each bucket gets one quorum. With `boundaries = [50, 500]`:

| Bucket | Mean absolute return | Default quorum |
|--------|----------------------|----------------|
| 0 (calm) | `< 50` bps | 1 |
| 1 (moderate) | `50..500` bps | 3 |
| 2 (volatile) | `>= 500` bps | 5 |

`n` boundaries require exactly `n + 1` quorums.

## Hysteresis, and its asymmetry

Transitions are damped, and the damping is **asymmetric**:

- moving **up** (volatility rising, quorum tightening) is **immediate**;
- moving **down** (volatility falling, quorum relaxing) requires the calmer
  bucket to hold for `relax_after` consecutive observations.

The asymmetry is the anti-manipulation property:

- If *both* directions were damped, an adversary who could push a calm asset
  into a high-volatility reading would strand it at the high quorum — safe, but
  a denial of service.
- If *relaxing* were immediate, a single submission reporting a calm price
  would talk the quorum down, and the attacker would then need only that many
  colluding votes. Making relaxation the slow direction means a downgrade must
  be **sustained**, so one submission cannot force it.

## The quorum is fixed at round start

`pin_round_quorum(asset, round)` freezes the effective quorum for a round and
stores it under the round's identity. Later regime changes update the asset's
*current* regime but cannot touch a round already in flight.

This means:

- a round's success criterion does not move under its participants, and
- an adversary cannot change the quorum mid-round to make a round unreachable
  (raising it) or trivially reachable (lowering it).

A consumer that wants to know what a round will actually require should read
`get_round_quorum`, not `get_effective_quorum`.

## Observability

| Endpoint | Purpose |
|----------|---------|
| `set_adaptive_quorum_config(config)` | Configure boundaries, quorums, hysteresis. Admin only. |
| `get_adaptive_quorum_config()` | Read the configuration. |
| `observe_volatility(asset, price)` | Record an observation; returns the new regime. |
| `get_quorum_regime(asset)` | Current bucket, quorum, estimate, sample count. |
| `get_effective_quorum(asset)` | The quorum the current regime implies. |
| `pin_round_quorum(asset, round)` | Freeze the quorum for a round. |
| `get_round_quorum(asset, round)` | The frozen quorum, if pinned. |

Both transitions are evented with the underlying volatility estimate:
`QuorumBucketChangedEvent` (with `relaxed` set when the move was a downgrade)
and `QuorumPinnedEvent` (with the round, bucket, quorum and estimate).
