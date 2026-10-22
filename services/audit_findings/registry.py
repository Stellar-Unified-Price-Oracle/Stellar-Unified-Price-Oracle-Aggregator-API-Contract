"""External audit finding closure evidence registry (#688).

Every finding from an external audit is tracked in one file,
`config/audit-findings.json`, with its severity, owner, fix release,
regression test and reviewer acceptance. The validator refuses a `CLOSED`
finding that does not link both the fix and the verification evidence, and
checks that the linked regression test actually exists in the repository.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REGISTRY = REPO_ROOT / "config" / "audit-findings.json"

SECRET_SEED_PATTERN = re.compile(r"\bS[A-Z2-7]{55}\b")
PRIVATE_KEY_PATTERN = re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY")

# `path/to/file::test_name`, e.g. `contracts/price-oracle/src/test.rs::test_upgrade_unauthorized`.
REGRESSION_TEST_PATTERN = re.compile(r"^(?P<path>[^:\s]+)::(?P<name>[A-Za-z_][A-Za-z0-9_]*)$")
# A fix is linked by a PR or commit URL in this repository's upstream.
FIX_REF_PATTERN = re.compile(r"^https://github\.com/[\w.-]+/[\w.-]+/(pull/\d+|commit/[0-9a-f]{7,40})$")


class Severity(str, Enum):
    CRITICAL = "CRITICAL"
    HIGH = "HIGH"
    MEDIUM = "MEDIUM"
    LOW = "LOW"
    INFORMATIONAL = "INFORMATIONAL"


class Status(str, Enum):
    OPEN = "OPEN"
    FIXED = "FIXED"  # fix merged, verification pending
    CLOSED = "CLOSED"  # fix released, regression test linked, reviewer accepted


REQUIRED_FIELDS = ("id", "title", "audit", "severity", "owner", "status", "reported_at")
CLOSURE_FIELDS = ("fix_ref", "fix_release", "regression_test", "reviewer", "accepted_at")


@dataclass
class Violation:
    finding_id: str
    rule: str
    detail: str

    def __str__(self) -> str:
        return f"{self.finding_id}: [{self.rule}] {self.detail}"


@dataclass
class RegistryReport:
    total: int
    by_status: Dict[str, int]
    by_severity: Dict[str, int]
    violations: List[Violation] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.violations


def _parse_date(value: Any) -> Optional[datetime]:
    try:
        return datetime.fromisoformat(str(value))
    except ValueError:
        return None


def _check_regression_test(ref: str, repo_root: Path) -> Optional[str]:
    """Returns a problem description, or None if the test exists."""
    m = REGRESSION_TEST_PATTERN.match(ref)
    if not m:
        return f"regression_test must be `path::test_name`, got {ref!r}"
    path = repo_root / m.group("path")
    if not path.is_file():
        return f"regression test file {m.group('path')} does not exist"
    name = re.escape(m.group("name"))
    if not re.search(rf"\b(fn|def)\s+{name}\b", path.read_text(encoding="utf-8")):
        return f"test {m.group('name')} not found in {m.group('path')}"
    return None


def validate_finding(finding: Dict[str, Any], repo_root: Path = REPO_ROOT) -> List[Violation]:
    """Checks one finding. A CLOSED finding must carry complete fix and verification evidence."""
    fid = str(finding.get("id") or "<missing id>")
    out: List[Violation] = []

    missing = [f for f in REQUIRED_FIELDS if not finding.get(f)]
    if missing:
        out.append(Violation(fid, "required-fields", f"missing {', '.join(missing)}"))
    if finding.get("severity") not in {s.value for s in Severity}:
        out.append(Violation(fid, "severity", f"unknown severity {finding.get('severity')!r}"))
    if finding.get("status") not in {s.value for s in Status}:
        out.append(Violation(fid, "status", f"unknown status {finding.get('status')!r}"))
        return out
    if finding.get("reported_at") and _parse_date(finding["reported_at"]) is None:
        out.append(Violation(fid, "reported-at", "reported_at must be an ISO-8601 date"))

    status = Status(finding["status"])
    if status in (Status.FIXED, Status.CLOSED):
        ref = finding.get("fix_ref")
        if not ref:
            out.append(Violation(fid, "fix-evidence", f"{status.value} finding has no fix_ref"))
        elif not FIX_REF_PATTERN.match(ref):
            out.append(Violation(fid, "fix-evidence", "fix_ref must be a GitHub pull request or commit URL"))

    if status is Status.CLOSED:
        missing = [f for f in CLOSURE_FIELDS if not finding.get(f)]
        if missing:
            out.append(Violation(fid, "closure-evidence", f"CLOSED finding is missing {', '.join(missing)}"))
        if finding.get("regression_test"):
            problem = _check_regression_test(finding["regression_test"], repo_root)
            if problem:
                out.append(Violation(fid, "verification-evidence", problem))
        if finding.get("reviewer") and finding.get("reviewer") == finding.get("owner"):
            out.append(Violation(fid, "independent-review", "reviewer must not be the finding owner"))
        accepted = _parse_date(finding.get("accepted_at")) if finding.get("accepted_at") else None
        reported = _parse_date(finding.get("reported_at")) if finding.get("reported_at") else None
        if finding.get("accepted_at") and accepted is None:
            out.append(Violation(fid, "accepted-at", "accepted_at must be an ISO-8601 date"))
        elif accepted and reported and accepted < reported:
            out.append(Violation(fid, "accepted-at", "accepted_at is before reported_at"))

    text = json.dumps(finding)
    if SECRET_SEED_PATTERN.search(text) or PRIVATE_KEY_PATTERN.search(text):
        out.append(Violation(fid, "no-secrets", "finding contains key material"))
    return out


def validate_registry(registry: Dict[str, Any], repo_root: Path = REPO_ROOT) -> RegistryReport:
    findings = registry.get("findings")
    if not isinstance(findings, list):
        return RegistryReport(0, {}, {}, [Violation("<registry>", "schema", "`findings` must be a list")])

    violations: List[Violation] = []
    seen = set()
    for f in findings:
        fid = f.get("id")
        if fid in seen:
            violations.append(Violation(str(fid), "unique-id", "duplicate finding id"))
        seen.add(fid)
        violations.extend(validate_finding(f, repo_root))

    count = lambda key, values: {v: sum(1 for f in findings if f.get(key) == v) for v in values}
    return RegistryReport(
        total=len(findings),
        by_status=count("status", [s.value for s in Status]),
        by_severity=count("severity", [s.value for s in Severity]),
        violations=violations,
    )


def render_report(report: RegistryReport) -> str:
    lines = [
        "# Audit Finding Registry Report\n",
        f"- **Findings**: {report.total}",
        "- **By status**: " + ", ".join(f"{k}: {v}" for k, v in report.by_status.items()),
        "- **By severity**: " + ", ".join(f"{k}: {v}" for k, v in report.by_severity.items()),
        f"- **Result**: `{'PASS' if report.ok else 'FAIL'}`\n",
    ]
    lines += [f"- {v}" for v in report.violations]
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Validate the external audit finding registry")
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--report", type=Path, help="Write a markdown summary report")
    args = parser.parse_args(argv)

    registry = json.loads(args.registry.read_text(encoding="utf-8"))
    report = validate_registry(registry)
    text = render_report(report)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(text, encoding="utf-8")
        print(f"Wrote audit finding report to {args.report}")
    print(text)
    return 0 if report.ok else 1


if __name__ == "__main__":
    sys.exit(main())
