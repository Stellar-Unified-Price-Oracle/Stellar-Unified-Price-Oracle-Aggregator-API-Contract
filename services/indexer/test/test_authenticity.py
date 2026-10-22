"""Tests for indexer event authenticity and gap-safe cursors (#685)."""

import json

import pytest

from services.indexer.authenticity import (
    AuthenticityCode,
    EventAuthenticityError,
    TrustPolicy,
    verify_event,
)
from services.indexer.explorer import route
from services.indexer.indexer import GapError, Indexer

C = "CORACLE"
OTHER = "CSPOOFEDCONTRACT"


def ev(ledger, topic, data, h=None, ts=None, contract=C):
    return {
        "ledger": ledger,
        "timestamp": ts or ledger * 5,
        "contract_id": contract,
        "topic": topic,
        "data": data,
        "ledger_hash": h or f"h{ledger}",
    }


def boot(contract=C):
    """Ledgers 1..2: three sources, one aggregate."""
    return [
        ev(1, "source_added", {"source": "S1", "name": "a"}, contract=contract),
        ev(1, "source_added", {"source": "S2", "name": "b"}, contract=contract),
        ev(2, "price_submitted", {"asset": "XLM", "source": "S1", "price": 100}, contract=contract),
        ev(2, "price_submitted", {"asset": "XLM", "source": "S2", "price": 120}, contract=contract),
        ev(2, "price_aggregated", {"asset": "XLM", "price": 110, "num_sources": 2}, contract=contract),
    ]


def later(ledger):
    return [ev(ledger, "price_submitted", {"asset": "XLM", "source": "S1", "price": 130})]


# -- contract identity -----------------------------------------------------


def test_spoofed_contract_event_is_rejected_without_touching_the_store():
    idx = Indexer()
    idx.ingest(boot())
    cursor, trusted, history = idx.cursor, idx.trusted_ledger, idx.history("XLM")
    assert idx.contract_id == C

    with pytest.raises(EventAuthenticityError) as excinfo:
        idx.ingest([ev(3, "price_aggregated", {"asset": "XLM", "price": 1}, contract=OTHER)])

    assert excinfo.value.code is AuthenticityCode.CONTRACT_MISMATCH
    # Nothing advanced: not the cursor, not the trust point, not the history.
    assert idx.cursor == cursor == 2
    assert idx.trusted_ledger == trusted == 2
    assert idx.history("XLM") == history
    assert idx.db.execute("SELECT COUNT(*) FROM events WHERE ledger = 3").fetchone()[0] == 0
    assert [r["code"] for r in idx.rejections()] == ["contract_mismatch"]


def test_pinned_contract_survives_a_restart(tmp_path):
    db = str(tmp_path / "i.db")
    Indexer(db).ingest(boot())
    reopened = Indexer(db)
    assert reopened.contract_id == C
    with pytest.raises(EventAuthenticityError):
        reopened.ingest([ev(3, "price_aggregated", {"asset": "XLM", "price": 1}, contract=OTHER)])


def test_configured_contract_is_enforced_from_the_first_event():
    idx = Indexer(policy=TrustPolicy(expected_contract_id="CEXPECTED"))
    with pytest.raises(EventAuthenticityError) as excinfo:
        idx.ingest(boot())
    assert excinfo.value.code is AuthenticityCode.CONTRACT_MISMATCH
    assert idx.cursor is None and idx.trusted_ledger is None


def test_unknown_topic_is_rejected_when_the_policy_declares_topics():
    policy = TrustPolicy(allowed_topics=("price_submitted", "price_aggregated"))
    idx = Indexer(policy=policy)
    with pytest.raises(EventAuthenticityError) as excinfo:
        idx.ingest([ev(1, "admin_changed", {"admin": "GX"})])
    assert excinfo.value.code is AuthenticityCode.UNKNOWN_TOPIC
    assert idx.rejections()[0]["code"] == "unknown_topic"


# -- ledger continuity -----------------------------------------------------


def test_missing_ledgers_fail_closed_and_do_not_advance_the_cursor():
    idx = Indexer()
    idx.ingest(boot())
    with pytest.raises(GapError):
        idx.ingest(later(5))  # 3 and 4 never arrived

    assert idx.cursor == 2
    assert idx.trusted_ledger == 2
    assert idx.gaps() == [(3, 4)]
    assert idx.db.execute("SELECT COUNT(*) FROM events WHERE ledger = 5").fetchone()[0] == 0


def test_allowed_gap_records_the_interval_but_never_advances_trust():
    idx = Indexer()
    idx.ingest(boot())
    assert idx.ingest(later(5), allow_gaps=True) == 1

    # The resume point moved; the trustworthy history did not.
    assert idx.cursor == 5
    assert idx.trusted_ledger == 2
    assert idx.gaps() == [(3, 4)]


def test_replaying_missing_ledgers_closes_the_gap_and_restores_trust():
    idx = Indexer()
    idx.ingest(boot())
    idx.ingest(later(5), allow_gaps=True)

    idx.ingest(later(3) + later(4), allow_gaps=True)
    assert idx.gaps() == []
    assert idx.trusted_ledger == 5


def test_replaying_one_ledger_splits_the_gap_instead_of_swallowing_it():
    idx = Indexer()
    idx.ingest(boot())
    idx.ingest(later(6), allow_gaps=True)  # gap 3..5
    assert idx.gaps() == [(3, 5)]

    idx.ingest(later(4), allow_gaps=True)
    assert idx.gaps() == [(3, 3), (5, 5)]
    assert idx.trusted_ledger == 2  # still a hole before ledger 4


def test_tampered_close_time_is_rejected():
    idx = Indexer()
    idx.ingest(boot())
    with pytest.raises(EventAuthenticityError) as excinfo:
        idx.ingest([ev(3, "price_submitted", {"asset": "XLM", "source": "S1", "price": 130}, ts=1)])
    assert excinfo.value.code is AuthenticityCode.LEDGER_TIME_REGRESSION
    assert idx.cursor == 2


def test_ledger_below_the_configured_floor_is_rejected():
    idx = Indexer(policy=TrustPolicy(min_ledger=10))
    with pytest.raises(EventAuthenticityError) as excinfo:
        idx.ingest([ev(5, "price_submitted", {"asset": "XLM", "source": "S1", "price": 1})])
    assert excinfo.value.code is AuthenticityCode.LEDGER_RANGE


# -- reorgs ----------------------------------------------------------------


def test_reorg_is_rolled_back_and_surfaced():
    idx = Indexer()
    idx.ingest(boot() + later(3))
    forked = [
        ev(3, "price_submitted", {"asset": "XLM", "source": "S1", "price": 999}, h="fork3"),
    ]
    idx.ingest(forked)
    assert [r["ledger"] for r in idx.reorgs()] == [3]
    assert idx.reorgs()[0]["new_hash"] == "fork3"
    assert idx.last_price("XLM")["price"] == 110


def test_conflicting_hashes_inside_one_batch_are_rejected():
    idx = Indexer()
    batch = [
        ev(1, "source_added", {"source": "S1", "name": "a"}, h="h1"),
        ev(1, "source_added", {"source": "S2", "name": "b"}, h="different-h1"),
    ]
    with pytest.raises(EventAuthenticityError) as excinfo:
        idx.ingest(batch)
    assert excinfo.value.code is AuthenticityCode.LEDGER_HASH_INCONSISTENT
    assert idx.rejections()[-1]["code"] == "ledger_hash_inconsistent"


def test_hash_is_required_when_the_policy_says_so():
    policy = TrustPolicy(require_ledger_hash=True)
    idx = Indexer(policy=policy)
    row = ev(1, "source_added", {"source": "S1"})
    del row["ledger_hash"]
    with pytest.raises(EventAuthenticityError) as excinfo:
        idx.ingest([row])
    assert excinfo.value.code is AuthenticityCode.LEDGER_HASH_MISSING


# -- verify_event in isolation --------------------------------------------


@pytest.mark.parametrize(
    "row",
    [
        "not-an-object",
        {"ledger": "1", "timestamp": 5, "contract_id": C, "topic": "t", "data": {}},
        {"ledger": 1, "timestamp": 0, "contract_id": C, "topic": "t", "data": {}},
        {"ledger": 1, "timestamp": 5, "contract_id": "", "topic": "t", "data": {}},
        {"ledger": 1, "timestamp": 5, "contract_id": C, "topic": "", "data": {}},
        {"ledger": 1, "timestamp": 5, "contract_id": C, "topic": "t", "data": []},
        {"ledger": True, "timestamp": 5, "contract_id": C, "topic": "t", "data": {}},
    ],
)
def test_malformed_events_are_rejected(row):
    with pytest.raises(EventAuthenticityError) as excinfo:
        verify_event(row, TrustPolicy(), pinned_contract_id=C)
    assert excinfo.value.code in {
        AuthenticityCode.MALFORMED_EVENT,
        AuthenticityCode.LEDGER_RANGE,
    }


def test_verified_event_coerces_fields_once():
    event = verify_event(ev(7, "source_added", {"source": "S1"}), TrustPolicy(), event_index=0)
    assert (event.ledger, event.event_index, event.ledger_hash) == (7, 0, "h7")
    assert event.topic == "source_added" and event.contract_id == C


# -- observability ---------------------------------------------------------


def test_explorer_exposes_rejections_and_the_trusted_ledger():
    idx = Indexer()
    idx.ingest(boot())
    with pytest.raises(EventAuthenticityError):
        idx.ingest([ev(3, "price_aggregated", {"asset": "XLM", "price": 1}, contract=OTHER)])
    with pytest.raises(GapError):
        idx.ingest(later(9))

    code, _, body = route(idx, "/api/rejections", {})
    assert code == 200 and '"contract_mismatch"' in body
    code, _, body = route(idx, "/api/gaps", {})
    assert code == 200 and json.loads(body) == [[3, 8]]
    code, _, page = route(idx, "/", {})
    assert code == 200 and "Trusted ledger:</b> 2" in page
