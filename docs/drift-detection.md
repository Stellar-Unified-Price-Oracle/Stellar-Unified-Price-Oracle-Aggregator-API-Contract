# Oracle-vs-Benchmark Long-Horizon Drift Detection (#497)

> Compares the published aggregate against an independent external benchmark
> over a long rolling window and reports a *sustained directional* bias.

## Problem

A slow, small bias evades every per-ledger deviation check entirely while
systematically mispricing consumers. A 2 % lean that is present on every single
round never trips a threshold, because no single round is wrong by much — and
over a month that compounds into real mispricing. Long-horizon comparison is the
only way to see it.

## Time alignment

Oracle and benchmark snapshots are taken at different instants. Comparing an
aggregate stamped at `t0` against a benchmark stamped at `t1` measures the
price move *between the two*, not the oracle's error. Samples are admitted only
when

```text
|benchmark_timestamp - aggregate_timestamp| <= max_alignment_secs
```

and are otherwise counted in `misaligned_skipped` and dropped.

This is a deliberate trade-off, and the default (`300 s`, matching the default
`resolution`) is a compromise:

- A **tight** window discards real samples during fast markets, so drift becomes
  harder to detect exactly when it matters most.
- A **loose** window lets snapshot skew masquerade as bias, producing false
  positives on any trending market.

Misaligned samples are *counted*, not silently dropped, so an operator can see
how much of the benchmark feed is unusable and tune the tolerance rather than
wondering why the window is empty. The skew is not assumed to be zero-mean: a
systematically late benchmark shows up as a persistently misaligned feed rather
than as phantom drift.

## Sustained drift vs transient divergence

A single large divergence — a benchmark print on a thin book, a flash move — is
**not** drift. Drift is directional *and* repeated.

`DriftReport` carries `directional_consistency_bps`: the share of samples that
**individually** breach `bias_threshold_bps` *and* sit on the same side as the
mean bias. Both halves matter:

- Requiring each sample to breach the threshold is what rejects a transient
  spike. One +100 % sample among twelve on-benchmark samples scores ~8 %, not
  ~92 % — the mean is dragged to +833 bps, but only one sample is actually out
  of line.
- Requiring the same sign is what rejects a whipsaw, where every sample is large
  but they cancel out.

Sustained drift requires **all three** of:

1. at least `min_samples` aligned samples (default 8),
2. `|mean_bias_bps| >= bias_threshold_bps` (default 100 bps = 1 %), and
3. `directional_consistency_bps >= 7000` (70 %).

| Series (12 samples, threshold 100 bps) | `mean_bias_bps` | consistency | alerts? |
|---|---|---|---|
| steady +200 bps | +200 | 10 000 | **yes** — sustained drift |
| steady −200 bps | −200 | 10 000 | **yes** — drift, sign flipped |
| one +10 000 bps spike | +833 | 833 | no — transient |
| alternating ±2 000 bps | ~0 | 0 | no — whipsaw |
| 3 × +500 bps | +500 | 10 000 | no — too few samples |

## Divergence is a signal, not proof

The benchmark can be wrong, stale or manipulated; a divergence may be the
benchmark that is wrong. This module therefore:

- **never mutates an on-chain price** — `record_sample` and `get_report` touch
  only their own keys, and `drift_never_mutates_on_chain_prices` asserts the
  aggregate and the stored submission are byte-identical afterwards;
- **never changes a source set or a configuration**;
- **never triggers remediation.** An automated response to a possibly wrong
  benchmark is how a benchmark attack becomes a price attack.

## Querying and bounds

| Endpoint | Purpose |
|---|---|
| `record_drift_sample(asset, oracle_price, oracle_ts, benchmark_price, benchmark_ts)` | Records one comparison. Permissionless: a benchmark feed is untrusted input, and the sample has no effect on any price. |
| `get_drift_report(asset)` | The metrics above. Pure read. |
| `set_drift_thresholds(min_samples, bias_threshold_bps, max_alignment_secs)` / `get_drift_thresholds()` | Admin-configured thresholds. |
| `reset_drift_window(asset)` | Admin-only; clears the window and the misaligned counter. |

The rolling window holds at most `WINDOW_CAPACITY` (64) samples, so the
per-asset footprint is fixed no matter how long the contract runs and how many
samples are fed.

## Out of scope

Replacing on-chain sources with benchmark data. The benchmark is a second
opinion for operators, never a substitute for the source set.

See `docs/source-coverage.md` for the source-set-side reporting.
