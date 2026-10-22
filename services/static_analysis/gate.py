"""CI entry point for the static-analysis / SAST gate (#500).

Runs, in one command and one report:

1. the source-level SAST pass (`services.static_analysis.sast`);
2. the unsafe/finding-count baseline check (`policy.check_unsafe`);
3. the dependency-advisory check over `cargo audit --json` output
   (`policy.check_advisories`), which is also what `cargo deny check advisories`
   and `cargo deny check bans` cover via `deny.toml`.

Every finding that is not covered by an owned, unexpired, path-scoped allowlist
entry fails the job, and a human-readable report is written for the run
artifacts.

    python -m services.static_analysis.gate --root contracts \\
        --baseline config/security-baseline.json \\
        --allowlist config/security-allowlist.json \\
        --report security-artifacts/static-analysis.md
"""
from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import List, Optional, Sequence

from services.static_analysis.policy import (
    check_advisories,
    check_unsafe,
    effective_baseline,
    exceedances,
    filter_allowlisted,
    load_allowlist,
    load_baseline,
    parse_cargo_audit,
    rebaseline,
)
from services.static_analysis.sast import render_report, scan_tree, unsafe_counts

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BASELINE = REPO_ROOT / "config" / "security-baseline.json"
DEFAULT_ALLOWLIST = REPO_ROOT / "config" / "security-allowlist.json"
DEFAULT_ROOT = REPO_ROOT / "contracts"


def run_cargo_audit(timeout: int = 300) -> List:
    """Runs `cargo audit --json` if cargo-audit is installed.

    Returns an empty list when the tool is unavailable, so the SAST and
    baseline gates still run on a machine without the Rust toolchain. CI
    installs cargo-audit explicitly, so there the advisory check is live.
    """
    try:
        proc = subprocess.run(
            ["cargo", "audit", "--json", "--file", "Cargo.lock"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return []
    if not proc.stdout.strip():
        return []
    try:
        raw = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return []
    return parse_cargo_audit(raw)


def gate(
    root: Path = DEFAULT_ROOT,
    baseline_path: Path = DEFAULT_BASELINE,
    allowlist_path: Path = DEFAULT_ALLOWLIST,
    advisories: Optional[Sequence] = None,
) -> dict:
    """Runs every check and returns a structured result.

    `result["ok"]` is the single boolean the CI job branches on.
    """
    findings = scan_tree(root)

    allowlist, allowlist_errors = load_allowlist(allowlist_path)
    baseline, baseline_errors = load_baseline(baseline_path)
    # The digest is checked against the committed file, then the allowlisted
    # `rule:file` pairs are taken out of scope for the count comparison — in
    # both directions, so an allowlisted pair is neither "unsafe creep" nor
    # "baseline drift".
    effective = effective_baseline(baseline, allowlist)
    in_scope, allowlisted = filter_allowlisted(findings, allowlist)
    counts = unsafe_counts(in_scope)

    new_unsafe, unsafe_errors = check_unsafe(counts, effective)
    # Anything beyond the baseline is a new finding.
    new_findings = exceedances(in_scope, effective)
    new_advisories, allowed_advisories, notes = check_advisories(
        list(advisories or []), allowlist
    )

    errors: List[str] = list(allowlist_errors) + list(baseline_errors) + list(unsafe_errors)
    errors.extend(notes)
    errors.extend(
        f"new SAST finding `{f.id}` at {f.path}:{f.line} — {f.message}"
        for f in new_findings
    )
    errors.extend(
        f"new {a.id} for crate `{a.crate}`"
        + ("" if a.has_fix else " (no patched version)")
        for a in new_advisories
    )
    for key in new_unsafe:
        errors.append(
            f"unsafe addition beyond baseline — {key}; fix it, or re-baseline "
            "deliberately with `--rebaseline <reviewer>`"
        )

    return {
        "ok": not errors,
        "findings": findings,
        "counts": counts,
        "new_findings": new_findings,
        "allowlisted": allowlisted,
        "allowlist": allowlist,
        "baseline": baseline,
        "advisories": list(advisories or []),
        "new_advisories": new_advisories,
        "allowed_advisories": allowed_advisories,
        "errors": errors,
        "report": render_report(
            findings,
            advisories or [],
            new_findings,
            allowlisted,
            notes,
            counts,
        ),
    }


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Static analysis gate (#500)")
    parser.add_argument("--root", default=str(DEFAULT_ROOT))
    parser.add_argument("--baseline", default=str(DEFAULT_BASELINE))
    parser.add_argument("--allowlist", default=str(DEFAULT_ALLOWLIST))
    parser.add_argument("--report", default=None, help="write a markdown report here")
    parser.add_argument(
        "--skip-advisories",
        action="store_true",
        help="do not shell out to cargo-audit (SAST + baseline only)",
    )
    parser.add_argument(
        "--rebaseline",
        metavar="REVIEWER",
        default=None,
        help="rewrite the baseline for the current findings, approved by REVIEWER",
    )
    args = parser.parse_args(argv)

    advisories = [] if args.skip_advisories else run_cargo_audit()
    result = gate(
        root=Path(args.root),
        baseline_path=Path(args.baseline),
        allowlist_path=Path(args.allowlist),
        advisories=advisories,
    )

    if args.rebaseline:
        baseline = rebaseline(result["counts"], args.rebaseline)
        Path(args.baseline).write_text(
            json.dumps(baseline.to_dict(), indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(f"re-baselined {args.baseline} (approved_by={args.rebaseline})")
        return 0

    if args.report:
        report_path = Path(args.report)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(result["report"], encoding="utf-8")
        print(f"report written to {report_path}")

    for error in result["errors"]:
        print(f"::error::{error}")
    print(
        f"{len(result['findings'])} finding(s), {len(result['new_findings'])} new, "
        f"{len(result['advisories'])} advisory(ies), {len(result['errors'])} error(s)"
    )
    return 0 if result["ok"] else 1


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
