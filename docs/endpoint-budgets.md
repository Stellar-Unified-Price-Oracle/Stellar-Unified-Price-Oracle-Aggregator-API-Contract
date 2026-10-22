# Per-Endpoint Compute and Storage Budgets (#692)

Every public endpoint of the production surface is measured for CPU, memory
and storage under a representative and a boundary input, and compared with a
committed budget on every CI run.

* Gate: `contracts/price-oracle/src/endpoint_budget_tests.rs`
* Budgets: the `BUDGETS` table in that file
* Tolerance: `TOLERANCE_PCT = 5`
* Run: `make endpoint-budgets` (CI step "Per-endpoint budgets", and part of `make test`)

This complements `docs/gas-budget.md`, which gates the hot paths on an
adversarial corpus; this gate covers the whole production surface.

## Inputs

| Input | Sources | History entries | Purpose |
|---|---|---|---|
| `representative` | 3 | 3 | ordinary production shape |
| `boundary` | 10 | 10 (= `max_history_length`) | supported source maximum (`docs/gas-budget.md`, Ceilings) and full history, so writes also evict |

Read-only endpoints share one fixture per input. Every write endpoint runs on
its own fresh fixture, so no measurement depends on another.

## Dimensions

| Dimension | Measured as |
|---|---|
| `cpu` | CPU instructions of the single top-level invocation |
| `mem` | memory bytes of the single top-level invocation |
| `storage` | growth in retained history bytes (`get_total_storage_budget().total_estimated_bytes`) caused by the call |

## Endpoints and budgets

| Endpoint | Measured call | CPU | Mem (bytes) | Storage (bytes) |
|---|---|---|---|---|
| `initialize` | on a fresh contract | 10,000,000 | 5,000,000 | 0 |
| `add_source` | one more source on top of the input's set | 10,000,000 | 5,000,000 | 0 |
| `remove_source` | last source, after it contributed (recomputes the aggregate) | 25,000,000 | 10,000,000 | 0 |
| `register_asset` | one new asset | 10,000,000 | 5,000,000 | 0 |
| `submit_price` | last source completing a round in a new ledger | 25,000,000 | 10,000,000 | 96 |
| `submit_prices` | same, as a 1-asset batch | 25,000,000 | 10,000,000 | 96 |
| `trigger_aggregation` | full re-aggregation over every submission | 25,000,000 | 10,000,000 | 96 |
| `prune_history` | prune to 1 entry | 25,000,000 | 10,000,000 | 0 |
| `get_price` | aggregate read | 10,000,000 | 5,000,000 | 0 |
| `get_all_prices` | every per-source submission | 10,000,000 | 5,000,000 | 0 |
| `get_historical_price` | most recent history entry | 10,000,000 | 5,000,000 | 0 |
| `get_historical_prices_paginated` | full page (50) from the start | 10,000,000 | 5,000,000 | 0 |
| `lastprice` | SEP-40 latest price | 10,000,000 | 5,000,000 | 0 |
| `price` | SEP-40 lookup at the oldest retained timestamp (longest backward scan) | 10,000,000 | 5,000,000 | 0 |
| `prices` | SEP-40 history, one record per seeded entry | 10,000,000 | 5,000,000 | 0 |
| `assets` | SEP-40 asset list | 5,000,000 | 3,000,000 | 0 |
| `decimals` | SEP-40 decimals | 5,000,000 | 3,000,000 | 0 |
| `resolution` | SEP-40 resolution | 5,000,000 | 3,000,000 | 0 |
| `base` | SEP-40 base asset | 5,000,000 | 3,000,000 | 0 |

A storage budget of 96 bytes is one history entry: write paths may add at
most one entry per call; at the boundary input the oldest entry is evicted in
the same call, so the net growth is 0.

## Reading a failure

Each measurement prints

```text
ENDPOINT_BUDGET endpoint=<name> input=<input> cpu=<n> mem=<n> storage=<n>
```

and every dimension over budget + 5% is reported in one failure, for example:

```text
ENDPOINT BUDGET EXCEEDED endpoint=submit_price input=boundary dimension=cpu measured=… budget=25000000 ceiling=26250000 delta=+… (+…%)
```

The line names the endpoint, the input that triggered it and the dimension, so
the regression can be reproduced with `make endpoint-budgets` and bisected.

The gate also fails if an endpoint in `BUDGETS` was not measured under every
input, so an endpoint cannot silently drop out, and
`endpoint_budget_gate_rejects_inflated_cost` checks that an inflated
measurement is rejected with that message.

## Updating a budget

The process in `docs/gas-budget.md` ("Updating a budget") applies: a raise is
its own change with the endpoint, old → new value, percentage change and a
written justification, approved by a reviewer other than the author. Lowering
a budget needs no justification. A new public endpoint is added to `BUDGETS`
and measured in `measure_all` in the same change.
