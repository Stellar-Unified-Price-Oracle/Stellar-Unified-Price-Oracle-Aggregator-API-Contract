import pytest

from services.indexer.explorer import route
from services.indexer.indexer import GapError, Indexer, median

C = "CORACLE"


def ev(ledger, topic, data, h=None, ts=None):
    return {"ledger": ledger, "timestamp": ts or ledger * 5, "contract_id": C,
            "topic": topic, "data": data, "ledger_hash": h or f"h{ledger}"}


def stream():
    return [
        ev(1, "source_added", {"source": "S1", "name": "a"}),
        ev(1, "source_added", {"source": "S2", "name": "b"}),
        ev(1, "source_added", {"source": "S3", "name": "c"}),
        ev(2, "price_submitted", {"asset": "XLM", "source": "S1", "price": 100}),
        ev(2, "price_submitted", {"asset": "XLM", "source": "S2", "price": 110}),
        ev(2, "price_submitted", {"asset": "XLM", "source": "S3", "price": 130}),
        ev(2, "price_aggregated", {"asset": "XLM", "price": 110, "num_sources": 3}),
        ev(3, "source_removed", {"source": "S3"}),
        ev(4, "price_submitted", {"asset": "XLM", "source": "S1", "price": 101}),
        ev(4, "price_aggregated", {"asset": "XLM", "price": 105, "num_sources": 2}),
    ]


def test_median_matches_contract_rounding():
    assert median([1, 2, 3]) == 2
    assert median([1, 4]) == 2
    assert median([-5, -2]) == -4  # -5 + trunc(3/2)


def test_reproduces_onchain_aggregates():
    idx = Indexer()
    idx.ingest(stream())
    assert idx.mismatches() == []
    assert idx.last_price("XLM")["price"] == 105
    chain = {"XLM": 105}
    assert idx.verify_against_chain(chain.get) == []
    assert idx.verify_against_chain({"XLM": 1}.get)


def test_replay_is_idempotent():
    idx = Indexer()
    assert idx.ingest(stream()) == 10
    snapshot = idx.history("XLM")
    assert idx.ingest(stream()) == 0
    assert idx.ingest(stream()[:5]) == 0
    assert idx.history("XLM") == snapshot


def test_restart_resumes_from_disk(tmp_path):
    db = str(tmp_path / "i.db")
    Indexer(db).ingest(stream()[:6])
    idx = Indexer(db)
    assert idx.cursor == 2
    idx.ingest(stream())  # replay from the start after restart
    assert idx.mismatches() == [] and len(idx.history("XLM")) == 2


def test_gap_detected_then_closed():
    s = stream()
    idx = Indexer()
    idx.ingest(s[:7])
    with pytest.raises(GapError):
        idx.ingest(s[8:])  # skips ledger 3
    assert idx.gaps() == [(3, 3)]
    idx.ingest(s[7:])
    assert idx.gaps() == [] and idx.mismatches() == []


def test_reorg_rolls_back_and_reindexes():
    idx = Indexer()
    idx.ingest(stream())
    forked = [ev(4, "price_submitted", {"asset": "XLM", "source": "S1", "price": 120}, h="fork4"),
              ev(4, "price_aggregated", {"asset": "XLM", "price": 115, "num_sources": 2}, h="fork4")]
    idx.ingest(forked)
    assert idx.last_price("XLM")["price"] == 115
    assert idx.mismatches() == []


def test_explorer_exposes_history_and_provenance():
    idx = Indexer()
    idx.ingest(stream())
    code, _, body = route(idx, "/api/provenance", {"asset": ["XLM"], "ledger": ["4"], "event_index": ["1"]})
    assert code == 200 and '"S1"' in body and '"S3"' not in body
    code, _, page = route(idx, "/", {"asset": ["XLM"]})
    assert code == 200 and "105" in page
