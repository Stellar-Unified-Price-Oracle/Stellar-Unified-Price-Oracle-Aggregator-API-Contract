from __future__ import annotations

from pathlib import Path

from services.sla_monitor.monitor import (
    SLA_CLAUSES,
    MonitorConfig,
    Round,
    SlaMonitor,
    check_sla_consistency,
    recompute_median,
)

SLA_MD = Path(__file__).resolve().parents[3] / "docs" / "SLA.md"
SOURCES = {"A", "B", "C", "D", "E"}


def _round(ledger, counted, published=None, **kw):
    return Round(
        asset="XLM/USD",
        ledger=ledger,
        timestamp=ledger * 5,
        published_price=recompute_median(counted.values()) if published is None else published,
        counted=counted,
        admitted=kw.pop("admitted", set(SOURCES)),
        **kw,
    )


def _clauses(vs):
    return {v.clause for v in vs}


def test_sla_md_and_monitor_are_consistent():
    assert check_sla_consistency(SLA_MD.read_text()) == []


def test_consistency_check_detects_drift():
    drifted = SLA_MD.read_text().replace("| §9.5 |", "| §9.9 |")
    errors = check_sla_consistency(drifted)
    assert any("9.5" in e for e in errors) and any("9.9" in e for e in errors)


def test_recompute_matches_contract_median():
    assert recompute_median([3, 1, 2]) == 2
    assert recompute_median([1, 4]) == 2
    assert recompute_median([]) == 0


def test_minority_manipulation_fires_with_full_uptime():
    """Two of five sources shift the median; uptime and freshness stay green."""
    mon = SlaMonitor()
    honest = 1_000_000
    counted = {"A": honest, "B": honest, "C": 1_100_000, "D": 1_200_000, "E": 1_200_000}
    vs = mon.observe(_round(1, counted, fresh=True, reference_prices=[honest] * 3))
    fired = _clauses(vs)
    # Availability-style clauses stay green...
    assert not fired & {"1.2", "2", "4.1", "4.2", "9.1"}
    # ...but agreement with the reference set catches the plausible aggregate.
    assert "3" in fired


def test_divergence_detected_in_same_ledger():
    mon = SlaMonitor()
    vs = mon.observe(_round(42, {"A": 100, "B": 110, "C": 120}, published=115))
    assert [(v.clause, v.ledger) for v in vs if v.clause == "9.1"] == [("9.1", 42)]


def test_flatline_while_inputs_move():
    mon = SlaMonitor(MonitorConfig(flatline_rounds=3))
    out = []
    for i in range(3):
        out += mon.observe(_round(i, {"A": 100 + i, "B": 110 - i}, published=105))
    assert "9.2" in _clauses(out)


def test_admitted_but_never_counted_and_unadmitted_influence():
    mon = SlaMonitor(MonitorConfig(participation_window=3))
    out = []
    for i in range(3):
        out += mon.observe(_round(i, {"A": 100, "B": 100, "X": 100}, admitted={"A", "B", "C"}))
    assert "9.4" in _clauses(out)  # C admitted, never counted
    assert "9.5" in _clauses(out)  # X counted without admission


def test_inverse_relationship_violation():
    mon = SlaMonitor()
    mon.observe(Round("XLM/USD", 1, 0, 2_000_000, {"A": 2_000_000, "B": 2_000_000}, admitted={"A", "B"}))
    mon.observe(Round("USD/XLM", 1, 0, 60_000_000, {"A": 60_000_000, "B": 60_000_000}, admitted={"A", "B"}))
    vs = mon.check_relationships(1, [("inverse", "XLM/USD", "USD/XLM", "")])
    assert _clauses(vs) == {"9.3"}


def test_burn_rate_alert_and_report_references_clause():
    mon = SlaMonitor(MonitorConfig(burn_short_window=2, burn_long_window=4))
    for i in range(4):
        mon.observe(_round(i, {"A": 100, "B": 100}, fresh=False))
    assert "4.1" in _clauses(mon.violations)
    report = mon.report()
    assert "§4.1" in report and "§1.2" in report
    assert set(SLA_CLAUSES) <= {
        line.split('"')[1] for line in mon.prometheus().splitlines() if "clause=" in line
    }
