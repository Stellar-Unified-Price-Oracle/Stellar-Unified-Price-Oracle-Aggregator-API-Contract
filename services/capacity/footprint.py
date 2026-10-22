"""Storage footprint forecast per supported asset (#698).

The capacity model (``services/capacity/model.py``) projects storage from a
deployment *profile*. This module forecasts it from *measured history*
instead: it reads an indexer store, measures the per-asset event and
aggregation rates and the bytes the store actually spends per event, and
extrapolates both the contract's and the indexer's footprint over a stated
horizon.

The two stores grow differently, and the forecast keeps them apart:

* **Contract** — per asset, a fixed set of entries, a fixed number per source,
  and price history. History is *bounded*: it is pruned to
  ``min(max_history_length, max_history_per_asset)`` entries
  (``prices.rs``), and on the default temporary tier an entry that is not
  re-bumped lapses after ``TEMPORARY_RETENTION_LEDGERS``. Contract growth
  therefore saturates.
* **Indexer** — every event is kept, so the store grows linearly with the
  event rate for as long as it runs.

Every assumption is an explicit field of :class:`ContractAssumptions` or an
argument, and :func:`reconcile` checks a forecast against an observed store
within the capacity model's stated :data:`~services.capacity.model.TOLERANCE`.

    python -m services.capacity.footprint oracle-index.db --horizon-days 90

See ``docs/capacity-planning.md`` §7.
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from dataclasses import asdict, dataclass
from typing import Dict, List, Optional, Sequence

from services.capacity.model import TOLERANCE, compare_to_measured
from services.indexer.indexer import SCHEMA, Indexer

#: Stellar closes a ledger roughly every 5 seconds.
LEDGER_SECONDS = 5
LEDGERS_PER_DAY = 86_400 // LEDGER_SECONDS


@dataclass(frozen=True)
class ContractAssumptions:
    """What the contract retains per asset. Defaults mirror the contract."""

    #: ``DEFAULT_MAX_HISTORY`` (admin.rs) — global history cap.
    max_history_length: int = 100
    #: ``DEFAULT_MAX_HISTORY_PER_ASSET`` (admin.rs) — per-asset history cap.
    max_history_per_asset: int = 1_000
    #: ``TEMPORARY_RETENTION_LEDGERS`` (types.rs) — temporary-tier TTL window.
    #: ``None`` models the persistent tier, which has no protocol-level bound.
    retention_ledgers: Optional[int] = 10_000
    #: ``AssetRegistered``, ``Aggregate``, ``PriceHistoryLedgers``. Optional
    #: per-asset features (bounds, pause flags, metadata) are not modelled.
    fixed_entries_per_asset: int = 3
    #: ``Submission(asset, source)`` and ``LastSubmissionLedger(source, asset)``.
    entries_per_source: int = 2
    #: Estimated serialized key + value sizes. These are planning figures, not
    #: measured: reconciliation covers entry counts, not contract bytes.
    bytes_per_fixed_entry: int = 128
    bytes_per_source_entry: int = 96
    bytes_per_history_entry: int = 112
    #: Each retained history ledger also costs a u32 in ``PriceHistoryLedgers``.
    bytes_per_history_index: int = 4

    @property
    def history_cap(self) -> int:
        return min(self.max_history_length, self.max_history_per_asset)


@dataclass(frozen=True)
class AssetMeasurement:
    asset: str
    sources: int
    events: int
    aggregation_ledgers: int


@dataclass(frozen=True)
class Measurement:
    """What an indexer store holds, and what it costs."""

    first_ledger: int
    last_ledger: int
    events: int
    #: Bytes of an empty store (schema only).
    base_bytes: int
    store_bytes: int
    assets: tuple

    @property
    def span_ledgers(self) -> int:
        return self.last_ledger - self.first_ledger + 1

    @property
    def bytes_per_event(self) -> float:
        return (self.store_bytes - self.base_bytes) / self.events

    @property
    def other_events(self) -> int:
        """Events not attributable to an asset (source lifecycle and the like)."""
        return self.events - sum(a.events for a in self.assets)


@dataclass(frozen=True)
class AssetForecast:
    asset: str
    sources: int
    history_entries: int
    contract_entries: int
    contract_bytes: int
    indexer_events: int
    indexer_bytes: int


@dataclass(frozen=True)
class Forecast:
    horizon_ledgers: int
    assets: tuple
    contract_entries: int
    contract_bytes: int
    indexer_events: int
    indexer_bytes: int

    def to_dict(self) -> Dict[str, object]:
        d = asdict(self)
        d["horizon_days"] = round(self.horizon_ledgers / LEDGERS_PER_DAY, 2)
        return d


def store_bytes(db: sqlite3.Connection) -> int:
    """Allocated size of a SQLite store, in bytes."""
    pages = db.execute("PRAGMA page_count").fetchone()[0]
    size = db.execute("PRAGMA page_size").fetchone()[0]
    return pages * size


def _empty_store_bytes() -> int:
    db = sqlite3.connect(":memory:")
    db.executescript(SCHEMA)
    return store_bytes(db)


def _asset_rows(db: sqlite3.Connection, since_ledger: Optional[int] = None) -> List[tuple]:
    floor = since_ledger if since_ledger is not None else -1
    return db.execute(
        """SELECT a.asset,
                  (SELECT COUNT(DISTINCT source) FROM submissions WHERE asset = a.asset),
                  (SELECT COUNT(*) FROM submissions WHERE asset = a.asset)
                + (SELECT COUNT(*) FROM aggregates WHERE asset = a.asset),
                  (SELECT COUNT(DISTINCT ledger) FROM aggregates
                    WHERE asset = a.asset AND ledger > ?)
           FROM (SELECT asset FROM submissions UNION SELECT asset FROM aggregates) a
           ORDER BY a.asset""",
        (floor,),
    ).fetchall()


def measure(indexer: Indexer) -> Measurement:
    """Measures per-asset counts and the store's real bytes per event."""
    db = indexer.db
    first, last, events = db.execute(
        "SELECT MIN(ledger), MAX(ledger), COUNT(*) FROM events"
    ).fetchone()
    if not events:
        raise ValueError("cannot forecast from an empty store")
    assets = tuple(AssetMeasurement(*row) for row in _asset_rows(db))
    return Measurement(
        first_ledger=first,
        last_ledger=last,
        events=events,
        base_bytes=_empty_store_bytes(),
        store_bytes=store_bytes(db),
        assets=assets,
    )


def _contract(sources: int, history: int, a: ContractAssumptions) -> tuple:
    entries = a.fixed_entries_per_asset + a.entries_per_source * sources + history
    size = (
        a.fixed_entries_per_asset * a.bytes_per_fixed_entry
        + a.entries_per_source * sources * a.bytes_per_source_entry
        + history * (a.bytes_per_history_entry + a.bytes_per_history_index)
    )
    return entries, size


def forecast(
    m: Measurement,
    horizon_ledgers: int,
    assumptions: ContractAssumptions = ContractAssumptions(),
) -> Forecast:
    """Extrapolates measured rates ``horizon_ledgers`` past the last measured
    ledger.

    Assumptions, all explicit: per-asset event and aggregation rates stay at
    their measured averages; the source set stays as measured; asset-less
    events (source lifecycle) do not recur; and the store's bytes per event
    stays at its measured value.
    """
    if horizon_ledgers < 0:
        raise ValueError("horizon must be non-negative")
    total_ledgers = m.span_ledgers + horizon_ledgers
    window = total_ledgers
    if assumptions.retention_ledgers is not None:
        window = min(window, assumptions.retention_ledgers)

    rows: List[AssetForecast] = []
    for a in m.assets:
        events = round(a.events * total_ledgers / m.span_ledgers)
        history = min(
            assumptions.history_cap,
            round(a.aggregation_ledgers * window / m.span_ledgers),
        )
        entries, size = _contract(a.sources, history, assumptions)
        rows.append(
            AssetForecast(
                asset=a.asset,
                sources=a.sources,
                history_entries=history,
                contract_entries=entries,
                contract_bytes=size,
                indexer_events=events,
                indexer_bytes=round(events * m.bytes_per_event),
            )
        )

    indexer_events = m.other_events + sum(r.indexer_events for r in rows)
    return Forecast(
        horizon_ledgers=horizon_ledgers,
        assets=tuple(rows),
        contract_entries=sum(r.contract_entries for r in rows),
        contract_bytes=sum(r.contract_bytes for r in rows),
        indexer_events=indexer_events,
        indexer_bytes=m.base_bytes + round(indexer_events * m.bytes_per_event),
    )


def observe(
    indexer: Indexer, assumptions: ContractAssumptions = ContractAssumptions()
) -> Dict[str, object]:
    """The footprint a store actually has, in the forecast's units.

    Contract entries are what the contract retains for the indexed events:
    history is the distinct aggregation ledgers inside the retention window,
    pruned to the history cap.
    """
    db = indexer.db
    last = db.execute("SELECT MAX(ledger) FROM events").fetchone()[0] or 0
    since = None
    if assumptions.retention_ledgers is not None:
        since = last - assumptions.retention_ledgers
    entries = 0
    per_asset: Dict[str, int] = {}
    for asset, sources, _, agg_ledgers in _asset_rows(db, since):
        history = min(assumptions.history_cap, agg_ledgers)
        count, _ = _contract(sources, history, assumptions)
        per_asset[asset] = count
        entries += count
    return {
        "contract_entries": entries,
        "contract_entries_per_asset": per_asset,
        "indexer_events": db.execute("SELECT COUNT(*) FROM events").fetchone()[0],
        "indexer_bytes": store_bytes(db),
    }


def reconcile(
    fc: Forecast, observed: Dict[str, object], tolerance: float = TOLERANCE
) -> Dict[str, object]:
    """Compares a forecast with an observed store at the forecast horizon."""
    keys = ("contract_entries", "indexer_events", "indexer_bytes")
    result = compare_to_measured(
        [float(getattr(fc, k)) for k in keys],
        [float(observed[k]) for k in keys],  # type: ignore[arg-type]
        tolerance,
    )
    result["metrics"] = dict(zip(keys, result["errors"]))
    return result


def render(fc: Forecast) -> str:
    lines = [
        f"# Storage footprint forecast ({fc.to_dict()['horizon_days']} days ahead)",
        "",
        "| Asset | Sources | History | Contract entries | Contract bytes | Indexer events | Indexer bytes |",
        "|---|---|---|---|---|---|---|",
    ]
    for r in fc.assets:
        lines.append(
            f"| {r.asset} | {r.sources} | {r.history_entries} | {r.contract_entries} "
            f"| {r.contract_bytes} | {r.indexer_events} | {r.indexer_bytes} |"
        )
    lines.append(
        f"| **total** | | | {fc.contract_entries} | {fc.contract_bytes} "
        f"| {fc.indexer_events} | {fc.indexer_bytes} |"
    )
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("db", help="indexer SQLite store to measure")
    p.add_argument("--horizon-days", type=float, default=90.0)
    p.add_argument("--persistent-tier", action="store_true",
                   help="model the persistent history tier (no TTL bound)")
    p.add_argument("--json", action="store_true")
    args = p.parse_args(argv)

    assumptions = ContractAssumptions(
        retention_ledgers=None if args.persistent_tier else ContractAssumptions.retention_ledgers
    )
    fc = forecast(measure(Indexer(args.db)), round(args.horizon_days * LEDGERS_PER_DAY), assumptions)
    print(json.dumps(fc.to_dict(), indent=2) if args.json else render(fc).rstrip("\n"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
