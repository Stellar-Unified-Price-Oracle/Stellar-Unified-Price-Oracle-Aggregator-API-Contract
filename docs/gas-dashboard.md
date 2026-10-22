# Gas Cost Dashboard and Amplification

This page covers per-endpoint and per-caller cost attribution. It
complements the average-cost tables in [gas-usage.md](gas-usage.md).

## Generating the dashboard

```bash
cargo test -p price-oracle --lib gas_amplification -- --nocapture --test-threads=1 \
  | python3 scripts/gas_dashboard.py > gas-dashboard.md
```

CI runs this in the `gas-dashboard` job and adds the report to the job summary.

## Amplification ratio

```text
amplification = cost imposed on others / cost paid by the caller
```

*Cost paid* is the CPU the caller's own call consumes. *Cost imposed* is the
extra CPU consumed by a fixed victim workload (an honest source's
`submit_price` plus a consumer's `get_price`), measured just before and just
after the caller's action. The ratio is measured for every core
state-modifying endpoint: `submit_price`, `add_source`, `register_asset`,
`mark_price_pending`, `try_finalize_price`.

A caller is flagged as a **sustained amplifier** when its ratio exceeds
`AMPLIFICATION_THRESHOLD` (default `1.0`) for `SUSTAINED_WINDOWS` (default
`3`) consecutive samples. Raw totals are ignored: a caller who pays a lot
but imposes little is never flagged. `gas_amplification_synthetic_high_amplifier_is_flagged`
checks that a synthetic griefer is flagged and a whale is not.

## Why means are insufficient

The mean hides the callers that matter. A single adversarial input (for
example, the call made after many sources have been admitted) can cost
several times the average, and budgets must cover that case. For this
reason the dashboard reports p95, p99 and worst-case cost next to the mean,
and bases budget projections on the **worst case**.

## Budget projection

Projected daily cost = worst-case CPU × `CALLS_PER_DAY`, plus rent: persistent
entries written per call (`RENT_ENTRIES_PER_CALL`), each re-bumped
`TTL_BUMPS_PER_DAY` times at `RENT_CPU_PER_ENTRY_BUMP`. All of these values
can be set with environment variables.

## Known amplification surface

Each admitted source adds to the footprint that every later `submit_price`
on the same asset reads for its median. So `add_source` imposes a
cost on every honest submitter that grows with each admission. Measured over 12
rounds (Soroban test budget, CPU instructions):

| Endpoint | mean paid | worst paid | amplification mean | amplification max |
|---|---|---|---|---|
| `add_source` | 1,164,615 | 1,682,533 | 0.239 | 0.327 |
| `try_finalize_price` | 518,342 | 698,376 | 0.125 | 0.834 |
| `mark_price_pending` | 828,705 | 1,188,251 | 0.109 | 0.226 |
| `register_asset` | 1,930,084 | 2,855,039 | 0.022 | 0.104 |
| `submit_price` | 5,043,324 | 6,423,783 | 0.008 | 0.030 |

`try_finalize_price` is permissionless and peaks at 0.834, the closest of any
endpoint to the 1.0 threshold, so it is the first follow-up candidate.
