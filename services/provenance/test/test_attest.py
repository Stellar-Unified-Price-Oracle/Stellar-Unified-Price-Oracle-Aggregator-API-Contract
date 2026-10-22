from __future__ import annotations

import json
from pathlib import Path

import pytest

from services.provenance.attest import (
    DEFAULT_BUILDER,
    PREDICATE_TYPE,
    STATEMENT_TYPE,
    UPSTREAM_REPO,
    WASM_NAME,
    BuildEnvironment,
    Provenance,
    ProvenanceBundle,
    main,
    sha256_file,
    signer_identity,
    verify,
)

SHA = "a" * 40
OTHER_SHA = "c" * 40
WORKFLOW_REF = f"{UPSTREAM_REPO}/.github/workflows/release.yml@refs/heads/main"
SIGNER = signer_identity("release.yml", "refs/tags/v1.0.0")


@pytest.fixture()
def artifact(tmp_path) -> Path:
    p = tmp_path / WASM_NAME
    p.write_bytes(b"\x00asm\x01\x00\x00\x00 fake contract wasm")
    return p


def make_bundle(artifact: Path, commit: str = SHA) -> ProvenanceBundle:
    prov = Provenance(
        subject_name=WASM_NAME,
        subject_sha256=sha256_file(artifact),
        commit_sha=commit,
        repository=UPSTREAM_REPO,
        workflow_ref=WORKFLOW_REF,
        run_id="4242",
    )
    return ProvenanceBundle(statement=prov.to_statement(), signer_identity=SIGNER)


def test_statement_is_in_toto_v1_with_slsa_predicate(artifact):
    st = make_bundle(artifact).statement
    assert st["_type"] == STATEMENT_TYPE
    assert st["predicateType"] == PREDICATE_TYPE


def test_provenance_binds_artifact_commit_and_environment(artifact):
    st = make_bundle(artifact).statement
    assert st["subject"][0]["digest"]["sha256"] == sha256_file(artifact)
    build = st["predicate"]["buildDefinition"]
    run = st["predicate"]["runDetails"]
    assert build["externalParameters"]["commit"] == SHA
    assert build["externalParameters"]["repository"] == UPSTREAM_REPO
    assert build["externalParameters"]["workflowRef"] == WORKFLOW_REF
    # The build *steps* are covered, not just an artifact hash: the resolved
    # source dependency and the toolchain are both recorded.
    assert build["resolvedDependencies"][0]["digest"]["gitCommit"] == SHA
    assert run["builder"]["id"] == DEFAULT_BUILDER
    env = run["metadata"]["environment"]
    assert env["toolchain"] and env["target"] == "wasm32v1-none"
    assert env["build_command"] == "make build"


def test_fresh_build_matches_the_published_artifact(artifact):
    res = verify(make_bundle(artifact), artifact)
    assert res.ok, res.errors
    assert "artifact matches the attested digest" in res.checked


def test_tampered_artifact_fails_verification(artifact, tmp_path):
    tampered = tmp_path / "tampered.wasm"
    tampered.write_bytes(artifact.read_bytes() + b"malicious")
    res = verify(make_bundle(artifact), tampered)
    assert not res.ok
    assert any("fresh build digest" in e for e in res.errors)


def test_missing_artifact_fails_rather_than_passes_silently(artifact, tmp_path):
    res = verify(make_bundle(artifact), tmp_path / "nope.wasm")
    assert not res.ok
    assert any("does not exist" in e for e in res.errors)


def test_provenance_for_a_different_commit_is_rejected(artifact):
    res = verify(make_bundle(artifact), artifact, expected_commit=OTHER_SHA)
    assert not res.ok
    assert any("release commit" in e for e in res.errors)


def test_provenance_for_another_workflow_is_rejected(artifact):
    res = verify(make_bundle(artifact), artifact, expected_workflow_ref="other/ref")
    assert not res.ok
    assert any("workflowRef" in e for e in res.errors)


def test_signer_identity_mismatch_is_rejected(artifact):
    res = verify(make_bundle(artifact), artifact, expected_signer="someone-else")
    assert not res.ok
    assert any("signer identity" in e for e in res.errors)


def test_a_different_workflow_cannot_claim_our_signer_identity(artifact):
    res = verify(
        make_bundle(artifact), artifact,
        expected_signer=signer_identity("attacker.yml", "refs/tags/v1.0.0"),
    )
    assert not res.ok


def test_incomplete_statement_is_rejected(artifact):
    bundle = make_bundle(artifact)
    del bundle.statement["predicate"]["runDetails"]["metadata"]["environment"]
    res = verify(bundle, artifact)
    assert not res.ok
    assert any("environment is missing" in e for e in res.errors)


def test_wrong_predicate_type_is_rejected(artifact):
    bundle = make_bundle(artifact)
    bundle.statement["predicateType"] = "https://example.invalid/provenance"
    assert not verify(bundle, artifact).ok


def test_resolved_dependency_must_match_the_commit(artifact):
    """A statement that claims one commit but builds another is rejected."""
    bundle = make_bundle(artifact)
    bundle.statement["predicate"]["buildDefinition"]["resolvedDependencies"][0][
        "digest"
    ]["gitCommit"] = OTHER_SHA
    res = verify(bundle, artifact)
    assert not res.ok
    assert any("resolvedDependencies" in e for e in res.errors)


def test_bundle_digest_is_stable_and_content_sensitive(artifact):
    b1 = make_bundle(artifact)
    b2 = make_bundle(artifact)
    assert b1.digest() == b2.digest()
    b2.statement["subject"][0]["digest"]["sha256"] = OTHER_SHA * 2
    assert b1.digest() != b2.digest()


def test_no_signing_key_is_needed_to_verify(artifact):
    """Verification is offline: no key, no network, no secret."""
    res = verify(make_bundle(artifact), artifact, expected_signer=SIGNER)
    assert res.ok
    assert res.report().startswith("# Provenance verification — PASS")


def test_cli_generate_then_verify(artifact, tmp_path):
    bundle_path = tmp_path / "provenance.json"
    rc = main([
        "generate", "--artifact", str(artifact), "--commit", SHA,
        "--workflow-ref", WORKFLOW_REF, "--run-id", "4242",
        "--tag", "v1.0.0", "--out", str(bundle_path),
    ])
    assert rc == 0
    payload = json.loads(bundle_path.read_text())
    assert payload["signerIdentity"] == SIGNER
    assert payload["transparencyLog"].startswith("https://rekor.sigstore.dev")

    report = tmp_path / "report.md"
    rc = main([
        "verify", "--bundle", str(bundle_path), "--artifact", str(artifact),
        "--commit", SHA, "--signer", SIGNER, "--report", str(report),
    ])
    assert rc == 0
    assert "artifact matches the attested digest" in report.read_text()

    # A different artifact must fail the CLI too.
    other = tmp_path / "other.wasm"
    other.write_bytes(b"different bytes")
    assert main([
        "verify", "--bundle", str(bundle_path), "--artifact", str(other),
    ]) == 1


def test_build_environment_is_carried_verbatim():
    env = BuildEnvironment(
        builder_id="https://example.invalid/builder",
        runner_image="ubuntu-24.04",
        toolchain="rust 1.92.0",
        target="wasm32v1-none",
        build_command="make build",
    )
    prov = Provenance(
        subject_name=WASM_NAME, subject_sha256="d" * 64, commit_sha=SHA,
        repository=UPSTREAM_REPO, workflow_ref=WORKFLOW_REF, run_id="1",
        environment=env,
    )
    recorded = prov.to_statement()["predicate"]["runDetails"]["metadata"]["environment"]
    assert recorded == env.to_dict()
    assert recorded["runner_image"] == "ubuntu-24.04"
