import pytest

from services.dr_gameday.gameday import (
    SCENARIOS, ProductionTargetError, Sandbox, Scenario, issue_drafts, run_all, run_drill,
)


def test_every_scenario_recovers_within_sla():
    report = run_all(lambda n: Sandbox(id=f"gameday-{n}"))
    assert {r["scenario"] for r in report["results"]} == {s.name for s in SCENARIOS}
    assert report["regressions"] == 0 and report["issues"] == []


def test_refuses_production():
    with pytest.raises(ProductionTargetError):
        run_drill(SCENARIOS[0], Sandbox(id="gameday-x", network="mainnet"))
    with pytest.raises(ProductionTargetError):
        run_drill(SCENARIOS[0], Sandbox(id="prod"))


def test_slow_recovery_is_flagged_and_filed():
    ticks = iter([0.0, 10_000.0])
    r = run_drill(SCENARIOS[0], Sandbox(id="gameday-a"), clock=lambda: next(ticks))
    assert any("RTO" in g for g in r.gaps)
    assert len(issue_drafts([r])) == len(r.gaps)


def test_broken_step_is_a_gap():
    def boom(sb):
        raise RuntimeError("no key")
    s = Scenario("key_unavailability", lambda sb: None, [boom], lambda sb: True)
    r = run_drill(s, Sandbox(id="gameday-k"))
    assert not r.recovered and r.gaps[0].startswith("recovery step failed")
