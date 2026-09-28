from __future__ import annotations

import json
from pathlib import Path

import pytest

from services.ci_policy.policy import (
    REQUIRED_CLAIM_MATCHES,
    UPSTREAM_REPO,
    ClaimMatcher,
    DeploymentAttestation,
    TrustPolicy,
    audit,
    deploy_policies,
    evaluate,
    find_long_lived_secrets,
    is_retired,
    sample_claims,
    secret_inventory,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
SHA = "a" * 40
DIGEST = "b" * 64
CANARY_REF = f"{UPSTREAM_REPO}/.github/workflows/canary.yml@refs/heads/main"


def test_shipped_policies_are_scoped():
    for policy in deploy_policies():
        assert policy.validate() == [], policy.validate()
        for claim in REQUIRED_CLAIM_MATCHES:
            assert claim in policy.pinned_claims()


def test_intended_workflow_can_assume_the_role():
    policy = deploy_policies()[0]
    allowed, reason = evaluate(policy, sample_claims())
    assert allowed, reason


def test_fork_cannot_assume_the_role():
    allowed, reason = evaluate(
        deploy_policies()[0], sample_claims(repository="attacker/oracle-fork")
    )
    assert not allowed
    assert "repository" in reason


def test_unrelated_workflow_in_same_repo_cannot_assume_the_role():
    allowed, reason = evaluate(
        deploy_policies()[0],
        sample_claims(
            workflow_ref=f"{UPSTREAM_REPO}/.github/workflows/attacker.yml@refs/heads/main"
        ),
    )
    assert not allowed
    assert "workflow_ref" in reason


def test_workflow_from_a_pull_request_ref_cannot_assume_the_role():
    """Pinning the ref in workflow_ref stops a PR branch from deploying."""
    allowed, reason = evaluate(
        deploy_policies()[0],
        sample_claims(
            workflow_ref=(
                f"{UPSTREAM_REPO}/.github/workflows/testnet-lifecycle.yml"
                "@refs/pull/1/merge"
            )
        ),
    )
    assert not allowed
    assert "workflow_ref" in reason


def test_job_outside_the_environment_cannot_assume_the_role():
    allowed, reason = evaluate(deploy_policies()[0], sample_claims(environment=""))
    assert not allowed
    assert "environment" in reason


def test_mainnet_role_is_not_reachable_from_the_testnet_environment():
    mainnet = next(p for p in deploy_policies() if p.role.endswith("mainnet-deploy"))
    allowed, _ = evaluate(mainnet, sample_claims(environment="testnet"))
    assert not allowed


def test_mainnet_role_is_reachable_from_production_via_canary():
    mainnet = next(p for p in deploy_policies() if p.role.endswith("mainnet-deploy"))
    allowed, reason = evaluate(
        mainnet,
        sample_claims(
            environment="production",
            workflow_ref=CANARY_REF,
            sub=f"repo:{UPSTREAM_REPO}:environment:production",
        ),
    )
    assert allowed, reason


def test_missing_claim_is_rejected_rather_than_defaulting_open():
    claims = sample_claims()
    del claims["environment"]
    allowed, reason = evaluate(deploy_policies()[0], claims)
    assert not allowed
    assert "<absent>" in reason


def test_policy_that_does_not_pin_the_required_claims_is_rejected():
    loose = TrustPolicy(
        role="too-loose", matchers=(ClaimMatcher("repository", (UPSTREAM_REPO,)),)
    )
    errors = " ".join(loose.validate())
    assert "workflow_ref" in errors and "environment" in errors
    allowed, reason = evaluate(loose, sample_claims())
    assert not allowed
    assert "unsafe" in reason


def test_bare_wildcard_matcher_is_rejected_as_pinning_nothing():
    wildcard = TrustPolicy(
        role="wildcard",
        matchers=(
            ClaimMatcher("repository", ("*",)),
            ClaimMatcher("workflow_ref", ("*",)),
            ClaimMatcher("environment", ("*",)),
        ),
    )
    assert any("bare wildcard" in e for e in wildcard.validate())


def test_retired_secret_classification():
    assert is_retired("CANARY_SECRET_KEY")
    assert is_retired("DEPLOY_STELLAR_KEY")
    assert is_retired("AWS_SECRET_ACCESS_KEY")
    assert is_retired("NPM_TOKEN")
    # Identifiers and short-lived config endpoints are not credentials.
    assert not is_retired("DEPLOY_ROLE_ARN")
    assert not is_retired("SOAK_PROMETHEUS_PUSH_URL")


def test_no_long_lived_secret_remains_in_the_repository():
    """The issue's headline acceptance criterion, checked against the tree."""
    found = find_long_lived_secrets(REPO_ROOT)
    assert found == [], [u.to_dict() for u in found]


def test_secret_inventory_is_empty_of_retired_entries():
    assert secret_inventory(REPO_ROOT)["retired"] == {}


def test_scanner_detects_a_reintroduced_long_lived_secret(tmp_path):
    wf = tmp_path / ".github" / "workflows"
    wf.mkdir(parents=True)
    (wf / "bad.yml").write_text(
        "jobs:\n  deploy:\n    steps:\n"
        "      - run: deploy.sh\n"
        "        env:\n"
        "          KEY: ${{ secrets.DEPLOY_STELLAR_SECRET_KEY }}\n"
    )
    found = find_long_lived_secrets(tmp_path)
    assert [u.name for u in found] == ["DEPLOY_STELLAR_SECRET_KEY"]
    assert found[0].path == ".github/workflows/bad.yml"
    assert any("long-lived secret" in p for p in audit(tmp_path))


def test_audit_passes_on_this_repository():
    assert audit(REPO_ROOT) == []


def test_deployment_attestation_binds_commit_and_run():
    att = DeploymentAttestation(
        repository=UPSTREAM_REPO,
        commit_sha=SHA,
        workflow="canary.yml",
        workflow_ref=CANARY_REF,
        run_id="12345",
        run_attempt="2",
        environment="testnet",
        role="price-oracle-testnet-deploy",
        artifact_sha256=DIGEST,
        actor="a-maintainer",
    )
    d = att.to_dict()
    assert d["commit_sha"] == SHA
    assert d["run_id"] == "12345" and d["run_attempt"] == "2"
    assert d["artifact_sha256"] == DIGEST
    summary = att.as_github_step_summary()
    assert SHA in summary and "12345/2" in summary and DIGEST in summary


def test_attestation_rejects_unattributable_records():
    with pytest.raises(ValueError, match="commit_sha"):
        DeploymentAttestation(
            repository=UPSTREAM_REPO, commit_sha="not-a-sha", workflow="w.yml",
            workflow_ref="w", run_id="1", run_attempt="1", environment="testnet",
            role="r", artifact_sha256=DIGEST,
        )
    with pytest.raises(ValueError, match="run_id"):
        DeploymentAttestation(
            repository=UPSTREAM_REPO, commit_sha=SHA, workflow="w.yml",
            workflow_ref="w", run_id="", run_attempt="1", environment="testnet",
            role="r", artifact_sha256=DIGEST,
        )


def test_attest_cli_writes_the_record(tmp_path):
    from services.ci_policy.attest import main

    out = tmp_path / "att.json"
    rc = main([
        "--repository", UPSTREAM_REPO, "--commit", SHA,
        "--workflow-ref", CANARY_REF, "--run-id", "99", "--run-attempt", "1",
        "--environment", "testnet", "--role", "price-oracle-testnet-deploy",
        "--artifact", DIGEST, "--actor", "a-maintainer", "--out", str(out),
    ])
    assert rc == 0
    payload = json.loads(out.read_text())
    assert payload["commit_sha"] == SHA
    assert payload["workflow"] == "canary.yml"
    # A malformed commit must fail the job rather than emit a weak record.
    bad = tmp_path / "bad.json"
    assert main([
        "--repository", UPSTREAM_REPO, "--commit", "short",
        "--workflow-ref", "w", "--run-id", "1", "--environment", "testnet",
        "--role", "r", "--artifact", DIGEST, "--out", str(bad),
    ]) == 1
    assert not bad.exists()
