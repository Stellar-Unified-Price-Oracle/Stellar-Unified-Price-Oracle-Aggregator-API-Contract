"""Capacity headroom review from observed traffic (#704).

Evaluates the four primary resource bottlenecks across the Oracle system:
1. RPC bandwidth and rate limits
2. Compute (signature verification, aggregation)
3. Storage (contract data TTL, index growth)
4. Operator staffing (monitoring load, paging frequency)

Computes headroom ratios against observed peak and sustained production traffic,
evaluates quantitative expansion triggers, and maps triggered bottlenecks
to remediation playbooks with designated team owners.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import asdict, dataclass, field
from datetime import date
from typing import Any, Dict, List, Optional, Sequence


# Headroom thresholds for expansion triggers
# When headroom drops below these ratios of total capacity, triggers fire.
DEFAULT_WARNING_HEADROOM_RATIO = 0.30       # < 30% headroom remaining (Warning)
DEFAULT_EXPANSION_HEADROOM_RATIO = 0.15     # < 15% headroom remaining (Immediate Expansion)

# Standard bottleneck owners
OWNERS: Dict[str, str] = {
    "rpc": "infra-ops",
    "compute": "core-contracts",
    "storage": "infra-ops",
    "staffing": "oracle-oncall",
}

# Remediation playbooks for each bottleneck
REMEDIATIONS: Dict[str, str] = {
    "rpc": (
        "1. Provision secondary/failover RPC provider endpoints.\n"
        "2. Enable client-side batching and response cache for static metadata.\n"
        "3. Segregate read query traffic from transaction submission pipes.\n"
        "4. Upgrade RPC subscription tier with higher burst limits."
    ),
    "compute": (
        "1. Horizontal scale-out of worker pods / verification processes.\n"
        "2. Enable cached-key fast-path for validated oracle source signatures.\n"
        "3. Distribute batch aggregation workers across independent CPU affinity pools.\n"
        "4. Profile hot spots and optimize SIMD / cryptographic verification routines."
    ),
    "storage": (
        "1. Trigger table compaction and vacuuming on derived SQLite/PostgreSQL stores.\n"
        "2. Adjust Soroban contract instance and entry TTL extension scheduling.\n"
        "3. Move historical archives past 90 days to cold object storage (S3/GCS).\n"
        "4. Expand persistent volume allocation before reaching 85% disk saturation."
    ),
    "staffing": (
        "1. Audit firing alerts and tune false-positive noisy thresholds.\n"
        "2. Introduce secondary on-call rotation to absorb non-paging triage load.\n"
        "3. Shift recurring alert mitigations to self-healing automated runbooks.\n"
        "4. Re-evaluate on-call shift length and engineer compensation/rotation pool."
    ),
}


@dataclass(frozen=True)
class ObservedTraffic:
    """Observed traffic measurements over a measurement window."""

    observation_period_days: int = 14
    # RPC requests per second (sustained and peak)
    rpc_sustained_rps: float = 120.0
    rpc_peak_rps: float = 280.0
    # Compute CPU cores used (sustained and peak)
    compute_sustained_cores: float = 2.4
    compute_peak_cores: float = 4.8
    # Storage usage in Gigabytes
    storage_current_gb: float = 65.0
    storage_daily_growth_gb: float = 1.2
    storage_projected_90d_gb: float = 173.0
    # Operator on-call paging alerts per week
    pages_per_week: float = 4.0


@dataclass(frozen=True)
class CapacityLimits:
    """Configured maximum capacity limits per bottleneck."""

    rpc_capacity_rps: float = 500.0
    compute_capacity_cores: float = 8.0
    storage_capacity_gb: float = 250.0
    staffing_max_pages_per_week: float = 10.0


@dataclass
class BottleneckReview:
    """Headroom evaluation and trigger status for a single bottleneck."""

    name: str
    unit: str
    capacity: float
    observed_sustained: float
    observed_peak: float
    headroom_sustained_ratio: float
    headroom_peak_ratio: float
    status: str  # "HEALTHY", "WARNING", "CRITICAL_EXPANSION"
    owner: str
    remediation_playbook: str
    trigger_message: Optional[str] = None


@dataclass
class HeadroomReviewReport:
    """Full capacity headroom review report."""

    review_date: str
    period_days: int
    reviews: Dict[str, BottleneckReview] = field(default_factory=dict)
    has_warning_triggers: bool = False
    has_expansion_triggers: bool = False

    def to_dict(self) -> Dict[str, Any]:
        return {
            "review_date": self.review_date,
            "period_days": self.period_days,
            "has_warning_triggers": self.has_warning_triggers,
            "has_expansion_triggers": self.has_expansion_triggers,
            "bottlenecks": {k: asdict(v) for k, v in self.reviews.items()},
        }

    def generate_markdown(self) -> str:
        lines = [
            f"# Capacity Headroom Review Report ({self.review_date})",
            "",
            f"**Observation Period:** {self.period_days} days  ",
            f"**Review Cadence:** Quarterly review & automated continuous evaluation  ",
            f"**Overall Status:** {'CRITICAL: Expansion Required' if self.has_expansion_triggers else ('WARNING: Elevated Utilization' if self.has_warning_triggers else 'HEALTHY: All Headrooms Within Target')}",
            "",
            "## 1. Bottleneck Headroom Summary",
            "",
            "| Bottleneck | Capacity | Observed Sustained | Observed Peak | Headroom (Peak) | Status | Owner |",
            "|---|---|---|---|---|---|---|",
        ]

        for b in self.reviews.values():
            pct = round(b.headroom_peak_ratio * 100, 1)
            status_badge = (
                "**CRITICAL EXPANSION**" if b.status == "CRITICAL_EXPANSION"
                else ("*WARNING*" if b.status == "WARNING" else "HEALTHY")
            )
            lines.append(
                f"| **{b.name.upper()}** | {b.capacity:g} {b.unit} | {b.observed_sustained:g} {b.unit} | {b.observed_peak:g} {b.unit} | {pct}% | {status_badge} | `{b.owner}` |"
            )

        lines.extend([
            "",
            "## 2. Expansion Triggers & Findings",
            "",
        ])

        triggered_any = False
        for b in self.reviews.values():
            if b.status != "HEALTHY":
                triggered_any = True
                lines.append(f"### {b.name.upper()} ({b.status})")
                lines.append(f"* **Trigger:** {b.trigger_message}")
                lines.append(f"* **Accountable Owner:** `{b.owner}`")
                lines.append("* **Action Plan:**")
                for step in b.remediation_playbook.strip().split("\n"):
                    lines.append(f"  {step}")
                lines.append("")

        if not triggered_any:
            lines.append("No expansion triggers fired. All four bottlenecks maintain healthy headroom margins (> 30%).\n")

        lines.extend([
            "## 3. Remediation Playbooks Overview",
            "",
        ])
        for name, playbook in REMEDIATIONS.items():
            lines.append(f"### `{name.upper()}` Remediation Playbook (Owner: `{OWNERS[name]}`)")
            lines.append(playbook)
            lines.append("")

        return "\n".join(lines)


def evaluate_headroom(
    observed: ObservedTraffic,
    limits: Optional[CapacityLimits] = None,
    warning_threshold: float = DEFAULT_WARNING_HEADROOM_RATIO,
    expansion_threshold: float = DEFAULT_EXPANSION_HEADROOM_RATIO,
    review_date: Optional[str] = None,
) -> HeadroomReviewReport:
    """Evaluate capacity headroom and expansion triggers from observed traffic."""
    if limits is None:
        limits = CapacityLimits()

    d_str = review_date or date.today().isoformat()
    report = HeadroomReviewReport(review_date=d_str, period_days=observed.observation_period_days)

    # 1. RPC Bottleneck
    rpc_peak_headroom = max(0.0, (limits.rpc_capacity_rps - observed.rpc_peak_rps) / limits.rpc_capacity_rps)
    rpc_sustained_headroom = max(0.0, (limits.rpc_capacity_rps - observed.rpc_sustained_rps) / limits.rpc_capacity_rps)
    rpc_status = "HEALTHY"
    rpc_msg = None
    if rpc_peak_headroom < expansion_threshold:
        rpc_status = "CRITICAL_EXPANSION"
        rpc_msg = f"Peak RPC throughput ({observed.rpc_peak_rps} rps) leaves only {round(rpc_peak_headroom * 100, 1)}% headroom (< {round(expansion_threshold * 100)}%)."
    elif rpc_peak_headroom < warning_threshold:
        rpc_status = "WARNING"
        rpc_msg = f"Peak RPC throughput ({observed.rpc_peak_rps} rps) leaves {round(rpc_peak_headroom * 100, 1)}% headroom (< {round(warning_threshold * 100)}%)."

    report.reviews["rpc"] = BottleneckReview(
        name="rpc",
        unit="rps",
        capacity=limits.rpc_capacity_rps,
        observed_sustained=observed.rpc_sustained_rps,
        observed_peak=observed.rpc_peak_rps,
        headroom_sustained_ratio=round(rpc_sustained_headroom, 4),
        headroom_peak_ratio=round(rpc_peak_headroom, 4),
        status=rpc_status,
        owner=OWNERS["rpc"],
        remediation_playbook=REMEDIATIONS["rpc"],
        trigger_message=rpc_msg,
    )

    # 2. Compute Bottleneck
    compute_peak_headroom = max(0.0, (limits.compute_capacity_cores - observed.compute_peak_cores) / limits.compute_capacity_cores)
    compute_sustained_headroom = max(0.0, (limits.compute_capacity_cores - observed.compute_sustained_cores) / limits.compute_capacity_cores)
    compute_status = "HEALTHY"
    compute_msg = None
    if compute_peak_headroom < expansion_threshold:
        compute_status = "CRITICAL_EXPANSION"
        compute_msg = f"Peak compute usage ({observed.compute_peak_cores} cores) leaves only {round(compute_peak_headroom * 100, 1)}% headroom (< {round(expansion_threshold * 100)}%)."
    elif compute_peak_headroom < warning_threshold:
        compute_status = "WARNING"
        compute_msg = f"Peak compute usage ({observed.compute_peak_cores} cores) leaves {round(compute_peak_headroom * 100, 1)}% headroom (< {round(warning_threshold * 100)}%)."

    report.reviews["compute"] = BottleneckReview(
        name="compute",
        unit="cores",
        capacity=limits.compute_capacity_cores,
        observed_sustained=observed.compute_sustained_cores,
        observed_peak=observed.compute_peak_cores,
        headroom_sustained_ratio=round(compute_sustained_headroom, 4),
        headroom_peak_ratio=round(compute_peak_headroom, 4),
        status=compute_status,
        owner=OWNERS["compute"],
        remediation_playbook=REMEDIATIONS["compute"],
        trigger_message=compute_msg,
    )

    # 3. Storage Bottleneck (uses current and 90-day projection)
    storage_peak_headroom = max(0.0, (limits.storage_capacity_gb - observed.storage_projected_90d_gb) / limits.storage_capacity_gb)
    storage_sustained_headroom = max(0.0, (limits.storage_capacity_gb - observed.storage_current_gb) / limits.storage_capacity_gb)
    storage_status = "HEALTHY"
    storage_msg = None
    if storage_peak_headroom < expansion_threshold:
        storage_status = "CRITICAL_EXPANSION"
        storage_msg = f"Projected 90-day storage ({observed.storage_projected_90d_gb} GB) leaves only {round(storage_peak_headroom * 100, 1)}% headroom (< {round(expansion_threshold * 100)}%)."
    elif storage_peak_headroom < warning_threshold:
        storage_status = "WARNING"
        storage_msg = f"Projected 90-day storage ({observed.storage_projected_90d_gb} GB) leaves {round(storage_peak_headroom * 100, 1)}% headroom (< {round(warning_threshold * 100)}%)."

    report.reviews["storage"] = BottleneckReview(
        name="storage",
        unit="GB",
        capacity=limits.storage_capacity_gb,
        observed_sustained=observed.storage_current_gb,
        observed_peak=observed.storage_projected_90d_gb,
        headroom_sustained_ratio=round(storage_sustained_headroom, 4),
        headroom_peak_ratio=round(storage_peak_headroom, 4),
        status=storage_status,
        owner=OWNERS["storage"],
        remediation_playbook=REMEDIATIONS["storage"],
        trigger_message=storage_msg,
    )

    # 4. Operator Staffing Bottleneck
    staffing_headroom = max(0.0, (limits.staffing_max_pages_per_week - observed.pages_per_week) / limits.staffing_max_pages_per_week)
    staffing_status = "HEALTHY"
    staffing_msg = None
    if staffing_headroom < expansion_threshold:
        staffing_status = "CRITICAL_EXPANSION"
        staffing_msg = f"Weekly pages ({observed.pages_per_week}/wk) exceed sustainable fatigue threshold, leaving only {round(staffing_headroom * 100, 1)}% headroom."
    elif staffing_headroom < warning_threshold:
        staffing_status = "WARNING"
        staffing_msg = f"Weekly pages ({observed.pages_per_week}/wk) approach fatigue threshold ({round(staffing_headroom * 100, 1)}% headroom)."

    report.reviews["staffing"] = BottleneckReview(
        name="staffing",
        unit="pages/wk",
        capacity=limits.staffing_max_pages_per_week,
        observed_sustained=observed.pages_per_week,
        observed_peak=observed.pages_per_week,
        headroom_sustained_ratio=round(staffing_headroom, 4),
        headroom_peak_ratio=round(staffing_headroom, 4),
        status=staffing_status,
        owner=OWNERS["staffing"],
        remediation_playbook=REMEDIATIONS["staffing"],
        trigger_message=staffing_msg,
    )

    # Determine aggregate trigger state
    report.has_warning_triggers = any(b.status in ("WARNING", "CRITICAL_EXPANSION") for b in report.reviews.values())
    report.has_expansion_triggers = any(b.status == "CRITICAL_EXPANSION" for b in report.reviews.values())

    return report


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Capacity headroom review from observed traffic (#704)")
    parser.add_argument("--input", help="Path to observed traffic JSON file")
    parser.add_argument("--rpc-sustained-rps", type=float, default=120.0, help="Observed sustained RPC rps")
    parser.add_argument("--rpc-peak-rps", type=float, default=280.0, help="Observed peak RPC rps")
    parser.add_argument("--compute-sustained-cores", type=float, default=2.4, help="Observed sustained CPU cores")
    parser.add_argument("--compute-peak-cores", type=float, default=4.8, help="Observed peak CPU cores")
    parser.add_argument("--storage-current-gb", type=float, default=65.0, help="Current storage in GB")
    parser.add_argument("--storage-growth-gb", type=float, default=1.2, help="Daily storage growth in GB")
    parser.add_argument("--pages-per-week", type=float, default=4.0, help="Weekly paging alert count")
    parser.add_argument("--json", action="store_true", help="Output results as JSON")
    parser.add_argument("--report", help="Save markdown report to specified filepath")
    parser.add_argument("--fail-on-trigger", action="store_true", help="Exit non-zero if any expansion trigger fired")

    args = parser.parse_args(argv)

    if args.input:
        with open(args.input, "r", encoding="utf-8") as f:
            data = json.load(f)
        observed = ObservedTraffic(
            observation_period_days=data.get("observation_period_days", 14),
            rpc_sustained_rps=data.get("rpc_sustained_rps", 120.0),
            rpc_peak_rps=data.get("rpc_peak_rps", 280.0),
            compute_sustained_cores=data.get("compute_sustained_cores", 2.4),
            compute_peak_cores=data.get("compute_peak_cores", 4.8),
            storage_current_gb=data.get("storage_current_gb", 65.0),
            storage_daily_growth_gb=data.get("storage_daily_growth_gb", 1.2),
            storage_projected_90d_gb=data.get("storage_current_gb", 65.0) + (data.get("storage_daily_growth_gb", 1.2) * 90),
            pages_per_week=data.get("pages_per_week", 4.0),
        )
    else:
        proj_90d = args.storage_current_gb + (args.storage_growth_gb * 90)
        observed = ObservedTraffic(
            rpc_sustained_rps=args.rpc_sustained_rps,
            rpc_peak_rps=args.rpc_peak_rps,
            compute_sustained_cores=args.compute_sustained_cores,
            compute_peak_cores=args.compute_peak_cores,
            storage_current_gb=args.storage_current_gb,
            storage_daily_growth_gb=args.storage_growth_gb,
            storage_projected_90d_gb=proj_90d,
            pages_per_week=args.pages_per_week,
        )

    review_report = evaluate_headroom(observed)

    if args.report:
        with open(args.report, "w", encoding="utf-8") as f:
            f.write(review_report.generate_markdown())

    if args.json:
        print(json.dumps(review_report.to_dict(), indent=2))
    else:
        print(review_report.generate_markdown())

    if args.fail_on_trigger and review_report.has_expansion_triggers:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
