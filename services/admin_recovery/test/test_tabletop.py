"""Unit tests for production admin key recovery tabletop exercise (#680)."""
import json
import pytest

from services.admin_recovery.tabletop import (
    AdminRecoveryTabletop,
    SECRET_SEED_PATTERN,
    PRIVATE_KEY_PATTERN,
)


def test_tabletop_exercise_success_and_sla_compliance():
    tabletop = AdminRecoveryTabletop(
        scenario_id="tabletop-drill-test-01",
        initial_admin="GA_COMPROMISED_KEY_001",
        rescue_admin="GA_RESCUE_ADMIN_002",
    )
    result = tabletop.run_exercise(
        simulated_detection_delay_s=300.0,
        simulated_containment_delay_s=1800.0,
        pause_decision=True,
    )

    assert result.scenario_id == "tabletop-drill-test-01"
    assert result.pause_executed is True
    assert result.signer_revoked is True
    assert result.guardians_approved is True
    assert result.recovery_executed is True
    assert result.final_admin == "GA_RESCUE_ADMIN_002"
    assert result.sla_met is True
    assert len(result.phases) == 5


def test_tabletop_records_evidence_without_leaking_key_material():
    tabletop = AdminRecoveryTabletop()
    result = tabletop.run_exercise()

    evidence_json = json.dumps(result.to_dict())

    # Strictly ensure zero Stellar secret seeds or PEM private keys are recorded
    assert SECRET_SEED_PATTERN.search(evidence_json) is None
    assert PRIVATE_KEY_PATTERN.search(evidence_json) is None

    # Verify chain of custody hashes exist for each phase
    assert len(result.evidence_bundle["chain_of_custody_hashes"]) == 5
    for h in result.evidence_bundle["chain_of_custody_hashes"]:
        assert len(h) == 64


def test_tabletop_sla_failure_detection_when_exceeded():
    tabletop = AdminRecoveryTabletop()
    # Exceed containment SLA (> 7200s)
    result = tabletop.run_exercise(
        simulated_detection_delay_s=500.0,
        simulated_containment_delay_s=8000.0,
    )
    assert result.sla_met is False
    assert result.time_to_contain_s > 7200.0


def test_tabletop_secret_injection_fails_safely():
    tabletop = AdminRecoveryTabletop()
    # Deliberately attempt injecting key material into alert
    fake_seed = "S" + "B" * 55
    tabletop.initial_admin = fake_seed

    with pytest.raises(ValueError, match="CRITICAL: Secret key material detected"):
        tabletop.run_exercise()
