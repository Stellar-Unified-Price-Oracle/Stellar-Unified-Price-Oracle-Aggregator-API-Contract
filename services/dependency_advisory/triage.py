"""Dependency advisory triage and remediation SLA system (#681).

Defines severity-based deadlines and ownership across Rust, JavaScript, and
operational dependencies, generating structured tickets and release-blocking
decisions.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import asdict, dataclass
from datetime import datetime, timedelta, timezone
from enum import Enum
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

SECRET_SEED_PATTERN = re.compile(r"\bS[A-Z2-7]{55}\b")
PRIVATE_KEY_PATTERN = re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY")


class Severity(str, Enum):
    CRITICAL = "CRITICAL"
    HIGH = "HIGH"
    MEDIUM = "MEDIUM"
    LOW = "LOW"


class Ecosystem(str, Enum):
    RUST = "RUST"
    JAVASCRIPT = "JAVASCRIPT"
    OPERATIONAL = "OPERATIONAL"


# Strict severity deadlines in days
SLA_DEADLINES_DAYS: Dict[Severity, int] = {
    Severity.CRITICAL: 1,    # 24 hours
    Severity.HIGH: 7,        # 7 calendar days
    Severity.MEDIUM: 30,     # 30 calendar days
    Severity.LOW: 90,        # 90 calendar days
}

# Release blocking policy: Critical and High severities block releases
RELEASE_BLOCKING_POLICY: Dict[Severity, bool] = {
    Severity.CRITICAL: True,
    Severity.HIGH: True,
    Severity.MEDIUM: False,
    Severity.LOW: False,
}

# Responsible owners per ecosystem
ECOSYSTEM_OWNERSHIP: Dict[Ecosystem, str] = {
    Ecosystem.RUST: "Core Contracts Tech Lead",
    Ecosystem.JAVASCRIPT: "SDK & Frontend Tech Lead",
    Ecosystem.OPERATIONAL: "DevOps & Infrastructure Lead",
}


@dataclass(frozen=True)
class AdvisoryRecord:
    advisory_id: str
    ecosystem: Ecosystem
    package_name: str
    vulnerable_versions: str
    severity: Severity
    title: str
    description: str
    discovered_at: str  # ISO-8601


@dataclass
class AdvisoryTicket:
    ticket_id: str
    advisory_id: str
    ecosystem: str
    package_name: str
    severity: str
    assigned_owner: str
    discovered_at: str
    due_date: str
    release_blocked: bool
    blocking_decision_reason: str
    remediation_guidance: str
    ticket_markdown: str

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


@dataclass
class TriageReport:
    total_advisories: int
    release_blocked: bool
    critical_count: int
    high_count: int
    medium_count: int
    low_count: int
    tickets: List[AdvisoryTicket]

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


def triage_advisory(
    advisory: AdvisoryRecord,
    reference_time: Optional[datetime] = None,
) -> AdvisoryTicket:
    """Evaluates an advisory, calculates SLA due date and release blocking decision."""
    ref_time = reference_time or datetime.now(timezone.utc)
    try:
        discovered_dt = datetime.fromisoformat(advisory.discovered_at)
    except Exception:
        discovered_dt = ref_time

    days_allowed = SLA_DEADLINES_DAYS[advisory.severity]
    due_dt = discovered_dt + timedelta(days=days_allowed)
    due_date_str = due_dt.strftime("%Y-%m-%d")

    is_blocking = RELEASE_BLOCKING_POLICY[advisory.severity]
    owner = ECOSYSTEM_OWNERSHIP[advisory.ecosystem]

    if is_blocking:
        reason = f"RELEASE BLOCKED: {advisory.severity.value} severity advisory {advisory.advisory_id} on {advisory.package_name} violates SLA threshold ({days_allowed}d SLA)."
    else:
        reason = f"RELEASE PERMITTED: {advisory.severity.value} severity advisory tracked with {days_allowed}d remediation window."

    guidance = (
        f"1. Check if an upstream patch exists for {advisory.package_name}.\n"
        f"2. Update pin in workspace manifest and regenerate lockfile.\n"
        f"3. Verify reproducible build and test suite.\n"
        f"4. If no patch exists, file formal security waiver in security-artifacts/."
    )

    ticket_md = (
        f"### [{advisory.severity.value}] {advisory.advisory_id}: {advisory.title}\n\n"
        f"- **Package**: `{advisory.package_name}` ({advisory.ecosystem.value})\n"
        f"- **Vulnerable Versions**: `{advisory.vulnerable_versions}`\n"
        f"- **Severity**: **{advisory.severity.value}**\n"
        f"- **Discovered At**: {advisory.discovered_at}\n"
        f"- **Remediation SLA Due Date**: **{due_date_str}** ({days_allowed} days)\n"
        f"- **Owner**: `{owner}`\n"
        f"- **Release Blocking Decision**: `{'YES - BLOCKS RELEASE' if is_blocking else 'NO - TRACKED'}`\n\n"
        f"**Description**:\n{advisory.description}\n\n"
        f"**Remediation Steps**:\n{guidance}\n"
    )

    ticket = AdvisoryTicket(
        ticket_id=f"TICKET-{advisory.ecosystem.value[:3]}-{advisory.advisory_id}",
        advisory_id=advisory.advisory_id,
        ecosystem=advisory.ecosystem.value,
        package_name=advisory.package_name,
        severity=advisory.severity.value,
        assigned_owner=owner,
        discovered_at=advisory.discovered_at,
        due_date=due_date_str,
        release_blocked=is_blocking,
        blocking_decision_reason=reason,
        remediation_guidance=guidance,
        ticket_markdown=ticket_md,
    )

    # Validate zero key exposure
    ticket_str = json.dumps(ticket.to_dict())
    if SECRET_SEED_PATTERN.search(ticket_str) or PRIVATE_KEY_PATTERN.search(ticket_str):
        raise ValueError("CRITICAL: Secret key material detected in generated advisory ticket!")

    return ticket


def evaluate_advisories(
    advisories: Sequence[AdvisoryRecord],
    reference_time: Optional[datetime] = None,
) -> TriageReport:
    """Triages a batch of advisories and returns aggregated report."""
    tickets = [triage_advisory(a, reference_time) for a in advisories]
    any_blocking = any(t.release_blocked for t in tickets)
    crit_count = sum(1 for t in tickets if t.severity == Severity.CRITICAL.value)
    high_count = sum(1 for t in tickets if t.severity == Severity.HIGH.value)
    med_count = sum(1 for t in tickets if t.severity == Severity.MEDIUM.value)
    low_count = sum(1 for t in tickets if t.severity == Severity.LOW.value)

    return TriageReport(
        total_advisories=len(tickets),
        release_blocked=any_blocking,
        critical_count=crit_count,
        high_count=high_count,
        medium_count=med_count,
        low_count=low_count,
        tickets=tickets,
    )


def create_seeded_advisory(
    severity: Severity = Severity.CRITICAL,
    ecosystem: Ecosystem = Ecosystem.RUST,
) -> AdvisoryRecord:
    """Generates a realistic seeded advisory for drill and CI testing."""
    return AdvisoryRecord(
        advisory_id=f"SEEDED-{ecosystem.value}-2026-901",
        ecosystem=ecosystem,
        package_name="soroban-token-helper" if ecosystem == Ecosystem.RUST else "stellar-sdk-adapter",
        vulnerable_versions="< 1.4.2",
        severity=severity,
        title="Unchecked arithmetic boundary overflow under adversary input",
        description="A flaw in token balance normalization enables integer wraps under boundary conditions.",
        discovered_at=datetime.now(timezone.utc).isoformat(),
    )


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Dependency Advisory Triage and Remediation SLA Evaluator")
    parser.add_argument("--seed", choices=["CRITICAL", "HIGH", "MEDIUM", "LOW"], help="Generate and triage a seeded advisory")
    parser.add_argument("--ecosystem", choices=["RUST", "JAVASCRIPT", "OPERATIONAL"], default="RUST")
    parser.add_argument("--out", type=Path, help="Output JSON path")
    parser.add_argument("--report", type=Path, help="Output markdown summary report")
    args = parser.parse_args(argv)

    advisories: List[AdvisoryRecord] = []
    if args.seed:
        advisories.append(create_seeded_advisory(Severity(args.seed), Ecosystem(args.ecosystem)))

    report = evaluate_advisories(advisories)
    report_dict = report.to_dict()

    if args.out:
        args.out.write_text(json.dumps(report_dict, indent=2) + "\n", encoding="utf-8")
        print(f"Wrote triage JSON to {args.out}")

    if args.report:
        lines = [
            f"# Dependency Advisory Triage Report\n",
            f"- **Total Advisories**: {report.total_advisories}",
            f"- **Release Blocked**: `{'YES' if report.release_blocked else 'NO'}`",
            f"- **Breakdown**: Critical: {report.critical_count}, High: {report.high_count}, Medium: {report.medium_count}, Low: {report.low_count}\n",
        ]
        for t in report.tickets:
            lines.append(t.ticket_markdown)
        args.report.write_text("\n".join(lines), encoding="utf-8")
        print(f"Wrote triage report to {args.report}")

    return 1 if report.release_blocked else 0


if __name__ == "__main__":
    sys.exit(main())
