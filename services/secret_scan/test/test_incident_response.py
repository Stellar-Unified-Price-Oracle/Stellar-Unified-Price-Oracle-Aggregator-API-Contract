"""Tests for secret scanning incident response procedure (#683)."""
from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path

import pytest

from services.secret_scan.incident_response import (
    HistoryAssessment,
    IncidentResponseReport,
    PublicIncidentRecord,
    RevocationReceipt,
    SecretExposureFinding,
    SecurityLeakError,
    execute_incident_response,
    redact_secret_value,
    verify_zero_secret_in_records,
)

# Test fragments assembled at runtime to prevent static scanner triggers
DUMMY_STELLAR_SEED = "S" + "B" * 55
DUMMY_AWS_KEY = "AK" + "IA" + "Z9876543210ABCDE"
DUMMY_GITHUB_PAT = "ghp" + "_" + "k" * 36


def test_redact_secret_value():
    assert redact_secret_value("") == ""
    assert redact_secret_value("short") == "*****"
    assert redact_secret_value("12345678") == "********"
    long_secret = "1234567890abcdef"
    redacted = redact_secret_value(long_secret)
    assert redacted.startswith("1234")
    assert redacted.endswith("cdef")
    assert "*" * 8 in redacted
    assert long_secret not in redacted


def test_simulated_finding_follows_full_process_with_zero_public_leak():
    finding = SecretExposureFinding(
        finding_id="FIND-2026-TEST-001",
        rule_id="SECRET-SOROBAN-SECRET-SEED",
        raw_secret=DUMMY_STELLAR_SEED,
        file_path="contracts/oracle/src/test_signer.rs",
        line_number=42,
        detected_at=datetime(2026, 4, 1, 10, 0, 0, tzinfo=timezone.utc),
        commit_hash="c0ffee12",
        author="operator@stellar-oracle.org",
        detection_source="scanner-history",
    )

    report = execute_incident_response(finding)

    assert report.zero_secret_verified is True
    assert report.public_record.status == "CREDENTIAL_REVOKED"
    assert report.public_record.rule_id == "SECRET-SOROBAN-SECRET-SEED"
    assert report.public_record.commit_hash == "c0ffee12"

    # Revocation receipt validation
    assert report.public_record.revocation_receipt.status == "REVOKED"
    assert report.public_record.revocation_receipt.provider == "StellarNetworkSignerRegistry"
    assert report.public_record.revocation_receipt.receipt_id.startswith("REV-")

    # History & blast radius assessment
    assert report.public_record.history_assessment.blast_radius == "CRITICAL"
    assert "OracleDeployerSigner" in report.public_record.history_assessment.service_impact
    assert report.public_record.history_assessment.first_commit == "c0ffee12"

    # Verify zero-secret invariant across public outputs
    rendered_md = report.public_record.to_markdown()
    serialized_dict = report.to_dict()
    serialized_json = json.dumps(serialized_dict)

    assert DUMMY_STELLAR_SEED not in rendered_md
    assert DUMMY_STELLAR_SEED not in serialized_json
    for notification in report.notifications:
        assert DUMMY_STELLAR_SEED not in notification.content
        assert DUMMY_STELLAR_SEED not in notification.subject

    # Strict validator confirms no leakage
    assert verify_zero_secret_in_records(report, DUMMY_STELLAR_SEED) is True


def test_detector_flags_and_rejects_raw_secret_leak_in_records():
    finding = SecretExposureFinding(
        finding_id="FIND-2026-TEST-002",
        rule_id="SECRET-AWS-ACCESS-KEY-ID",
        raw_secret=DUMMY_AWS_KEY,
        file_path="deploy/aws-credentials.env",
        line_number=5,
        detected_at=datetime.now(timezone.utc),
    )

    report = execute_incident_response(finding)

    # Artificially inject raw secret into remediation notes
    report.public_record.remediation_notes.append(f"Accidental leak: {DUMMY_AWS_KEY}")

    with pytest.raises(SecurityLeakError, match="CRITICAL: Raw secret value was found"):
        verify_zero_secret_in_records(report, DUMMY_AWS_KEY)


def test_revocation_failure_is_recorded_as_containment_failed():
    finding = SecretExposureFinding(
        finding_id="FIND-2026-TEST-003",
        rule_id="SECRET-GITHUB-TOKEN",
        raw_secret=DUMMY_GITHUB_PAT,
        file_path=".github/workflows/sync.yml",
        line_number=19,
        detected_at=datetime.now(timezone.utc),
    )

    def failing_revoker(_finding: SecretExposureFinding) -> RevocationReceipt:
        return RevocationReceipt(
            receipt_id="REV-ERR-001",
            status="FAILED",
            rule_id=_finding.rule_id,
            revoked_at=datetime.now(timezone.utc).isoformat(),
            provider="GitHub_App_Token_Service",
            audit_id="AUD-FAIL-001",
            details={"error": "API rate limit exceeded"},
        )

    report = execute_incident_response(finding, revocation_handler=failing_revoker)

    assert report.public_record.status == "CONTAINMENT_FAILED"
    assert report.public_record.revocation_receipt.status == "FAILED"
    assert report.zero_secret_verified is True
    assert DUMMY_GITHUB_PAT not in json.dumps(report.to_dict())


def test_history_assessment_without_commit_hash():
    finding = SecretExposureFinding(
        finding_id="FIND-2026-TEST-004",
        rule_id="SECRET-GENERIC-ASSIGNMENT",
        raw_secret="pass" + "word1234567890",
        file_path="config/staging.toml",
        line_number=12,
        detected_at=datetime.now(timezone.utc),
        commit_hash=None,
    )

    report = execute_incident_response(finding)

    assert report.public_record.commit_hash is None
    assert report.public_record.history_assessment.first_commit is None
    assert report.public_record.history_assessment.blast_radius == "MEDIUM"
    assert "Incident Record: SEC-INC-" in report.public_record.to_markdown()
