# Ledger Burst Submission Capacity Test (#695)

Simultaneous source submissions and multi-asset traffic are driven ledger by
ledger against target ledger and RPC capacity. The test reports sustainable
throughput, queue growth and the first resource limit reached.

* Test: `contracts/price-oracle/src/ledger_burst_capacity_tests.rs`
* Run: `make burst-capacity` (also part of `make test`, so CI runs it)

```bash
make burst-capacity   # cargo test -p price-oracle --lib burst_capacity -- --nocapture --test-threads=1
```

## Capacity targets

| Target | Value | Meaning |
|---|---|---|
| `LEDGER_CPU_TARGET` | 500,000,000 | ledger-wide CPU instructions available to the oracle's submissions |
| `LEDGER_TX_TARGET` | 100 | Soroban transactions per ledger available to the oracle |
| `RPC_SUBMIT_TARGET` | 60 | RPC `sendTransaction` requests per ledger (one per submission) |

These are the deployment's targets, set as constants at the top of the test.
Change them there to match the network configuration and RPC provider in use.

## Workload

| Phase | Ledgers | Offered per ledger |
|---|---|---|
| `quiet` | 5 | 3 sources × 4 assets = 12 |
| `burst` | 3 | 10 sources × 12 assets = 120 (every source, every asset, same ledger) |
| `drain` | until the queue is empty (max 20) | 12 |

10 sources is the supported maximum (`docs/gas-budget.md`, Ceilings).

## Admission model

Each ledger:

1. New submissions join the back of the queue. A newer submission for a
   (source, asset) pair that is still queued replaces the queued one in place
   (counted as `coalesced`) — a submitter only ever sends its latest price.
2. The queue is admitted oldest first until one of the targets would be
   exceeded. The CPU check uses the worst single-submission cost seen so far,
   so a submission is only sent when it is known to fit.
3. Every admitted submission executes against the contract and is metered;
   the rest stay queued for the next ledger.

## Output

One line per ledger:

```text
BURST ledger=<seq> phase=<phase> offered=<n> coalesced=<n> admitted=<n> cpu_used=<n> queue=<n> limit=<ledger_cpu|ledger_tx|rpc_submit|none>
```

and a summary:

```text
BURST summary sources=10 assets=12 sustainable_per_ledger=<n> worst_submit_cpu=<n> peak_queue=<n> burst_queue_growth=[…] first_limit=<limit>
```

| Reported | Definition |
|---|---|
| `sustainable_per_ledger` | mean submissions admitted per saturated ledger (a ledger where a limit stopped admission) |
| `burst_queue_growth` | change in queue length over each burst ledger |
| `peak_queue` | largest backlog at the end of any ledger |
| `first_limit` | the first target that stopped admission |

## Assertions

* No ledger exceeds any target.
* Below capacity (`quiet`), nothing queues and no limit is reached.
* Above capacity (`burst`), every ledger reaches a limit and leaves a backlog.
* Back at quiet load (`drain`), the backlog drains completely within 20 ledgers.
* Every offered submission is either admitted or superseded by a newer one
  for the same pair — none are lost or duplicated.
* Every asset ends with a published aggregate.
