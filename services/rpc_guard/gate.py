"""Replays the RPC trust-boundary fixture corpus (#684).

A validator that only has unit tests drifts: a field check gets relaxed during
a refactor and nothing notices until a bad response reaches production. This
gate replays a committed corpus of good and malformed responses and fails if
any case stops being accepted or stops being rejected with its documented
code, so the boundary is a repeatable check rather than a claim in a doc.

    python3 -m services.rpc_guard.gate
    python3 -m services.rpc_guard.gate --report security-artifacts/rpc-boundary.md
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

from services.rpc_guard.validator import (
    NetworkPolicy,
    RpcGuard,
    RpcValidationError,
)

FIXTURES = Path(__file__).resolve().parent / "fixtures"
CASES = FIXTURES / "cases.json"


@dataclass(frozen=True)
class CaseResult:
    name: str
    step: str
    expected: str
    observed: str
    ok: bool

    def line(self) -> str:
        mark = "PASS" if self.ok else "FAIL"
        return f"{mark} {self.step:<11} {self.expected:<22} {self.observed:<22} {self.name}"


def load_manifest(path: Path = CASES) -> Dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _fixture(name: str) -> Dict[str, Any]:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def _run_case(case: Dict[str, Any], manifest: Dict[str, Any]) -> CaseResult:
    policy = NetworkPolicy(
        passphrase=manifest["policy"]["passphrase"],
        network_id=manifest["policy"]["network_id"],
        min_protocol_version=manifest["policy"]["min_protocol_version"],
        max_protocol_version=manifest["policy"]["max_protocol_version"],
    )
    guard = RpcGuard(policy)
    step = case["step"]

    # A ledger cannot be validated before a network; a transaction cannot be
    # validated before a ledger. The gate exercises the same ordering the
    # production callers must use.
    guard.accept_network(_fixture(manifest["baseline"]["network"]))
    if step != "network":
        guard.accept_ledger(_fixture(manifest["baseline"]["ledger"]))

    payload = _fixture(case["fixture"])
    try:
        if step == "network":
            guard.accept_network(payload)
        elif step == "ledger":
            guard.accept_ledger(payload)
        elif step == "transaction":
            guard.accept_transaction(payload, expected_hash=manifest["transaction_hash"])
        else:  # pragma: no cover - manifest is part of the corpus
            raise AssertionError(f"unknown step {step!r}")
        observed = "ok"
    except RpcValidationError as error:
        observed = error.code.value

    return CaseResult(
        name=case["name"],
        step=step,
        expected=case["expect"],
        observed=observed,
        ok=observed == case["expect"],
    )


def run(manifest: Optional[Dict[str, Any]] = None) -> List[CaseResult]:
    manifest = manifest or load_manifest()
    return [_run_case(case, manifest) for case in manifest["cases"]]


def render_report(results: Sequence[CaseResult]) -> str:
    passed = sum(1 for r in results if r.ok)
    lines = [
        "# RPC trust-boundary gate (#684)",
        "",
        f"- cases: {len(results)}",
        f"- passed: {passed}",
        f"- failed: {len(results) - passed}",
        "",
        "| Step | Expected | Observed | Result | Case |",
        "|---|---|---|---|---|",
    ]
    for r in results:
        mark = "pass" if r.ok else "**fail**"
        lines.append(f"| {r.step} | {r.expected} | {r.observed} | {mark} | {r.name} |")
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="RPC trust-boundary fixture gate (#684)")
    parser.add_argument("--report", type=Path, default=None, help="write a markdown report here")
    args = parser.parse_args(argv)

    results = run()
    for result in results:
        print(result.line())
    passed = sum(1 for r in results if r.ok)
    print(f"\n{passed}/{len(results)} RPC boundary cases behave as documented")

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(render_report(results), encoding="utf-8")
        print(f"report written to {args.report}")

    return 0 if passed == len(results) else 1


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
