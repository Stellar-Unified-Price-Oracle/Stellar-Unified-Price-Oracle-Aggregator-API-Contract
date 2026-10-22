"""Unit tests for the external audit finding closure evidence registry (#688)."""
import json

from services.audit_findings.registry import (
    DEFAULT_REGISTRY,
    REPO_ROOT,
    main,
    validate_finding,
    validate_registry,
)

# A real test in this repository, used as the linked regression test.
EXISTING_TEST = "contracts/price-oracle/src/test.rs::test_upgrade_unauthorized"


def closed_finding(**overrides):
    finding = {
        "id": "AUD-2026-001",
        "title": "Upgrade callable without admin authorization",
        "audit": "Synthetic audit 2026-Q3",
        "severity": "HIGH",
        "owner": "core-contracts-lead",
        "status": "CLOSED",
        "reported_at": "2026-09-01",
        "fix_ref": "https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/pull/101",
        "fix_release": "v0.4.2",
        "regression_test": EXISTING_TEST,
        "reviewer": "external-auditor",
        "accepted_at": "2026-09-20",
    }
    finding.update(overrides)
    return finding


def rules(violations):
    return {v.rule for v in violations}


def test_committed_registry_is_valid():
    registry = json.loads(DEFAULT_REGISTRY.read_text(encoding="utf-8"))
    report = validate_registry(registry)
    assert report.ok, [str(v) for v in report.violations]


def test_closed_finding_with_full_evidence_passes():
    assert validate_finding(closed_finding()) == []


def test_open_finding_needs_no_closure_evidence():
    finding = {k: v for k, v in closed_finding(status="OPEN").items()
               if k not in ("fix_ref", "fix_release", "regression_test", "reviewer", "accepted_at")}
    assert validate_finding(finding) == []


def test_fixed_finding_must_link_the_fix():
    finding = closed_finding(status="FIXED")
    del finding["fix_ref"]
    assert "fix-evidence" in rules(validate_finding(finding))


def test_closed_finding_without_fix_evidence_is_rejected():
    for missing in ("fix_ref", "fix_release"):
        finding = closed_finding()
        del finding[missing]
        assert "closure-evidence" in rules(validate_finding(finding)), missing


def test_closed_finding_without_verification_evidence_is_rejected():
    for missing in ("regression_test", "reviewer", "accepted_at"):
        finding = closed_finding()
        del finding[missing]
        assert "closure-evidence" in rules(validate_finding(finding)), missing


def test_regression_test_must_exist_in_the_repository():
    missing_file = closed_finding(regression_test="contracts/price-oracle/src/nope.rs::test_x")
    missing_test = closed_finding(regression_test="contracts/price-oracle/src/test.rs::test_does_not_exist")
    malformed = closed_finding(regression_test="see the PR")
    for finding in (missing_file, missing_test, malformed):
        assert "verification-evidence" in rules(validate_finding(finding, REPO_ROOT))


def test_fix_ref_must_be_a_pr_or_commit_link():
    assert "fix-evidence" in rules(validate_finding(closed_finding(fix_ref="fixed in main")))
    commit = "https://github.com/Stellar-Unified-Price-Oracle/repo/commit/2bc7c4c"
    assert validate_finding(closed_finding(fix_ref=commit)) == []


def test_owner_cannot_accept_their_own_fix():
    finding = closed_finding(reviewer="core-contracts-lead")
    assert "independent-review" in rules(validate_finding(finding))


def test_acceptance_cannot_predate_the_report():
    finding = closed_finding(accepted_at="2026-08-01")
    assert "accepted-at" in rules(validate_finding(finding))


def test_unknown_severity_and_status_are_rejected():
    assert "severity" in rules(validate_finding(closed_finding(severity="URGENT")))
    assert "status" in rules(validate_finding(closed_finding(status="DONE")))


def test_duplicate_ids_are_rejected():
    report = validate_registry({"findings": [closed_finding(), closed_finding()]})
    assert "unique-id" in rules(report.violations)


def test_key_material_is_rejected():
    finding = closed_finding(title="Leaked " + "S" + "C" * 55)
    assert "no-secrets" in rules(validate_finding(finding))


def test_report_counts_and_cli_exit_code(tmp_path):
    registry = {"findings": [closed_finding(), closed_finding(id="AUD-2026-002", status="OPEN")]}
    report = validate_registry(registry)
    assert report.total == 2
    assert report.by_status["CLOSED"] == 1 and report.by_status["OPEN"] == 1
    assert report.by_severity["HIGH"] == 2

    good = tmp_path / "good.json"
    good.write_text(json.dumps(registry), encoding="utf-8")
    assert main(["--registry", str(good), "--report", str(tmp_path / "out.md")]) == 0
    assert "PASS" in (tmp_path / "out.md").read_text(encoding="utf-8")

    bad = tmp_path / "bad.json"
    bad.write_text(json.dumps({"findings": [closed_finding(reviewer="core-contracts-lead")]}), encoding="utf-8")
    assert main(["--registry", str(bad)]) == 1
