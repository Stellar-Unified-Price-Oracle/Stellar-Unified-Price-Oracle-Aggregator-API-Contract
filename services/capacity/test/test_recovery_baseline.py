"""Tests for cold-start and recovery performance baseline (#703)."""

from pathlib import Path
import tempfile
import pytest

from services.capacity.recovery_baseline import (
    RecoveryBudget,
    RecoveryMetrics,
    benchmark_rebuild,
    generate_synthetic_events,
    main,
    prometheus_metrics,
)


def test_generate_synthetic_events():
    events = generate_synthetic_events(100)
    assert len(events) >= 100
    # Must include source_added, price_submitted, and price_aggregated
    topics = {e["topic"] for e in events}
    assert "source_added" in topics
    assert "price_submitted" in topics
    assert "price_aggregated" in topics


def test_fresh_deployment_rebuild_records_time_and_resources(tmp_path: Path):
    db_path = tmp_path / "fresh_test.db"
    events = generate_synthetic_events(500)

    budget = RecoveryBudget(
        max_seconds_per_1k_events=5.0,
        max_memory_mb=128.0,
        max_total_recovery_seconds=30.0,
    )

    metrics = benchmark_rebuild(events=events, db_path=db_path, mode="fresh", budget=budget)

    assert metrics.mode == "fresh"
    assert metrics.events_processed == len(events)
    assert metrics.wall_time_seconds > 0
    assert metrics.events_per_second > 0
    assert metrics.derived_state_bytes > 0
    assert metrics.peak_memory_mb >= 0
    assert metrics.passed_threshold is True
    assert len(metrics.violations) == 0
    assert db_path.exists()


def test_snapshot_recovery_rebuild_fast(tmp_path: Path):
    db_path = tmp_path / "snapshot_test.db"
    all_events = generate_synthetic_events(400)
    split_idx = 100
    snapshot_events = all_events[:split_idx]
    replay_events = all_events[split_idx:]

    budget = RecoveryBudget(max_seconds_per_1k_events=5.0, max_memory_mb=128.0)

    metrics = benchmark_rebuild(
        events=replay_events,
        db_path=db_path,
        mode="snapshot",
        budget=budget,
        initial_snapshot_events=snapshot_events,
    )

    assert metrics.mode == "snapshot"
    assert metrics.events_processed == len(replay_events)
    assert metrics.passed_threshold is True


def test_recovery_regression_threshold_enforced(tmp_path: Path):
    db_path = tmp_path / "regression_test.db"
    events = generate_synthetic_events(200)

    # Impossibly tight budget to verify threshold violations are caught
    strict_budget = RecoveryBudget(
        max_seconds_per_1k_events=0.000001,
        max_memory_mb=0.000001,
        max_total_recovery_seconds=0.000001,
    )

    metrics = benchmark_rebuild(
        events=events,
        db_path=db_path,
        mode="fresh",
        budget=strict_budget,
    )

    assert metrics.passed_threshold is False
    assert len(metrics.violations) > 0
    assert any("Recovery rate regression" in v for v in metrics.violations)


def test_prometheus_formatting():
    metrics = RecoveryMetrics(
        mode="fresh",
        events_processed=1000,
        ledgers_processed=200,
        wall_time_seconds=0.5,
        cpu_time_seconds=0.45,
        peak_memory_mb=12.5,
        derived_state_bytes=102400,
        passed_threshold=True,
    )
    prom = prometheus_metrics(metrics)
    assert 'oracle_recovery_wall_time_seconds{mode="fresh"} 0.5' in prom
    assert 'oracle_recovery_events_per_second{mode="fresh"} 2000.0' in prom
    assert 'oracle_recovery_budget_pass{mode="fresh"} 1' in prom


def test_cli_smoke(tmp_path: Path):
    db_path = tmp_path / "cli_test.db"
    rc = main(["--mode", "fresh", "--events", "100", "--db-path", str(db_path), "--json"])
    assert rc == 0
