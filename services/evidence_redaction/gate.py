"""Repeatable redaction gate (#687).

    python3 -m services.evidence_redaction.gate
    python3 -m services.evidence_redaction.gate --report security-artifacts/redaction.md

Proves, on every run, that:

1. a synthetic incident bundle containing a Stellar seed, an
   ``Authorization:`` header, a bearer token, an ``--admin-key`` flag, env
   assignments, a JSON credential and a URL password loses every one of those
   values;
2. the surrounding structure survives — the artifact is still diagnosable;
3. a rule that stops matching is a failure, not a silent pass (the gate fails
   if any rule reports zero findings);
4. the CLI fails closed when redaction is disabled, and writes no artifact;
5. the report itself contains none of the secrets.

The corpus is generated at runtime (see ``services/evidence_redaction/fixtures``)
so the repository never commits a key-shaped string.
"""

from __future__ import annotations

import argparse
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional, Sequence

from services.evidence_redaction import fixtures
from services.evidence_redaction.redact import (
    RULES,
    RedactionDisabled,
    Redactor,
    main as redact_main,
    redact_text,
    render_report,
)


@dataclass(frozen=True)
class Check:
    name: str
    ok: bool
    detail: str = ""

    def line(self) -> str:
        mark = "PASS" if self.ok else "FAIL"
        return f"{mark} {self.name}" + (f" — {self.detail}" if self.detail else "")


def run(seed: int = 7) -> List[Check]:
    checks: List[Check] = []
    evidence = fixtures.synthetic_incident_log(seed)
    secrets = fixtures.synthetic_secrets(seed)
    result = Redactor().redact(evidence)

    for secret in secrets:
        checks.append(
            Check(
                f"synthetic secret removed ({secret[:2]}…, {len(secret)} chars)",
                secret not in result.text,
            )
        )

    checks.append(Check("all expected rules fired", set(result.counts) == {r.id for r in RULES},
                        f"fired {sorted(result.counts)}"))
    checks.append(Check("every rule accounted for at least one value",
                        all(result.counts.get(rule.id, 0) > 0 for rule in RULES)))
    checks.append(Check("markers name the rule but not the value",
                        "[REDACTED:stellar-seed]" in result.text
                        and "[REDACTED:auth-header]" in result.text))
    checks.append(Check("admin flag label is preserved",
                        "--admin-key [REDACTED:" in result.text))
    checks.append(Check("authorization scheme is preserved",
                        "Authorization: Basic [REDACTED:auth-header]" in result.text))
    checks.append(Check("public account and contract ids survive",
                        fixtures.OPERATOR_ACCOUNT in result.text
                        and fixtures.CONTRACT_ID in result.text))
    checks.append(Check("evidence stays diagnosable",
                        "401 Unauthorized" in result.text and "state dump: admin=" in result.text))

    report = render_report(result, "synthetic-incident.log")
    checks.append(Check("report quotes no secret material", all(s not in report for s in secrets)))
    checks.append(Check("report is bounded to rule ids and lines",
                        "| stellar-seed |" in report and "| auth-header |" in report))

    try:
        Redactor(enabled=False)
        checks.append(Check("disabled redaction raises", False, "Redactor(enabled=False) returned"))
    except RedactionDisabled:
        checks.append(Check("disabled redaction raises", True))

    try:
        redact_text(evidence, enabled=False)
        checks.append(Check("disabled redaction never returns the input", False))
    except RedactionDisabled:
        checks.append(Check("disabled redaction never returns the input", True))

    with tempfile.TemporaryDirectory() as tmp:
        raw = Path(tmp) / "raw.log"
        raw.write_text(evidence, encoding="utf-8")
        out = Path(tmp) / "redacted.log"
        rc = redact_main([str(raw), "--out", str(out)])
        checks.append(Check("CLI redacts a file end to end", rc == 0 and out.exists()
                            and all(s not in out.read_text() for s in secrets)))

        refused = Path(tmp) / "refused.log"
        rc = redact_main([str(raw), "--no-redact", "--out", str(refused)])
        checks.append(Check("CLI refuses --no-redact and writes nothing", rc == 2 and not refused.exists()))

        rc = redact_main([str(raw), "--check"])
        checks.append(Check("--check fails on unredacted evidence", rc == 1))
        rc = redact_main([str(out), "--check"])
        checks.append(Check("--check passes on redacted evidence", rc == 0))

    return checks


def render_report_md(checks: Sequence[Check]) -> str:
    passed = sum(1 for c in checks if c.ok)
    lines = [
        "# Evidence redaction gate (#687)",
        "",
        f"- checks: {len(checks)}",
        f"- passed: {passed}",
        f"- failed: {len(checks) - passed}",
        "",
        "| Result | Check | Detail |",
        "|---|---|---|",
    ]
    for check in checks:
        lines.append(f"| {'pass' if check.ok else '**fail**'} | {check.name} | {check.detail} |")
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Evidence redaction gate (#687)")
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--report", type=Path, default=None)
    args = parser.parse_args(argv)

    checks = run(args.seed)
    for check in checks:
        print(check.line())
    passed = sum(1 for c in checks if c.ok)
    print(f"\n{passed}/{len(checks)} redaction checks passed")

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(render_report_md(checks), encoding="utf-8")
        print(f"report written to {args.report}")

    return 0 if passed == len(checks) else 1


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
