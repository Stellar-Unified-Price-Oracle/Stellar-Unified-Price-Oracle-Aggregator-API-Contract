"""Production access review and least-privilege evidence (#691).

Takes a quarterly inventory of human and automation access to the deployment,
monitoring, source and treasury systems, caps every grant at the ceiling of the
principal's roles, and produces the approved access list. A review passes only
when every principal's owner has approved the list and every removed grant has
a recorded probe showing that the removed access no longer works.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import asdict, dataclass, field
from datetime import date, timedelta
from enum import Enum, IntEnum
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

SECRET_SEED_PATTERN = re.compile(r"\bS[A-Z2-7]{55}\b")
PRIVATE_KEY_PATTERN = re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY")

REVIEW_INTERVAL_DAYS = 92  # quarterly
INACTIVITY_DAYS = 90  # CONTRIBUTING.md "Inactivity Expiry"


class System(str, Enum):
    DEPLOYMENT = "DEPLOYMENT"  # deploy environments, OIDC deploy roles, upgrade signing
    MONITORING = "MONITORING"  # dashboards, alerting, metrics push
    SOURCE = "SOURCE"  # the GitHub repository
    TREASURY = "TREASURY"  # treasury multisig signing


class Level(IntEnum):
    NONE = 0
    READ = 1
    TRIAGE = 2
    WRITE = 3
    ADMIN = 4


# Contributor ladder from CONTRIBUTING.md, lowest first. Inactive humans are
# reviewed one rung lower.
LADDER = ["contributor", "triager", "maintainer", "core-maintainer"]

# The most each role may hold per system. Anything not listed is NONE.
ROLE_CEILINGS: Dict[str, Dict[System, Level]] = {
    # Humans (CONTRIBUTING.md "Least-Privilege Permission Map").
    "contributor": {System.SOURCE: Level.READ},
    "triager": {System.SOURCE: Level.TRIAGE},
    "maintainer": {System.SOURCE: Level.WRITE, System.MONITORING: Level.READ},
    "core-maintainer": {
        System.SOURCE: Level.ADMIN,
        System.DEPLOYMENT: Level.WRITE,
        System.MONITORING: Level.READ,
    },
    "treasury-signer": {System.TREASURY: Level.WRITE},  # multisig governor
    "on-call": {System.MONITORING: Level.WRITE},
    # Automation (docs/secretless-ci.md).
    "deploy-workflow": {System.DEPLOYMENT: Level.WRITE},
    "metrics-exporter": {System.MONITORING: Level.WRITE},
    "ci": {System.SOURCE: Level.READ},
}
AUTOMATION_ROLES = {"deploy-workflow", "metrics-exporter", "ci"}


@dataclass
class Removal:
    principal: str
    system: str
    granted: str
    allowed: str
    reason: str


@dataclass
class ReviewReport:
    review_id: str
    started_at: str
    next_review_due: str
    approved_access: Dict[str, Dict[str, str]]
    removals: List[Removal] = field(default_factory=list)
    violations: List[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.violations

    def to_dict(self) -> Dict[str, Any]:
        return {**asdict(self), "ok": self.ok}


def _date(value: Any) -> Optional[date]:
    try:
        return date.fromisoformat(str(value)[:10])
    except ValueError:
        return None


def ceiling(roles: Sequence[str], inactive: bool = False) -> Dict[System, Level]:
    """The highest level any of `roles` allows per system."""
    roles = list(roles)
    if inactive:
        ladder = [r for r in roles if r in LADDER]
        if ladder:
            top = max(ladder, key=LADDER.index)
            roles.remove(top)
            if LADDER.index(top) > 0:
                roles.append(LADDER[LADDER.index(top) - 1])
    out = {s: Level.NONE for s in System}
    for role in roles:
        for system, level in ROLE_CEILINGS.get(role, {}).items():
            out[system] = max(out[system], level)
    return out


def review_access(inventory: Dict[str, Any]) -> ReviewReport:
    started = _date(inventory.get("started_at"))
    violations: List[str] = []
    if started is None:
        violations.append("started_at is missing or not an ISO date")
        started = date.min
    if SECRET_SEED_PATTERN.search(json.dumps(inventory)) or PRIVATE_KEY_PATTERN.search(json.dumps(inventory)):
        violations.append("inventory contains key material; record principals and levels only")

    principals = inventory.get("principals") or []
    ids = [p.get("id") for p in principals]
    humans = {p.get("id") for p in principals if p.get("kind") == "human"}
    if len(set(ids)) != len(ids):
        violations.append("duplicate principal ids")

    approved: Dict[str, Dict[str, str]] = {}
    removals: List[Removal] = []
    for p in principals:
        pid, roles, kind = p.get("id"), p.get("roles") or [], p.get("kind")
        if kind not in ("human", "automation"):
            violations.append(f"{pid}: kind must be human or automation")
            continue
        unknown = [r for r in roles if r not in ROLE_CEILINGS]
        if unknown:
            violations.append(f"{pid}: unknown roles {', '.join(unknown)}")
        if kind == "automation" and any(r not in AUTOMATION_ROLES for r in roles):
            violations.append(f"{pid}: automation may only hold automation roles")
        if kind == "human" and any(r in AUTOMATION_ROLES for r in roles):
            violations.append(f"{pid}: humans may not hold automation roles")
        owner = p.get("owner")
        if not owner or owner == pid or owner not in humans:
            violations.append(f"{pid}: owner must be another human principal in the inventory")

        last_active = _date(p.get("last_active"))
        inactive = kind == "human" and (last_active is None or (started - last_active).days > INACTIVITY_DAYS)
        cap = ceiling(roles, inactive)

        approved[pid] = {}
        for system_name, level_name in (p.get("grants") or {}).items():
            try:
                system, granted = System(system_name), Level[level_name]
            except (ValueError, KeyError):
                violations.append(f"{pid}: unknown grant {system_name}={level_name}")
                continue
            allowed = min(granted, cap[system])
            if allowed > Level.NONE:
                approved[pid][system.value] = allowed.name
            if granted > allowed:
                reason = f"inactive over {INACTIVITY_DAYS} days" if inactive and granted <= ceiling(roles)[system] \
                    else f"exceeds the ceiling of {', '.join(roles) or 'no role'}"
                removals.append(Removal(pid, system.value, granted.name, allowed.name, reason))

    # The approved list must be signed off by each principal's owner.
    approvals = {a.get("principal"): a for a in inventory.get("approvals") or []}
    owners = {p.get("id"): p.get("owner") for p in principals}
    for pid in approved:
        a = approvals.get(pid)
        if not a:
            violations.append(f"{pid}: access list not approved by its owner")
        elif a.get("approved_by") != owners.get(pid) or a.get("approved_by") == pid:
            violations.append(f"{pid}: approved by {a.get('approved_by')}, not by its owner {owners.get(pid)}")
        elif (_date(a.get("approved_at")) or date.min) < started:
            violations.append(f"{pid}: approval predates this review")

    # Every removal must be verified: the removed access was tried and denied.
    checks = inventory.get("revocation_checks") or []
    for r in removals:
        match = [c for c in checks if c.get("principal") == r.principal and c.get("system") == r.system
                 and (_date(c.get("checked_at")) or date.min) >= started]
        if not match:
            violations.append(f"{r.principal}: removal of {r.system} {r.granted} is not verified")
        elif any(c.get("result") != "DENIED" for c in match):
            violations.append(f"{r.principal}: removed {r.system} access still works")

    return ReviewReport(
        review_id=str(inventory.get("review_id") or ""),
        started_at=started.isoformat() if started != date.min else "",
        next_review_due=(started + timedelta(days=REVIEW_INTERVAL_DAYS)).isoformat() if started != date.min else "",
        approved_access=approved,
        removals=removals,
        violations=violations,
    )


def create_seeded_inventory(started_at: str = "2026-10-01") -> Dict[str, Any]:
    """A synthetic inventory for drills and CI, with one excess grant of each kind."""
    return {
        "review_id": "SEEDED-2026-Q4",
        "started_at": started_at,
        "principals": [
            {"id": "core-a", "kind": "human", "roles": ["core-maintainer"], "owner": "core-b",
             "last_active": "2026-09-20", "grants": {"SOURCE": "ADMIN", "DEPLOYMENT": "WRITE"}},
            {"id": "core-b", "kind": "human", "roles": ["core-maintainer", "treasury-signer"], "owner": "core-a",
             "last_active": "2026-09-28", "grants": {"SOURCE": "ADMIN", "TREASURY": "WRITE"}},
            # Excess: a maintainer with deploy access.
            {"id": "maint-a", "kind": "human", "roles": ["maintainer"], "owner": "core-a",
             "last_active": "2026-09-15", "grants": {"SOURCE": "WRITE", "DEPLOYMENT": "WRITE"}},
            # Inactive: triager untouched since spring is reviewed as a contributor.
            {"id": "triage-a", "kind": "human", "roles": ["triager"], "owner": "core-a",
             "last_active": "2026-04-01", "grants": {"SOURCE": "TRIAGE"}},
            # Excess: the deploy workflow can also write to the repository.
            {"id": "workflow:canary.yml", "kind": "automation", "roles": ["deploy-workflow"], "owner": "core-b",
             "last_active": "2026-09-30", "grants": {"DEPLOYMENT": "WRITE", "SOURCE": "WRITE"}},
        ],
        "approvals": [
            {"principal": "core-a", "approved_by": "core-b", "approved_at": "2026-10-03"},
            {"principal": "core-b", "approved_by": "core-a", "approved_at": "2026-10-03"},
            {"principal": "maint-a", "approved_by": "core-a", "approved_at": "2026-10-03"},
            {"principal": "triage-a", "approved_by": "core-a", "approved_at": "2026-10-03"},
            {"principal": "workflow:canary.yml", "approved_by": "core-b", "approved_at": "2026-10-03"},
        ],
        "revocation_checks": [
            {"principal": "maint-a", "system": "DEPLOYMENT", "result": "DENIED", "checked_at": "2026-10-04"},
            {"principal": "triage-a", "system": "SOURCE", "result": "DENIED", "checked_at": "2026-10-04"},
            {"principal": "workflow:canary.yml", "system": "SOURCE", "result": "DENIED", "checked_at": "2026-10-04"},
        ],
    }


def render_report(report: ReviewReport) -> str:
    lines = [
        f"# Access Review {report.review_id}\n",
        f"- **Started**: {report.started_at}",
        f"- **Next review due**: {report.next_review_due}",
        f"- **Result**: `{'PASS' if report.ok else 'FAIL'}`\n",
        "## Approved access\n",
        "| Principal | Access |",
        "|---|---|",
    ]
    for pid, grants in report.approved_access.items():
        access = ", ".join(f"{s}={lvl}" for s, lvl in grants.items()) or "none"
        lines.append(f"| `{pid}` | {access} |")
    lines += ["", "## Removed access\n", "| Principal | System | Granted | Now | Reason |", "|---|---|---|---|---|"]
    lines += [f"| `{r.principal}` | {r.system} | {r.granted} | {r.allowed} | {r.reason} |" for r in report.removals]
    if report.violations:
        lines += ["", "## Violations\n"] + [f"- {v}" for v in report.violations]
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Production access review and least-privilege evidence")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--inventory", type=Path, help="Quarterly access inventory (JSON)")
    source.add_argument("--seed", action="store_true", help="Review the seeded synthetic inventory")
    parser.add_argument("--out", type=Path, help="Write the review as JSON")
    parser.add_argument("--report", type=Path, help="Write the review as markdown")
    args = parser.parse_args(argv)

    inventory = create_seeded_inventory() if args.seed else json.loads(args.inventory.read_text(encoding="utf-8"))
    report = review_access(inventory)
    if args.out:
        args.out.write_text(json.dumps(report.to_dict(), indent=2) + "\n", encoding="utf-8")
        print(f"Wrote access review JSON to {args.out}")
    if args.report:
        args.report.write_text(render_report(report), encoding="utf-8")
        print(f"Wrote access review report to {args.report}")
    print("PASS" if report.ok else "FAIL")
    for v in report.violations:
        print(f"  {v}")
    return 0 if report.ok else 1


if __name__ == "__main__":
    sys.exit(main())
