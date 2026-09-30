"""Reference indexer for oracle contract events (#539).

Consumes the canonical event envelope (see services/common/events.py and
docs/event-streaming/README.md) and materializes a SQLite store of sources,
raw submissions and aggregates that can be queried for history and provenance.

Guarantees (docs/indexer.md):

* Idempotent: every event is keyed by ``(ledger, event_index)``; replaying an
  already-indexed range is a no-op, so restarts can always resume from an
  earlier cursor.
* Gap-aware: ledgers must arrive contiguously. A jump past ``cursor + 1`` is
  recorded in the ``gaps`` table and ``ingest`` raises ``GapError`` unless
  ``allow_gaps=True``; the gap is closed when the missing ledgers are replayed.
* Reorg-safe: if a ledger arrives with a ``ledger_hash`` different from the one
  already stored, everything from that ledger onwards is rolled back and
  re-indexed from the new events.

Usage:
    python -m services.indexer.indexer events.jsonl --db oracle.db
"""
from __future__ import annotations

import argparse
import json
import sqlite3
from typing import Iterable, Optional

from services.common.events import EventSource, _iter_raw

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
    def __init__(self, db_path: str = ":memory:"):
        self.db = sqlite3.connect(db_path)
        self.db.executescript(SCHEMA)

    # -- cursor ---------------------------------------------------------
    @property
    def cursor(self) -> Optional[int]:
        row = self.db.execute("SELECT ledger FROM cursor WHERE id = 0").fetchone()
        return row[0] if row else None

    def _set_cursor(self, ledger: Optional[int]) -> None:
        if ledger is None:
            self.db.execute("DELETE FROM cursor")
        else:
            self.db.execute("INSERT OR REPLACE INTO cursor (id, ledger) VALUES (0, ?)", (ledger,))

    # -- ingestion ------------------------------------------------------
    def ingest(self, source: EventSource, allow_gaps: bool = False) -> int:
        """Ingests events in ledger order. Returns the number of new events."""
        added = 0
        counters: dict[int, int] = {}
        for row in _iter_raw(source):
            ledger = int(row["ledger"])
            idx = row.get("event_index")
            if idx is None:
                idx = counters.get(ledger, 0)
            counters[ledger] = int(idx) + 1
            added += self._ingest_one(row, ledger, int(idx), allow_gaps)
        self.db.commit()
        return added

    def _ingest_one(self, row: dict, ledger: int, idx: int, allow_gaps: bool) -> int:
        lhash = row.get("ledger_hash")
        stored = self.db.execute(
            "SELECT ledger_hash FROM events WHERE ledger = ? LIMIT 1", (ledger,)
        ).fetchone()
        if stored and lhash and stored[0] and stored[0] != lhash:
            self.rollback_to(ledger - 1)
        if self.db.execute(
            "SELECT 1 FROM events WHERE ledger = ? AND event_index = ?", (ledger, idx)
        ).fetchone():
            return 0  # already indexed: replay is a no-op

        cur = self.cursor
        if cur is not None and ledger > cur + 1:
            self.db.execute("INSERT OR IGNORE INTO gaps VALUES (?, ?)", (cur + 1, ledger - 1))
            if not allow_gaps:
                self.db.commit()
                raise GapError(f"missing ledgers {cur + 1}..{ledger - 1}")
        # Replaying earlier ledgers closes any gap they fall into.
        self.db.execute(
            "DELETE FROM gaps WHERE from_ledger <= ? AND to_ledger >= ?", (ledger, ledger)
        )

        data = dict(row["data"])
        ts = int(row["timestamp"])
        topic = str(row["topic"])
        self.db.execute(
            "INSERT INTO events VALUES (?, ?, ?, ?, ?, ?, ?)",
            (ledger, idx, lhash, ts, str(row["contract_id"]), topic, json.dumps(data, sort_keys=True)),
        )
        self._materialize(topic, data, ledger, idx, ts)
        if cur is None or ledger > cur:
            self._set_cursor(ledger)
        return 1

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
    args = p.parse_args(argv)
    idx = Indexer(args.db)
    n = idx.ingest(args.events, allow_gaps=args.allow_gaps)
    print(f"indexed {n} new events; cursor={idx.cursor}; gaps={idx.gaps()}")
    bad = idx.mismatches()
    for m in bad:
        print(f"MISMATCH {m}")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
