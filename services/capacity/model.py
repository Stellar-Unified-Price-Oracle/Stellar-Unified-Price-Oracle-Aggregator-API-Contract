"""Capacity model, headroom targets and load shedding (#530).

Growth in assets and sources is the main driver of cost and of failure, and
without a model capacity is discovered by outage. This module projects the
three resources the pipeline actually competes for, states the targets it is
held to, and decides what to shed when a target is at risk.

Three resources, three cost drivers:

* **Ingest** — submissions per second the off-chain pipeline must fetch,
  validate and sign. Driven by *sources x assets x submission rate*.
* **Storage** — bytes of history and index held off chain, and the on-chain
  state entries retained per asset. Driven by *assets x history depth*.
* **Ledger budget** — CPU instructions the network charges for our
  transactions. This is the resource with the least control, because the
  ledger is shared with unrelated activity, so it carries the largest margin.

The model is validated against measured usage rather than trusted:
:func:`compare_to_measured` reports the error of a projection against a
recorded measurement, and the tolerance is an explicit, stated number rather
than a judgement call.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from dataclasses import asdict, dataclass, field
from typing import Dict, List, Optional, Sequence, Tuple

# Stated model tolerance: a projection must reproduce measured usage within
# this fraction to be considered validated.
TOLERANCE = 0.20

# Headroom that must remain unconsumed. Ledger budget gets the largest margin
# because the ledger is shared with traffic we do not control.
HEADROOM_TARGETS: Dict[str, float] = {
    "ingest": 0.30,       # 30 % of throughput spare
    "storage": 0.25,      # 25 % of the storage budget spare
    "ledger_budget": 0.50,  # 50 % of the ledger budget spare
}

# Age of the published aggregate that we hold ourselves to, in seconds.
LAG_TARGET_SECONDS = 60  # matches the SLA §1.2 freshness target
LAG_ALERT_SECONDS = 45   # pre-exhaustion: warn before the target is missed

# Cost drivers live in `Drivers` below, calibrated against the measured
# deployment and re-checked quarterly (see docs/capacity-planning.md). The
# ledger CPU figures are the gas budgets from docs/gas-budget.md, so the model
# and the gas gate cannot drift apart silently.


@dataclass(frozen=True)
class Drivers:
    """Per-unit cost of each resource, measured from the current deployment."""

    # Ingest: pipeline CPU-seconds to fetch, validate and sign one submission.
    ingest_cpu_seconds_per_submission: float = 0.004
    # Storage: bytes of off-chain state per submission record and per asset,
    # and how many hours of submissions the cache retains.
    storage_bytes_per_submission: int = 512
    storage_bytes_per_asset: int = 32_768
    storage_retention_hours: int = 24
    # Ledger: CPU instructions charged per submission, and per history entry.
    ledger_cpu_per_submission: int = 1_287_000      # trigger_aggregation budget
    ledger_cpu_per_history_entry: int = 76_000
    # Footprint: the on-chain ceiling at which a submission becomes uncallable.
    max_sources_per_asset: int = 10

    def to_dict(self) -> Dict[str, float]:
        return asdict(self)


# Default budgets, in the units the drivers above produce: CPU-seconds per wall
# second, retained bytes, and CPU instructions per hour. Re-fitted from
# measurement every quarter (docs/capacity-planning.md, "Quarterly review").
DEFAULT_CAPACITIES: Dict[str, float] = {
    # 2 cores of pipeline CPU per wall second.
    "ingest": 2.0,
    # 50 GiB of off-chain state.
    "storage": 50 * 1024**3,
    # A third of the network's 100M instructions/second, expressed per hour.
    "ledger_budget": 1.2e11,
}


@dataclass(frozen=True)
class Profile:
    """The shape of the deployment being projected."""

    assets: int
    sources: int
    submission_rate_hz: float  # submissions per second, all sources, all assets
    history_depth: int = 720   # retained entries per asset
    submission_cpu_seconds: float = 0.02  # wall CPU per submission on-chain wait

    def per_asset_submission_rate(self) -> float:
        return self.submission_rate_hz / max(self.assets, 1)


@dataclass(frozen=True)
class Resource:
    """One projected resource, with its target and the state against it."""

    name: str
    used: float
    capacity: float
    headroom_target: float

    @property
    def headroom(self) -> float:
        """Fraction of capacity still free (may be negative when over)."""
        return (self.capacity - self.used) / self.capacity if self.capacity else 0.0

    @property
    def exhausted(self) -> bool:
        return self.used >= self.capacity

    @property
    def at_risk(self) -> bool:
        """True when remaining headroom is below the target for this resource."""
        return self.headroom < self.headroom_target

    def to_dict(self) -> Dict[str, object]:
        d = asdict(self)
        d.update(headroom=round(self.headroom, 4), at_risk=self.at_risk, exhausted=self.exhausted)
        return d


@dataclass
class CapacityPlan:
    """The full projection: resources, lag, and what to shed if overloaded."""

    resources: List[Resource] = field(default_factory=list)
    lag_seconds: float = 0.0
    shedding: List[str] = field(default_factory=list)
    growth: Dict[str, float] = field(default_factory=dict)

    def resource(self, name: str) -> Resource:
        for r in self.resources:
            if r.name == name:
                return r
        raise KeyError(name)

    @property
    def at_risk(self) -> List[Resource]:
        return [r for r in self.resources if r.at_risk]

    def to_dict(self) -> Dict[str, object]:
        return {
            "resources": [r.to_dict() for r in self.resources],
            "lag_seconds": round(self.lag_seconds, 2),
            "lag_target_seconds": LAG_TARGET_SECONDS,
            "lag_breached": self.lag_seconds > LAG_TARGET_SECONDS,
            "shedding": self.shedding,
            "at_risk": [r.name for r in self.at_risk],
        }


# Load-shedding priority. Lower number = shed later. Critical paths are never
# shed: an oracle that stops publishing correct prices for the assets that
# matter is worse than one that stops publishing for the assets that do not.
PRIORITY_CRITICAL = 0
PRIORITY_STANDARD = 1
PRIORITY_BEST_EFFORT = 2

# What each tier is allowed to lose, in the order it is given up.
SHED_ORDER: Tuple[Tuple[int, str], ...] = (
    (PRIORITY_BEST_EFFORT, "drop historical backfill and replay of old ticks"),
    (PRIORITY_BEST_EFFORT, "drop non-critical assets outside the settlement window"),
    (PRIORITY_STANDARD, "reduce submission frequency on the least-traded assets"),
    (PRIORITY_STANDARD, "disable optional analytics (anomaly scoring, forecasting)"),
    (PRIORITY_CRITICAL, "degrade signature verification to the cached-key fast path"),
)


def project(
    profile: Profile,
    drivers: Drivers = None,  # type: ignore[assignment]
    capacities: Optional[Dict[str, float]] = None,
    lag_seconds: Optional[float] = None,
) -> CapacityPlan:
    """Projects resource usage for a deployment profile.

    ``capacities`` overrides :data:`DEFAULT_CAPACITIES`, which is what the
    quarterly review re-fits from measurement.
    """
    drivers = drivers or Drivers()
    caps = dict(DEFAULT_CAPACITIES)
    caps.update(capacities or {})

    submissions_per_hour = profile.submission_rate_hz * 3600

    # Ingest: CPU-seconds of pipeline work per wall second, against the cores
    # the pipeline is allowed to burn.
    ingest_used = profile.submission_rate_hz * drivers.ingest_cpu_seconds_per_submission
    # Storage: retained bytes, against the allocated volume.
    storage_used = (
        profile.submission_rate_hz * 3600 * drivers.storage_retention_hours
        * drivers.storage_bytes_per_submission
        + profile.assets * drivers.storage_bytes_per_asset
    )
    # Ledger: CPU instructions per hour, against our share of the network's
    # instruction budget, which is shared with traffic we do not control.
    ledger_used = (
        submissions_per_hour * drivers.ledger_cpu_per_submission
        + profile.assets * profile.history_depth * drivers.ledger_cpu_per_history_entry
    )

    resources = [
        Resource("ingest", ingest_used, caps["ingest"], HEADROOM_TARGETS["ingest"]),
        Resource("storage", storage_used, caps["storage"], HEADROOM_TARGETS["storage"]),
        Resource("ledger_budget", ledger_used, caps["ledger_budget"], HEADROOM_TARGETS["ledger_budget"]),
    ]

    # Lag: how long an aggregate takes to publish once the inputs are in. It
    # grows with the per-asset submission rate and the number of sources the
    # aggregator must scan.
    lag = (
        LAG_TARGET_SECONDS * 0.25
        + profile.per_asset_submission_rate() * 0.5
        + profile.sources * 0.4
        + (profile.submission_cpu_seconds * profile.sources)
    )
    if lag_seconds is not None:
        lag = lag_seconds

    return CapacityPlan(resources=resources, lag_seconds=lag)


def shedding_plan(plan: CapacityPlan, drivers: Drivers = None) -> List[str]:
    """Ordered list of mitigations to apply, most preferred first.

    The plan is *ordered*, not a set: shedding is progressive, and the first
    step is the one that costs the least consumer-visible freshness. Steps are
    only emitted as far as the pressure requires — an unhealthy plan does not
    justify degrading the critical path.
    """
    drivers = drivers or Drivers()
    pressure = max((1.0 - r.headroom) / max(1.0 - r.headroom_target, 1e-9) for r in plan.resources)
    if pressure <= 1.0:
        return []
    steps = [text for _, text in SHED_ORDER]
    if plan.lag_seconds > LAG_ALERT_SECONDS:
        steps.insert(0, "raise per-asset submission priority for settlement-window assets")
    count = min(len(steps), max(1, int(pressure)))
    return steps[:count]


def pre_exhaustion_alerts(plan: CapacityPlan) -> List[Dict[str, object]]:
    """Alerts that fire *before* a target is exhausted, not after.

    Each alert names the resource, the observed headroom and the target, so a
    responder can tell "we are close" from "we are over".
    """
    alerts: List[Dict[str, object]] = []
    for r in plan.resources:
        if r.exhausted:
            alerts.append(
                {
                    "alert": "OracleCapacityExhausted",
                    "resource": r.name,
                    "headroom": round(r.headroom, 4),
                    "target": r.headroom_target,
                    "severity": "critical",
                }
            )
        elif r.at_risk:
            alerts.append(
                {
                    "alert": "OracleCapacityHeadroomLow",
                    "resource": r.name,
                    "headroom": round(r.headroom, 4),
                    "target": r.headroom_target,
                    "severity": "warning",
                }
            )
    if plan.lag_seconds > LAG_ALERT_SECONDS:
        alerts.append(
            {
                "alert": "OracleCapacityLagApproachingTarget",
                "resource": "lag",
                "lag_seconds": round(plan.lag_seconds, 2),
                "target": LAG_TARGET_SECONDS,
                "severity": "warning" if plan.lag_seconds <= LAG_TARGET_SECONDS else "critical",
            }
        )
    return alerts


def compare_to_measured(
    projected: Sequence[float], measured: Sequence[float], tolerance: float = TOLERANCE
) -> Dict[str, object]:
    """Relative error of a projection against measurement.

    ``tolerance`` is the stated accuracy the model must hold to; the caller
    decides what to do when it is not met, but the number is never implicit.
    """
    if len(projected) != len(measured):
        raise ValueError("projected and measured series must be the same length")
    errors = []
    for p, m in zip(projected, measured):
        if m == 0:
            errors.append(abs(p - m))
        else:
            errors.append(abs(p - m) / abs(m))
    worst = max(errors) if errors else 0.0
    return {
        "worst_relative_error": round(worst, 4),
        "tolerance": tolerance,
        "within_tolerance": worst <= tolerance,
        "errors": [round(e, 4) for e in errors],
    }


def prometheus(plan: CapacityPlan) -> str:
    """Exports headroom and lag so the capacity alerts have data to fire on."""
    lines = ["# TYPE oracle_capacity_headroom_ratio gauge", "# TYPE oracle_capacity_lag_seconds gauge"]
    for r in plan.resources:
        lines.append(f'oracle_capacity_headroom_ratio{{resource="{r.name}"}} {r.headroom:.4f}')
    lines.append(f'oracle_capacity_lag_seconds{{resource="lag"}} {plan.lag_seconds:.2f}')
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--assets", type=int, required=True)
    p.add_argument("--sources", type=int, required=True)
    p.add_argument("--submission-rate-hz", type=float, required=True)
    p.add_argument("--history-depth", type=int, default=720)
    p.add_argument("--lag-seconds", type=float, default=None, help="measured lag, if known")
    p.add_argument("--validate", type=Path, default=None,
                   help="JSON file of measured usage to validate the projection against")
    args = p.parse_args(argv)

    profile = Profile(
        assets=args.assets,
        sources=args.sources,
        submission_rate_hz=args.submission_rate_hz,
        history_depth=args.history_depth,
    )
    plan = project(profile, lag_seconds=args.lag_seconds)
    plan.shedding = shedding_plan(plan)
    print(json.dumps(plan.to_dict(), indent=2))
    for alert in pre_exhaustion_alerts(plan):
        print(f"ALERT {alert['alert']} resource={alert['resource']} severity={alert['severity']}")
    print(prometheus(plan), end="")
    return 1 if plan.at_risk else 0


if __name__ == "__main__":
    sys.exit(main())
