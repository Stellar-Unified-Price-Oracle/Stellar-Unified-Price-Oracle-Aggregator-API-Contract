"""Tests for the capacity model, headroom targets and load shedding (#530)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from services.capacity.model import (
    DEFAULT_CAPACITIES,
    HEADROOM_TARGETS,
    LAG_ALERT_SECONDS,
    LAG_TARGET_SECONDS,
    PRIORITY_CRITICAL,
    SHED_ORDER,
    TOLERANCE,
    CapacityPlan,
    Drivers,
    Profile,
    Resource,
    compare_to_measured,
    pre_exhaustion_alerts,
    project,
    prometheus,
    shedding_plan,
)

# The reference deployment the model is calibrated against: 20 assets, 5
# sources, 8 submissions/second in aggregate.
NOMINAL = Profile(assets=20, sources=5, submission_rate_hz=8.0)

# A month of quarterly-review measurements of the same deployment. The model
# must reproduce these within TOLERANCE, which is the validation criterion.
MEASURED_INGEST = [0.031, 0.033, 0.030, 0.034, 0.032, 0.031, 0.033]
MEASURED_LEDGER = [3.81e10, 3.84e10, 3.79e10, 3.86e10, 3.82e10, 3.80e10, 3.83e10]


def _overloaded(**kw) -> CapacityPlan:
    return project(Profile(assets=200, sources=9, submission_rate_hz=90.0), **kw)


# ── Model reproduces measured usage ──────────────────────────────────────────

def test_model_reproduces_measured_ingest_within_tolerance():
    projected = [project(NOMINAL).resource("ingest").used for _ in MEASURED_INGEST]
    result = compare_to_measured(projected, MEASURED_INGEST)
    assert result["within_tolerance"], result
    assert result["worst_relative_error"] <= TOLERANCE


def test_model_reproduces_measured_ledger_budget_within_tolerance():
    projected = [project(NOMINAL).resource("ledger_budget").used for _ in MEASURED_LEDGER]
    result = compare_to_measured(projected, MEASURED_LEDGER)
    assert result["within_tolerance"], result


def test_tolerance_is_stated_and_not_implicit():
    result = compare_to_measured([1.0], [1.0])
    assert result["tolerance"] == TOLERANCE
    assert result["within_tolerance"] is True


def test_comparison_reports_a_projection_that_drifts_out_of_tolerance():
    result = compare_to_measured([2.0, 2.0], [1.0, 1.0], tolerance=0.2)
    assert not result["within_tolerance"]
    assert result["worst_relative_error"] == 1.0


def test_comparison_rejects_mismatched_series():
    with pytest.raises(ValueError):
        compare_to_measured([1.0], [1.0, 2.0])


# ── Cost drivers per source, asset and submission rate ───────────────────────

def test_every_resource_grows_with_submission_rate():
    low = project(Profile(assets=20, sources=5, submission_rate_hz=4.0))
    high = project(Profile(assets=20, sources=5, submission_rate_hz=8.0))
    for name in ("ingest", "storage", "ledger_budget"):
        assert high.resource(name).used > low.resource(name).used


def test_storage_grows_with_asset_count():
    few = project(Profile(assets=5, sources=5, submission_rate_hz=8.0))
    many = project(Profile(assets=50, sources=5, submission_rate_hz=8.0))
    assert many.resource("storage").used > few.resource("storage").used


def test_ledger_budget_grows_with_history_depth():
    shallow = project(Profile(assets=20, sources=5, submission_rate_hz=8.0, history_depth=100))
    deep = project(Profile(assets=20, sources=5, submission_rate_hz=8.0, history_depth=2000))
    assert deep.resource("ledger_budget").used > shallow.resource("ledger_budget").used


def test_ledger_cpu_driver_matches_the_published_gas_budget():
    """The model and docs/gas-budget.md must not drift apart silently."""
    assert Drivers().ledger_cpu_per_submission == 1_287_000


def test_lag_grows_with_sources_and_per_asset_rate():
    few = project(Profile(assets=20, sources=3, submission_rate_hz=8.0))
    many = project(Profile(assets=20, sources=9, submission_rate_hz=8.0))
    assert many.lag_seconds > few.lag_seconds


# ── Headroom and lag targets ─────────────────────────────────────────────────

def test_headroom_targets_are_defined_per_resource():
    assert set(HEADROOM_TARGETS) == set(DEFAULT_CAPACITIES)
    assert all(0 < v < 1 for v in HEADROOM_TARGETS.values())


def test_ledger_budget_carries_the_largest_margin():
    """The ledger is shared with traffic we do not control, so it gets the most."""
    assert HEADROOM_TARGETS["ledger_budget"] == max(HEADROOM_TARGETS.values())


def test_nominal_deployment_is_within_every_target():
    plan = project(NOMINAL)
    assert plan.at_risk == []
    assert plan.lag_seconds < LAG_TARGET_SECONDS


def test_resource_reports_at_risk_before_it_is_exhausted():
    r = Resource("ingest", used=80.0, capacity=100.0, headroom_target=0.30)
    assert r.headroom == pytest.approx(0.20)
    assert r.at_risk and not r.exhausted


def test_resource_reports_exhausted_when_used_exceeds_capacity():
    r = Resource("ledger_budget", used=120.0, capacity=100.0, headroom_target=0.5)
    assert r.exhausted and r.headroom < 0


def test_unknown_resource_raises():
    with pytest.raises(KeyError):
        project(NOMINAL).resource("does-not-exist")


# ── Load shedding prioritises the critical path ──────────────────────────────

def test_healthy_plan_sheds_nothing():
    assert shedding_plan(project(NOMINAL)) == []


def test_shedding_is_ordered_least_impactful_first():
    steps = shedding_plan(_overloaded())
    assert steps == [text for _, text in SHED_ORDER][: len(steps)]
    assert steps[0].startswith("drop historical backfill")


def test_critical_path_work_is_shed_last():
    """Critical-path degradation exists, but nothing else may precede it."""
    priorities = [p for p, _ in SHED_ORDER]
    assert priorities[-1] == PRIORITY_CRITICAL
    assert priorities.count(PRIORITY_CRITICAL) == 1


def test_pressure_sheds_only_as_far_as_it_must():
    mild = project(Profile(assets=20, sources=5, submission_rate_hz=20.0))
    severe = _overloaded()
    assert 0 < len(shedding_plan(mild)) < len(shedding_plan(severe))


def test_shedding_raises_asset_priority_when_lag_is_approaching():
    plan = project(Profile(assets=20, sources=5, submission_rate_hz=20.0), lag_seconds=58.0)
    steps = shedding_plan(plan)
    assert steps[0].startswith("raise per-asset submission priority")


def test_shedding_does_not_reduce_ingest_capacity_itself():
    """Shedding must degrade gracefully: it lowers offered load, never raises it."""
    for text in (t for _, t in SHED_ORDER):
        assert "submit more" not in text.lower()


# ── Pre-exhaustion alerting ──────────────────────────────────────────────────

def test_no_alerts_for_a_healthy_plan():
    assert pre_exhaustion_alerts(project(NOMINAL)) == []


def test_headroom_alert_fires_before_exhaustion():
    plan = project(Profile(assets=20, sources=5, submission_rate_hz=20.0))
    alerts = pre_exhaustion_alerts(plan)
    assert any(a["alert"] == "OracleCapacityHeadroomLow" for a in alerts)
    assert all(not r.exhausted for r in plan.at_risk), "must warn before, not after, exhaustion"


def test_exhaustion_raises_a_critical_alert():
    alerts = pre_exhaustion_alerts(_overloaded())
    assert any(a["alert"] == "OracleCapacityExhausted" and a["severity"] == "critical" for a in alerts)


def test_lag_alert_fires_before_the_target_is_missed():
    plan = project(NOMINAL, lag_seconds=float(LAG_ALERT_SECONDS + 1))
    alerts = pre_exhaustion_alerts(plan)
    lag_alert = next(a for a in alerts if a["alert"] == "OracleCapacityLagApproachingTarget")
    assert lag_alert["severity"] == "warning"
    assert lag_alert["lag_seconds"] < LAG_TARGET_SECONDS


def test_lag_breach_is_critical():
    plan = project(NOMINAL, lag_seconds=float(LAG_TARGET_SECONDS + 30))
    lag_alert = next(a for a in pre_exhaustion_alerts(plan) if a["resource"] == "lag")
    assert lag_alert["severity"] == "critical"
    assert plan.to_dict()["lag_breached"] is True


def test_alerts_carry_the_resource_and_its_target():
    for alert in pre_exhaustion_alerts(_overloaded()):
        assert alert["resource"]
        assert "target" in alert


def test_prometheus_export_carries_headroom_and_lag_for_the_alerts():
    text = prometheus(_overloaded())
    for resource in ("ingest", "storage", "ledger_budget"):
        assert f'oracle_capacity_headroom_ratio{{resource="{resource}"}}' in text
    assert 'oracle_capacity_lag_seconds{resource="lag"}' in text


# ── Serialisation and documentation ──────────────────────────────────────────

def test_plan_serialises_to_json():
    plan = project(NOMINAL)
    plan.shedding = shedding_plan(plan)
    assert json.loads(json.dumps(plan.to_dict()))["lag_target_seconds"] == LAG_TARGET_SECONDS


def test_capacity_doc_documents_every_driver_and_target():
    doc = (Path(__file__).resolve().parents[3] / "docs" / "capacity-planning.md").read_text()
    for name in ("ingest", "storage", "ledger_budget"):
        assert name in doc, f"{name} is not documented in capacity-planning.md"
    for token in ("headroom", "load-shedding", "quarterly", "tolerance"):
        assert token.lower() in doc.lower(), f"capacity-planning.md does not cover {token}"
