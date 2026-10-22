"""Tamper corpus for the privileged transaction review guard (#686).

Each case starts from a payload and an approval that *do* agree, applies one
realistic mutation, and asserts the guard still refuses it. Keeping the corpus
as mutations of a valid pair (rather than as hand-written bad fixtures) means
every case is provably about the thing it names, and the digests cannot go
stale.

The mutations are the ways a privileged transaction gets approved and then
changed:

* after the reviews, the argument/sequence/source is edited;
* a second operation is appended to a payload that was reviewed for one;
* an untyped argument is slipped in, or a field the guard does not model;
* the approval's digest or rendered summary is copied from another payload;
* the simulation is swapped, failed, or its fee is understated;
* the review is self-approved, duplicated, stale, dated ahead, or too long.

See ``docs/security/privileged-transaction-review.md``.
"""

from __future__ import annotations

import copy
import datetime as dt
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Mapping, Optional, Sequence, Tuple

from services.tx_review.payload import PayloadError
from services.tx_review.review import ApprovalError, ReviewPolicy, verify_approval

FIXTURES = Path(__file__).resolve().parent / "fixtures"
MANIFEST = FIXTURES / "cases.json"


@dataclass(frozen=True)
class CaseResult:
    name: str
    expected: str
    observed: str
    ok: bool

    def line(self) -> str:
        mark = "PASS" if self.ok else "FAIL"
        return f"{mark} expected={self.expected:<22} observed={self.observed:<22} {self.name}"


def load_manifest(path: Path = MANIFEST) -> Dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def fixture(name: str) -> Dict[str, Any]:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def _clone(value: Any) -> Any:
    return copy.deepcopy(value)


def apply_tamper(
    payload: Dict[str, Any], approval: Dict[str, Any], tamper: Mapping[str, Any]
) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    """Applies one named mutation to a valid (payload, approval) pair."""
    payload, approval = _clone(payload), _clone(approval)
    kind = tamper["kind"]

    if kind == "payload_arg":
        operation = payload["operations"][tamper.get("operation", 0)]
        operation["args"][tamper["arg"]] = tamper["value"]
    elif kind == "payload_sequence":
        payload["sequence"] = int(tamper["value"])
    elif kind == "payload_source":
        payload["source_account"] = tamper["value"]
    elif kind == "payload_fee":
        payload["fee"] = int(tamper["value"])
    elif kind == "payload_max_time":
        payload["max_time"] = int(tamper["value"])
    elif kind == "payload_extra_operation":
        payload["operations"].append(_clone(tamper["value"]))
    elif kind == "payload_drop_operation":
        payload["operations"] = payload["operations"][:-1] or payload["operations"]
    elif kind == "payload_extra_field":
        payload[tamper["field"]] = tamper.get("value", "smuggled")
    elif kind == "payload_untyped_arg":
        payload["operations"][0]["args"][tamper["arg"]] = tamper["value"]
    elif kind == "payload_unknown_function":
        payload["operations"][0]["function"] = tamper["value"]
    elif kind == "payload_swap":
        payload = fixture(tamper["fixture"])
    elif kind == "approval_digest":
        approval["payload_sha256"] = tamper["value"]
    elif kind == "approval_summary":
        approval["rendered_summary"] = tamper["value"]
    elif kind == "approval_simulation_digest":
        approval["simulation"]["payload_sha256"] = tamper["value"]
    elif kind == "approval_simulation_network":
        approval["simulation"]["network"] = tamper["value"]
    elif kind == "approval_simulation_failed":
        approval["simulation"]["success"] = False
    elif kind == "approval_fee":
        approval["simulation"]["fee_stroops"] = int(tamper["value"])
    elif kind == "approval_single_reviewer":
        approval["reviewed_by"] = approval["reviewed_by"][:1]
    elif kind == "approval_duplicate_reviewer":
        approval["reviewed_by"] = [approval["reviewed_by"][0]] * 2
    elif kind == "approval_self_review":
        approval["reviewed_by"] = [approval["requested_by"], *approval["reviewed_by"][1:]]
    elif kind == "approval_expires_at":
        approval["expires_at"] = tamper["value"]
    elif kind == "approval_reviewed_at":
        approval["reviewed_at"] = tamper["value"]
    elif kind == "approval_no_simulation":
        approval.pop("simulation", None)
    elif kind == "approval_unknown_field":
        approval[tamper["field"]] = tamper.get("value", "extra")
    elif kind == "approval_schema":
        approval["schema"] = tamper["value"]
    else:  # pragma: no cover - manifest is part of the corpus
        raise AssertionError(f"unknown tamper kind {kind!r}")
    return payload, approval


def run_case(case: Mapping[str, Any], manifest: Mapping[str, Any]) -> CaseResult:
    payload = fixture(case["payload"])
    approval = fixture(case["approval"])
    if case.get("tamper"):
        payload, approval = apply_tamper(payload, approval, case["tamper"])

    policy = ReviewPolicy(**manifest["policy"])
    moment = case.get("now", manifest["now"])
    now = dt.datetime.fromisoformat(str(moment).replace("Z", "+00:00"))
    try:
        verify_approval(payload, approval, policy=policy, now=now)
        observed = "approved"
    except (ApprovalError, PayloadError) as error:
        observed = error.code.value

    return CaseResult(
        name=case["name"],
        expected=case["expect"],
        observed=observed,
        ok=observed == case["expect"],
    )


def run(manifest: Optional[Mapping[str, Any]] = None) -> List[CaseResult]:
    manifest = manifest or load_manifest()
    return [run_case(case, manifest) for case in manifest["cases"]]


def render_report(results: Sequence[CaseResult]) -> str:
    passed = sum(1 for r in results if r.ok)
    lines = [
        "# Privileged transaction review gate (#686)",
        "",
        f"- cases: {len(results)}",
        f"- passed: {passed}",
        f"- failed: {len(results) - passed}",
        "",
        "| Expected | Observed | Result | Case |",
        "|---|---|---|---|",
    ]
    for r in results:
        mark = "pass" if r.ok else "**fail**"
        lines.append(f"| {r.expected} | {r.observed} | {mark} | {r.name} |")
    return "\n".join(lines) + "\n"
