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

## Authenticity (#685)

Every event is verified before the first write: the contract id must be the one
the store indexes (pinned on first ingest, or fixed with `--contract-id`), the
ledger must be positive and at or above the configured floor, the topic must be
one the indexer interprets, and a ledger's close time may not go backwards.
A refused event is recorded in `rejected` with a bounded reason code and never
moves the cursor. Details and the full rejection table:
[security/indexer-authenticity.md](security/indexer-authenticity.md).

```bash
python3 -m services.indexer.indexer events.jsonl --db oracle-index.db --contract-id CDEMO...
```

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

## Memory ceiling and backpressure (#697)

`services/indexer/backpressure.py` streams a sustained event feed into the
indexer through a bounded buffer, so a slow store slows the reader down instead
of growing memory:

```python
from services.indexer.backpressure import IngestPump

report = IngestPump(idx, max_buffered_events=1_000, max_buffered_bytes=1 << 20,
                    batch_size=100).run("events.jsonl")
```

* **Ceiling.** At most `max_buffered_events` events and `max_buffered_bytes`
  bytes are buffered, with each event costed at its serialized JSON size. An
  event larger than the byte cap is admitted only into an empty buffer, so it
  cannot deadlock the pump.
* **Backpressure.** When the buffer is full the reader blocks until the store
  drains a batch. No event is dropped. `report.producer_waits` counts how often
  the reader was held back, and `report.high_water_events` /
  `high_water_bytes` show the peak occupancy.
* **Resume.** If the store fails, the reader is stopped, buffered events are
  discarded, and the error is re-raised. Running the pump again on the same
  stream resumes from the cursor ledger. That ledger is replayed, because a
  batch boundary may have fallen inside it, and re-ingesting stored events is
  a no-op, so nothing is lost or duplicated.
* **Stable indices.** Events without an `event_index` are numbered within their
  ledger before batching. A ledger split across two batches would otherwise
  reuse indices, and its second half would be skipped as a replay.

`services/indexer/test/test_backpressure.py` replays a sustained stream against
a slowed store. It checks the event and byte ceilings, flat Python memory as the
stream grows six-fold, and a store crash mid-stream followed by a resume that
ends with every event indexed and no gaps.

## Gap handling

Ledgers must arrive contiguously. If a batch jumps past `cursor + 1`:

1. the missing range is recorded in `gaps`;
2. `ingest` raises `GapError` and does **not** store the jumping event (pass
   `--allow-gaps` to keep going and backfill);
3. replaying the missing ledgers closes the gap — splitting it when only part of
   the interval arrives.

Two cursors, because they answer different questions:

* `cursor` — highest ledger ingested; the resume point for a replay.
* `trusted_ledger` — highest ledger with no open gap before it; the point a
  consumer may rely on for a complete history. It never moves past a missing
  interval, including when `--allow-gaps` was used.

While a gap is open, aggregates after it may have incomplete provenance; the
explorer shows open gaps and the trusted ledger on its index page.

## Reorgs

When a ledger arrives with a `ledger_hash` different from the stored one, all
data from that ledger onwards is rolled back (`rollback_to`) and rebuilt from the
new events, and the replacement is recorded in `reorgs` (`GET /api/reorgs`).
Two different hashes for the same ledger *inside one batch* are rejected as
contradictory input rather than treated as a reorg.

## Explorer

`/` renders sources, the trusted ledger, gaps, rejected events and, for
`?asset=`, the price history with the contributing submissions per row. JSON:
`/api/sources`, `/api/gaps`, `/api/rejections`, `/api/reorgs`,
`/api/history?asset=`, `/api/provenance?asset=&ledger=&event_index=`.

## CI

The `indexer` job runs `services/indexer/test` (on-chain aggregate reproduction,
idempotent replay, restart, gap, reorg, explorer) alongside the hermetic harness
job. `make indexer-trust` runs the same suite as a named check, including the
authenticity cases: spoofed contract event, missing ledger interval with an
unchanged cursor, and a batch with conflicting ledger hashes.
