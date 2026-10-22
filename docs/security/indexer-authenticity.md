# Indexer event authenticity and gap detection (#685)

The reference indexer turns an event export into the price history that
provenance queries, the explorer and the anomaly/volatility pipelines read. It
keys every event by `(ledger, event_index)`, so once a spoofed event is stored
it is indistinguishable from a real one. Authenticity therefore has to be
decided *before* the first write.

Implementation: [`services/indexer/authenticity.py`](../../services/indexer/authenticity.py),
the ingestion path in [`services/indexer/indexer.py`](../../services/indexer/indexer.py),
and the operator guide in [`docs/indexer.md`](../indexer.md).

## What an event must prove

`TrustPolicy` is the operator's statement of what this store indexes:

| Check | Rejection code | Notes |
|---|---|---|
| `contract_id` equals the configured contract, or the one the store was first populated from | `contract_mismatch` | with no configured id the store **pins** the first contract it accepts and persists it in `meta`, so a restart cannot adopt a different contract |
| ledger is a positive integer at or above `min_ledger` | `ledger_range` | |
| `timestamp` is a positive integer and not earlier than the last event indexed at or before this ledger | `ledger_time_regression` | a cheap local contradiction check: a replayed or out-of-sequence export shows up here without a trusted clock |
| `topic` is one the indexer interprets, when `allowed_topics` is set | `unknown_topic` | |
| `ledger_hash` present, when `require_ledger_hash` is set | `ledger_hash_missing` | required if you need reorg detection on the stream |
| the same ledger does not arrive with two different hashes in one batch | `ledger_hash_inconsistent` | contradictory input cannot be a reorg: a reorg replaces the whole height |
| envelope is well formed (`ledger`, `timestamp`, `contract_id`, `topic`, `data`) | `malformed_event` | |

A rejected event is **never written** and **never moves the cursor**. It is
recorded in `rejected` with its bounded reason code, so a stuck ingest is
diagnosable rather than silent:

```bash
python3 -m services.indexer.indexer events.jsonl --db oracle-index.db --contract-id CDEMO...
```

Exit codes: `0` success, `2` either an authenticity rejection or an open gap,
both printed with the ledger position.

## Two cursors, on purpose

`cursor` and `trusted_ledger` answer different questions:

* `cursor` — the highest ledger **ingested**. This is the resume point for a
  replay; after a crash, resume from the last cursor minus a few ledgers and
  idempotency makes the overlap free.
* `trusted_ledger` — the highest ledger with **no open gap before it**. This is
  the high-water mark a consumer may rely on for a complete history.

They differ only when an operator ingests across a missing interval with
`--allow-gaps`. Even then the trust point does not move:

| Step | `cursor` | `trusted_ledger` | `gaps` |
|---|---|---|---|
| ingest 1–2 | 2 | 2 | — |
| ingest 5, default | 2 | 2 | `(3, 4)`, `GapError` raised, ledger 5 not stored |
| ingest 5, `--allow-gaps` | 5 | **2** | `(3, 4)` |
| replay ledger 4 | 5 | 2 | `(3, 3)`, `(5, 5)` |
| replay ledger 3 | 5 | **5** | — |

Replaying a ledger inside a gap **splits** the gap instead of swallowing it, so
filling 4 first does not make 3 disappear from the gap list.

Default ingestion is fail-closed: the jumping event is not written and
`GapError` is raised. `--allow-gaps` exists for operators who would rather keep
up with the head and backfill, and they get `trusted_ledger` to know what is
safe to serve in the meantime.

## Reorgs are surfaced, not absorbed

When a ledger already stored arrives with a different hash, everything from that
height is rolled back and re-indexed, and the replacement is recorded:

```sql
SELECT ledger, old_hash, new_hash, replaced_at FROM reorgs;
```

`GET /api/reorgs` serves the same rows from the explorer. Two different hashes
for one ledger *inside a single batch* are rejected instead
(`ledger_hash_inconsistent`): that is contradictory input, not a reorg.

## Observability

* `GET /api/rejections` — refused events, reason codes, ledger and index.
* `GET /api/gaps` — intervals never delivered.
* `GET /api/reorgs` — replaced ledgers.
* The explorer index page shows `trusted_ledger`, open gaps and rejections.

Queries used by consumers should stop at `trusted_ledger` while a gap is open;
`Indexer.history()` is deliberately unfiltered so an operator can still see what
is there, and the gap view is what tells them how far to trust it.

## Tests

* `python3 -m pytest services/indexer -q` — the original indexer suite plus
  `test_authenticity.py`:

  * a spoofed contract event is rejected with the cursor, the trust point and
    the history unchanged;
  * a missing ledger interval raises `GapError` and does not advance the cursor;
  * `--allow-gaps` records the interval but leaves `trusted_ledger` behind;
  * replaying the missing ledgers closes — or splits — the gap correctly;
  * the pinned contract survives a restart;
  * a mid-batch hash conflict and a close-time regression are rejected;
  * a reorg is rolled back and surfaced.

* `make indexer-trust` — same suite as a single named check.

Related: [RPC trust boundary](rpc-trust-boundary.md) validates the response the
events were read from; this document validates the events themselves.
