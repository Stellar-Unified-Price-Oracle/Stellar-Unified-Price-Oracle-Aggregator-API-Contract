"""Unit tests for dependency advisory triage and remediation SLA (#681)."""
from datetime import datetime, timezone
import pytest

from services.dependency_advisory.triage import (
    AdvisoryRecord,
    Ecosystem,
    Severity,
    create_seeded_advisory,
    evaluate_advisories,
    triage_advisory,
    SLA_DEADLINES_DAYS,
    RELEASE_BLOCKING_POLICY,
    ECOSYSTEM_OWNERSHIP,
)


def test_seeded_critical_advisory_produces_expected_ticket_and_blocks_release():
    seeded = create_seeded_advisory(Severity.CRITICAL, Ecosystem.RUST)
    ref_time = datetime(2026, 10, 20, 12, 0, 0, tzinfo=timezone.utc)
    ticket = triage_advisory(seeded, reference_time=ref_time)

    assert ticket.severity == "CRITICAL"
    assert ticket.release_blocked is True
    assert "RELEASE BLOCKED" in ticket.blocking_decision_reason
    assert ticket.assigned_owner == "Core Contracts Tech Lead"
    # Due date must be 1 day from discovery
    assert ticket.due_date == "2026-10-21"
    assert "### [CRITICAL]" in ticket.ticket_markdown


def test_seeded_high_advisory_blocks_release_with_7_day_sla():
    seeded = create_seeded_advisory(Severity.HIGH, Ecosystem.JAVASCRIPT)
    ref_time = datetime(2026, 10, 20, 12, 0, 0, tzinfo=timezone.utc)
    ticket = triage_advisory(seeded, reference_time=ref_time)

    assert ticket.severity == "HIGH"
    assert ticket.release_blocked is True
    assert ticket.assigned_owner == "SDK & Frontend Tech Lead"
    assert ticket.due_date == "2026-10-27"  # +7 days


def test_medium_and_low_advisories_do_not_block_release():
    med_adv = create_seeded_advisory(Severity.MEDIUM, Ecosystem.OPERATIONAL)
    low_adv = create_seeded_advisory(Severity.LOW, Ecosystem.RUST)

    med_ticket = triage_advisory(med_adv)
    low_ticket = triage_advisory(low_adv)

    assert med_ticket.release_blocked is False
    assert "RELEASE PERMITTED" in med_ticket.blocking_decision_reason
    assert med_ticket.assigned_owner == "DevOps & Infrastructure Lead"

    assert low_ticket.release_blocked is False
    assert low_ticket.assigned_owner == "Core Contracts Tech Lead"


def test_batch_evaluation_aggregates_blocking_status():
    advs = [
        create_seeded_advisory(Severity.MEDIUM, Ecosystem.RUST),
        create_seeded_advisory(Severity.LOW, Ecosystem.JAVASCRIPT),
    ]
    report_non_blocking = evaluate_advisories(advs)
    assert report_non_blocking.release_blocked is False

    # Adding a critical advisory causes batch to block release
    advs.append(create_seeded_advisory(Severity.CRITICAL, Ecosystem.RUST))
    report_blocking = evaluate_advisories(advs)
    assert report_blocking.release_blocked is True
    assert report_blocking.critical_count == 1
    assert report_blocking.medium_count == 1
    assert report_blocking.low_count == 1


def test_advisory_ticket_contains_no_secret_key_material():
    fake_seed = "S" + "C" * 55
    adv = AdvisoryRecord(
        advisory_id="TEST-001",
        ecosystem=Ecosystem.RUST,
        package_name="test-pkg",
        vulnerable_versions="< 1.0",
        severity=Severity.HIGH,
        title="Title",
        description=f"Leaked seed: {fake_seed}",
        discovered_at="2026-10-20T12:00:00Z",
    )
    with pytest.raises(ValueError, match="CRITICAL: Secret key material detected"):
        triage_advisory(adv)
