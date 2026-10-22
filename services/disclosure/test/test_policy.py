"""Unit tests for security contact and disclosure process validation (#690)."""
import hashlib
import json

from services.disclosure.policy import (
    DEFAULT_POLICY,
    DEFAULT_SECURITY_MD,
    main,
    new_synthetic_report,
    score_drill,
    validate_policy,
)

POLICY = json.loads(DEFAULT_POLICY.read_text(encoding="utf-8"))
SECURITY_MD = DEFAULT_SECURITY_MD.read_text(encoding="utf-8")


def observation(**overrides):
    """A passing drill: acknowledged in 5h by a responder; an outsider is denied."""
    report = new_synthetic_report()
    obs = {
        "report_id": report["report_id"],
        "content_sha256": report["content_sha256"],
        "submitted_at": "2026-10-20T09:00:00Z",
        "acknowledged_at": "2026-10-20T14:00:00Z",
        "acknowledged_by": "security-lead",
        "responders": ["security-lead", "core-maintainer"],
        "access_checks": [
            {"principal": "security-lead", "can_read": True},
            {"principal": "core-maintainer", "can_read": True},
            {"principal": "outside-contributor", "can_read": False},
        ],
    }
    obs.update(overrides)
    return obs


def with_policy(**overrides):
    policy = json.loads(json.dumps(POLICY))
    policy.update(overrides)
    return policy


def test_committed_policy_and_security_md_agree():
    result = validate_policy(POLICY, SECURITY_MD)
    assert result.ok, result.violations


def test_public_channel_is_rejected():
    issues = "https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/issues/new"
    result = validate_policy(with_policy(channel={"type": "github-issues", "url": issues}))
    assert any("not a private disclosure channel" in v for v in result.violations)
    assert any("private advisory form" in v for v in result.violations)


def test_response_owner_must_be_named():
    result = validate_policy(with_policy(response_owner={"role": "", "team": "maintainers"}))
    assert any("role is missing" in v for v in result.violations)
    assert any("team handle" in v for v in result.violations)


def test_acknowledgement_and_embargo_bounds():
    assert validate_policy(with_policy(acknowledgement_target_hours=72)).ok
    assert not validate_policy(with_policy(acknowledgement_target_hours=73)).ok
    assert not validate_policy(with_policy(acknowledgement_target_hours=0)).ok
    assert validate_policy(with_policy(embargo_days=90)).ok
    assert not validate_policy(with_policy(embargo_days=91)).ok


def test_security_md_must_match_the_policy():
    result = validate_policy(with_policy(acknowledgement_target_hours=48), SECURITY_MD)
    assert any("acknowledgement target (48 hours)" in v for v in result.violations)


def test_security_md_must_not_point_reporters_to_public_issues():
    md = SECURITY_MD + "\nOr file https://github.com/org/repo/issues/new\n"
    result = validate_policy(POLICY, md)
    assert any("public issue" in v for v in result.violations)


def test_synthetic_report_is_identified_only_by_its_digest():
    report = new_synthetic_report()
    assert report["report_id"].startswith("SYNTHETIC-")
    assert report["content_sha256"] == hashlib.sha256(report["body"].encode("utf-8")).hexdigest()
    result = score_drill(POLICY, observation())
    assert result.ok, result.violations
    assert result.ack_delay_hours == 5.0
    assert report["body"] not in json.dumps(result.to_dict())


def test_late_acknowledgement_fails_the_drill():
    result = score_drill(POLICY, observation(acknowledged_at="2026-10-23T10:00:00Z"))
    assert any("target is 72h" in v for v in result.violations)


def test_acknowledgement_exactly_at_target_passes():
    assert score_drill(POLICY, observation(acknowledged_at="2026-10-23T09:00:00Z")).ok


def test_unacknowledged_report_fails_the_drill():
    obs = observation()
    del obs["acknowledged_at"]
    assert any("not acknowledged" in v for v in score_drill(POLICY, obs).violations)


def test_acknowledgement_by_a_non_responder_fails():
    result = score_drill(POLICY, observation(acknowledged_by="outside-contributor"))
    assert any("outside the responders" in v for v in result.violations)


def test_report_readable_by_a_non_responder_fails():
    checks = [{"principal": "security-lead", "can_read": True},
              {"principal": "outside-contributor", "can_read": True}]
    result = score_drill(POLICY, observation(access_checks=checks))
    assert any("non-responder outside-contributor can read" in v for v in result.violations)


def test_responder_without_access_fails():
    checks = [{"principal": "security-lead", "can_read": False},
              {"principal": "outside-contributor", "can_read": False}]
    result = score_drill(POLICY, observation(access_checks=checks))
    assert any("responder security-lead cannot read" in v for v in result.violations)


def test_private_access_must_be_probed_from_outside():
    checks = [{"principal": "security-lead", "can_read": True}]
    result = score_drill(POLICY, observation(access_checks=checks))
    assert any("private access is unverified" in v for v in result.violations)


def test_observation_must_not_carry_report_contents():
    result = score_drill(POLICY, observation(body="steps to reproduce ..."))
    assert any("contains report contents" in v for v in result.violations)


def test_non_synthetic_report_is_refused():
    result = score_drill(POLICY, observation(report_id="GHSA-xxxx-yyyy-zzzz"))
    assert any("SYNTHETIC-" in v for v in result.violations)


def test_cli_exit_codes(tmp_path):
    good = tmp_path / "drill.json"
    good.write_text(json.dumps(observation()), encoding="utf-8")
    assert main(["--drill", str(good)]) == 0

    bad = tmp_path / "late.json"
    bad.write_text(json.dumps(observation(acknowledged_at="2026-10-25T09:00:00Z")), encoding="utf-8")
    assert main(["--drill", str(bad)]) == 1
