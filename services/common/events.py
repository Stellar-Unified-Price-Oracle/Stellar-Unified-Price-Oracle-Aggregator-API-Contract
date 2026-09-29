"""Reads the canonical oracle event envelope described in
docs/event-streaming/README.md:

    {"ledger": 12345, "timestamp": 67890, "contract_id": "C...",
     "topic": "price_submitted", "data": {...}}

Shared by the anomaly-detection, volatility-forecast, and reliability-score
services so each pipeline consumes the same indexed event stream (a JSONL
export of the `oracle_events` table, or the table itself via Postgres).
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Iterator, Optional, Union

# Topics emitted by contracts/price-oracle/src/events.rs that these services
# care about (see PriceSubmittedEvent / PriceAggregatedEvent).
TOPIC_PRICE_SUBMITTED = "price_submitted"
TOPIC_PRICE_AGGREGATED = "price_aggregated"
# #499: `PriceCorrectedEvent` (contracts/price-oracle/src/corrections.rs) —
# the only on-chain path that revises an already-published aggregate.
TOPIC_PRICE_CORRECTED = "price_corrected"

EventSource = Union[str, Path, Iterable[dict]]


@dataclass(frozen=True)
class EventEnvelope:
    ledger: int
    timestamp: int
    contract_id: str
    topic: str
    data: dict

    @staticmethod
    def from_dict(row: dict) -> "EventEnvelope":
        return EventEnvelope(
            ledger=int(row["ledger"]),
            timestamp=int(row["timestamp"]),
            contract_id=str(row["contract_id"]),
            topic=str(row["topic"]),
            data=dict(row["data"]),
        )


@dataclass(frozen=True)
class SubmissionEvent:
    """A single source's raw price submission (`PriceSubmittedEvent`)."""

    ledger: int
    timestamp: int
    contract_id: str
    asset: str
    source: str
    price: int


@dataclass(frozen=True)
class AggregationEvent:
    """A newly computed aggregate price (`PriceAggregatedEvent`)."""

    ledger: int
    timestamp: int
    contract_id: str
    asset: str
    price: int
    num_sources: int


def _iter_raw(source: EventSource) -> Iterator[dict]:
    """Yields raw envelope dicts from a JSONL file path or an in-memory iterable."""
    if isinstance(source, (str, Path)):
        with open(source, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    yield json.loads(line)
    else:
        for row in source:
            yield row


def iter_envelopes(source: EventSource, topics: Optional[set] = None) -> Iterator[EventEnvelope]:
    for row in _iter_raw(source):
        env = EventEnvelope.from_dict(row)
        if topics is None or env.topic in topics:
            yield env


def iter_submissions(source: EventSource) -> Iterator[SubmissionEvent]:
    for env in iter_envelopes(source, topics={TOPIC_PRICE_SUBMITTED}):
        yield SubmissionEvent(
            ledger=env.ledger,
            timestamp=env.timestamp,
            contract_id=env.contract_id,
            asset=str(env.data["asset"]),
            source=str(env.data["source"]),
            price=int(env.data["price"]),
        )


def iter_aggregations(source: EventSource) -> Iterator[AggregationEvent]:
    for env in iter_envelopes(source, topics={TOPIC_PRICE_AGGREGATED}):
        yield AggregationEvent(
            ledger=env.ledger,
            timestamp=env.timestamp,
            contract_id=env.contract_id,
            asset=str(env.data["asset"]),
            price=int(env.data["price"]),
            num_sources=int(env.data.get("num_sources", 0)),
        )


@dataclass(frozen=True)
class RevisionEvent:
    """A revision of an already-published aggregate (`PriceCorrectedEvent`).

    Mirrors the on-chain event: the actor that filed it, the mandatory reason,
    the value before and after, the index in the immutable revision chain, and
    whether the superseded value had already been consumed downstream. The
    optional `source` names the oracle source the correction is attributed to
    when the filing pipeline could determine one (#499); it is absent for a
    correction that is not attributable to a single source.
    """

    ledger: int
    timestamp: int
    contract_id: str
    asset: str
    actor: str
    reason: str
    old_price: int
    new_price: int
    revision_index: int
    affects_downstream: bool
    source: Optional[str] = None


def iter_revisions(source: EventSource) -> Iterator[RevisionEvent]:
    for env in iter_envelopes(source, topics={TOPIC_PRICE_CORRECTED}):
        yield RevisionEvent(
            ledger=env.ledger,
            timestamp=env.timestamp,
            contract_id=env.contract_id,
            asset=str(env.data["asset"]),
            actor=str(env.data["actor"]),
            reason=str(env.data.get("reason", "")),
            old_price=int(env.data.get("old_price", 0)),
            new_price=int(env.data.get("new_price", 0)),
            revision_index=int(env.data.get("revision_index", 0)),
            affects_downstream=bool(env.data.get("affects_downstream", False)),
            source=(str(env.data["source"]) if env.data.get("source") else None),
        )


def iter_from_postgres(
    dsn: str,
    topics: Optional[set] = None,
    since_id: int = 0,
    batch_size: int = 1000,
) -> Iterator[dict]:
    """Streams raw envelope rows from the `oracle_events` table (see
    docs/event-streaming/postgresql_schema.sql), ordered by `id`.

    Requires `psycopg2` (or `psycopg2-binary`); imported lazily so the rest of
    this module has no hard dependency on it.
    """
    import psycopg2
    import psycopg2.extras

    where = "id > %s"
    if topics:
        where += " AND topic = ANY(%s)"
    query = (
        f"SELECT id, ledger, timestamp, contract_id, topic, data "
        f"FROM oracle_events WHERE {where} ORDER BY id ASC LIMIT %s"
    )

    with psycopg2.connect(dsn) as conn:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cursor_id = since_id
            while True:
                params: list[Any] = [cursor_id]
                if topics:
                    params.append(list(topics))
                params.append(batch_size)
                cur.execute(query, params)
                rows = cur.fetchall()
                if not rows:
                    return
                for row in rows:
                    cursor_id = row["id"]
                    yield dict(row)
