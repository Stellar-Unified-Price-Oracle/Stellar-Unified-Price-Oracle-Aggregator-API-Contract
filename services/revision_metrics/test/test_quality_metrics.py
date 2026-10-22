"""Tests for comprehensive price-quality metrics (#520).

Covers freshness, disagreement, and missing-data metric calculations,
including reconciliation and fixture-based integration testing.
"""

import json
import time
from typing import List

import pytest

from services.revision_metrics.quality_metrics import (
    ComplianceClass,
    DisagreementMetrics,
    DisagreementThresholds,
    FreshnessMetrics,
    FreshnessThresholds,
    HealthStatus,
    MissingDataMetrics,
    MissingDataThresholds,
    PerSourceFreshness,
    QualityMetricsBatch,
    StalenessClass,
    compute_disagreement_metrics,
    compute_freshness_metrics,
    compute_missing_data_metrics,
    compute_quality_metrics,
)
from services.common.events import AggregationEvent, SubmissionEvent


@pytest.fixture
def fresh_events():
    """Fixture: Recent events with all submissions fresh."""
    now = int(time.time())
    return [
        json.dumps({
            "ledger": 1000,
            "timestamp": now - 30,
            "contract_id": "CONTRACT_A",
            "topic": "price_aggregated",
            "data": {
                "asset": "ASSET_A",
                "price": 100_000_000,
                "num_sources": 2,
            },
        }),
        json.dumps({
            "ledger": 999,
            "timestamp": now - 35,
            "contract_id": "CONTRACT_A",
            "topic": "price_submitted",
            "data": {
                "asset": "ASSET_A",
                "source": "SOURCE_1",
                "price": 100_000_000,
            },
        }),
        json.dumps({
            "ledger": 998,
            "timestamp": now - 40,
            "contract_id": "CONTRACT_A",
            "topic": "price_submitted",
            "data": {
                "asset": "ASSET_A",
                "source": "SOURCE_2",
                "price": 100_500_000,
            },
        }),
    ]


@pytest.fixture
def stale_events():
    """Fixture: Price is stale (beyond resolution window)."""
    now = int(time.time())
    return [
        json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,
            "num_sources": 1,
            "timestamp": now - 300,  # 5 minutes old
            "ledger": 1000,
        }),
        json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": now - 305,
            "ledger": 999,
        }),
    ]


@pytest.fixture
def critical_events():
    """Fixture: Price is critical (exceeds max staleness)."""
    now = int(time.time())
    return [
        json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,
            "num_sources": 1,
            "timestamp": now - 4000,  # Well over 1 hour
            "ledger": 1000,
        }),
        json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": now - 4005,
            "ledger": 999,
        }),
    ]


@pytest.fixture
def disagreement_events():
    """Fixture: One source significantly disagrees with aggregate."""
    now = int(time.time())
    events = []
    # Multiple aggregations at median price
    for i in range(5):
        ts = now - (i * 100)
        events.append(json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,  # Median
            "num_sources": 2,
            "timestamp": ts,
            "ledger": 1000 - i,
        }))
        # Source 1: submits at median
        events.append(json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": ts - 5,
            "ledger": 999 - i,
        }))
        # Source 2: submits significantly higher
        events.append(json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_2",
            "price": 110_000_000,  # 10% higher = 1000 bps
            "timestamp": ts - 10,
            "ledger": 998 - i,
        }))
    return events


@pytest.fixture
def missing_data_events():
    """Fixture: One source missing most submissions."""
    now = int(time.time())
    events = []
    # Source 1: submits regularly
    for i in range(20):
        ts = now - (i * 60)
        events.append(json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": ts,
            "ledger": 2000 - i,
        }))
    
    # Source 2: only 2 submissions in the same window
    for i in [0, 1]:
        ts = now - (i * 600)
        events.append(json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_2",
            "price": 100_500_000,
            "timestamp": ts,
            "ledger": 2000 - i * 10,
        }))
    
    # Aggregations for context
    for i in range(5):
        ts = now - (i * 300)
        events.append(json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_250_000,
            "num_sources": 2,
            "timestamp": ts,
            "ledger": 2000 - i * 5,
        }))
    
    return events


# --- Freshness Tests ---


def test_freshness_fresh_asset(fresh_events):
    """Asset with recent price is classified as Fresh."""
    batch = compute_quality_metrics(fresh_events)
    
    assert "ASSET_A" in batch.freshness_metrics
    metrics = batch.freshness_metrics["ASSET_A"]
    assert metrics.staleness_class == StalenessClass.FRESH
    assert metrics.time_since_last_aggregate_secs < 60


def test_freshness_stale_asset(stale_events):
    """Asset beyond resolution window is classified as Stale."""
    batch = compute_quality_metrics("\n".join(stale_events))
    
    metrics = batch.freshness_metrics["ASSET_A"]
    assert metrics.staleness_class == StalenessClass.STALE
    assert metrics.time_since_last_aggregate_secs > 60


def test_freshness_critical_asset(critical_events):
    """Asset beyond max staleness is classified as Critical."""
    batch = compute_quality_metrics("\n".join(critical_events))
    
    metrics = batch.freshness_metrics["ASSET_A"]
    assert metrics.staleness_class == StalenessClass.CRITICAL
    assert metrics.time_since_last_aggregate_secs > 3600


def test_freshness_per_source_compliant(fresh_events):
    """Sources submitting regularly are classified as Compliant."""
    batch = compute_quality_metrics("\n".join(fresh_events))
    
    metrics = batch.freshness_metrics["ASSET_A"]
    assert "SOURCE_1" in metrics.per_source
    assert metrics.per_source["SOURCE_1"].submission_compliance_class == ComplianceClass.COMPLIANT


def test_freshness_reconciliation_reports_time_distribution():
    """Freshness metrics report time distribution across staleness classes."""
    now = int(time.time())
    events = []
    
    # Create alternating fresh and stale periods
    for i in range(10):
        ts = now - (i * 500)  # 500-second intervals
        events.append(json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,
            "num_sources": 1,
            "timestamp": ts,
            "ledger": 1000 - i,
        }))
        events.append(json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": ts - 5,
            "ledger": 999 - i,
        }))
    
    batch = compute_quality_metrics("\n".join(events))
    metrics = batch.freshness_metrics["ASSET_A"]
    
    # Should have some distribution across classes
    total_pct = metrics.time_in_fresh_pct + metrics.time_in_stale_pct + metrics.time_in_critical_pct
    assert 95 <= total_pct <= 105  # Allow small rounding error


# --- Disagreement Tests ---


def test_disagreement_normal_variance(fresh_events):
    """Small disagreement is classified as Normal."""
    batch = compute_quality_metrics("\n".join(fresh_events))
    
    assert ("ASSET_A", "SOURCE_1") in batch.disagreement_metrics
    metrics = batch.disagreement_metrics[("ASSET_A", "SOURCE_1")]
    assert metrics.incident_level == "Normal"
    assert metrics.max_deviation_bps < 100  # Small variance


def test_disagreement_systematic_divergence(disagreement_events):
    """Source consistently far from aggregate is flagged as systematically divergent."""
    batch = compute_quality_metrics("\n".join(disagreement_events))
    
    assert ("ASSET_A", "SOURCE_2") in batch.disagreement_metrics
    metrics = batch.disagreement_metrics[("ASSET_A", "SOURCE_2")]
    assert metrics.systematically_divergent
    assert metrics.incident_level in ("Warning", "Critical")
    assert metrics.above_threshold_pct > 0


def test_disagreement_metrics_per_mille_calculation(disagreement_events):
    """Disagreement is correctly calculated in basis points."""
    batch = compute_quality_metrics("\n".join(disagreement_events))
    
    metrics_compliant = batch.disagreement_metrics[("ASSET_A", "SOURCE_1")]
    metrics_divergent = batch.disagreement_metrics[("ASSET_A", "SOURCE_2")]
    
    # SOURCE_1 should be close to aggregate
    assert metrics_compliant.mean_deviation_bps < 100
    
    # SOURCE_2 should be far from aggregate
    assert metrics_divergent.mean_deviation_bps > 500


# --- Missing Data Tests ---


def test_missing_data_healthy_source(fresh_events):
    """Source submitting regularly is Healthy."""
    batch = compute_quality_metrics("\n".join(fresh_events))
    
    assert ("SOURCE_1", "ASSET_A") in batch.missing_data_metrics
    metrics = batch.missing_data_metrics[("SOURCE_1", "ASSET_A")]
    assert metrics.health_status == HealthStatus.HEALTHY
    assert metrics.missing_rate_pct < 10


def test_missing_data_offline_source(missing_data_events):
    """Source with many missing submissions is Offline."""
    batch = compute_quality_metrics("\n".join(missing_data_events))
    
    assert ("SOURCE_2", "ASSET_A") in batch.missing_data_metrics
    metrics = batch.missing_data_metrics[("SOURCE_2", "ASSET_A")]
    assert metrics.health_status == HealthStatus.OFFLINE
    assert metrics.missing_rate_pct > 80


def test_missing_data_tracking_consecutive_misses(missing_data_events):
    """Missing-data metrics track consecutive miss streaks."""
    batch = compute_quality_metrics("\n".join(missing_data_events))
    
    metrics = batch.missing_data_metrics[("SOURCE_2", "ASSET_A")]
    assert metrics.consecutive_miss_streak > 10


def test_missing_data_reconciliation_counts_match():
    """Missing data: expected + actual + missing = total expected."""
    now = int(time.time())
    events = []
    
    # Exactly 5 submissions in a 300-second window
    for i in range(5):
        ts = now - (i * 60)
        events.append(json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": ts,
            "ledger": 1000 - i,
        }))
    
    thresholds = MissingDataThresholds(submission_interval_secs=60, window_secs=300)
    batch = compute_quality_metrics(
        "\n".join(events),
        missing_data_thresholds=thresholds
    )
    
    metrics = batch.missing_data_metrics[("SOURCE_1", "ASSET_A")]
    assert metrics.expected_submissions == 5
    assert metrics.actual_submissions == 5
    assert metrics.missing_count == 0


# --- Integration Tests ---


def test_quality_metrics_batch_completeness():
    """Full batch contains all three metric categories."""
    now = int(time.time())
    events = [
        json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,
            "num_sources": 1,
            "timestamp": now - 30,
            "ledger": 1000,
        }),
        json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": now - 35,
            "ledger": 999,
        }),
    ]
    
    batch = compute_quality_metrics("\n".join(events))
    
    assert batch.timestamp is not None
    assert len(batch.freshness_metrics) > 0
    assert len(batch.disagreement_metrics) > 0
    assert len(batch.missing_data_metrics) > 0


def test_quality_metrics_json_serialization():
    """Quality metrics batch is JSON-serializable and preserves structure."""
    now = int(time.time())
    events = [
        json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,
            "num_sources": 1,
            "timestamp": now - 30,
            "ledger": 1000,
        }),
        json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": now - 35,
            "ledger": 999,
        }),
    ]
    
    batch = compute_quality_metrics("\n".join(events))
    d = batch.to_dict()
    
    # Should be JSON-serializable
    json_str = json.dumps(d)
    reconstructed = json.loads(json_str)
    
    assert reconstructed["timestamp"] is not None
    assert "freshness" in reconstructed
    assert "disagreement" in reconstructed
    assert "missing_data" in reconstructed


def test_quality_metrics_empty_stream():
    """Empty event stream produces a valid (empty) batch."""
    batch = compute_quality_metrics([])
    
    assert batch.timestamp is not None
    assert len(batch.freshness_metrics) == 0
    assert len(batch.disagreement_metrics) == 0
    assert len(batch.missing_data_metrics) == 0


def test_quality_metrics_multiple_assets():
    """Metrics correctly handle multiple assets."""
    now = int(time.time())
    events = []
    
    for asset in ["ASSET_A", "ASSET_B"]:
        for i in range(2):
            ts = now - (i * 100)
            events.append(json.dumps({
                "event_name": "PriceAggregated",
                "asset": asset,
                "price": 100_000_000,
                "num_sources": 1,
                "timestamp": ts,
                "ledger": 1000 - i,
            }))
            events.append(json.dumps({
                "event_name": "PriceSubmitted",
                "asset": asset,
                "source": "SOURCE_1",
                "price": 100_000_000,
                "timestamp": ts - 5,
                "ledger": 999 - i,
            }))
    
    batch = compute_quality_metrics("\n".join(events))
    
    assert "ASSET_A" in batch.freshness_metrics
    assert "ASSET_B" in batch.freshness_metrics
    assert ("ASSET_A", "SOURCE_1") in batch.disagreement_metrics
    assert ("ASSET_B", "SOURCE_1") in batch.disagreement_metrics


# --- Threshold Customization Tests ---


def test_freshness_thresholds_custom_resolution():
    """Custom resolution threshold affects staleness classification."""
    now = int(time.time())
    events = [
        json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,
            "num_sources": 1,
            "timestamp": now - 120,  # 2 minutes old
            "ledger": 1000,
        }),
        json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 100_000_000,
            "timestamp": now - 125,
            "ledger": 999,
        }),
    ]
    
    # With high resolution threshold, 2 minutes is Fresh
    thresholds_lenient = FreshnessThresholds(resolution_secs=300)
    batch_lenient = compute_quality_metrics(
        "\n".join(events),
        freshness_thresholds=thresholds_lenient
    )
    assert batch_lenient.freshness_metrics["ASSET_A"].staleness_class == StalenessClass.FRESH
    
    # With low resolution threshold, 2 minutes is Stale
    thresholds_strict = FreshnessThresholds(resolution_secs=30)
    batch_strict = compute_quality_metrics(
        "\n".join(events),
        freshness_thresholds=thresholds_strict
    )
    assert batch_strict.freshness_metrics["ASSET_A"].staleness_class == StalenessClass.STALE


def test_disagreement_thresholds_custom_deviation():
    """Custom deviation threshold affects incident classification."""
    now = int(time.time())
    events = []
    
    # Create submissions with 500 bps (5%) disagreement
    for i in range(3):
        ts = now - (i * 100)
        events.append(json.dumps({
            "event_name": "PriceAggregated",
            "asset": "ASSET_A",
            "price": 100_000_000,
            "num_sources": 1,
            "timestamp": ts,
            "ledger": 1000 - i,
        }))
        events.append(json.dumps({
            "event_name": "PriceSubmitted",
            "asset": "ASSET_A",
            "source": "SOURCE_1",
            "price": 105_000_000,  # 5% higher
            "timestamp": ts - 5,
            "ledger": 999 - i,
        }))
    
    # With high threshold (1000 bps = 10%), should be Normal
    thresholds_lenient = DisagreementThresholds(max_deviation_bps=1000)
    batch_lenient = compute_quality_metrics(
        "\n".join(events),
        disagreement_thresholds=thresholds_lenient
    )
    metrics_lenient = batch_lenient.disagreement_metrics.get(("ASSET_A", "SOURCE_1"))
    if metrics_lenient:
        assert metrics_lenient.incident_level == "Normal"
    
    # With low threshold (100 bps = 1%), should be Critical
    thresholds_strict = DisagreementThresholds(
        max_deviation_bps=100,
        warning_deviation_bps=50,
        critical_deviation_bps=100
    )
    batch_strict = compute_quality_metrics(
        "\n".join(events),
        disagreement_thresholds=thresholds_strict
    )
    metrics_strict = batch_strict.disagreement_metrics.get(("ASSET_A", "SOURCE_1"))
    if metrics_strict:
        assert metrics_strict.incident_level == "Critical"
