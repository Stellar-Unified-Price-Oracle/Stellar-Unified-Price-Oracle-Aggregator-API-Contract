"""Unit tests for the production access review (#691)."""
import json

from services.access_review.review import (
    Level,
    System,
    ceiling,
    create_seeded_inventory,
    main,
    review_access,
)


def removal_keys(report):
    return {(r.principal, r.system, r.granted, r.allowed) for r in report.removals}


def principal(inv, pid):
    return next(p for p in inv["principals"] if p["id"] == pid)


def test_seeded_review_passes_and_removes_excess_access():
    report = review_access(create_seeded_inventory())
    assert report.ok, report.violations
    assert removal_keys(report) == {
        ("maint-a", "DEPLOYMENT", "WRITE", "NONE"),
        ("triage-a", "SOURCE", "TRIAGE", "READ"),
        ("workflow:canary.yml", "SOURCE", "WRITE", "NONE"),
    }
    assert report.approved_access["maint-a"] == {"SOURCE": "WRITE"}
    assert report.approved_access["workflow:canary.yml"] == {"DEPLOYMENT": "WRITE"}
    assert report.approved_access["core-b"] == {"SOURCE": "ADMIN", "TREASURY": "WRITE"}
    assert report.next_review_due == "2027-01-01"


def test_role_ceilings_follow_the_contributor_ladder():
    assert ceiling(["contributor"])[System.SOURCE] == Level.READ
    assert ceiling(["maintainer"])[System.DEPLOYMENT] == Level.NONE
    assert ceiling(["core-maintainer"])[System.TREASURY] == Level.NONE
    assert ceiling(["core-maintainer", "treasury-signer"])[System.TREASURY] == Level.WRITE
    assert ceiling(["core-maintainer"], inactive=True)[System.SOURCE] == Level.WRITE


def test_inactivity_boundary_is_ninety_days():
    inv = create_seeded_inventory()
    principal(inv, "triage-a")["last_active"] = "2026-07-03"  # exactly 90 days before 2026-10-01
    inv["revocation_checks"] = [c for c in inv["revocation_checks"] if c["principal"] != "triage-a"]
    report = review_access(inv)
    assert report.ok, report.violations
    assert ("triage-a", "SOURCE", "TRIAGE", "READ") not in removal_keys(report)


def test_removed_access_that_still_works_fails_the_review():
    inv = create_seeded_inventory()
    inv["revocation_checks"][0]["result"] = "ALLOWED"
    report = review_access(inv)
    assert any("maint-a: removed DEPLOYMENT access still works" in v for v in report.violations)


def test_unverified_removal_fails_the_review():
    inv = create_seeded_inventory()
    inv["revocation_checks"] = inv["revocation_checks"][1:]
    report = review_access(inv)
    assert any("maint-a: removal of DEPLOYMENT WRITE is not verified" in v for v in report.violations)


def test_probe_from_before_the_review_does_not_count():
    inv = create_seeded_inventory()
    inv["revocation_checks"][0]["checked_at"] = "2026-09-01"
    assert not review_access(inv).ok


def test_access_list_needs_owner_approval():
    inv = create_seeded_inventory()
    inv["approvals"] = [a for a in inv["approvals"] if a["principal"] != "maint-a"]
    assert any("maint-a: access list not approved" in v for v in review_access(inv).violations)

    inv = create_seeded_inventory()
    inv["approvals"][2]["approved_by"] = "maint-a"  # self-approval
    assert any("not by its owner" in v for v in review_access(inv).violations)


def test_stale_approval_does_not_count():
    inv = create_seeded_inventory()
    inv["approvals"][0]["approved_at"] = "2026-06-30"
    assert any("core-a: approval predates this review" in v for v in review_access(inv).violations)


def test_every_principal_needs_a_human_owner():
    inv = create_seeded_inventory()
    principal(inv, "workflow:canary.yml")["owner"] = "workflow:canary.yml"
    assert any("owner must be another human" in v for v in review_access(inv).violations)


def test_automation_and_human_roles_do_not_mix():
    inv = create_seeded_inventory()
    principal(inv, "workflow:canary.yml")["roles"] = ["core-maintainer"]
    principal(inv, "maint-a")["roles"] = ["maintainer", "deploy-workflow"]
    violations = review_access(inv).violations
    assert any("automation may only hold automation roles" in v for v in violations)
    assert any("humans may not hold automation roles" in v for v in violations)


def test_unknown_roles_and_grants_are_rejected():
    inv = create_seeded_inventory()
    principal(inv, "maint-a")["roles"] = ["superuser"]
    principal(inv, "core-a")["grants"]["BILLING"] = "ADMIN"
    violations = review_access(inv).violations
    assert any("unknown roles superuser" in v for v in violations)
    assert any("unknown grant BILLING=ADMIN" in v for v in violations)


def test_key_material_is_rejected():
    inv = create_seeded_inventory()
    principal(inv, "core-a")["note"] = "S" + "C" * 55
    assert any("key material" in v for v in review_access(inv).violations)


def test_cli_writes_report_and_exit_codes(tmp_path):
    assert main(["--seed", "--report", str(tmp_path / "review.md"), "--out", str(tmp_path / "review.json")]) == 0
    md = (tmp_path / "review.md").read_text(encoding="utf-8")
    assert "`maint-a` | DEPLOYMENT | WRITE | NONE" in md
    assert json.loads((tmp_path / "review.json").read_text(encoding="utf-8"))["ok"] is True

    inv = create_seeded_inventory()
    inv["revocation_checks"] = []
    bad = tmp_path / "inventory.json"
    bad.write_text(json.dumps(inv), encoding="utf-8")
    assert main(["--inventory", str(bad)]) == 1
