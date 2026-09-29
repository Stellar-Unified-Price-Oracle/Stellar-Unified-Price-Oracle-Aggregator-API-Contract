from __future__ import annotations

from services.common.events import iter_submissions
from services.common.synthetic import generate_submissions
from services.reliability_score.scorer import (
    ScoreWeights,
    compute_scores,
    derive_aggregations,
    get_source_score,
)


def _score_synthetic(**kwargs):
    events = generate_submissions(**kwargs)
    submissions = list(iter_submissions(events))
    aggregations = derive_aggregations(submissions)
    return compute_scores(submissions, aggregations)


def test_perfectly_reliable_source_scores_near_100_on_every_dimension():
    scores = _score_synthetic(
        n_rounds=100, sources=("PERFECT",), noise_bps=0.0, seed=1, interval_secs=60
    )
    score = get_source_score(scores, asset=scores[0].asset, source="PERFECT")
    assert score is not None
    assert score.uptime_score == 100.0
    assert score.accuracy_score == 100.0
    assert score.composite_score == 100.0


def test_source_with_downtime_gets_proportionally_lower_uptime_score():
    missing_rounds = set(range(0, 100, 2))  # misses exactly half the rounds
    scores = _score_synthetic(
        n_rounds=100,
        sources=("FLAKY", "STABLE"),
        noise_bps=1.0,
        seed=2,
        missing={"FLAKY": missing_rounds},
    )
    flaky = get_source_score(scores, scores[0].asset, "FLAKY")
    stable = get_source_score(scores, scores[0].asset, "STABLE")

    assert flaky.rounds_participated == 50
    assert flaky.uptime_score == 50.0
    assert stable.uptime_score == 100.0
    assert flaky.composite_score < stable.composite_score


def test_inaccurate_source_scores_lower_than_accurate_source():
    scores = _score_synthetic(
        n_rounds=100,
        sources=("ACCURATE", "SLOPPY", "SLOPPY2"),
        noise_bps=1.0,
        seed=3,
        anomalies={i: {"SLOPPY": 2000.0} for i in range(100)},
    )
    accurate = get_source_score(scores, scores[0].asset, "ACCURATE")
    sloppy = get_source_score(scores, scores[0].asset, "SLOPPY")

    assert accurate.accuracy_score > sloppy.accuracy_score
    assert accurate.composite_score > sloppy.composite_score


def test_slow_submitter_gets_lower_freshness_than_fast_submitter():
    # Build submissions manually so we control lag precisely: FAST submits
    # right at the round timestamp, SLOW submits 250s late each round.
    from services.common.events import SubmissionEvent, AggregationEvent

    submissions = []
    aggregations = []
    for ledger in range(1, 21):
        ts = 1_000_000 + ledger * 60
        submissions.append(SubmissionEvent(ledger, ts, "C", "ASSET", "FAST", 100_000))
        submissions.append(SubmissionEvent(ledger, ts - 250, "C", "ASSET", "SLOW", 100_000))
        aggregations.append(AggregationEvent(ledger, ts, "C", "ASSET", 100_000, 2))

    scores = compute_scores(submissions, aggregations, max_lag_secs=300.0)
    fast = get_source_score(scores, "ASSET", "FAST")
    slow = get_source_score(scores, "ASSET", "SLOW")

    assert fast.freshness_score == 100.0
    assert slow.freshness_score < fast.freshness_score
    assert slow.freshness_score > 0.0  # 250s lag < 300s cap, so not fully zeroed


def test_scores_are_reproducible_for_the_same_input():
    events = generate_submissions(n_rounds=50, sources=("A", "B", "C"), seed=99)
    submissions = list(iter_submissions(events))
    aggregations = derive_aggregations(submissions)

    first = compute_scores(submissions, aggregations)
    second = compute_scores(submissions, aggregations)
    assert first == second


def test_weights_must_sum_to_one():
    import pytest

    with pytest.raises(ValueError):
        ScoreWeights(uptime=0.5, freshness=0.5, accuracy=0.5)


def test_get_source_score_returns_none_for_unknown_source():
    scores = _score_synthetic(n_rounds=10, sources=("A",), seed=4)
    assert get_source_score(scores, scores[0].asset, "NOBODY") is None
