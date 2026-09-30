"""Tests for the price-revision frequency and correction-rate metrics (#499)."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from services.common.events import TOPIC_PRICE_CORRECTED, TOPIC_PRICE_AGGREGATED
from services.common.events import TOPIC_PRICE_SUBMITTED
from services.revision_metrics.metrics import (
    BASELINE_RATE_FLOOR_PERMILLE,
    DEFAULT_MIN_PUBLICATIONS,
    RevisionCause,
    Thresholds,
    classify,
    compute_metrics,
    compute_source_metrics,
    evaluate_alerts,
    normalized_rate,
    reconcile,
)
from services.revision_metrics.metrics import main as metrics_main

ASSET_A = "CASSETA"
ASSET_B = "CASSETB"
ADMIN = "GADMIN"
DELEGATE = "GDELEGATE"
START_TS = 1_700_000_000
HOUR = 3_600


def _pub(asset, ts, price=100, ledger=1, sources=("S1", "S2", "S3")):
    return {
        "ledger": ledger,
        "timestamp": ts,
        "contract_id": "CORACLE",
        "topic": TOPIC_PRICE_AGGREGATED,
        "data": {"asset": asset, "price": price, "num_sources": len(sources)},
    }


def _sub(asset, source, ts, price=100, ledger=1):
    return {
        "ledger": ledger,
        "timestamp": ts,
        "contract_id": "CORACLE",
        "topic": TOPIC_PRICE_SUBMITTED,
        "data": {"asset": asset, "source": source, "price": price},
    }


def _rev(asset, ts, actor=ADMIN, source=None, ledger=1, affects=False, reason="typo"):
    data = {
        "asset": asset,
        "actor": actor,
        "reason": reason,
        "old_price": 100,
        "new_price": 101,
        "revision_index": 1,
        "affects_downstream": affects,
    }
    if source:
        data["source"] = source
    return {
        "ledger": ledger,
        "timestamp": ts,
        "contract_id": "CORACLE",
        "topic": TOPIC_PRICE_CORRECTED,
        "data": data,
    }


def _stream(n_rounds=200, interval=60, start_ts=START_TS, sources=("S1", "S2", "S3")):
    """n_rounds of publications plus per-source submissions."""
    events = []
    for i in range(n_rounds):
        ts = start_ts + i * interval
        for s in sources:
            events.append(_sub(ASSET_A, s, ts, ledger=i))
        events.append(_pub(ASSET_A, ts, ledger=i))
    return events


# -- cause separation -------------------------------------------------------


def test_authorized_actor_and_source_and_unattributed_are_separate_causes():
    auth = classify(_rev(ASSET_A, 0, actor=ADMIN) and _ev(ADMIN), (ADMIN,))
    assert auth is RevisionCause.AUTHORIZED_CORRECTION
    assert classify(_ev(DELEGATE, source="S2"), (ADMIN,)) is RevisionCause.SOURCE_CORRECTION
    assert classify(_ev(DELEGATE), (ADMIN,)) is RevisionCause.UNATTRIBUTED


def test_authorized_actor_wins_over_source_attribution():
    # The filing path is what the operator controls, so an authorized filer is
    # never reclassified as a source fault just because a source was named.
    assert classify(_ev(ADMIN, source="S2"), (ADMIN,)) is RevisionCause.AUTHORIZED_CORRECTION


def _ev(actor, source=None):
    from services.common.events import iter_revisions

    event = next(iter(iter_revisions([_rev(ASSET_A, START_TS, actor=actor, source=source)])))
    return event


def test_every_revision_is_counted_exactly_once_in_one_cause_bucket():
    events = _stream(n_rounds=60)
    events += [
        _rev(ASSET_A, START_TS + 61 * HOUR, actor=ADMIN),
        _rev(ASSET_A, START_TS + 62 * HOUR, actor=DELEGATE, source="S2"),
        _rev(ASSET_A, START_TS + 63 * HOUR, actor=DELEGATE),
    ]
    bundle = compute_metrics(events, Thresholds(authorized_actors=(ADMIN,)))
    m = bundle["assets"][ASSET_A]
    total = sum(m.lifetime.by_cause.values())
    assert total == m.lifetime.revisions == 3
    assert m.lifetime.by_cause[RevisionCause.AUTHORIZED_CORRECTION] == 1
    assert m.lifetime.by_cause[RevisionCause.SOURCE_CORRECTION] == 1
    assert m.lifetime.by_cause[RevisionCause.UNATTRIBUTED] == 1
    assert m.unattributed == 1
    assert m.operator_corrections == 1


def test_cause_classification_is_total_and_exclusive():
    from services.common.events import iter_revisions

    actors = (ADMIN, "GSTRANGER")
    seen = []
    for actor in actors:
        for source in (None, "S1"):
            rev = next(
                iter(
                    iter_revisions(
                        [_rev(ASSET_A, START_TS, actor=actor, source=source)]
                    )
                )
            )
            seen.append(classify(rev, (ADMIN,)))
    assert len(seen) == 4
    assert set(seen) == set(RevisionCause)
    # exactly one bucket per revision: the classifier is single-valued.
    assert all(isinstance(c, RevisionCause) for c in seen)


# -- rate computation and volume normalization ------------------------------


def test_revision_rate_is_exposed_per_asset():
    events = _stream(n_rounds=100)
    events += [_pub(ASSET_B, START_TS, ledger=500)]
    events += [_rev(ASSET_A, START_TS + 99 * 60, actor=ADMIN)]
    bundle = compute_metrics(events, Thresholds(authorized_actors=(ADMIN,)))
    assert set(bundle["assets"]) == {ASSET_A, ASSET_B}
    a = bundle["assets"][ASSET_A]
    assert a.lifetime.publications == 100
    assert a.lifetime.revisions == 1
    assert a.recent_rate_permille == pytest.approx(10.0)
    # The unrelated asset has no corrections at all.
    assert bundle["assets"][ASSET_B].lifetime.revisions == 0


def test_rate_is_volume_normalized_on_a_low_volume_asset():
    # One correction on an asset that publishes twice a day must not read as
    # 100 % (1000 per-mille): the denominator is floored.
    events = [_pub(ASSET_A, START_TS, ledger=1), _pub(ASSET_A, START_TS + HOUR, ledger=2)]
    events += [_rev(ASSET_A, START_TS + 2 * HOUR, actor=ADMIN)]
    bundle = compute_metrics(events)
    a = bundle["assets"][ASSET_A]
    assert a.recent.publications == 2
    assert a.recent.low_volume is True
    assert a.recent_rate_permille == pytest.approx(1000.0 / DEFAULT_MIN_PUBLICATIONS)
    assert a.recent_rate_permille < 1000.0


def test_normalized_rate_floors_the_denominator():
    assert normalized_rate(1, 0, 20) == pytest.approx(50.0)
    assert normalized_rate(2, 100, 20) == pytest.approx(20.0)


def test_volume_normalization_makes_a_quiet_asset_incomparable_to_a_busy_one():
    # Same single correction, very different publication volume: the busy
    # asset scores a far lower rate.
    busy = _stream(n_rounds=200) + [_rev(ASSET_A, START_TS + 199 * 60, actor=ADMIN)]
    quiet = [_pub(ASSET_A, START_TS, ledger=1), _pub(ASSET_A, START_TS + HOUR, ledger=2)]
    quiet += [_rev(ASSET_A, START_TS + 2 * HOUR, actor=ADMIN)]
    busy_rate = compute_metrics(busy)["assets"][ASSET_A].recent_rate_permille
    quiet_rate = compute_metrics(quiet)["assets"][ASSET_A].recent_rate_permille
    assert busy_rate < quiet_rate


# -- rolling windows and sustained increase ---------------------------------


def test_recent_and_baseline_windows_partition_the_stream():
    # 100 hourly rounds. `now` is the end of the last one; the recent window
    # holds the last 24 h (24 publications), the baseline window the 24 h
    # before it.
    events = _stream(n_rounds=100, interval=HOUR)
    now = START_TS + 99 * HOUR
    bundle = compute_metrics(events, now=now, thresholds=Thresholds(window_secs=24 * HOUR))
    a = bundle["assets"][ASSET_A]
    assert a.lifetime.publications == 100
    assert a.recent.publications == 25  # timestamps 76..99 inclusive
    assert a.baseline.publications == 24  # timestamps 52..75
    assert a.recent.publications + a.baseline.publications + 51 == a.lifetime.publications


def test_sustained_increase_triggers_an_alert():
    # Quiet baseline day (0 corrections), then a bad day with 8 corrections.
    interval = HOUR
    events = _stream(n_rounds=48, interval=interval)
    bad_day = START_TS + 24 * HOUR
    for i in range(8):
        events.append(_rev(ASSET_A, bad_day + i * interval, actor=ADMIN, ledger=100 + i))
    bundle = compute_metrics(
        events, Thresholds(authorized_actors=(ADMIN,)), now=START_TS + 47 * interval
    )
    a = bundle["assets"][ASSET_A]
    assert a.baseline.revisions == 0
    assert a.recent.revisions == 8
    assert a.sustained_increase is True
    alerts = [x for x in evaluate_alerts(bundle) if x.scope == f"asset:{ASSET_A}"]
    assert any(x.severity == "page" for x in alerts)
    assert any("baseline" in x.reason for x in alerts)


def test_a_single_correction_is_an_incident_not_a_trend():
    events = _stream(n_rounds=48, interval=HOUR)
    events.append(_rev(ASSET_A, START_TS + 40 * HOUR, actor=ADMIN, ledger=99))
    bundle = compute_metrics(
        events, Thresholds(authorized_actors=(ADMIN,)), now=START_TS + 47 * HOUR
    )
    a = bundle["assets"][ASSET_A]
    assert a.recent.revisions == 1
    # One correction still moves the rate a long way, but it must not alert.
    assert a.sustained_increase is False
    assert not [x for x in evaluate_alerts(bundle) if x.severity == "page"]


def test_low_volume_recent_window_cannot_alert_even_with_many_corrections():
    # A rarely-updated asset: two publications in the recent window, and every
    # one of them followed by a correction. The raw ratio is 300 %, and it must
    # still not alert — the volume is too small to mean anything.
    events = [
        _pub(ASSET_A, START_TS + 40 * HOUR, ledger=1),
        _pub(ASSET_A, START_TS + 47 * HOUR, ledger=2),
    ]
    for i in range(6):
        events.append(
            _rev(ASSET_A, START_TS + 40 * HOUR + i, actor=ADMIN, ledger=100 + i)
        )
    now = START_TS + 47 * HOUR
    bundle = compute_metrics(
        events, Thresholds(authorized_actors=(ADMIN,)), now=now
    )
    a = bundle["assets"][ASSET_A]
    assert a.recent.revisions == 6
    assert a.recent.publications == 2
    assert a.recent.low_volume is True
    assert a.sustained_increase is False
    assert not [x for x in evaluate_alerts(bundle) if x.severity == "page"]


def test_unattributed_revisions_alert_even_without_a_rate_increase():
    events = _stream(n_rounds=48, interval=HOUR)
    for i in range(4):
        events.append(
            _rev(ASSET_A, START_TS + 40 * HOUR + i, actor="GSTRANGER", ledger=100 + i)
        )
    bundle = compute_metrics(events, now=START_TS + 47 * HOUR)
    alerts = [x for x in evaluate_alerts(bundle) if x.scope == f"asset:{ASSET_A}"]
    assert any(x.severity == "warning" for x in alerts)
    assert any("attributed" in x.reason for x in alerts)


# -- per-source metrics -----------------------------------------------------


def test_source_metrics_use_that_sources_own_submissions_as_the_denominator():
    events = _stream(n_rounds=100)
    events += [_rev(ASSET_A, START_TS + 95 * 60, actor=DELEGATE, source="S2", ledger=300)]
    bundle = compute_metrics(events)
    s2 = bundle["sources"]["S2"]
    assert s2.lifetime.publications == 100  # its own submissions
    assert s2.lifetime.revisions == 1
    assert s2.by_asset == {ASSET_A: 1}
    # A source with no attributed corrections scores zero.
    assert bundle["sources"]["S1"].lifetime.revisions == 0


def test_source_regression_raises_a_source_scoped_alert():
    interval = HOUR
    events = _stream(n_rounds=48, interval=interval)
    for i in range(5):
        events.append(
            _rev(
                ASSET_A,
                START_TS + 30 * HOUR + i * interval,
                actor="S2_FEED",
                source="S2",
                ledger=500 + i,
            )
        )
    bundle = compute_metrics(events, now=START_TS + 47 * interval)
    s2 = bundle["sources"]["S2"]
    assert s2.sustained_increase is True
    assert any(x.scope == "source:S2" for x in evaluate_alerts(bundle))


def test_source_metrics_ignore_sources_they_do_not_cover():
    from services.common.events import iter_revisions, iter_submissions

    events = _stream(n_rounds=10)
    submissions = list(iter_submissions(events))
    revisions = list(iter_revisions([_rev(ASSET_A, START_TS, source="S9", ledger=1)]))
    m = compute_source_metrics("S9", submissions, revisions, Thresholds(), START_TS)
    assert m.lifetime.publications == 0  # S9 never submitted
    assert m.lifetime.revisions == 1
    assert m.recent.low_volume is True


# -- observability: the metric cannot be gamed ------------------------------


def test_reconcile_is_silent_when_the_stream_matches_the_chain():
    events = _stream(n_rounds=30)
    events.append(_rev(ASSET_A, START_TS + 31 * 60, actor=ADMIN, ledger=99))
    bundle = compute_metrics(events)
    # chain length = original (index 0) + 1 correction.
    assert reconcile(bundle, {ASSET_A: 2}) == []


def test_suppressed_corrections_show_up_as_a_reconciliation_gap():
    events = _stream(n_rounds=30)
    # A correction happened on chain (chain length 4 => 3 corrections) but the
    # indexed stream only carries one of them.
    events.append(_rev(ASSET_A, START_TS + 31 * 60, actor=ADMIN, ledger=99))
    bundle = compute_metrics(events)
    gaps = reconcile(bundle, {ASSET_A: 4})
    assert len(gaps) == 1
    assert "3 correction(s)" in gaps[0]
    assert "shows 1" in gaps[0]


def test_extra_corrections_in_the_stream_also_gap():
    events = _stream(n_rounds=30)
    events.append(_rev(ASSET_A, START_TS + 31 * 60, actor=ADMIN, ledger=99))
    bundle = compute_metrics(events)
    gaps = reconcile(bundle, {ASSET_A: 1})
    assert gaps and "shows 1" in gaps[0]


def test_metrics_are_pure_functions_of_the_event_stream():
    events = _stream(n_rounds=40)
    events.append(_rev(ASSET_A, START_TS + 41 * 60, actor=ADMIN, ledger=99))
    thresholds = Thresholds(authorized_actors=(ADMIN,))
    first = compute_metrics(events, thresholds=thresholds)["assets"][ASSET_A].to_dict()
    shuffled = list(reversed(events))
    second = compute_metrics(shuffled, thresholds=thresholds)["assets"][ASSET_A].to_dict()
    assert first == second


def test_metric_reads_only_events_and_never_suppresses_a_correction():
    # The metric is observational: adding corrections can only raise the rate.
    events = _stream(n_rounds=40)
    base = compute_metrics(events)["assets"][ASSET_A].lifetime.revisions
    events.append(_rev(ASSET_A, START_TS + 41 * 60, actor=ADMIN, ledger=99))
    after = compute_metrics(events)["assets"][ASSET_A].lifetime.revisions
    assert after == base + 1


def test_baseline_floor_stops_a_zero_baseline_reading_as_infinite():
    events = _stream(n_rounds=10)
    events.append(_rev(ASSET_A, START_TS + 20 * 60, actor=ADMIN, ledger=99))
    bundle = compute_metrics(events)
    a = bundle["assets"][ASSET_A]
    assert a.baseline_rate_permille == 0.0
    assert a.trend_multiplier == pytest.approx(
        a.recent_rate_permille / BASELINE_RATE_FLOOR_PERMILLE
    )
    assert a.trend_multiplier < 1000.0


# -- thresholds and CLI -----------------------------------------------------


def test_invalid_thresholds_are_rejected():
    with pytest.raises(ValueError):
        compute_metrics(_stream(), Thresholds(window_secs=0))
    with pytest.raises(ValueError):
        compute_metrics(_stream(), Thresholds(sustained_multiplier=0.5))


def test_cli_emits_a_report_and_fails_on_a_reconciliation_gap(tmp_path: Path, capsys):
    events = _stream(n_rounds=30) + [_rev(ASSET_A, START_TS + 31 * 60, actor=ADMIN)]
    path = tmp_path / "events.jsonl"
    path.write_text("\n".join(json.dumps(e) for e in events), encoding="utf-8")

    assert metrics_main([str(path), "--authorized-actor", ADMIN]) == 0
    report = json.loads(capsys.readouterr().out)
    assert report["assets"][ASSET_A]["lifetime"]["revisions"] == 1
    assert report["reconciliation_gaps"] == []

    # An inflated chain length means two corrections never reached the metric.
    assert (
        metrics_main(
            [str(path), "--authorized-actor", ADMIN, "--on-chain-revisions", '{"%s": 3}' % ASSET_A]
        )
        == 1
    )
    assert json.loads(capsys.readouterr().out)["reconciliation_gaps"]
