from __future__ import annotations

import pytest

from services.ingest_failover.router import (
    Health,
    Region,
    RegionCost,
    RegionRouter,
    Submission,
    Unreachable,
    cost_summary,
    run_drill,
)

REGIONS = ("us-east", "eu-west", "ap-south")


def make_regions(**kw) -> list:
    return [Region(name, f"https://{name}.example", **kw) for name in REGIONS]


class Chaos:
    """Transport whose failing regions can be toggled at will."""

    def __init__(self) -> None:
        self.down: set = set()
        self.accepted: list = []

    def __call__(self, region: Region, sub: Submission) -> None:
        if region.name in self.down:
            # A region that is simply down refuses the request: nothing was
            # attempted, so re-sending elsewhere is safe.
            raise Unreachable(f"{region.name} unreachable")
        if sub.ledger >= 0:
            self.accepted.append((sub.key, region.name))


def test_submission_key_is_deterministic_and_region_independent():
    a = Submission("SRC-A", "XLM/USD", 42, 100)
    assert a.key == Submission("SRC-A", "XLM/USD", 42, 100).key
    # A different logical submission must have a different key.
    assert a.key != Submission("SRC-A", "XLM/USD", 43, 100).key
    assert a.key != Submission("SRC-B", "XLM/USD", 42, 100).key


def test_resubmitting_the_same_logical_submission_is_suppressed():
    """The same submission retried after an ambiguous failure is not re-sent."""
    chaos = Chaos()
    router = RegionRouter(make_regions(), chaos)
    sub = Submission("SRC-A", "XLM/USD", 1, 100)
    first = router.submit(sub)
    assert first.accepted and not first.duplicate
    second = router.submit(sub)
    assert second.accepted and second.duplicate
    assert len(chaos.accepted) == 1, "the transport was called twice for one submission"


def test_region_failure_fails_over_automatically():
    chaos = Chaos()
    chaos.down.add("us-east")
    router = RegionRouter(make_regions(), chaos)
    res = router.submit(Submission("SRC-A", "XLM/USD", 1, 100))
    assert res.accepted
    assert res.region == "eu-west"
    assert router.active.name == "eu-west"
    assert any(e.kind == "failover" for e in router.events)


def test_failover_does_not_double_submit_the_ambiguous_submission():
    """us-east dies mid-submit: the retry must not also land on chain.

    The write may or may not have reached the chain, so the router must not
    re-send it into a second region. The key is quarantined instead.
    """
    chaos = Chaos()

    def flaky(region: Region, sub: Submission) -> None:
        if region.name == "us-east":
            # The region accepts the write, then dies before reporting back —
            # the classic ambiguous outcome.
            chaos.accepted.append((sub.key, region.name))
            raise ConnectionError("connection reset after write")
        chaos(region, sub)

    router = RegionRouter(make_regions(), flaky)
    sub = Submission("SRC-A", "XLM/USD", 7, 100)
    res = router.submit(sub)
    # The submission is reported as not accepted (we do not know), and the
    # transport was called exactly once — never a second time in eu-west.
    assert not res.accepted
    assert res.duplicate
    assert "ambiguous" in res.reason
    assert len(chaos.accepted) == 1
    assert sub.key in router.quarantined


def test_a_quarantined_submission_is_never_retried_automatically():
    chaos = Chaos()

    def flaky(region: Region, sub: Submission) -> None:
        if region.name == "us-east":
            chaos.accepted.append((sub.key, region.name))
            raise ConnectionError("reset after write")
        chaos(region, sub)

    router = RegionRouter(make_regions(), flaky)
    sub = Submission("SRC-A", "XLM/USD", 7, 100)
    router.submit(sub)
    before = len(chaos.accepted)
    # Re-offering the same logical submission must be refused, not re-sent.
    res = router.submit(sub)
    assert res.duplicate and not res.accepted
    assert "reconciliation" in res.reason
    assert len(chaos.accepted) == before


def test_failover_still_serves_subsequent_submissions():
    """One ambiguous loss must not stall the whole ingest path."""
    chaos = Chaos()

    def flaky(region: Region, sub: Submission) -> None:
        if region.name == "us-east":
            chaos.accepted.append((sub.key, region.name))
            raise ConnectionError("reset after write")
        chaos(region, sub)

    router = RegionRouter(make_regions(), flaky)
    router.submit(Submission("SRC-A", "XLM/USD", 1, 100))
    for ledger in range(2, 6):
        res = router.submit(Submission("SRC-A", "XLM/USD", ledger, 100 + ledger))
        assert res.accepted
        assert res.region == "eu-west"


def test_a_retry_after_failover_is_deduped_by_the_replicated_ledger():
    chaos = Chaos()
    router = RegionRouter(make_regions(), chaos)
    sub = Submission("SRC-A", "XLM/USD", 3, 100)
    router.submit(sub)
    chaos.down.add("us-east")
    # Same logical submission arriving again while the primary is down.
    assert router.submit(sub).duplicate
    assert len(chaos.accepted) == 1


def test_health_threshold_marks_a_region_unhealthy_and_records_degradation():
    chaos = Chaos()
    chaos.down.add("eu-west")
    regions = make_regions(failure_threshold=3, cooldown_secs=30)
    router = RegionRouter(regions, chaos, clock=lambda: 0.0)
    eu = regions[1]
    for _ in range(3):
        router.probe(eu)
    assert eu.health is Health.UNHEALTHY
    assert any(e.kind == "degraded" and e.from_region == "eu-west" for e in router.events)
    assert eu not in router.healthy_regions()


def test_a_single_failed_probe_does_not_trip_failover():
    chaos = Chaos()
    chaos.down.add("eu-west")
    regions = make_regions(failure_threshold=3)
    router = RegionRouter(regions, chaos)
    eu = regions[1]
    router.probe(eu)
    assert eu.health is Health.HEALTHY
    assert eu.consecutive_failures == 1


def test_cooldown_prevents_hammering_and_flapping():
    clock = {"t": 0.0}
    chaos = Chaos()
    chaos.down.add("eu-west")
    regions = make_regions(failure_threshold=1, cooldown_secs=60)
    router = RegionRouter(regions, chaos, clock=lambda: clock["t"])
    eu = regions[1]
    router.probe(eu)
    assert eu.health is Health.UNHEALTHY
    # Still down and still cooling down: probing again is a no-op.
    router.probe(eu)
    assert eu.health is Health.UNHEALTHY
    clock["t"] = 61.0
    router.probe(eu)
    assert eu.health is Health.UNHEALTHY


def test_recovery_returns_a_region_to_the_pool():
    clock = {"t": 0.0}
    chaos = Chaos()
    regions = make_regions(failure_threshold=1, cooldown_secs=10)
    router = RegionRouter(regions, chaos, clock=lambda: clock["t"])
    eu = regions[1]
    chaos.down.add("eu-west")
    router.probe(eu)
    assert eu.health is Health.UNHEALTHY
    chaos.down.clear()
    clock["t"] = 11.0
    assert router.probe(eu) is True
    assert eu.health is Health.HEALTHY
    assert eu in router.healthy_regions()


def test_total_outage_is_reported_rather_than_silently_dropped():
    chaos = Chaos()
    chaos.down.update(REGIONS)
    router = RegionRouter(make_regions(failure_threshold=1), chaos)
    res = router.submit(Submission("SRC-A", "XLM/USD", 1, 100))
    assert not res.accepted
    assert res.region is None
    assert "no healthy region" in res.reason
    assert any(e.kind == "unavailable" for e in router.events)


def test_failover_events_and_health_are_observable():
    chaos = Chaos()
    chaos.down.add("us-east")
    router = RegionRouter(make_regions(), chaos)
    router.submit(Submission("SRC-A", "XLM/USD", 1, 100))
    events = router.failover_events()
    assert events[0]["from_region"] == "us-east"
    assert events[0]["to_region"] == "eu-west"
    snapshot = {r["region"]: r["health"] for r in router.health_snapshot()}
    assert snapshot["eu-west"] == "healthy"
    text = router.prometheus()
    assert 'ingest_region_up{region="eu-west"} 1' in text
    assert 'ingest_failovers_total{from="us-east",to="eu-west"} 1' in text
    assert "ingest_quarantined_submissions 0" in text


def test_quarantined_submissions_are_exported_for_alerting():
    chaos = Chaos()

    def flaky(region: Region, sub: Submission) -> None:
        if region.name == "us-east":
            chaos.accepted.append((sub.key, region.name))
            raise ConnectionError("reset after write")
        chaos(region, sub)

    router = RegionRouter(make_regions(), flaky)
    router.submit(Submission("SRC-A", "XLM/USD", 1, 100))
    text = router.prometheus()
    # The IngestAmbiguousSubmissionsQuarantined alert fires off this metric.
    assert "ingest_quarantined_submissions 1" in text


# -- the drill (issue acceptance criteria) --------------------------------


def test_drill_region_failure_fails_over_without_double_submission():
    """The issue's headline criteria, demonstrated rather than asserted."""
    report = run_drill(make_regions(), lambda r, s: None, victim="us-east")
    assert report.passed
    assert report.double_submits == 0
    assert report.active_before == "us-east"
    assert report.active_after == "eu-west"
    assert report.failovers, "the drill recorded no failover"
    # Every logical submission landed exactly once, across the region change.
    assert report.submissions_after == 20


@pytest.mark.parametrize("victim", REGIONS)
def test_drill_passes_for_every_region(victim):
    report = run_drill(make_regions(), lambda r, s: None, victim=victim)
    assert report.passed, report.to_dict()
    assert report.double_submits == 0


def test_drill_costs_are_measured_and_justified():
    summary = cost_summary([RegionCost() for _ in REGIONS])
    assert summary["regions"] == 3
    assert summary["incremental_monthly_usd"] > 0
    # Redundancy costs less than the outage exposure it removes.
    assert summary["justified"]
    assert summary["payback_months"] < 1


def test_cost_model_refuses_redundancy_that_cannot_pay_back():
    summary = cost_summary(
        [RegionCost(instance_month_usd=5_000.0) for _ in REGIONS],
        single_region_outage_hours=1.0,
    )
    assert not summary["justified"]


def test_cli_drill_exits_zero(tmp_path, capsys):
    from services.ingest_failover.router import main

    out = tmp_path / "drill.json"
    rc = main(["--victim", "eu-west", "--json", str(out)])
    assert rc == 0
    assert out.read_text().count('"double_submits": 0') == 1
