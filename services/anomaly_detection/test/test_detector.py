from __future__ import annotations

from services.anomaly_detection.detector import (
    DetectorConfig,
    StreamingDetector,
    detect_anomalies,
    isolation_forest_outliers,
    score_round,
)
from services.common.events import iter_submissions
from services.common.synthetic import generate_submissions


def test_score_round_flags_the_seeded_outlier_and_spares_normal_peers():
    submissions = list(
        iter_submissions(
            generate_submissions(
                n_rounds=1,
                sources=("A", "B", "C", "D"),
                noise_bps=5.0,
                seed=1,
                anomalies={0: {"C": 1000.0}},  # +10% off from the true price
            )
        )
    )
    alerts = score_round("asset", 1, submissions, DetectorConfig())

    flagged_sources = {a.source for a in alerts}
    assert flagged_sources == {"C"}
    assert alerts[0].deviation_bps > 500  # clearly off from the round median


def test_score_round_no_alerts_when_all_sources_agree():
    submissions = list(
        iter_submissions(
            generate_submissions(n_rounds=1, sources=("A", "B", "C"), noise_bps=2.0, seed=2)
        )
    )
    alerts = score_round("asset", 1, submissions, DetectorConfig())
    assert alerts == []


def test_score_round_requires_minimum_peers():
    submissions = list(
        iter_submissions(
            generate_submissions(
                n_rounds=1, sources=("A", "B"), noise_bps=5.0, seed=3, anomalies={0: {"B": 5000.0}}
            )
        )
    )
    # Only 2 peers < DEFAULT_MIN_PEERS(3) — too little history to judge, no alert.
    alerts = score_round("asset", 1, submissions, DetectorConfig(min_peers=3))
    assert alerts == []


def test_detect_anomalies_batch_finds_all_seeded_anomalies_and_nothing_else():
    events = generate_submissions(
        n_rounds=200,
        sources=("SOURCE_A", "SOURCE_B", "SOURCE_C", "SOURCE_D", "SOURCE_E"),
        noise_bps=10.0,
        seed=42,
        anomalies={
            50: {"SOURCE_B": 800.0},
            120: {"SOURCE_D": -900.0},
            150: {"SOURCE_A": 1500.0},
        },
    )
    submissions = list(iter_submissions(events))
    alerts = detect_anomalies(submissions, DetectorConfig())

    # generate_submissions starts at ledger_start=1, so round_idx r is ledger r+1.
    seeded = {(51, "SOURCE_B"), (121, "SOURCE_D"), (151, "SOURCE_A")}
    found = {(a.ledger, a.source) for a in alerts}

    assert seeded.issubset(found), f"missing seeded anomalies: {seeded - found}"
    # The min-deviation-bps gate keeps the detector from firing on ordinary
    # Gaussian noise, so clean rounds shouldn't add any false positives here.
    assert found - seeded == set(), f"unexpected extra alerts: {found - seeded}"


def test_streaming_detector_matches_batch_detection():
    events = generate_submissions(
        n_rounds=60,
        sources=("A", "B", "C", "D"),
        noise_bps=8.0,
        seed=7,
        anomalies={10: {"B": 1000.0}, 30: {"D": -1100.0}},
    )
    batch_alerts = detect_anomalies(list(iter_submissions(events)), DetectorConfig())

    detector = StreamingDetector(config=DetectorConfig(), sinks=[])
    streamed_alerts = detector.run(events)

    batch_keys = sorted((a.ledger, a.source) for a in batch_alerts)
    streamed_keys = sorted((a.ledger, a.source) for a in streamed_alerts)
    assert batch_keys == streamed_keys
    assert detector.alerts_emitted == len(streamed_alerts)
    assert detector.rounds_scored == 60


def test_streaming_detector_flushes_the_final_round():
    # A single round with no follow-up round to trigger a flush via `feed`
    # alone — `run` must still score it via the trailing `flush_all`.
    events = generate_submissions(n_rounds=1, sources=("A", "B", "C"), anomalies={0: {"C": 2000.0}})
    detector = StreamingDetector(sinks=[])
    alerts = detector.run(events)
    assert len(alerts) == 1
    assert alerts[0].source == "C"


def test_isolation_forest_outliers_returns_none_without_sklearn_or_short_history():
    # With fewer than 10 points the function short-circuits regardless of
    # whether scikit-learn is installed, keeping this test dependency-free.
    assert isolation_forest_outliers([1.0, 2.0, 3.0]) is None
