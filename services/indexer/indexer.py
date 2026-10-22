"""Reference indexer for oracle contract events (#539).

Consumes the canonical event envelope (see services/common/events.py and
docs/event-streaming/README.md) and materializes a SQLite store of sources,
raw submissions and aggregates that can be queried for history and provenance.

Guarantees (docs/indexer.md):

* Authentic: every event is verified against the trust policy *before* the
  first write (#685) — contract identity, topic shape, ledger range and close
  time. A spoofed event is rejected, recorded in ``rejected``, and neither
  written nor allowed to advance the cursor.
* Idempotent: every event is keyed by ``(ledger, event_index)``; replaying an
  already-indexed range is a no-op, so restarts can always resume from an
  earlier cursor.
* Gap-aware: ledgers must arrive contiguously. A jump past ``cursor + 1`` is
  recorded in the ``gaps`` table and ``ingest`` raises ``GapError`` unless
  ``allow_gaps=True``; the gap is closed when the missing ledgers are replayed.
  ``cursor`` is the resume point; ``trusted_ledger`` is the highest ledger with
  no open gap before it and never moves past a missing interval.
* Reorg-safe: if a ledger arrives with a ``ledger_hash`` different from the one
  already stored, everything from that ledger onwards is rolled back and
  re-indexed from the new events, and the replacement is recorded in
  ``reorgs``.

Usage:
    python -m services.indexer.indexer events.jsonl --db oracle.db
"""
from __future__ import annotations

import argparse
import json
import sqlite3
from typing import Iterable, Optional

from services.common.events import EventSource, _iter_raw
from services.indexer.authenticity import (
    AuthenticityCode,
    EventAuthenticityError,
    TrustPolicy,
    VerifiedEvent,
    verify_event,
)

TOPIC_PRICE_SUBMITTED = "price_submitted"
TOPIC_PRICE_AGGREGATED = "price_aggregated"
TOPIC_SOURCE_ADDED = "source_added"
TOPIC_SOURCE_REMOVED = "source_removed"

SCHEMA = """
CREATE TABLE IF NOT EXISTS events (
    ledger INTEGER NOT NULL, event_index INTEGER NOT NULL,
    ledger_hash TEXT, timestamp INTEGER NOT NULL, contract_id TEXT NOT NULL,
    topic TEXT NOT NULL, data TEXT NOT NULL,
    PRIMARY KEY (ledger, event_index));
CREATE TABLE IF NOT EXISTS sources (
    source TEXT PRIMARY KEY, name TEXT, added_ledger INTEGER NOT NULL,
    removed_ledger INTEGER);
CREATE TABLE IF NOT EXISTS submissions (
    ledger INTEGER NOT NULL, event_index INTEGER NOT NULL, asset TEXT NOT NULL,
    source TEXT NOT NULL, price TEXT NOT NULL, timestamp INTEGER NOT NULL,
    PRIMARY KEY (ledger, event_index));
CREATE TABLE IF NOT EXISTS aggregates (
    ledger INTEGER NOT NULL, event_index INTEGER NOT NULL, asset TEXT NOT NULL,
    price TEXT NOT NULL, num_sources INTEGER NOT NULL, timestamp INTEGER NOT NULL,
    recomputed TEXT, PRIMARY KEY (ledger, event_index));
CREATE TABLE IF NOT EXISTS gaps (from_ledger INTEGER NOT NULL, to_ledger INTEGER NOT NULL,
    PRIMARY KEY (from_ledger, to_ledger));
CREATE TABLE IF NOT EXISTS cursor (id INTEGER PRIMARY KEY CHECK (id = 0), ledger INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS rejected (ledger INTEGER NOT NULL, event_index INTEGER NOT NULL,
    code TEXT NOT NULL, detail TEXT NOT NULL, PRIMARY KEY (ledger, event_index, code));
CREATE TABLE IF NOT EXISTS reorgs (ledger INTEGER NOT NULL, old_hash TEXT, new_hash TEXT,
    replaced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
"""


class GapError(RuntimeError):
    pass


def median(prices: list[int]) -> int:
    """Mirrors storage::compute_median, including Rust's truncating division."""
    if not prices:
        return 0
    s = sorted(prices)
    n = len(s)
    if n % 2:
        return s[n // 2]
    lower, upper = s[n // 2 - 1], s[n // 2]
    diff = upper - lower
    return lower + (abs(diff) // 2) * (1 if diff >= 0 else -1)


class Indexer:
    def __init__(self, db_path: str = ":memory:", policy: Optional[TrustPolicy] = None):
        self.db = sqlite3.connect(db_path)
        self.db.executescript(SCHEMA)
        self.policy = policy or TrustPolicy()
        #: Contract identity this store indexes: the policy's, else the one
        #: pinned by the first accepted event. Persisted so a restart cannot
        #: silently adopt a different contract.
        self.contract_id: Optional[str] = (
            self.policy.expected_contract_id or self._meta("contract_id")
        )
        self._batch_ledger_hash: Optional[tuple[int, str]] = None

    # -- meta -----------------------------------------------------------
    def _meta(self, key: str) -> Optional[str]:
        row = self.db.execute("SELECT value FROM meta WHERE key = ?", (key,)).fetchone()
        return row[0] if row else None

    def _set_meta(self, key: str, value: str) -> None:
        self.db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", (key, value))

    # -- cursor ---------------------------------------------------------
    @property
    def cursor(self) -> Optional[int]:
        """Highest ledger ingested. This is the resume point for a replay."""
        row = self.db.execute("SELECT ledger FROM cursor WHERE id = 0").fetchone()
        return row[0] if row else None

    @property
    def trusted_ledger(self) -> Optional[int]:
        """Highest ledger with no open gap before it.

        Aggregates above this point may have incomplete provenance, so a
        consumer that needs a complete history stops here until the gap is
        filled. Unlike ``cursor`` it never moves past a missing interval, even
        when the operator ingested across one with ``allow_gaps=True``.
        """
        gap = self.db.execute("SELECT MIN(from_ledger) FROM gaps").fetchone()[0]
        if gap is not None:
            return gap - 1
        return self.db.execute("SELECT MAX(ledger) FROM events").fetchone()[0]

    def _set_cursor(self, ledger: Optional[int]) -> None:
        if ledger is None:
            self.db.execute("DELETE FROM cursor")
        else:
            self.db.execute("INSERT OR REPLACE INTO cursor (id, ledger) VALUES (0, ?)", (ledger,))

    # -- ingestion ------------------------------------------------------
    def ingest(self, source: EventSource, allow_gaps: bool = False) -> int:
        """Ingests events in ledger order. Returns the number of new events.

        Raises ``EventAuthenticityError`` for the first event that fails the
        trust policy and ``GapError`` for the first ledger jump, in both cases
        after committing only what was already verified: a rejected event
        leaves no trace in ``events`` and does not move the cursor.
        """
        added = 0
        counters: dict[object, int] = {}
        self._batch_ledger_hash = None
        for row in _iter_raw(source):
            ledger = row.get("ledger") if isinstance(row, dict) else None
            idx = row.get("event_index") if isinstance(row, dict) else None
            if idx is None:
                idx = counters.get(ledger, 0)
            counters[ledger] = (idx if isinstance(idx, int) else 0) + 1
            added += self._ingest_one(row, idx if isinstance(idx, int) else 0, allow_gaps)
        self.db.commit()
        return added

    def _ingest_one(self, row: dict, idx: int, allow_gaps: bool) -> int:
        ledger = row.get("ledger") if isinstance(row, dict) else None
        explicit_idx = row.get("event_index") if isinstance(row, dict) else None

        # Authenticity is checked before anything is written, so a spoofed or
        # malformed event cannot change the store, the cursor or the gaps.
        try:
            event = verify_event(
                row,
                self.policy,
                pinned_contract_id=self.contract_id,
                last_close_time=self._last_close_time(ledger),
                event_index=explicit_idx if explicit_idx is not None else idx,
            )
        except EventAuthenticityError as error:
            self._record_rejection(ledger, explicit_idx if explicit_idx is not None else idx, error)
            self.db.commit()
            raise

        if self.contract_id is None:
            self.contract_id = event.contract_id
            self._set_meta("contract_id", event.contract_id)

        self._check_ledger_hash(event)

        if self.db.execute(
            "SELECT 1 FROM events WHERE ledger = ? AND event_index = ?",
            (event.ledger, event.event_index),
        ).fetchone():
            return 0  # already indexed: replay is a no-op

        cur = self.cursor
        if cur is not None and event.ledger > cur + 1:
            self.db.execute("INSERT OR IGNORE INTO gaps VALUES (?, ?)", (cur + 1, event.ledger - 1))
            if not allow_gaps:
                # Fail closed: the gap row is kept for triage, the event is not.
                self.db.commit()
                raise GapError(f"missing ledgers {cur + 1}..{event.ledger - 1}")
        # Replaying earlier ledgers closes any gap they fall into, splitting a
        # multi-ledger gap rather than swallowing the still-missing remainder.
        self._close_gap_at(event.ledger)

        data = dict(event.data)
        self.db.execute(
            "INSERT INTO events VALUES (?, ?, ?, ?, ?, ?, ?)",
            (
                event.ledger,
                event.event_index,
                event.ledger_hash,
                event.timestamp,
                event.contract_id,
                event.topic,
                json.dumps(data, sort_keys=True),
            ),
        )
        self._materialize(event.topic, data, event.ledger, event.event_index, event.timestamp)
        if cur is None or event.ledger > cur:
            self._set_cursor(event.ledger)
        if event.ledger_hash:
            self._batch_ledger_hash = (event.ledger, event.ledger_hash)
        return 1

    # -- authenticity helpers -------------------------------------------
    def _last_close_time(self, ledger: object) -> Optional[int]:
        """Close time of the last event already indexed at or before `ledger`."""
        if not isinstance(ledger, int) or isinstance(ledger, bool):
            return None
        row = self.db.execute(
            "SELECT timestamp FROM events WHERE ledger <= ? "
            "ORDER BY ledger DESC, event_index DESC LIMIT 1",
            (ledger,),
        ).fetchone()
        return row[0] if row else None

    def _check_ledger_hash(self, event: VerifiedEvent) -> None:
        """A re-delivered ledger is rolled back; a mid-ledger fork is rejected."""
        if not event.ledger_hash:
            return
        stored = self.db.execute(
            "SELECT ledger_hash FROM events WHERE ledger = ? LIMIT 1", (event.ledger,)
        ).fetchone()
        if not stored or not stored[0] or stored[0] == event.ledger_hash:
            return
        if self._batch_ledger_hash and self._batch_ledger_hash[0] == event.ledger:
            # Two different hashes for one ledger inside one batch cannot be a
            # reorg: a reorg replaces the whole height. It is contradictory
            # input, so it is rejected instead of being rolled back.
            error = EventAuthenticityError(
                AuthenticityCode.LEDGER_HASH_INCONSISTENT,
                event.ledger,
                "the same ledger arrived with two different hashes in one batch",
            )
            self._record_rejection(event.ledger, event.event_index, error)
            self.db.commit()
            raise error
        # A ledger already stored with a different hash means the chain was
        # reorganised: discard everything from this height and re-index it.
        self.db.execute(
            "INSERT INTO reorgs (ledger, old_hash, new_hash) VALUES (?, ?, ?)",
            (event.ledger, stored[0], event.ledger_hash),
        )
        self.rollback_to(event.ledger - 1)

    def _close_gap_at(self, ledger: int) -> None:
        for start, end in self.db.execute(
            "SELECT from_ledger, to_ledger FROM gaps WHERE from_ledger <= ? AND to_ledger >= ?",
            (ledger, ledger),
        ).fetchall():
            self.db.execute("DELETE FROM gaps WHERE from_ledger = ? AND to_ledger = ?", (start, end))
            if start <= ledger - 1:
                self.db.execute("INSERT OR IGNORE INTO gaps VALUES (?, ?)", (start, ledger - 1))
            if ledger + 1 <= end:
                self.db.execute("INSERT OR IGNORE INTO gaps VALUES (?, ?)", (ledger + 1, end))

    def _record_rejection(self, ledger: object, idx: object, error: EventAuthenticityError) -> None:
        safe_ledger = ledger if isinstance(ledger, int) and not isinstance(ledger, bool) else -1
        safe_idx = idx if isinstance(idx, int) and not isinstance(idx, bool) else -1
        self.db.execute(
            "INSERT OR REPLACE INTO rejected (ledger, event_index, code, detail) VALUES (?, ?, ?, ?)",
            (safe_ledger, safe_idx, error.code.value, error.detail),
        )

    def _materialize(self, topic: str, data: dict, ledger: int, idx: int, ts: int) -> None:
        if topic == TOPIC_SOURCE_ADDED:
            self.db.execute(
                "INSERT OR REPLACE INTO sources VALUES (?, ?, ?, NULL)",
                (str(data["source"]), data.get("name"), ledger),
            )
        elif topic == TOPIC_SOURCE_REMOVED:
            self.db.execute(
                "UPDATE sources SET removed_ledger = ? WHERE source = ?", (ledger, str(data["source"]))
            )
        elif topic == TOPIC_PRICE_SUBMITTED:
            self.db.execute(
                "INSERT INTO submissions VALUES (?, ?, ?, ?, ?, ?)",
                (ledger, idx, str(data["asset"]), str(data["source"]), str(int(data["price"])),
                 int(data.get("timestamp", ts))),
            )
        elif topic == TOPIC_PRICE_AGGREGATED:
            asset = str(data["asset"])
            recomputed = median(self._latest_prices(asset, ledger, idx))
            self.db.execute(
                "INSERT INTO aggregates VALUES (?, ?, ?, ?, ?, ?, ?)",
                (ledger, idx, asset, str(int(data["price"])), int(data.get("num_sources", 0)),
                 int(data.get("timestamp", ts)), str(recomputed)),
            )

    def _latest_prices(self, asset: str, ledger: int, idx: int) -> list[int]:
        """Latest submission per still-active source up to (ledger, idx)."""
        rows = self.db.execute(
            """SELECT s.source, s.price FROM submissions s
               LEFT JOIN sources src ON src.source = s.source
               WHERE s.asset = ? AND (s.ledger < ? OR (s.ledger = ? AND s.event_index < ?))
                 AND (src.removed_ledger IS NULL OR src.removed_ledger > ?)
               ORDER BY s.ledger, s.event_index""",
            (asset, ledger, ledger, idx, ledger),
        ).fetchall()
        latest: dict[str, int] = {}
        for src, price in rows:
            latest[src] = int(price)
        return list(latest.values())

    def rollback_to(self, ledger: int) -> None:
        """Discards everything after `ledger` (reorg handling)."""
        for table in ("events", "submissions", "aggregates"):
            self.db.execute(f"DELETE FROM {table} WHERE ledger > ?", (ledger,))
        self.db.execute("DELETE FROM sources WHERE added_ledger > ?", (ledger,))
        self.db.execute("UPDATE sources SET removed_ledger = NULL WHERE removed_ledger > ?", (ledger,))
        self.db.execute("DELETE FROM gaps WHERE from_ledger > ?", (ledger,))
        last = self.db.execute("SELECT MAX(ledger) FROM events").fetchone()[0]
        self._set_cursor(last)

    # -- queries --------------------------------------------------------
    def gaps(self) -> list[tuple[int, int]]:
        return self.db.execute("SELECT from_ledger, to_ledger FROM gaps ORDER BY 1").fetchall()

    def rejections(self) -> list[dict]:
        """Events refused by the trust policy, with their bounded reason code."""
        rows = self.db.execute(
            "SELECT ledger, event_index, code, detail FROM rejected ORDER BY ledger, event_index"
        ).fetchall()
        return [dict(zip(("ledger", "event_index", "code", "detail"), r)) for r in rows]

    def reorgs(self) -> list[dict]:
        """Ledgers replaced by a reorganised chain, oldest first."""
        rows = self.db.execute(
            "SELECT ledger, old_hash, new_hash, replaced_at FROM reorgs ORDER BY ledger"
        ).fetchall()
        return [dict(zip(("ledger", "old_hash", "new_hash", "replaced_at"), r)) for r in rows]

    def last_price(self, asset: str) -> Optional[dict]:
        h = self.history(asset, limit=1)
        return h[0] if h else None

    def history(self, asset: str, limit: int = 100) -> list[dict]:
        rows = self.db.execute(
            """SELECT ledger, event_index, price, num_sources, timestamp, recomputed
               FROM aggregates WHERE asset = ? ORDER BY ledger DESC, event_index DESC LIMIT ?""",
            (asset, limit),
        ).fetchall()
        return [
            {"ledger": r[0], "event_index": r[1], "price": int(r[2]), "num_sources": r[3],
             "timestamp": r[4], "recomputed": int(r[5]) if r[5] is not None else None}
            for r in rows
        ]

    def provenance(self, asset: str, ledger: int, event_index: int) -> list[dict]:
        """The submissions that fed the aggregate at (ledger, event_index)."""
        rows = self.db.execute(
            """SELECT s.source, s.price, s.ledger, s.timestamp FROM submissions s
               LEFT JOIN sources src ON src.source = s.source
               WHERE s.asset = ? AND (s.ledger < ? OR (s.ledger = ? AND s.event_index < ?))
                 AND (src.removed_ledger IS NULL OR src.removed_ledger > ?)
               ORDER BY s.ledger, s.event_index""",
            (asset, ledger, ledger, event_index, ledger),
        ).fetchall()
        latest: dict[str, dict] = {}
        for src, price, led, ts in rows:
            latest[src] = {"source": src, "price": int(price), "ledger": led, "timestamp": ts}
        return sorted(latest.values(), key=lambda r: r["source"])

    def sources(self) -> list[dict]:
        rows = self.db.execute("SELECT source, name, added_ledger, removed_ledger FROM sources ORDER BY 1")
        return [dict(zip(("source", "name", "added_ledger", "removed_ledger"), r)) for r in rows]

    def mismatches(self) -> list[dict]:
        """Aggregates whose on-chain price differs from the recomputed median."""
        rows = self.db.execute(
            "SELECT ledger, event_index, asset, price, recomputed FROM aggregates WHERE price != recomputed"
        ).fetchall()
        return [dict(zip(("ledger", "event_index", "asset", "price", "recomputed"), r)) for r in rows]

    def verify_against_chain(self, lastprice) -> list[str]:
        """Compares each asset's latest indexed aggregate with `lastprice(asset)`,
        a callable wrapping the contract query (returns the price or None)."""
        errors = []
        assets = [r[0] for r in self.db.execute("SELECT DISTINCT asset FROM aggregates")]
        for asset in assets:
            indexed = self.last_price(asset)["price"]
            onchain = lastprice(asset)
            if onchain != indexed:
                errors.append(f"{asset}: indexed {indexed} != on-chain {onchain}")
        return errors


def main(argv: Optional[Iterable[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("events", help="JSONL event export")
    p.add_argument("--db", default="oracle-index.db")
    p.add_argument("--allow-gaps", action="store_true")
    p.add_argument(
        "--contract-id",
        default=None,
        help="pin the oracle contract id (default: the id of the first accepted event)",
    )
    args = p.parse_args(argv)
    policy = TrustPolicy(expected_contract_id=args.contract_id)
    idx = Indexer(args.db, policy=policy)
    try:
        n = idx.ingest(args.events, allow_gaps=args.allow_gaps)
    except EventAuthenticityError as error:
        print(f"REJECTED {error.to_dict()}")
        return 2
    except GapError as error:
        print(f"GAP {error}; cursor={idx.cursor}; trusted={idx.trusted_ledger}")
        return 2
    print(
        f"indexed {n} new events; contract={idx.contract_id}; cursor={idx.cursor}; "
        f"trusted={idx.trusted_ledger}; gaps={idx.gaps()}; reorgs={len(idx.reorgs())}"
    )
    bad = idx.mismatches()
    for m in bad:
        print(f"MISMATCH {m}")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
