"""Tests for capacity headroom review from observed traffic (#704)."""

import json
from pathlib import Path
import pytest

from services.capacity.headroom_review import (
    CapacityLimits,
    ObservedTraffic,
    evaluate_headroom,
    main,
    OWNERS,
    REMEDIATIONS,
)


def test_all_four_bottlenecks_evaluated():
    observed = ObservedTraffic()
    report = evaluate_headroom(observed)

    assert set(report.reviews.keys()) == {"rpc", "compute", "storage", "staffing"}
    for name in ("rpc", "compute", "storage", "staffing"):
        b = report.reviews[name]
        assert b.name == name
        assert b.capacity > 0
        assert b.owner == OWNERS[name]
        assert len(b.remediation_playbook) > 0


def test_healthy_observed_traffic_has_no_triggers():
    observed = ObservedTraffic(
        rpc_sustained_rps=100.0,
        rpc_peak_rps=200.0,
        compute_sustained_cores=2.0,
        compute_peak_cores=4.0,
        storage_current_gb=50.0,
        storage_daily_growth_gb=0.5,
        storage_projected_90d_gb=95.0,
        pages_per_week=2.0,
    )
    report = evaluate_headroom(observed)

    assert report.has_warning_triggers is False
    assert report.has_expansion_triggers is False
    for b in report.reviews.values():
        assert b.status == "HEALTHY"
        assert b.headroom_peak_ratio >= 0.30


def test_warning_trigger_on_rpc_peak():
    # RPC capacity is 500 rps, 380 rps leaves 24% headroom (< 30% warning threshold)
    observed = ObservedTraffic(rpc_peak_rps=380.0)
    report = evaluate_headroom(observed)

    assert report.has_warning_triggers is True
    assert report.has_expansion_triggers is False
    assert report.reviews["rpc"].status == "WARNING"
    assert report.reviews["rpc"].trigger_message is not None
    assert "leaves 24.0% headroom" in report.reviews["rpc"].trigger_message


def test_critical_expansion_trigger_on_compute_peak():
    # Compute capacity is 8 cores, 7.2 cores leaves 10% headroom (< 15% expansion threshold)
    observed = ObservedTraffic(compute_peak_cores=7.2)
    report = evaluate_headroom(observed)

    assert report.has_expansion_triggers is True
    assert report.reviews["compute"].status == "CRITICAL_EXPANSION"
    assert report.reviews["compute"].trigger_message is not None
    assert "leaves only 10.0% headroom" in report.reviews["compute"].trigger_message


def test_storage_growth_projection_trigger():
    # Storage capacity is 250 GB, 90d projection of 230 GB leaves 8% headroom
    observed = ObservedTraffic(
        storage_current_gb=140.0,
        storage_daily_growth_gb=1.0,
        storage_projected_90d_gb=230.0,
    )
    report = evaluate_headroom(observed)

    assert report.has_expansion_triggers is True
    assert report.reviews["storage"].status == "CRITICAL_EXPANSION"
    assert "leaves only 8.0% headroom" in report.reviews["storage"].trigger_message


def test_operator_staffing_fatigue_trigger():
    # Staffing max is 10 pages/week, 9 pages/week leaves 10% headroom
    observed = ObservedTraffic(pages_per_week=9.0)
    report = evaluate_headroom(observed)

    assert report.has_expansion_triggers is True
    assert report.reviews["staffing"].status == "CRITICAL_EXPANSION"
    assert "exceed sustainable fatigue threshold" in report.reviews["staffing"].trigger_message


def test_markdown_report_formatting():
    observed = ObservedTraffic(compute_peak_cores=7.2)
    report = evaluate_headroom(observed, review_date="2026-10-21")
    md = report.generate_markdown()

    assert "# Capacity Headroom Review Report (2026-10-21)" in md
    assert "CRITICAL: Expansion Required" in md
    assert "| **COMPUTE** |" in md
    assert "### COMPUTE (CRITICAL_EXPANSION)" in md
    assert "Remediation Playbook (Owner: `core-contracts`)" in md


def test_cli_execution_with_json_and_report(tmp_path: Path):
    report_file = tmp_path / "headroom_report.md"
    exit_code = main([
        "--rpc-peak-rps", "250.0",
        "--compute-peak-cores", "3.0",
        "--pages-per-week", "3.0",
        "--json",
        "--report", str(report_file),
    ])

    assert exit_code == 0
    assert report_file.exists()
    assert report_file.read_text(encoding="utf-8").startswith("# Capacity Headroom Review Report")


def test_cli_fail_on_trigger_exits_nonzero(tmp_path: Path):
    input_data = {
        "observation_period_days": 14,
        "rpc_sustained_rps": 100.0,
        "rpc_peak_rps": 480.0,  # 4% headroom -> triggers CRITICAL_EXPANSION
        "compute_sustained_cores": 2.0,
        "compute_peak_cores": 4.0,
        "storage_current_gb": 50.0,
        "storage_daily_growth_gb": 0.5,
        "pages_per_week": 2.0,
    }
    input_file = tmp_path / "traffic.json"
    input_file.write_text(json.dumps(input_data), encoding="utf-8")

    exit_code = main([
        "--input", str(input_file),
        "--fail-on-trigger",
    ])
    assert exit_code == 1
