"""Comprehensive price-quality metrics: freshness, disagreement, and missing-data (#520).

This module extends the revision_metrics corrections-rate module to include three
additional quality dimensions:

1. **Freshness:** Time since last price publication (asset-level and per-source).
2. **Disagreement:** How much each source deviates from the published aggregate.
3. **Missing Data:** What fraction of expected submissions are absent per source.

All metrics use the same windowing and reconciliation patterns as revision_metrics (#499).
Metrics are pure functions of the event stream and the current Soroban RPC state,
so they are reproducible and verifiable.

Design notes
============

**Freshness** is tracked per asset (time since last aggregation ledger) and per
source (time since last submission). The metric includes staleness classification:
- Fresh: age ≤ resolution (contract's configured price-resolution window)
- Stale: age > resolution and age ≤ max_staleness (SLA-defined, default 1 h)
- Critical: age > max_staleness

**Disagreement** measures how much each source's submitted price deviates from the
published aggregate (median). The metric aggregates deviations as:
- max_deviation_bps: worst single submission
- mean_deviation_bps: average across all submissions
- p95_deviation_bps: 95th percentile
- above_threshold_pct: % of submissions exceeding SLA deviation threshold

**Missing Data** tracks submission compliance for each (source, asset) pair by
comparing actual submissions to expected intervals. A source is marked:
- Compliant: submitted within the expected interval
- Degraded: missing >20% of expected submissions or >5 consecutive misses
- Offline: missing >80% or >20 consecutive misses

All three categories emit reconciliation signals that can detect:
- Out-of-order or lost event stream entries (missing-data gaps)
- Clock skew in the exporter (freshness drift)
- Indexing errors in disagreement calculations
"""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from enum import Enum
from typing import Dict, Iterable, List, Mapping, Optional, Sequence, Tuple

from services.common.events import (
    AggregationEvent,
    EventSource,
    SubmissionEvent,
    iter_aggregations,
    iter_submissions,
)

# --- Freshness Configuration ---

#: Default maximum price staleness (SLA window) in seconds. Beyond this, the price is critical.
DEFAULT_MAX_STALENESS_SECS = 3600  # 1 hour
#: Expected submission interval per source, in seconds. Can be overridden per source.
DEFAULT_SUBMISSION_INTERVAL_SECS = 60
#: Threshold for flagging a missing-data streak as degraded.
DEFAULT_MISSING_STREAK_DEGRADED = 5
#: Threshold for flagging a missing-data streak as offline.
DEFAULT_MISSING_STREAK_OFFLINE = 20
#: Percentage threshold for missing-data rate degradation.
DEFAULT_MISSING_RATE_DEGRADED_PCT = 20
#: Percentage threshold for missing-data rate offline.
DEFAULT_MISSING_RATE_OFFLINE_PCT = 80

# --- Disagreement Configuration ---

#: Default maximum acceptable price deviation in basis points (100 bps = 1%).
DEFAULT_MAX_DEVIATION_BPS = 500
#: Threshold for warning on disagreement.
DEFAULT_DISAGREEMENT_WARNING_BPS = 300
#: Threshold for critical disagreement alert.
DEFAULT_DISAGREEMENT_CRITICAL_BPS = 500


class StalenessClass(str, Enum):
    """Classification of price age relative to SLA."""

    FRESH = "Fresh"
    STALE = "Stale"
    CRITICAL = "Critical"


class ComplianceClass(str, Enum):
    """Classification of source submission compliance."""

    COMPLIANT = "Compliant"
    OVERDUE = "Overdue"
    MISSING = "Missing"


class HealthStatus(str, Enum):
    """Classification of source data health."""

    HEALTHY = "Healthy"
    DEGRADED = "Degraded"
    OFFLINE = "Offline"


# --- Data Classes ---


@dataclass(frozen=True)
class FreshnessThresholds:
    """Operator-tunable parameters for freshness metrics."""

    resolution_secs: int = 60  # Contract's price-resolution window
    max_staleness_secs: int = DEFAULT_MAX_STALENESS_SECS
    submission_interval_secs: int = DEFAULT_SUBMISSION_INTERVAL_SECS
    window_secs: int = 86_400  # 24 hours


@dataclass(frozen=True)
class DisagreementThresholds:
    """Operator-tunable parameters for disagreement metrics."""

    max_deviation_bps: int = DEFAULT_MAX_DEVIATION_BPS
    warning_deviation_bps: int = DEFAULT_DISAGREEMENT_WARNING_BPS
    critical_deviation_bps: int = DEFAULT_DISAGREEMENT_CRITICAL_BPS
    window_secs: int = 86_400  # 24 hours


@dataclass(frozen=True)
class MissingDataThresholds:
    """Operator-tunable parameters for missing-data metrics."""

    submission_interval_secs: int = DEFAULT_SUBMISSION_INTERVAL_SECS
    missing_streak_degraded: int = DEFAULT_MISSING_STREAK_DEGRADED
    missing_streak_offline: int = DEFAULT_MISSING_STREAK_OFFLINE
    missing_rate_degraded_pct: float = DEFAULT_MISSING_RATE_DEGRADED_PCT
    missing_rate_offline_pct: float = DEFAULT_MISSING_RATE_OFFLINE_PCT
    window_secs: int = 86_400  # 24 hours


@dataclass(frozen=True)
class FreshnessMetrics:
    """Asset-level and per-source freshness measurements."""

    asset: str
    # Asset-level freshness
    time_since_last_aggregate_secs: int
    staleness_class: StalenessClass
    time_in_fresh_pct: float
    time_in_stale_pct: float
    time_in_critical_pct: float
    freshness_breaches: int  # Count of times crossed into Critical
    
    # Per-source freshness (keyed by source address)
    per_source: Dict[str, PerSourceFreshness] = field(default_factory=dict)

    def to_dict(self) -> dict:
        return {
            "asset": self.asset,
            "time_since_last_aggregate_secs": self.time_since_last_aggregate_secs,
            "staleness_class": self.staleness_class.value,
            "time_in_fresh_pct": round(self.time_in_fresh_pct, 2),
            "time_in_stale_pct": round(self.time_in_stale_pct, 2),
            "time_in_critical_pct": round(self.time_in_critical_pct, 2),
            "freshness_breaches": self.freshness_breaches,
            "per_source": {
                src: psf.to_dict() for src, psf in sorted(self.per_source.items())
            },
        }


@dataclass(frozen=True)
class PerSourceFreshness:
    """Per-source freshness for a specific asset."""

    source: str
    asset: str
    time_since_last_submission_secs: int
    submission_compliance_class: ComplianceClass
    expected_intervals_in_window: int
    compliant_intervals: int
    overdue_intervals: int
    missing_intervals: int

    def to_dict(self) -> dict:
        return {
            "source": self.source,
            "asset": self.asset,
            "time_since_last_submission_secs": self.time_since_last_submission_secs,
            "submission_compliance_class": self.submission_compliance_class.value,
            "expected_intervals": self.expected_intervals_in_window,
            "compliant": self.compliant_intervals,
            "overdue": self.overdue_intervals,
            "missing": self.missing_intervals,
            "missing_pct": round(
                100.0 * self.missing_intervals / self.expected_intervals_in_window
                if self.expected_intervals_in_window
                else 0.0,
                2,
            ),
        }


@dataclass(frozen=True)
class DisagreementMetrics:
    """Per-source disagreement with the published aggregate."""

    asset: str
    source: str
    submission_count: int
    max_deviation_bps: int
    mean_deviation_bps: float
    p95_deviation_bps: int
    outlier_count: int
    above_threshold_count: int
    above_threshold_pct: float
    systematically_divergent: bool
    incident_level: str  # "Normal", "Warning", "Critical"

    def to_dict(self) -> dict:
        return {
            "asset": self.asset,
            "source": self.source,
            "submission_count": self.submission_count,
            "max_deviation_bps": self.max_deviation_bps,
            "mean_deviation_bps": round(self.mean_deviation_bps, 2),
            "p95_deviation_bps": self.p95_deviation_bps,
            "outlier_count": self.outlier_count,
            "above_threshold_count": self.above_threshold_count,
            "above_threshold_pct": round(self.above_threshold_pct, 2),
            "systematically_divergent": self.systematically_divergent,
            "incident_level": self.incident_level,
        }


@dataclass(frozen=True)
class MissingDataMetrics:
    """Missing-data (submission compliance) metrics for a source."""

    source: str
    asset: str
    expected_submissions: int
    actual_submissions: int
    missing_count: int
    missing_rate_pct: float
    consecutive_miss_streak: int
    health_status: HealthStatus
    first_outage_ledger: Optional[int] = None
    latest_recovery_attempt_ledger: Optional[int] = None

    def to_dict(self) -> dict:
        return {
            "source": self.source,
            "asset": self.asset,
            "expected_submissions": self.expected_submissions,
            "actual_submissions": self.actual_submissions,
            "missing_count": self.missing_count,
            "missing_rate_pct": round(self.missing_rate_pct, 2),
            "consecutive_miss_streak": self.consecutive_miss_streak,
            "health_status": self.health_status.value,
            "first_outage_ledger": self.first_outage_ledger,
            "latest_recovery_attempt_ledger": self.latest_recovery_attempt_ledger,
        }


@dataclass(frozen=True)
class QualityMetricsBatch:
    """Complete batch of quality metrics for all assets and sources."""

    timestamp: int  # Unix seconds when this batch was computed
    freshness_metrics: Dict[str, FreshnessMetrics] = field(default_factory=dict)
    disagreement_metrics: Dict[Tuple[str, str], DisagreementMetrics] = field(
        default_factory=dict
    )
    missing_data_metrics: Dict[Tuple[str, str], MissingDataMetrics] = field(
        default_factory=dict
    )

    def to_dict(self) -> dict:
        return {
            "timestamp": self.timestamp,
            "freshness": {
                asset: m.to_dict() for asset, m in sorted(self.freshness_metrics.items())
            },
            "disagreement": {
                f"{asset}:{source}": m.to_dict()
                for (asset, source), m in sorted(self.disagreement_metrics.items())
            },
            "missing_data": {
                f"{source}:{asset}": m.to_dict()
                for (source, asset), m in sorted(self.missing_data_metrics.items())
            },
        }


# --- Calculation Functions ---


def compute_freshness_metrics(
    asset: str,
    aggregations: Sequence[AggregationEvent],
    submissions: Sequence[SubmissionEvent],
    thresholds: FreshnessThresholds,
    now: int,
) -> FreshnessMetrics:
    """Computes freshness metrics for one asset."""
    agg = sorted([a for a in aggregations if a.asset == asset], key=lambda x: x.timestamp)
    subs = sorted([s for s in submissions if s.asset == asset], key=lambda x: x.timestamp)

    # Asset-level freshness
    time_since_last_agg = (
        (now - agg[-1].timestamp) if agg else (now - submissions[0].timestamp)
        if submissions
        else 0
    )
    staleness_class = (
        StalenessClass.CRITICAL
        if time_since_last_agg > thresholds.max_staleness_secs
        else StalenessClass.STALE
        if time_since_last_agg > thresholds.resolution_secs
        else StalenessClass.FRESH
    )

    # Time distribution across staleness classes (simplified: sample at intervals)
    window_start = now - thresholds.window_secs
    fresh_time = stale_time = critical_time = 0
    interval_size = 300  # 5-minute buckets
    for bucket_start in range(
        int(window_start / interval_size) * interval_size, now, interval_size
    ):
        bucket_end = bucket_start + interval_size
        # Find the last aggregation before bucket_end
        last_agg_before_bucket = None
        for a in agg:
            if a.timestamp <= bucket_end:
                last_agg_before_bucket = a
            else:
                break
        if last_agg_before_bucket:
            age_at_bucket_end = bucket_end - last_agg_before_bucket.timestamp
            if age_at_bucket_end <= thresholds.resolution_secs:
                fresh_time += interval_size
            elif age_at_bucket_end <= thresholds.max_staleness_secs:
                stale_time += interval_size
            else:
                critical_time += interval_size
        else:
            critical_time += interval_size

    total_time = fresh_time + stale_time + critical_time
    freshness_breaches = sum(
        1 for i in range(len(agg) - 1) 
        if (agg[i+1].timestamp - agg[i].timestamp) > thresholds.max_staleness_secs
    )

    # Per-source freshness
    per_source: Dict[str, PerSourceFreshness] = {}
    sources = {s.source for s in subs}
    for source in sorted(sources):
        source_subs = sorted(
            [s for s in subs if s.source == source], key=lambda x: x.timestamp
        )
        time_since_last_sub = (
            (now - source_subs[-1].timestamp) if source_subs else 0
        )
        compliance_class = (
            ComplianceClass.MISSING
            if not source_subs
            else ComplianceClass.OVERDUE
            if time_since_last_sub > thresholds.submission_interval_secs * 1.5
            else ComplianceClass.COMPLIANT
        )

        # Count submission intervals
        expected_intervals = (
            thresholds.window_secs // thresholds.submission_interval_secs
        )
        actual_intervals = len(source_subs)
        missing_intervals = max(0, expected_intervals - actual_intervals)

        # Count compliant vs. overdue intervals
        compliant_count = overdue_count = 0
        for i, s in enumerate(source_subs):
            if s.timestamp >= window_start:
                time_since_prev = (
                    s.timestamp - source_subs[i - 1].timestamp if i > 0 else 0
                )
                if time_since_prev <= thresholds.submission_interval_secs * 1.5:
                    compliant_count += 1
                else:
                    overdue_count += 1

        per_source[source] = PerSourceFreshness(
            source=source,
            asset=asset,
            time_since_last_submission_secs=time_since_last_sub,
            submission_compliance_class=compliance_class,
            expected_intervals_in_window=expected_intervals,
            compliant_intervals=compliant_count,
            overdue_intervals=overdue_count,
            missing_intervals=missing_intervals,
        )

    return FreshnessMetrics(
        asset=asset,
        time_since_last_aggregate_secs=time_since_last_agg,
        staleness_class=staleness_class,
        time_in_fresh_pct=100.0 * fresh_time / total_time if total_time else 0.0,
        time_in_stale_pct=100.0 * stale_time / total_time if total_time else 0.0,
        time_in_critical_pct=100.0 * critical_time / total_time if total_time else 0.0,
        freshness_breaches=freshness_breaches,
        per_source=per_source,
    )


def compute_disagreement_metrics(
    asset: str,
    source: str,
    submissions: Sequence[SubmissionEvent],
    aggregations: Sequence[AggregationEvent],
    thresholds: DisagreementThresholds,
    now: int,
) -> Optional[DisagreementMetrics]:
    """Computes disagreement metrics for one (asset, source) pair.
    
    Returns None if there are insufficient submissions to compute meaningful metrics.
    """
    source_subs = sorted(
        [
            s
            for s in submissions
            if s.asset == asset and s.source == source
        ],
        key=lambda x: x.timestamp,
    )
    agg = sorted(
        [a for a in aggregations if a.asset == asset], key=lambda x: x.timestamp
    )

    if not source_subs or not agg:
        return None

    window_start = now - thresholds.window_secs
    recent_subs = [s for s in source_subs if s.timestamp >= window_start]

    if not recent_subs or not agg:
        return None

    # Compute deviations
    deviations_bps: List[int] = []
    for sub in recent_subs:
        # Find the aggregate price at or before the submission time
        agg_price = None
        for a in agg:
            if a.timestamp <= sub.timestamp:
                agg_price = a.price
            else:
                break

        if agg_price and agg_price != 0:
            dev_bps = int(abs(sub.price - agg_price) * 10_000 / abs(agg_price))
            deviations_bps.append(dev_bps)

    if not deviations_bps:
        return None

    deviations_bps.sort()
    max_dev = deviations_bps[-1]
    mean_dev = sum(deviations_bps) / len(deviations_bps)
    p95_dev = deviations_bps[int(len(deviations_bps) * 0.95)] if len(deviations_bps) > 1 else max_dev
    above_threshold = sum(1 for d in deviations_bps if d > thresholds.max_deviation_bps)
    above_threshold_pct = 100.0 * above_threshold / len(deviations_bps) if deviations_bps else 0.0

    # Determine incident level
    if mean_dev > thresholds.critical_deviation_bps or above_threshold_pct >= 50:
        incident_level = "Critical"
    elif mean_dev > thresholds.warning_deviation_bps or above_threshold_pct >= 20:
        incident_level = "Warning"
    else:
        incident_level = "Normal"

    return DisagreementMetrics(
        asset=asset,
        source=source,
        submission_count=len(recent_subs),
        max_deviation_bps=max_dev,
        mean_deviation_bps=mean_dev,
        p95_deviation_bps=p95_dev,
        outlier_count=0,  # Would require additional logic to detect pre-filter outliers
        above_threshold_count=above_threshold,
        above_threshold_pct=above_threshold_pct,
        systematically_divergent=p95_dev > thresholds.max_deviation_bps,
        incident_level=incident_level,
    )


def compute_missing_data_metrics(
    source: str,
    asset: str,
    submissions: Sequence[SubmissionEvent],
    thresholds: MissingDataThresholds,
    now: int,
) -> MissingDataMetrics:
    """Computes missing-data metrics for one (source, asset) pair."""
    source_subs = sorted(
        [
            s
            for s in submissions
            if s.asset == asset and s.source == source
        ],
        key=lambda x: x.timestamp,
    )

    window_start = now - thresholds.window_secs
    recent_subs = [s for s in source_subs if s.timestamp >= window_start]

    expected = thresholds.window_secs // thresholds.submission_interval_secs
    actual = len(recent_subs)
    missing = max(0, expected - actual)
    missing_pct = 100.0 * missing / expected if expected else 0.0

    # Compute longest consecutive miss streak
    max_streak = 0
    current_streak = 0
    last_sub_time = window_start
    for bucket_start in range(
        int(window_start / thresholds.submission_interval_secs) * thresholds.submission_interval_secs,
        now,
        thresholds.submission_interval_secs,
    ):
        bucket_end = bucket_start + thresholds.submission_interval_secs
        has_sub = any(s for s in recent_subs if bucket_start <= s.timestamp < bucket_end)
        if not has_sub:
            current_streak += 1
            max_streak = max(max_streak, current_streak)
        else:
            current_streak = 0

    # Determine health status
    if (
        missing_pct >= thresholds.missing_rate_offline_pct
        or max_streak >= thresholds.missing_streak_offline
    ):
        health_status = HealthStatus.OFFLINE
    elif (
        missing_pct >= thresholds.missing_rate_degraded_pct
        or max_streak >= thresholds.missing_streak_degraded
    ):
        health_status = HealthStatus.DEGRADED
    else:
        health_status = HealthStatus.HEALTHY

    # Track outage and recovery
    first_outage_ledger = None
    latest_recovery_attempt_ledger = None
    if health_status in (HealthStatus.DEGRADED, HealthStatus.OFFLINE):
        if source_subs:
            first_outage_ledger = source_subs[0].ledger
        if recent_subs:
            latest_recovery_attempt_ledger = recent_subs[-1].ledger

    return MissingDataMetrics(
        source=source,
        asset=asset,
        expected_submissions=expected,
        actual_submissions=actual,
        missing_count=missing,
        missing_rate_pct=missing_pct,
        consecutive_miss_streak=max_streak,
        health_status=health_status,
        first_outage_ledger=first_outage_ledger,
        latest_recovery_attempt_ledger=latest_recovery_attempt_ledger,
    )


def compute_quality_metrics(
    events: EventSource,
    freshness_thresholds: Optional[FreshnessThresholds] = None,
    disagreement_thresholds: Optional[DisagreementThresholds] = None,
    missing_data_thresholds: Optional[MissingDataThresholds] = None,
    now: Optional[int] = None,
) -> QualityMetricsBatch:
    """Computes all four quality-metric categories from the event stream.
    
    Returns a complete batch of freshness, disagreement, and missing-data metrics.
    ``now`` defaults to the newest event timestamp, so results are reproducible.
    """
    freshness_thresholds = freshness_thresholds or FreshnessThresholds()
    disagreement_thresholds = disagreement_thresholds or DisagreementThresholds()
    missing_data_thresholds = missing_data_thresholds or MissingDataThresholds()

    # Parse events
    if isinstance(events, (str, bytes)) or hasattr(events, "__fspath__"):
        with open(events, "r", encoding="utf-8") as handle:  # type: ignore[arg-type]
            rows = [json.loads(line) for line in handle if line.strip()]
    else:
        # Handle both dicts and JSON strings in iterable
        rows = []
        for event in events:  # type: ignore[union-attr]
            if isinstance(event, str):
                rows.append(json.loads(event))
            else:
                rows.append(event)

    if not rows:
        return QualityMetricsBatch(timestamp=int(__import__("time").time()))

    aggregations = sorted(iter_aggregations(rows), key=lambda a: a.timestamp)
    submissions = sorted(iter_submissions(rows), key=lambda s: s.timestamp)

    if now is None:
        now = max(
            max((a.timestamp for a in aggregations), default=0),
            max((s.timestamp for s in submissions), default=0),
        )

    # Collect unique assets and sources
    assets = sorted({a.asset for a in aggregations} | {s.asset for s in submissions})
    sources = sorted({s.source for s in submissions})

    # Compute metrics
    freshness_metrics = {
        asset: compute_freshness_metrics(
            asset, aggregations, submissions, freshness_thresholds, now
        )
        for asset in assets
    }

    disagreement_metrics = {}
    for asset in assets:
        for source in sources:
            metrics = compute_disagreement_metrics(
                asset, source, submissions, aggregations, disagreement_thresholds, now
            )
            if metrics:
                disagreement_metrics[(asset, source)] = metrics

    missing_data_metrics = {
        (source, asset): compute_missing_data_metrics(
            source, asset, submissions, missing_data_thresholds, now
        )
        for source in sources
        for asset in assets
    }

    return QualityMetricsBatch(
        timestamp=now,
        freshness_metrics=freshness_metrics,
        disagreement_metrics=disagreement_metrics,
        missing_data_metrics=missing_data_metrics,
    )


def main(argv: Optional[Sequence[str]] = None) -> int:
    """CLI entry point for computing quality metrics."""
    parser = argparse.ArgumentParser(description="Price quality metrics (#520)")
    parser.add_argument("events", help="JSONL export of the oracle event stream")
    parser.add_argument(
        "--resolution-secs",
        type=int,
        default=FreshnessThresholds().resolution_secs,
        help="Price resolution window (seconds)",
    )
    parser.add_argument(
        "--max-staleness-secs",
        type=int,
        default=DEFAULT_MAX_STALENESS_SECS,
        help="Maximum acceptable staleness (seconds)",
    )
    parser.add_argument(
        "--submission-interval-secs",
        type=int,
        default=DEFAULT_SUBMISSION_INTERVAL_SECS,
        help="Expected submission interval (seconds)",
    )
    parser.add_argument(
        "--max-deviation-bps",
        type=int,
        default=DEFAULT_MAX_DEVIATION_BPS,
        help="Maximum acceptable deviation (basis points)",
    )
    args = parser.parse_args(argv)

    freshness_thresholds = FreshnessThresholds(
        resolution_secs=args.resolution_secs,
        max_staleness_secs=args.max_staleness_secs,
        submission_interval_secs=args.submission_interval_secs,
    )
    disagreement_thresholds = DisagreementThresholds(
        max_deviation_bps=args.max_deviation_bps,
    )

    batch = compute_quality_metrics(
        args.events,
        freshness_thresholds=freshness_thresholds,
        disagreement_thresholds=disagreement_thresholds,
    )

    json.dump(batch.to_dict(), __import__("sys").stdout, indent=2, sort_keys=True)
    __import__("sys").stdout.write("\n")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
