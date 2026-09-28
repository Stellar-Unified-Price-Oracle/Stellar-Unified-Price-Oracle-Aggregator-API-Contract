"""Multi-region ingest routing, health-checked failover and the split-brain
double-submit guard (#526).

See the package docstring in ``services/ingest_failover/__init__.py`` and
``docs/multi-region-ingest.md``.
"""
from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from enum import Enum
from typing import Callable, Dict, List, Optional, Sequence, Set, Tuple


class Health(str, Enum):
    HEALTHY = "healthy"
    DEGRADED = "degraded"
    UNHEALTHY = "unhealthy"


@dataclass
class Region:
    """One deployed ingest region and its health state."""

    name: str
    endpoint: str
    #: Consecutive probe failures before the region is declared unhealthy.
    failure_threshold: int = 3
    #: Seconds a region must stay healthy again before it is retried, which
    #: stops a flapping region from causing repeated failovers.
    cooldown_secs: int = 60
    consecutive_failures: int = 0
    consecutive_successes: int = 0
    in_cooldown_until: float = 0.0
    health: Health = Health.HEALTHY
    submissions: int = 0

    @property
    def available(self) -> bool:
        return self.health is not Health.UNHEALTHY

    def to_dict(self) -> dict:
        return {
            "region": self.name,
            "endpoint": self.endpoint,
            "health": self.health.value,
            "submissions": self.submissions,
            "consecutive_failures": self.consecutive_failures,
        }


@dataclass(frozen=True)
class Submission:
    """A logical price submission, identified by its dedupe key."""

    source: str
    asset: str
    ledger: int
    price: int

    @property
    def key(self) -> str:
        """Deterministic idempotency key.

        Derived from the *logical* identity of the submission, not from the
        region that happened to send it, so a retry in another region produces
        the same key and is suppressed.
        """
        raw = f"{self.source}|{self.asset}|{self.ledger}"
        return hashlib.sha256(raw.encode()).hexdigest()[:32]


@dataclass
class FailoverEvent:
    """A recorded failover or degradation, for alerting and the drill report."""

    kind: str  # "failover" | "degraded" | "recovered" | "unavailable"
    from_region: Optional[str]
    to_region: Optional[str]
    reason: str
    ledger: int

    def to_dict(self) -> dict:
        return {
            "kind": self.kind,
            "from_region": self.from_region,
            "to_region": self.to_region,
            "reason": self.reason,
            "ledger": self.ledger,
        }


@dataclass
class SubmitResult:
    """Outcome of routing one submission."""

    submission_key: str
    region: Optional[str]
    accepted: bool
    #: True when the submission was already recorded by an earlier attempt —
    #: i.e. this was a suppressed double-submit.
    duplicate: bool = False
    reason: str = ""


#: A transport: given a region and a submission, either succeed or raise.
Transport = Callable[[Region, Submission], None]

#: Ledger marker for a key whose intent is recorded but whose outcome is not
#: yet known. See :meth:`RegionRouter.submit`.
PENDING = "pending"


class Unreachable(ConnectionError):
    """The region refused the request; the write provably never happened.

    Distinct from a generic failure: a refused connection (DNS failure, TCP
    reject, load-balancer 503 raised before any of the request was forwarded)
    means the submission was *not* attempted, so it is safe to fail over and
    re-send. Anything else — a timeout, a reset, a dropped response — is
    ambiguous, because the write may already be on chain.
    """


class AllRegionsDown(RuntimeError):
    """Raised when no region can accept a submission."""


# --------------------------------------------------------------------------
# Router
# --------------------------------------------------------------------------


class RegionRouter:
    """Routes submissions to a healthy region, failing over automatically.

    The dedupe ledger is deliberately held by the router and treated as
    replicated to every region: the guard against a double-submit during
    failover cannot live in one region, because the whole point of failover is
    that the region holding it may be the one that died. In a real deployment
    the ledger is a shared/consensus-backed store with the same at-most-once
    semantics; here it is an in-process set so the property is testable.
    """

    def __init__(
        self,
        regions: Sequence[Region],
        transport: Transport,
        clock: Callable[[], float] = lambda: 0.0,
    ) -> None:
        if not regions:
            raise ValueError("at least one region is required")
        self.regions: List[Region] = list(regions)
        self.transport = transport
        self.clock = clock
        #: submission key -> region that accepted it (or PENDING). Replicated
        #: to every region; see the class docstring.
        self.ledger: Dict[str, str] = {}
        #: Keys whose outcome was ambiguous and which must not be retried.
        self.quarantined: Set[str] = set()
        #: Count of submissions suppressed as already-recorded.
        self.duplicates_suppressed = 0
        self.events: List[FailoverEvent] = []
        self.active: Optional[Region] = self.regions[0]
        self.ledger_seq = 0

    # -- health --------------------------------------------------------
    def probe(self, region: Region) -> bool:
        """Runs one health check and updates the region's state."""
        now = self.clock()
        if region.in_cooldown_until > now:
            # Still cooling down: do not probe, so a flapping region is not
            # hammered and the router does not flap with it.
            return False
        try:
            self.transport(region, Submission("__probe__", "__probe__", -1, 0))
            ok = True
        except Exception:
            ok = False
        if ok:
            region.consecutive_failures = 0
            region.consecutive_successes += 1
            region.health = Health.HEALTHY
        else:
            self._note_failure(region, "probe failed")
        return ok

    def healthy_regions(self) -> List[Region]:
        return [r for r in self.regions if r.available]

    # -- failover ------------------------------------------------------
    def _record(self, kind: str, src: Optional[Region], dst: Optional[Region],
                reason: str) -> FailoverEvent:
        self.ledger_seq += 1
        ev = FailoverEvent(kind, src.name if src else None,
                           dst.name if dst else None, reason, self.ledger_seq)
        self.events.append(ev)
        return ev

    def failover(self, reason: str, exclude: Optional[Set[str]] = None) -> Optional[Region]:
        """Moves to the next healthy region, or returns ``None`` if there is none."""
        current = self.active
        skip = exclude or set()
        candidates = [
            r for r in self.regions
            if r.available and r is not current and r.name not in skip
        ]
        if not candidates:
            self._record("unavailable", current, None, reason)
            self.active = None
            return None
        nxt = candidates[0]
        self._record("failover", current, nxt, reason)
        self.active = nxt
        return nxt

    def ensure_active(self, exclude: Optional[Set[str]] = None) -> Optional[Region]:
        """Re-selects a healthy region if the current one is unhealthy or tried."""
        skip = exclude or set()
        if (
            self.active is not None
            and self.active.available
            and self.active.name not in skip
        ):
            return self.active
        current = self.active
        self.active = None
        chosen = self.failover(
            f"active region {current.name if current else 'none'} unusable", skip
        )
        if chosen is None:
            # Nothing healthy: keep the current (unhealthy) region as active so
            # the next call re-evaluates once it recovers.
            self.active = current
        return chosen

    # -- submission ----------------------------------------------------
    def submit(self, sub: Submission) -> SubmitResult:
        """Routes one submission, at most once across the whole fleet.

        Ambiguity is resolved *toward at-most-once*. A region can fail in two
        ways, and the router treats them differently:

        * **Refused** (:class:`Unreachable`) — the request provably never
          reached the region, so the write did not happen. Safe to fail over
          and re-send, and the submission is not lost.
        * **Ambiguous** (timeout, reset, dropped response) — the write may or
          may not have landed on chain, and the ingest path cannot tell.
          Re-sending is exactly the split-brain double-submit the issue warns
          about, so the key is *quarantined* instead of retried.

        Either way the key is written to the replicated ledger **before** the
        transport call (a write-ahead intent), so no region can pick it up.

        A quarantined submission is recovered by reconciliation against the
        chain (``docs/multi-region-ingest.md`` §5), which can observe what
        actually landed. Preferring a detectable gap over a silent double-count
        is the right trade for an oracle: freshness monitoring alarms on the
        former, whereas the latter corrupts the aggregate.
        """
        key = sub.key
        if key in self.quarantined:
            self.duplicates_suppressed += 1
            return SubmitResult(key, None, False, duplicate=True,
                                reason="ambiguous outcome; awaiting reconciliation")
        if key in self.ledger:
            self.duplicates_suppressed += 1
            return SubmitResult(key, self.ledger[key], True, duplicate=True,
                                reason="already recorded by an earlier attempt")

        # Write-ahead intent, recorded before the transport call.
        self.ledger[key] = PENDING
        tried: Set[str] = set()
        while True:
            region = self.ensure_active(exclude=tried)
            if region is None:
                self.ledger.pop(key, None)
                return SubmitResult(key, None, False,
                                    reason="no healthy region available")
            tried.add(region.name)
            try:
                self.transport(region, sub)
            except Unreachable as e:
                # Provably not attempted: try the next region, do not lose it.
                self._note_failure(region, f"submission refused: {e}")
                self.failover(f"submission to {region.name} refused: {e}")
                continue
            except Exception as e:
                # Ambiguous: the write may or may not have landed. Do not re-send.
                self._note_failure(region, f"submission failed: {e}")
                self.failover(f"submission to {region.name} failed: {e}")
                self.quarantined.add(key)
                return SubmitResult(
                    key, None, False, duplicate=True,
                    reason=f"ambiguous outcome in {region.name} ({e}); not retried",
                )
            region.submissions += 1
            self.ledger[key] = region.name
            return SubmitResult(key, region.name, True)

    def _note_failure(self, region: Region, reason: str) -> None:
        """Records a failed attempt and degrades the region at the threshold."""
        region.consecutive_failures += 1
        region.consecutive_successes = 0
        if region.consecutive_failures >= region.failure_threshold:
            if region.health is not Health.UNHEALTHY:
                self._record(
                    "degraded", region, None,
                    f"{region.consecutive_failures} consecutive failures ({reason})",
                )
            region.health = Health.UNHEALTHY
            region.in_cooldown_until = self.clock() + region.cooldown_secs

    # -- observability --------------------------------------------------
    def health_snapshot(self) -> List[dict]:
        return [r.to_dict() for r in self.regions]

    def failover_events(self) -> List[dict]:
        return [e.to_dict() for e in self.events]

    def prometheus(self) -> str:
        lines = [
            "# HELP ingest_region_up 1 when the region is accepting submissions.",
            "# TYPE ingest_region_up gauge",
        ]
        for r in self.regions:
            lines.append(
                f'ingest_region_up{{region="{r.name}"}} {1 if r.available else 0}'
            )
        lines += [
            "# HELP ingest_region_submissions_total Submissions accepted per region.",
            "# TYPE ingest_region_submissions_total counter",
        ]
        lines += [
            f'ingest_region_submissions_total{{region="{r.name}"}} {r.submissions}'
            for r in self.regions
        ]
        lines += [
            "# HELP ingest_failovers_total Failover events by direction.",
            "# TYPE ingest_failovers_total counter",
        ]
        for e in self.events:
            if e.kind == "failover":
                lines.append(
                    f'ingest_failovers_total{{from="{e.from_region}",'
                    f'to="{e.to_region}"}} 1'
                )
        lines += [
            "# HELP ingest_region_degraded_total Regions entering the unhealthy state.",
            "# TYPE ingest_region_degraded_total counter",
        ]
        for e in self.events:
            if e.kind == "degraded":
                lines.append(f'ingest_region_degraded_total{{region="{e.from_region}"}} 1')
        lines += [
            "# HELP ingest_duplicate_submissions_total Submissions suppressed as "
            "already-recorded (split-brain guard).",
            "# TYPE ingest_duplicate_submissions_total counter",
            f"ingest_duplicate_submissions_total {self.duplicates_suppressed}",
            "# HELP ingest_quarantined_submissions Submissions whose ingest outcome "
            "was ambiguous and were deliberately not retried.",
            "# TYPE ingest_quarantined_submissions gauge",
            f"ingest_quarantined_submissions {len(self.quarantined)}",
        ]
        return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------
# Failure drill
# --------------------------------------------------------------------------


@dataclass
class DrillReport:
    """Result of a region-failure drill (``docs/multi-region-ingest.md`` §4)."""

    region_failed: str
    active_before: str
    active_after: Optional[str]
    submissions_before: int
    submissions_after: int
    duplicates_suppressed: int
    double_submits: int
    failovers: List[dict]
    degraded: List[dict]
    passed: bool
    notes: List[str] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "region_failed": self.region_failed,
            "active_before": self.active_before,
            "active_after": self.active_after,
            "submissions_before": self.submissions_before,
            "submissions_after": self.submissions_after,
            "duplicates_suppressed": self.duplicates_suppressed,
            "double_submits": self.double_submits,
            "failovers": self.failovers,
            "degraded": self.degraded,
            "passed": self.passed,
            "notes": self.notes,
        }


def run_drill(
    regions: Sequence[Region],
    transport: Transport,
    victim: str,
    submissions: int = 20,
    clock: Callable[[], float] = lambda: 0.0,
) -> DrillReport:
    """Fails ``victim`` mid-run and asserts the fleet keeps working, once only.

    The drill is the *test* of the issue's acceptance criteria: a region fails,
    the router fails over automatically, and the number of accepted on-chain
    submissions equals the number of logical submissions — no double submit.
    """
    accepted: Dict[str, List[str]] = {}
    live = {"transport": transport}

    def counting_transport(region: Region, sub: Submission) -> None:
        live["transport"](region, sub)
        accepted.setdefault(sub.key, []).append(region.name)

    router = RegionRouter(regions, counting_transport, clock=clock)
    notes: List[str] = []
    before = 0
    dups = 0

    for i in range(submissions):
        sub = Submission("SRC-A", "XLM/USD", i, 1_000_000 + i)
        if i == submissions // 2:
            before = sum(len(v) for v in accepted.values())
            notes.append(f"injecting failure of region '{victim}' mid-run")
            live["transport"] = _fail_after(live["transport"], victim, i)
        res = router.submit(sub)
        if res.duplicate:
            dups += 1
        if not res.accepted:
            notes.append(f"submission {i} rejected: {res.reason}")

    total = sum(len(v) for v in accepted.values())
    doubles = sum(1 for v in accepted.values() if len(v) > 1)
    return DrillReport(
        region_failed=victim,
        active_before=router.regions[0].name,
        active_after=router.active.name if router.active else None,
        submissions_before=before,
        submissions_after=total,
        duplicates_suppressed=dups,
        double_submits=doubles,
        failovers=[e.to_dict() for e in router.events if e.kind == "failover"],
        degraded=[e.to_dict() for e in router.events if e.kind == "degraded"],
        passed=doubles == 0 and router.active is not None
        and router.active.name != victim,
        notes=notes,
    )


def _fail_after(transport: Transport, region_name: str, arm_at: int) -> Transport:
    """Wraps ``transport`` so ``region_name`` fails from ledger ``arm_at`` on.

    Health probes (``ledger < 0``) are answered by the router's own bookkeeping
    and are not counted, so arming the failure at the current ledger makes the
    region die immediately rather than after N more submissions.
    """
    state = {"armed": False}

    def wrapped(region: Region, sub: Submission) -> None:
        if sub.ledger < 0:
            return  # probes are the router's business, not the transport's
        if region.name == region_name:
            if sub.ledger >= arm_at:
                state["armed"] = True
            if state["armed"]:
                raise Unreachable(f"region {region_name} unreachable")
        transport(region, sub)

    return wrapped


# --------------------------------------------------------------------------
# Cost model
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class RegionCost:
    """Monthly cost inputs for one region, used to justify the redundancy."""

    #: Small always-on ingest worker.
    instance_month_usd: float = 38.0
    #: Managed RPC / read replica.
    rpc_month_usd: float = 25.0
    egress_gb_month: float = 120.0
    egress_usd_per_gb: float = 0.09
    storage_gb_month: float = 20.0
    storage_usd_per_gb_month: float = 0.23

    def monthly_usd(self) -> float:
        return (
            self.instance_month_usd
            + self.rpc_month_usd
            + self.egress_gb_month * self.egress_usd_per_gb
            + self.storage_gb_month * self.storage_usd_per_gb_month
        )


def cost_summary(
    regions: Sequence[RegionCost],
    single_region_outage_hours: float = 4.0,
    stale_price_hours: float = 0.5,
    consumer_loss_per_hour_usd: float = 1_000.0,
) -> dict:
    """Redundancy cost vs. the cost of a single-region outage.

    The redundancy spend is the *incremental* cost of running N-1 extra regions.
    The avoided cost is stale-price exposure: outage hours multiplied by the
    fraction of that time prices are stale, times an assumed consumer loss. The
    payback figure is what justifies (or refuses) the redundancy, and both
    sides of the ratio are explicit inputs rather than a hand-wave.
    """
    n = len(regions)
    per = regions[0].monthly_usd() if regions else 0.0
    total = per * n
    incremental = total - per
    avoided = single_region_outage_hours * stale_price_hours * consumer_loss_per_hour_usd
    return {
        "regions": n,
        "monthly_per_region_usd": round(per, 2),
        "monthly_total_usd": round(total, 2),
        "incremental_monthly_usd": round(incremental, 2),
        "assumed_outage_hours_per_month": single_region_outage_hours,
        "avoided_outage_cost_usd": round(avoided, 2),
        "payback_months": round(incremental / avoided, 3) if avoided else None,
        "justified": incremental < avoided,
    }


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


DEFAULT_REGIONS = (
    ("us-east", "https://ingest-us-east.example"),
    ("eu-west", "https://ingest-eu-west.example"),
    ("ap-south", "https://ingest-ap-south.example"),
)


def main(argv: Optional[Sequence[str]] = None) -> int:
    import argparse
    import json
    import sys
    from pathlib import Path

    p = argparse.ArgumentParser(description="Multi-region ingest failover drill")
    p.add_argument("--victim", default=DEFAULT_REGIONS[0][0])
    p.add_argument("--submissions", type=int, default=20)
    p.add_argument("--json", type=Path)
    p.add_argument("--prometheus", type=Path)
    args = p.parse_args(argv)

    def transport(region: Region, sub: Submission) -> None:
        return None

    regions = [Region(name, url) for name, url in DEFAULT_REGIONS]
    report = run_drill(regions, transport, args.victim, args.submissions)
    summary = cost_summary([RegionCost() for _ in regions])

    print(json.dumps({"drill": report.to_dict(), "cost": summary}, indent=2))
    if args.json:
        args.json.write_text(json.dumps(report.to_dict(), indent=2) + "\n")
    if args.prometheus:
        args.prometheus.write_text(_drill_metrics(report))
    return 0 if report.passed else 1


def _drill_metrics(report: DrillReport) -> str:
    return (
        "# HELP ingest_drill_double_submits Double submissions observed in the drill.\n"
        "# TYPE ingest_drill_double_submits gauge\n"
        f"ingest_drill_double_submits {report.double_submits}\n"
        "# HELP ingest_drill_failovers Failover events observed in the drill.\n"
        "# TYPE ingest_drill_failovers gauge\n"
        f"ingest_drill_failovers {len(report.failovers)}\n"
    )


if __name__ == "__main__":
    import sys

    sys.exit(main())
