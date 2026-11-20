#!/usr/bin/env python3
"""Timed operator handover / on-call readiness drill (#626).

    python3 scripts/readiness_drill.py --report readiness-drill-report.md

Runs the drill steps declared in `config/readiness-drill.json`, each with a
response-time budget in seconds. A step passes when its check command exits 0
within budget. The run fails (exit 1) when any step misses its budget so
corrective actions get owned and tracked. The markdown report is always
written, including on failure, so the CI artifact is useful either way.

Step kinds:
  * `file`  — the referenced path must exist (runbook / contact sheet present).
  * `cmd`   — the command must exit 0 within the budget.

See docs/on-call-readiness.md.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONFIG = ROOT / "config" / "readiness-drill.json"


def load_steps(path: Path = CONFIG) -> list[dict]:
    data = json.loads(path.read_text())
    steps = data.get("steps", [])
    if not steps:
        raise ValueError(f"{path}: no steps declared")
    return steps


def run_step(step: dict, root: Path = ROOT) -> dict:
    """Execute one drill step and return its result record."""
    name = step.get("name", "?")
    budget = float(step.get("budget_seconds", 60))
    kind = step.get("kind", "cmd")
    started = time.monotonic()
    ok = False
    detail = ""
    try:
        if kind == "file":
            target = root / step["path"]
            ok = target.exists()
            detail = f"{step['path']} {'present' if ok else 'missing'}"
        elif kind == "cmd":
            proc = subprocess.run(
                step["command"],
                shell=True,
                cwd=root,
                capture_output=True,
                text=True,
                timeout=budget,
            )
            ok = proc.returncode == 0
            output = (proc.stdout or proc.stderr or "").strip().splitlines()
            detail = output[-1] if output else f"exit {proc.returncode}"
        else:
            detail = f"unknown step kind {kind!r}"
    except subprocess.TimeoutExpired:
        detail = f"timed out after {budget:g}s"
    except (KeyError, OSError) as exc:
        detail = f"error: {exc}"
    elapsed = time.monotonic() - started
    return {
        "name": name,
        "kind": kind,
        "budget_seconds": budget,
        "elapsed_seconds": round(elapsed, 3),
        "within_budget": elapsed <= budget,
        "passed": ok and elapsed <= budget,
        "detail": detail,
    }


def render_report(results: list[dict]) -> str:
    failed = [r for r in results if not r["passed"]]
    lines = [
        "# On-call readiness drill report",
        "",
        f"Steps: {len(results)} · passed: {len(results) - len(failed)} · failed: {len(failed)}",
        "",
        "| Step | Kind | Budget (s) | Elapsed (s) | Result | Detail |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for r in results:
        verdict = "PASS" if r["passed"] else "FAIL"
        if r["passed"] is False and not r["within_budget"]:
            verdict = "FAIL (over budget)"
        detail = r["detail"].replace("|", "\\|")
        lines.append(
            f"| {r['name']} | {r['kind']} | {r['budget_seconds']:g} | "
            f"{r['elapsed_seconds']:.3f} | {verdict} | {detail} |"
        )
    lines.append("")
    if failed:
        lines.append("## Corrective actions")
        lines.append("")
        for r in failed:
            lines.append(f"- [ ] {r['name']}: {r['detail']}")
        lines.append("")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Timed on-call readiness drill (#626)")
    parser.add_argument("--report", default="readiness-drill-report.md", help="markdown report path")
    parser.add_argument("--config", default=str(CONFIG), help="drill step config (JSON)")
    args = parser.parse_args(argv)

    try:
        steps = load_steps(Path(args.config))
    except (OSError, ValueError) as exc:
        print(f"readiness drill: {exc}", file=sys.stderr)
        return 2

    results = [run_step(s) for s in steps]
    report = render_report(results)
    Path(args.report).write_text(report + "\n")
    print(report)

    failed = [r for r in results if not r["passed"]]
    if failed:
        print(f"readiness drill: {len(failed)} step(s) failed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
