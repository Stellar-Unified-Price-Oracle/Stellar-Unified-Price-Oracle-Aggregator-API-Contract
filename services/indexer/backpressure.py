"""Bounded, backpressured ingest pump for the indexer (#697).

``Indexer.ingest`` is synchronous: when the store slows down (disk pressure,
a long reorg rollback, a slow downstream consumer), a producer that keeps
reading the event stream has to keep what it has read somewhere. Unbounded,
that buffer is how the indexer runs out of memory under a sustained stream.

The pump puts a bounded buffer between the stream and the store:

* **Memory ceiling** — the buffer holds at most ``max_buffered_events`` events
  and ``max_buffered_bytes`` bytes (each event costed at its serialized JSON
  size). A single event larger than the byte cap is admitted only into an empty
  buffer, so it can never deadlock the pump and the ceiling is exceeded by at
  most that one event.
* **Backpressure** — when the buffer is full the producer *blocks*: it stops
  reading the stream until the store has drained a batch. Nothing is dropped,
  so a slow store slows the reader instead of growing memory.
* **Safe resume** — the store's ``cursor`` is the resume point. If the store
  fails mid-stream, buffered events are discarded rather than half-written,
  the producer is stopped, and the next run replays from the cursor ledger.
  Replaying already-indexed events is a no-op (events are keyed by
  ``(ledger, event_index)``), so no event is lost or duplicated.
* **Stable event indices** — an event without an explicit ``event_index`` is
  stamped with its position in its ledger *before* it is batched.
  ``Indexer.ingest`` numbers such events per call, so a ledger split across
  two batches would otherwise reuse indices and drop the second half as
  replays.

See ``docs/indexer.md`` ("Memory ceiling and backpressure").
"""
from __future__ import annotations

import json
import threading
from collections import deque
from dataclasses import dataclass
from typing import Callable, Deque, Iterator, List, Optional, Tuple

from services.common.events import EventSource, _iter_raw
from services.indexer.indexer import Indexer

#: The store side of the pump: ingests one batch, returns events added.
Downstream = Callable[[List[dict]], int]


def event_size(row: dict) -> int:
    """Bytes an event is charged against the buffer's byte ceiling."""
    return len(json.dumps(row, separators=(",", ":"), default=str))


class BoundedBuffer:
    """Thread-safe FIFO bounded by event count and bytes."""

    def __init__(self, max_events: int, max_bytes: int) -> None:
        if max_events < 1 or max_bytes < 1:
            raise ValueError("buffer limits must be positive")
        self.max_events = max_events
        self.max_bytes = max_bytes
        self._items: Deque[Tuple[dict, int]] = deque()
        self._bytes = 0
        self._cond = threading.Condition()
        self.closed = False
        self.aborted = False
        self.high_water_events = 0
        self.high_water_bytes = 0
        #: Times the producer blocked because the buffer was full.
        self.producer_waits = 0

    def _full_for(self, size: int) -> bool:
        if not self._items:
            return False  # an empty buffer always admits one event
        return len(self._items) >= self.max_events or self._bytes + size > self.max_bytes

    def put(self, row: dict) -> bool:
        """Blocks while the buffer is full. Returns False once aborted."""
        size = event_size(row)
        with self._cond:
            if self._full_for(size) and not self.aborted:
                self.producer_waits += 1
                while self._full_for(size) and not self.aborted:
                    self._cond.wait()
            if self.aborted:
                return False
            self._items.append((row, size))
            self._bytes += size
            self.high_water_events = max(self.high_water_events, len(self._items))
            self.high_water_bytes = max(self.high_water_bytes, self._bytes)
            self._cond.notify_all()
            return True

    def get_batch(self, max_items: int) -> List[dict]:
        """Blocks until an event is available; ``[]`` once closed and drained."""
        with self._cond:
            while not self._items and not self.closed:
                self._cond.wait()
            batch: List[dict] = []
            while self._items and len(batch) < max_items:
                row, size = self._items.popleft()
                self._bytes -= size
                batch.append(row)
            self._cond.notify_all()
            return batch

    def close(self) -> None:
        with self._cond:
            self.closed = True
            self._cond.notify_all()

    def abort(self) -> None:
        """Stops the producer and discards everything still buffered."""
        with self._cond:
            self.aborted = True
            self.closed = True
            self._items.clear()
            self._bytes = 0
            self._cond.notify_all()


def stamp_and_resume(source: EventSource, resume_from: Optional[int]) -> Iterator[dict]:
    """Yields events with a stable ``event_index``, skipping ledgers below
    ``resume_from``.

    The cursor ledger itself is replayed: a batch boundary may have fallen
    inside it, and replaying what is already indexed is a no-op.

    The stream is in ledger order (the indexer requires it), so only the
    current ledger's counter is kept and memory does not grow with the stream.
    """
    current: object = None
    next_idx = 0
    for row in _iter_raw(source):
        if not isinstance(row, dict):
            yield row  # malformed: let the indexer's trust policy reject it
            continue
        ledger = row.get("ledger")
        if ledger != current:
            current, next_idx = ledger, 0
        idx = row.get("event_index")
        if idx is None:
            idx = next_idx
            row = dict(row, event_index=idx)
        if isinstance(idx, int) and not isinstance(idx, bool):
            next_idx = idx + 1
        if (
            resume_from is not None
            and isinstance(ledger, int)
            and not isinstance(ledger, bool)
            and ledger < resume_from
        ):
            continue
        yield row


@dataclass
class PumpReport:
    resumed_from: Optional[int]
    events_ingested: int
    batches: int
    high_water_events: int
    high_water_bytes: int
    producer_waits: int

    def to_dict(self) -> dict:
        return dict(self.__dict__)


class IngestPump:
    """Streams events into an :class:`Indexer` through a :class:`BoundedBuffer`.

    The producer (stream reader) runs on a background thread; the store is
    driven from the calling thread, which is the thread that owns the
    indexer's SQLite connection.
    """

    def __init__(
        self,
        indexer: Indexer,
        max_buffered_events: int = 1_000,
        max_buffered_bytes: int = 1 << 20,
        batch_size: int = 100,
        downstream: Optional[Downstream] = None,
    ) -> None:
        if batch_size < 1:
            raise ValueError("batch_size must be positive")
        self.indexer = indexer
        self.max_buffered_events = max_buffered_events
        self.max_buffered_bytes = max_buffered_bytes
        self.batch_size = batch_size
        self.downstream: Downstream = downstream or indexer.ingest
        self.producer: Optional[threading.Thread] = None
        self.producer_error: Optional[BaseException] = None

    def run(self, source: EventSource) -> PumpReport:
        """Ingests ``source`` from the indexer's cursor onwards.

        Re-raises a store failure after stopping the producer; calling ``run``
        again with the same stream resumes from the committed cursor.
        """
        buffer = BoundedBuffer(self.max_buffered_events, self.max_buffered_bytes)
        resume_from = self.indexer.cursor
        self.producer_error = None

        def produce() -> None:
            try:
                for row in stamp_and_resume(source, resume_from):
                    if not buffer.put(row):
                        return
            except BaseException as error:  # surfaced on the consumer side
                self.producer_error = error
            finally:
                buffer.close()

        self.producer = threading.Thread(target=produce, name="indexer-pump", daemon=True)
        self.producer.start()

        added = 0
        batches = 0
        try:
            while True:
                batch = buffer.get_batch(self.batch_size)
                if not batch:
                    break
                added += self.downstream(batch)
                batches += 1
        except BaseException:
            buffer.abort()
            raise
        finally:
            self.producer.join()

        if self.producer_error is not None:
            raise self.producer_error
        return PumpReport(
            resumed_from=resume_from,
            events_ingested=added,
            batches=batches,
            high_water_events=buffer.high_water_events,
            high_water_bytes=buffer.high_water_bytes,
            producer_waits=buffer.producer_waits,
        )
