"""Release artifact provenance verification drill automation (#682).

Exercises verification of signatures, in-toto statements, and SLSA v1.0 build
provenance from a clean environment using published verification instructions.
Asserts that legitimate artifacts pass, and that altered artifacts and
mismatched provenance statements are strictly rejected.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

from services.provenance.attest import (
    PREDICATE_TYPE,
    STATEMENT_TYPE,
    UPSTREAM_REPO,
    WASM_NAME,
    BuildEnvironment,
    Provenance,
    ProvenanceBundle,
    sha256_file,
    signer_identity,
    verify,
)

SECRET_SEED_PATTERN = re.compile(r"\bS[A-Z2-7]{55}\b")
PRIVATE_KEY_PATTERN = re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY")


@dataclass
class DrillStepResult:
    step_name: str
    scenario: str
    passed: bool
    expected_rejection: bool
    details: Dict[str, Any]
    error_message: Optional[str] = None


@dataclass
class DrillReport:
    drill_id: str
    all_scenarios_passed: bool
    genuine_artifact_verified: bool
    altered_artifact_rejected: bool
    mismatched_provenance_rejected: bool
    mismatched_signer_rejected: bool
    zero_secrets_verified: bool
    steps: List[DrillStepResult]

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


class ProvenanceVerificationDrill:
    """Executes the release provenance verification drill in an isolated sandbox."""

    def __init__(
        self,
        drill_id: str = "provenance-drill-2026",
        commit_sha: str = "a" * 40,
        workflow_ref: str = ".github/workflows/release.yml@refs/tags/v1.0.0",
        tag: str = "v1.0.0",
    ) -> None:
        self.drill_id = drill_id
        self.commit_sha = commit_sha
        self.workflow_ref = workflow_ref
        self.tag = tag
        self.workflow_name = "release.yml"
        self.signer = signer_identity(self.workflow_name, f"refs/tags/{tag}")

    def run_drill(self, clean_dir: Path) -> DrillReport:
        clean_dir.mkdir(parents=True, exist_ok=True)
        artifact_path = clean_dir / WASM_NAME

        # 1. Produce genuine release binary fixture (valid mock WASM bytecode header)
        genuine_bytes = b"\x00asm\x01\x00\x00\x00" + b"\x01\x04\x01\x60\x00\x00" * 32
        artifact_path.write_bytes(genuine_bytes)
        artifact_sha256 = hashlib.sha256(genuine_bytes).hexdigest()

        # Generate genuine signed provenance bundle
        prov = Provenance(
            subject_name=WASM_NAME,
            subject_sha256=artifact_sha256,
            commit_sha=self.commit_sha,
            repository=UPSTREAM_REPO,
            workflow_ref=self.workflow_ref,
            run_id="99887766",
            environment=BuildEnvironment(),
        )
        bundle = ProvenanceBundle(
            statement=prov.to_statement(),
            signer_identity=self.signer,
        )

        steps: List[DrillStepResult] = []

        # ---------------------------------------------------------------------
        # Scenario 1: Genuine Release Verification (Clean Environment)
        # ---------------------------------------------------------------------
        res_genuine = verify(
            bundle=bundle,
            artifact=artifact_path,
            expected_commit=self.commit_sha,
            expected_workflow_ref=self.workflow_ref,
            expected_signer=self.signer,
        )
        steps.append(
            DrillStepResult(
                step_name="Verify Genuine Release",
                scenario="genuine_artifact_valid_provenance",
                passed=res_genuine.ok,
                expected_rejection=False,
                details={"errors": res_genuine.errors, "checks": len(res_genuine.checked)},
                error_message=None if res_genuine.ok else "; ".join(res_genuine.errors),
            )
        )

        # ---------------------------------------------------------------------
        # Scenario 2: Altered Artifact Rejection (Tampered binary)
        # ---------------------------------------------------------------------
        altered_path = clean_dir / f"tampered_{WASM_NAME}"
        # Tamper exactly 1 byte in the payload
        tampered_bytes = bytearray(genuine_bytes)
        tampered_bytes[-1] ^= 0xFF
        altered_path.write_bytes(bytes(tampered_bytes))

        res_altered = verify(
            bundle=bundle,
            artifact=altered_path,
            expected_commit=self.commit_sha,
            expected_workflow_ref=self.workflow_ref,
            expected_signer=self.signer,
        )
        # MUST BE REJECTED (ok == False)
        altered_rejected = (not res_altered.ok) and any("digest" in e for e in res_altered.errors)
        steps.append(
            DrillStepResult(
                step_name="Detect Altered Artifact",
                scenario="tampered_artifact_rejection",
                passed=altered_rejected,
                expected_rejection=True,
                details={"errors": res_altered.errors},
                error_message="Tampered artifact was unexpectedly accepted!" if not altered_rejected else None,
            )
        )

        # ---------------------------------------------------------------------
        # Scenario 3: Mismatched Provenance Statement (Wrong commit binding)
        # ---------------------------------------------------------------------
        fake_commit = "b" * 40
        res_mismatched_commit = verify(
            bundle=bundle,
            artifact=artifact_path,
            expected_commit=fake_commit,  # expecting different commit
            expected_workflow_ref=self.workflow_ref,
            expected_signer=self.signer,
        )
        commit_mismatch_rejected = (not res_mismatched_commit.ok) and any("commit" in e for e in res_mismatched_commit.errors)
        steps.append(
            DrillStepResult(
                step_name="Reject Mismatched Commit Provenance",
                scenario="mismatched_commit_statement",
                passed=commit_mismatch_rejected,
                expected_rejection=True,
                details={"errors": res_mismatched_commit.errors},
            )
        )

        # ---------------------------------------------------------------------
        # Scenario 4: Mismatched Signer Identity (Forged signer)
        # ---------------------------------------------------------------------
        fake_signer = "attacker/evil-workflow/.github/workflows/fake.yml@refs/heads/main"
        forged_bundle = ProvenanceBundle(
            statement=bundle.statement,
            signer_identity=fake_signer,
        )
        res_forged_signer = verify(
            bundle=forged_bundle,
            artifact=artifact_path,
            expected_commit=self.commit_sha,
            expected_workflow_ref=self.workflow_ref,
            expected_signer=self.signer,
        )
        signer_mismatch_rejected = (not res_forged_signer.ok) and any("signer identity" in e for e in res_forged_signer.errors)
        steps.append(
            DrillStepResult(
                step_name="Reject Forged Signer Identity",
                scenario="forged_signer_rejection",
                passed=signer_mismatch_rejected,
                expected_rejection=True,
                details={"errors": res_forged_signer.errors},
            )
        )

        all_passed = all(s.passed for s in steps)

        report = DrillReport(
            drill_id=self.drill_id,
            all_scenarios_passed=all_passed,
            genuine_artifact_verified=res_genuine.ok,
            altered_artifact_rejected=altered_rejected,
            mismatched_provenance_rejected=commit_mismatch_rejected,
            mismatched_signer_rejected=signer_mismatch_rejected,
            zero_secrets_verified=True,
            steps=steps,
        )

        # Verify no secret material leaks in the report
        report_str = json.dumps(report.to_dict())
        if SECRET_SEED_PATTERN.search(report_str) or PRIVATE_KEY_PATTERN.search(report_str):
            raise ValueError("CRITICAL: Secret key material detected in provenance drill report!")

        return report


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Release Artifact Provenance Verification Drill")
    parser.add_argument("--dir", type=Path, default=Path("/tmp/provenance_drill"), help="Clean environment directory")
    parser.add_argument("--out", type=Path, help="Output drill report JSON path")
    args = parser.parse_args(argv)

    drill = ProvenanceVerificationDrill()
    report = drill.run_drill(args.dir)
    report_dict = report.to_dict()

    if args.out:
        args.out.write_text(json.dumps(report_dict, indent=2) + "\n", encoding="utf-8")
        print(f"Wrote drill report to {args.out}")
    else:
        print(json.dumps(report_dict, indent=2))

    return 0 if report.all_scenarios_passed else 1


if __name__ == "__main__":
    sys.exit(main())
