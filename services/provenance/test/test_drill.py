"""Unit tests for release artifact provenance verification drill (#682)."""
import json
from pathlib import Path
import pytest

from services.provenance.drill import ProvenanceVerificationDrill


def test_provenance_drill_executes_successfully_and_verifies_rejections(tmp_path: Path):
    drill = ProvenanceVerificationDrill(drill_id="test-clean-env-drill")
    report = drill.run_drill(tmp_path)

    assert report.all_scenarios_passed is True
    assert report.genuine_artifact_verified is True
    assert report.altered_artifact_rejected is True
    assert report.mismatched_provenance_rejected is True
    assert report.mismatched_signer_rejected is True
    assert report.zero_secrets_verified is True
    assert len(report.steps) == 4


def test_provenance_drill_contains_zero_secrets(tmp_path: Path):
    drill = ProvenanceVerificationDrill()
    report = drill.run_drill(tmp_path)

    report_str = json.dumps(report.to_dict())
    assert "SECRET" not in report_str or "zero_secrets_verified" in report_str
    assert "PRIVATE KEY" not in report_str


def test_provenance_drill_catches_tampered_artifact(tmp_path: Path):
    drill = ProvenanceVerificationDrill()
    report = drill.run_drill(tmp_path)

    tampered_step = next(s for s in report.steps if s.scenario == "tampered_artifact_rejection")
    assert tampered_step.passed is True
    assert tampered_step.expected_rejection is True
