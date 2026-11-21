"""Production deployment approval evidence capture and verification (#729).

Bundles a verified privileged transaction review payload (``oracle.tx_review.v1``),
its two-reviewer production approval (``oracle.tx_review.approval.v1``), the
release tag, the target contract ID, and the green WASM SHA-256 digest into a
canonical, tamper-evident deployment evidence artifact
(``oracle.deployment_approval_evidence.v1``).

Capture fails closed if:
* the payload or approval fails ``services.tx_review.review.verify_approval``
  under the ``production``/``mainnet`` policy (at least two distinct non-self
  reviewers, valid simulation, unexpired window);
* any operation in the payload targets a contract other than ``contract_id``;
* an ``upgrade`` operation's ``new_wasm_hash`` argument does not equal
  ``green_wasm_sha256``; or
* secret key material (Stellar ``S...`` seed or PEM private key) is present.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any, Dict, Mapping, Optional, Sequence

from services.tx_review.payload import PayloadError, load_payload
from services.tx_review.review import (
    ApprovalError,
    ReviewPolicy,
    verify_approval,
)

EVIDENCE_SCHEMA = "oracle.deployment_approval_evidence.v1"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
SECRET_SEED_RE = re.compile(r"\bS[A-Z2-7]{55}\b")
PEM_KEY_RE = re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY")


class DeploymentEvidenceError(ValueError):
    """Raised when deployment approval evidence is incomplete, mismatched, or tampered."""


def _canonical_digest(body: Mapping[str, Any]) -> str:
    canonical = json.dumps(body, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def _parse_iso(ts: str) -> dt.datetime:
    parsed = dt.datetime.fromisoformat(ts.replace("Z", "+00:00"))
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=dt.timezone.utc)


def capture_deployment_evidence(
    *,
    payload_data: Mapping[str, Any],
    approval_data: Mapping[str, Any],
    release_tag: str,
    contract_id: str,
    green_wasm_sha256: str,
    blue_wasm_sha256: Optional[str] = None,
    environment: str = "production",
    captured_at: Optional[dt.datetime] = None,
) -> Dict[str, Any]:
    """Validate and package production deployment approval evidence."""
    if not release_tag or not release_tag.strip():
        raise DeploymentEvidenceError("release_tag is required")
    if not contract_id or not contract_id.strip():
        raise DeploymentEvidenceError("contract_id is required")

    green_hash = green_wasm_sha256.strip().lower()
    if not SHA256_RE.match(green_hash):
        raise DeploymentEvidenceError("green_wasm_sha256 must be a 64-char hex sha256")

    blue_hash: Optional[str] = None
    if blue_wasm_sha256 is not None:
        blue_hash = blue_wasm_sha256.strip().lower()
        if not SHA256_RE.match(blue_hash):
            raise DeploymentEvidenceError("blue_wasm_sha256 must be a 64-char hex sha256")

    moment = captured_at or dt.datetime.now(dt.timezone.utc)
    policy = ReviewPolicy(environment=environment)
    try:
        approved = verify_approval(
            payload_data,
            approval_data,
            policy=policy,
            now=moment,
        )
    except (ApprovalError, PayloadError) as exc:
        raise DeploymentEvidenceError(f"approval verification failed: {exc}") from exc

    for op in approved.payload.operations:
        if op.contract_id != contract_id:
            raise DeploymentEvidenceError(
                f"operation contract_id {op.contract_id!r} != target {contract_id!r}"
            )
        if op.function == "upgrade":
            arg_map = {name: val for name, _t, val in op.args}
            op_hash = str(arg_map.get("new_wasm_hash", "")).lower()
            if op_hash != green_hash:
                raise DeploymentEvidenceError(
                    f"upgrade new_wasm_hash {op_hash!r} != green_wasm_sha256 {green_hash!r}"
                )

    body: Dict[str, Any] = {
        "schema": EVIDENCE_SCHEMA,
        "release_tag": release_tag.strip(),
        "environment": policy.environment,
        "contract_id": contract_id.strip(),
        "green_wasm_sha256": green_hash,
        "blue_wasm_sha256": blue_hash,
        "captured_at": moment.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
        "payload_sha256": approved.digest,
        "requested_by": approval_data["requested_by"],
        "reviewers": list(approved.reviewers),
        "reviewed_at": approved.reviewed_at.astimezone(dt.timezone.utc)
        .isoformat()
        .replace("+00:00", "Z"),
        "expires_at": approved.expires_at.astimezone(dt.timezone.utc)
        .isoformat()
        .replace("+00:00", "Z"),
        "rendered_summary": approval_data["rendered_summary"],
        "simulation": dict(approval_data["simulation"]),
        "payload": dict(payload_data),
        "approval": dict(approval_data),
    }

    serialized = json.dumps(body, sort_keys=True)
    if SECRET_SEED_RE.search(serialized) or PEM_KEY_RE.search(serialized):
        raise DeploymentEvidenceError("secret key material detected in deployment evidence")

    body["evidence_sha256"] = _canonical_digest(body)
    return body


def verify_deployment_evidence(evidence: Mapping[str, Any]) -> Dict[str, Any]:
    """Re-verify a captured deployment approval evidence bundle offline."""
    if not isinstance(evidence, Mapping):
        raise DeploymentEvidenceError("evidence must be a JSON object")
    if evidence.get("schema") != EVIDENCE_SCHEMA:
        raise DeploymentEvidenceError(f"unsupported evidence schema: {evidence.get('schema')!r}")

    recorded_digest = evidence.get("evidence_sha256")
    if not isinstance(recorded_digest, str) or not SHA256_RE.match(recorded_digest):
        raise DeploymentEvidenceError("missing or invalid evidence_sha256")

    without_digest = {k: v for k, v in evidence.items() if k != "evidence_sha256"}
    expected_digest = _canonical_digest(without_digest)
    if recorded_digest != expected_digest:
        raise DeploymentEvidenceError(
            f"evidence_sha256 mismatch: recorded {recorded_digest} != computed {expected_digest}"
        )

    captured_at = _parse_iso(str(evidence.get("captured_at", "")))
    rebuilt = capture_deployment_evidence(
        payload_data=evidence["payload"],
        approval_data=evidence["approval"],
        release_tag=str(evidence["release_tag"]),
        contract_id=str(evidence["contract_id"]),
        green_wasm_sha256=str(evidence["green_wasm_sha256"]),
        blue_wasm_sha256=evidence.get("blue_wasm_sha256"),
        environment=str(evidence.get("environment", "production")),
        captured_at=captured_at,
    )
    if rebuilt["evidence_sha256"] != recorded_digest:
        raise DeploymentEvidenceError("re-verified evidence digest does not match recorded digest")
    return {
        "ok": True,
        "evidence_sha256": recorded_digest,
        "release_tag": rebuilt["release_tag"],
        "contract_id": rebuilt["contract_id"],
        "green_wasm_sha256": rebuilt["green_wasm_sha256"],
        "reviewers": rebuilt["reviewers"],
    }


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Production deployment approval evidence capture and verification (#729)"
    )
    sub = parser.add_subparsers(dest="cmd", required=True)

    cap = sub.add_parser("capture", help="capture and verify a deployment approval evidence bundle")
    cap.add_argument("--payload", type=Path, required=True)
    cap.add_argument("--approval", type=Path, required=True)
    cap.add_argument("--release-tag", required=True)
    cap.add_argument("--contract-id", required=True)
    cap.add_argument("--green-wasm-sha256", required=True)
    cap.add_argument("--blue-wasm-sha256", default=None)
    cap.add_argument("--environment", default="production")
    cap.add_argument("--captured-at", default=None)
    cap.add_argument("--out", type=Path, required=True)

    ver = sub.add_parser("verify", help="verify a captured deployment approval evidence bundle")
    ver.add_argument("--evidence", type=Path, required=True)

    args = parser.parse_args(argv)
    try:
        if args.cmd == "capture":
            payload_data = json.loads(args.payload.read_text(encoding="utf-8"))
            approval_data = json.loads(args.approval.read_text(encoding="utf-8"))
            captured_at = _parse_iso(args.captured_at) if args.captured_at else None
            bundle = capture_deployment_evidence(
                payload_data=payload_data,
                approval_data=approval_data,
                release_tag=args.release_tag,
                contract_id=args.contract_id,
                green_wasm_sha256=args.green_wasm_sha256,
                blue_wasm_sha256=args.blue_wasm_sha256,
                environment=args.environment,
                captured_at=captured_at,
            )
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
            print(f"wrote deployment approval evidence to {args.out} ({bundle['evidence_sha256']})")
            return 0

        data = json.loads(args.evidence.read_text(encoding="utf-8"))
        summary = verify_deployment_evidence(data)
        print(json.dumps(summary, indent=2))
        return 0
    except DeploymentEvidenceError as exc:
        print(f"REFUSED: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
