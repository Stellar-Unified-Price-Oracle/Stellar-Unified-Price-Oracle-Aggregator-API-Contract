"""Memory ceiling and backpressure tests for the indexer pump (#697)."""

import threading
import time
import tracemalloc

import pytest

from services.indexer.backpressure import (
    BoundedBuffer,
    IngestPump,
    event_size,
    stamp_and_resume,
)
from services.indexer.indexer import Indexer

C = "CORACLE"
SOURCES = ("S1", "S2", "S3")


def sustained_stream(ledgers, start=1):
    """Lazily yields a submission per source per ledger, with no event_index."""
    for ledger in range(start, start + ledgers):
        for i, source in enumerate(SOURCES):
            yield {"ledger": ledger, "timestamp": 1_700_000_000 + ledger * 5,
                   "contract_id": C, "topic": "price_submitted",
                   "data": {"asset": "XLM", "source": source, "price": 1_000 + ledger + i}}


def slow(indexer, delay=0.001):
    def downstream(batch):
        time.sleep(delay)
        return indexer.ingest(batch)
    return downstream


def stored_events(indexer):
    return indexer.db.execute("SELECT COUNT(*) FROM events").fetchone()[0]


# ── Normal operation ─────────────────────────────────────────────────────────

def test_sustained_stream_is_fully_indexed_within_the_buffer_ceiling():
    idx = Indexer()
    pump = IngestPump(idx, max_buffered_events=50, max_buffered_bytes=1 << 20,
                      batch_size=10, downstream=slow(idx))
    report = pump.run(sustained_stream(1_000))

    assert report.events_ingested == 3_000
    assert stored_events(idx) == 3_000
    assert idx.cursor == 1_000 and idx.gaps() == []
    assert report.high_water_events <= 50
    # The store was slower than the reader, so the reader was held back.
    assert report.producer_waits > 0
    assert not pump.producer.is_alive()


def test_byte_ceiling_is_enforced_when_it_binds_before_the_event_count():
    idx = Indexer()
    one = event_size(next(sustained_stream(1)))
    cap = one * 8
    pump = IngestPump(idx, max_buffered_events=10_000, max_buffered_bytes=cap,
                      batch_size=4, downstream=slow(idx))
    report = pump.run(sustained_stream(300))

    assert stored_events(idx) == 900
    assert report.high_water_bytes <= cap
    assert report.producer_waits > 0


def test_ledgers_split_across_batches_keep_every_event():
    # Three events per ledger, two per batch: without stable indices the second
    # half of each ledger would be renumbered from 0 and skipped as a replay.
    idx = Indexer()
    report = IngestPump(idx, max_buffered_events=4, batch_size=2).run(sustained_stream(100))
    assert report.events_ingested == 300
    assert stored_events(idx) == 300


# ── Boundaries ───────────────────────────────────────────────────────────────

def test_oversized_event_is_admitted_alone_instead_of_deadlocking():
    buffer = BoundedBuffer(max_events=10, max_bytes=1)
    row = next(sustained_stream(1))
    assert buffer.put(row)
    assert buffer.get_batch(10) == [row]
    assert buffer.high_water_events == 1


def test_limits_must_be_positive():
    with pytest.raises(ValueError):
        BoundedBuffer(0, 1)
    with pytest.raises(ValueError):
        IngestPump(Indexer(), batch_size=0)


def test_python_memory_stays_flat_as_the_stream_grows():
    def peak(ledgers):
        idx = Indexer()
        tracemalloc.start()
        IngestPump(idx, max_buffered_events=100, batch_size=25).run(sustained_stream(ledgers))
        _, high = tracemalloc.get_traced_memory()
        tracemalloc.stop()
        return high

    short, long = peak(500), peak(3_000)
    # Six times the stream must not mean six times the memory: the pump only
    # ever holds the bounded buffer and one batch.
    assert long < short * 2 + 256 * 1024


# ── Failure and recovery ─────────────────────────────────────────────────────

def test_store_failure_stops_the_reader_and_resume_loses_nothing():
    idx = Indexer()
    calls = {"n": 0}

    def crashing(batch):
        calls["n"] += 1
        if calls["n"] == 37:
            raise RuntimeError("store unavailable")
        return idx.ingest(batch)

    pump = IngestPump(idx, max_buffered_events=20, batch_size=7, downstream=crashing)
    with pytest.raises(RuntimeError, match="store unavailable"):
        pump.run(sustained_stream(400))
    assert not pump.producer.is_alive()
    committed = idx.cursor
    assert committed is not None and committed < 400

    # Restart with the same stream from the beginning: the pump resumes from
    # the cursor ledger and the replayed overlap is a no-op.
    report = IngestPump(idx, max_buffered_events=20, batch_size=7).run(sustained_stream(400))
    assert report.resumed_from == committed
    assert stored_events(idx) == 1_200
    assert idx.cursor == 400 and idx.gaps() == []


def test_resume_skips_ledgers_below_the_cursor_but_replays_the_cursor_ledger():
    rows = list(stamp_and_resume(sustained_stream(5), resume_from=3))
    assert {r["ledger"] for r in rows} == {3, 4, 5}
    assert [r["event_index"] for r in rows if r["ledger"] == 3] == [0, 1, 2]


def test_reader_failure_is_surfaced_after_committed_events():
    def broken():
        yield from sustained_stream(10)
        raise OSError("stream dropped")

    idx = Indexer()
    with pytest.raises(OSError, match="stream dropped"):
        IngestPump(idx, max_buffered_events=5, batch_size=5).run(broken())
    assert stored_events(idx) == 30
    assert idx.cursor == 10


def test_abort_wakes_a_blocked_producer():
    buffer = BoundedBuffer(max_events=1, max_bytes=1 << 20)
    rows = sustained_stream(3)
    buffer.put(next(rows))
    results = []
    t = threading.Thread(target=lambda: results.append(buffer.put(next(rows))))
    t.start()
    time.sleep(0.05)
    buffer.abort()
    t.join(timeout=2)
    assert not t.is_alive()
    assert results == [False]
