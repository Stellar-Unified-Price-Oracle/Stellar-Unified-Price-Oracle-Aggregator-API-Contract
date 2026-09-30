# Reference Indexer & Explorer

A minimal, stdlib-only reference for turning contract events into a queryable
history (#539). Code: [`services/indexer/`](../services/indexer/).

```bash
python3 -m services.indexer.indexer events.jsonl --db oracle-index.db
python3 -m services.indexer.explorer --db oracle-index.db --port 8088
```

Input is the canonical event envelope ([event-streaming](event-streaming/README.md)),
one JSON object per line, optionally carrying `event_index` (position within the
ledger; defaults to arrival order) and `ledger_hash` (enables reorg detection).

## What it materializes

| Table | From topic | Used for |
|---|---|---|
| `sources` | `source_added` / `source_removed` | active source set over time |
| `submissions` | `price_submitted` | provenance of each aggregate |
| `aggregates` | `price_aggregated` | price history; `recomputed` holds the indexer's own median |
| `gaps` | — | ledgers known to be missing |

Every aggregate is re-derived as the median of the latest submission from each
source still active at that ledger, using the contract's rounding
(`storage::compute_median`). `Indexer.mismatches()` lists any disagreement, and
`verify_against_chain(lastprice)` compares each asset's latest indexed price with
the contract's `lastprice` query.

## Restart, replay and idempotency

Events are keyed by `(ledger, event_index)`. Re-ingesting anything already
stored is a no-op, so after a crash simply replay from any earlier point — the
safest choice is always "from the last cursor minus a few ledgers".

## Gap handling

Ledgers must arrive contiguously. If a batch jumps past `cursor + 1`:

1. the missing range is recorded in `gaps`;
2. `ingest` raises `GapError` (pass `--allow-gaps` to keep going and fill later);
3. replaying the missing ledgers deletes the gap row.

While a gap is open, aggregates after it may have incomplete provenance; the
explorer shows open gaps on its index page.

## Reorgs

When a ledger arrives with a `ledger_hash` different from the stored one, all
data from that ledger onwards is rolled back (`rollback_to`) and rebuilt from the
new events.

## Explorer

`/` renders sources, gaps and, for `?asset=`, the price history with the
contributing submissions per row. JSON: `/api/sources`, `/api/gaps`,
`/api/history?asset=`, `/api/provenance?asset=&ledger=&event_index=`.

## CI

The `indexer` job runs `services/indexer/test` (on-chain aggregate reproduction,
idempotent replay, restart, gap, reorg, explorer) alongside the hermetic harness job.
