"""Tests for the privileged transaction review guard (#686)."""

import copy
import datetime as dt
import json

import pytest

from services.tx_review import cases, gate
from services.tx_review.payload import (
    PRIVILEGED_FUNCTIONS,
    PayloadCode,
    PayloadError,
    digest_of,
    load_payload,
    normalise,
)
from services.tx_review.render import render
from services.tx_review.review import (
    ApprovalError,
    ReviewCode,
    ReviewPolicy,
    build_approval,
    verify_approval,
)

MAINNET = "Public Global Stellar Network ; September 2015"
CONTRACT = "C" + "C" * 55
SOURCE = "G" + "A" * 55
ADMIN = "G" + "B" * 55
REVIEWED_AT = dt.datetime(2026, 10, 20, 9, 0, tzinfo=dt.timezone.utc)
EXPIRES_AT = dt.datetime(2026, 10, 20, 10, 0, tzinfo=dt.timezone.utc)
NOW = dt.datetime(2026, 10, 20, 9, 30, tzinfo=dt.timezone.utc)


def payload(**over):
    base = {
        "schema": "oracle.tx_review.v1",
        "network": MAINNET,
        "source_account": SOURCE,
        "fee": 1000,
        "sequence": 123456789,
        "max_time": 0,
        "operations": [
            {
                "contract_id": CONTRACT,
                "function": "set_admin",
                "args": {"new_admin": {"type": "address", "value": ADMIN}},
            }
        ],
    }
    base.update(over)
    return base


def approval_for(body, **over):
    sim = {
        "payload_sha256": digest_of(body),
        "network": body["network"],
        "success": True,
        "fee_stroops": 1500,
    }
    approval = build_approval(
        body,
        sim,
        requested_by="ops-bot",
        reviewed_by=["alice", "bob"],
        reviewed_at=REVIEWED_AT,
        expires_at=EXPIRES_AT,
    )
    approval.update(over)
    return approval


# -- canonical form and digest --------------------------------------------


def test_digest_ignores_json_key_order_and_argument_order():
    reordered = payload()
    reordered["operations"][0]["args"] = {
        "z_extra": {"type": "u32", "value": 7},
        "new_admin": {"type": "address", "value": ADMIN},
    }
    shuffled = payload()
    shuffled["operations"][0]["args"] = {
        "new_admin": {"type": "address", "value": ADMIN},
        "z_extra": {"type": "u32", "value": 7},
    }
    assert digest_of(reordered) == digest_of(shuffled)
    assert digest_of(payload()) != digest_of(reordered)


@pytest.mark.parametrize(
    "override",
    [
        {"sequence": 123456790},
        {"source_account": "G" + "E" * 55},
        {"fee": 2000},
        {"max_time": 1792494000},
        {"network": "Test SDF Network ; September 2015"},
    ],
)
def test_digest_covers_every_signed_field(override):
    assert digest_of(payload()) != digest_of(payload(**override))


def test_digest_distinguishes_argument_types():
    as_address = payload()
    as_string = payload()
    as_string["operations"][0]["args"]["new_admin"] = {"type": "string", "value": ADMIN}
    assert digest_of(as_address) != digest_of(as_string)


def test_digest_covers_operation_order():
    first = payload()
    second = copy.deepcopy(first)
    second["operations"].insert(
        0, {"contract_id": CONTRACT, "function": "pause", "args": {}}
    )
    assert digest_of(first) != digest_of(second)


def test_typed_integer_arguments_accept_decimal_strings_canonically():
    as_int = payload()
    as_int["operations"][0]["args"]["limit"] = {"type": "i128", "value": 100}
    as_str = payload()
    as_str["operations"][0]["args"]["limit"] = {"type": "i128", "value": "100"}
    assert digest_of(as_int) == digest_of(as_str)


@pytest.mark.parametrize(
    "bad,code",
    [
        ({"memo": "x"}, PayloadCode.UNKNOWN_FIELD),
        ({"operations": []}, PayloadCode.MALFORMED),
        ({"sequence": 0}, PayloadCode.ARGUMENT_VALUE),
        ({"fee": -1}, PayloadCode.ARGUMENT_VALUE),
        ({"network": ""}, PayloadCode.MALFORMED),
    ],
)
def test_payload_shape_is_strict(bad, code):
    with pytest.raises(PayloadError) as excinfo:
        normalise(payload(**bad))
    assert excinfo.value.code is code


def test_untyped_argument_is_refused():
    body = payload()
    body["operations"][0]["args"]["new_admin"] = ADMIN
    with pytest.raises(PayloadError) as excinfo:
        normalise(body)
    assert excinfo.value.code is PayloadCode.UNTYPED_ARGUMENT


def test_unknown_argument_type_is_refused():
    body = payload()
    body["operations"][0]["args"]["new_admin"] = {"type": "scval", "value": ADMIN}
    with pytest.raises(PayloadError) as excinfo:
        normalise(body)
    assert excinfo.value.code is PayloadCode.UNKNOWN_ARGUMENT_TYPE


def test_unmodelled_function_is_refused():
    body = payload()
    body["operations"][0]["function"] = "drain_treasury"
    with pytest.raises(PayloadError) as excinfo:
        normalise(body)
    assert excinfo.value.code is PayloadCode.UNKNOWN_FUNCTION


def test_every_modelled_function_is_actually_in_the_contract():
    # The guard refuses anything it does not model, so this set is a contract
    # with the deployed endpoints; keep it honest rather than aspirational.
    import re
    from pathlib import Path

    source = Path("contracts/price-oracle/src/lib.rs").read_text(encoding="utf-8")
    endpoints = set(re.findall(r"pub fn ([a-z_][a-z0-9_]*)\s*\(", source))
    assert PRIVILEGED_FUNCTIONS <= endpoints


# -- rendering -------------------------------------------------------------


def test_render_is_deterministic_and_human_readable():
    first = render(load_payload(payload()))
    assert first == render(load_payload(payload()))
    assert first.startswith("PRIVILEGED TRANSACTION REVIEW")
    assert "set_admin(new_admin=address:G" + "B" * 55 + ")" in first
    assert "network: " + MAINNET in first


def test_render_numbers_multiple_operations():
    body = payload()
    body["operations"].append({"contract_id": CONTRACT, "function": "pause", "args": {}})
    text = render(load_payload(body))
    assert "  1. " in text and "  2. " in text


# -- approval verification -------------------------------------------------


def test_valid_approval_is_accepted():
    body = payload()
    approved = verify_approval(body, approval_for(body), now=NOW)
    assert approved.digest == digest_of(body)
    assert approved.reviewers == ("alice", "bob")
    assert approved.environment == "production"


def test_approval_rejects_an_edited_payload():
    body = payload()
    approval = approval_for(body)
    edited = payload(sequence=123456790)
    with pytest.raises(ApprovalError) as excinfo:
        verify_approval(edited, approval, now=NOW)
    assert excinfo.value.code is ReviewCode.PAYLOAD_MISMATCH


def test_approval_rejects_a_summary_from_another_payload():
    body = payload()
    approval = approval_for(body, rendered_summary="PRIVILEGED TRANSACTION REVIEW\nnetwork: other")
    with pytest.raises(ApprovalError) as excinfo:
        verify_approval(body, approval, now=NOW)
    assert excinfo.value.code is ReviewCode.SUMMARY_MISMATCH


def test_summary_trailing_newline_is_tolerated():
    body = payload()
    approval = approval_for(body, rendered_summary=render(load_payload(body)) + "\n")
    assert verify_approval(body, approval, now=NOW).digest == digest_of(body)


@pytest.mark.parametrize(
    "mutation,code",
    [
        (lambda a: a.pop("simulation"), ReviewCode.SIMULATION_MISSING),
        (lambda a: a["simulation"].update(success=False), ReviewCode.SIMULATION_FAILED),
        (lambda a: a["simulation"].update(network="Test SDF Network ; September 2015"),
         ReviewCode.SIMULATION_MISMATCH),
        (lambda a: a["simulation"].update(payload_sha256="0" * 64), ReviewCode.SIMULATION_MISMATCH),
        (lambda a: a["simulation"].update(fee_stroops=10_000_001), ReviewCode.FEE_ABOVE_CAP),
        (lambda a: a.update(reviewed_by=["alice"]), ReviewCode.INSUFFICIENT_REVIEWERS),
        (lambda a: a.update(reviewed_by=["alice", "alice"]), ReviewCode.INSUFFICIENT_REVIEWERS),
        (lambda a: a.update(reviewed_by=["ops-bot", "alice"]), ReviewCode.SELF_REVIEW),
        (lambda a: a.update(schema="oracle.tx_review.approval.v0"), ReviewCode.UNSUPPORTED_SCHEMA),
        (lambda a: a.update(emergency_override=True), ReviewCode.UNKNOWN_FIELD),
        (lambda a: a.pop("payload_sha256"), ReviewCode.MALFORMED_APPROVAL),
        (lambda a: a.update(reviewed_by="alice"), ReviewCode.MALFORMED_APPROVAL),
        (lambda a: a.update(expires_at="2026-10-20T09:00:00"), ReviewCode.MALFORMED_APPROVAL),
        (lambda a: a.update(expires_at="2026-10-20T08:00:00Z"), ReviewCode.REVIEW_WINDOW_TOO_LONG),
        (lambda a: a.update(expires_at="2026-10-22T09:00:00Z"), ReviewCode.REVIEW_WINDOW_TOO_LONG),
    ],
)
def test_approval_mutations_are_refused(mutation, code):
    body = payload()
    approval = approval_for(body)
    mutation(approval)
    with pytest.raises(ApprovalError) as excinfo:
        verify_approval(body, approval, now=NOW)
    assert excinfo.value.code is code


def test_expired_and_future_approvals_are_refused():
    body = payload()
    approval = approval_for(body)
    with pytest.raises(ApprovalError) as expired:
        verify_approval(body, approval, now=EXPIRES_AT + dt.timedelta(seconds=1))
    assert expired.value.code is ReviewCode.APPROVAL_EXPIRED
    with pytest.raises(ApprovalError) as early:
        verify_approval(body, approval, now=REVIEWED_AT - dt.timedelta(seconds=1))
    assert early.value.code is ReviewCode.APPROVAL_NOT_YET_VALID


def test_production_policy_cannot_lower_the_reviewer_floor():
    with pytest.raises(ValueError):
        ReviewPolicy(environment="production", required_reviewers=1)
    assert ReviewPolicy(environment="staging", required_reviewers=1)


def test_non_production_environment_needs_one_independent_reviewer():
    body = payload()
    approval = approval_for(body)
    approval["reviewed_by"] = ["alice"]
    policy = ReviewPolicy(environment="staging", required_reviewers=1)
    assert verify_approval(body, approval, policy=policy, now=NOW).reviewers == ("alice",)


def test_naive_now_is_treated_as_utc():
    body = payload()
    approved = verify_approval(body, approval_for(body), now=NOW.replace(tzinfo=None))
    assert approved.digest == digest_of(body)


def test_approval_error_carries_a_bounded_code():
    body = payload()
    with pytest.raises(ApprovalError) as excinfo:
        verify_approval(payload(sequence=2), approval_for(body), now=NOW)
    assert excinfo.value.to_dict()["code"] == "payload_mismatch"
    assert len(excinfo.value.detail) <= 160


# -- corpus and CLI --------------------------------------------------------


def test_tamper_corpus_all_pass():
    results = cases.run()
    failed = [r.line() for r in results if not r.ok]
    assert not failed, "\n".join(failed)
    assert len(results) == 27


def test_corpus_covers_every_rejection_family():
    expected = {case["expect"] for case in cases.load_manifest()["cases"]}
    assert {
        "approved",
        "payload_mismatch",
        "summary_mismatch",
        "simulation_mismatch",
        "simulation_failed",
        "simulation_missing",
        "fee_above_cap",
        "insufficient_reviewers",
        "self_review",
        "approval_expired",
        "approval_not_yet_valid",
        "review_window_too_long",
        "unsupported_schema",
        "untyped_argument",
        "unknown_function",
        "unknown_field",
    } <= expected


def test_gate_replay_exits_zero_and_writes_a_report(tmp_path):
    report = tmp_path / "tx-review.md"
    assert gate.main(["replay", "--report", str(report)]) == 0
    assert "passed: 27" in report.read_text()


def test_gate_cli_render_and_verify(tmp_path):
    body_path = tmp_path / "payload.json"
    body_path.write_text(json.dumps(payload()))
    assert gate.main(["render", "--payload", str(body_path)]) == 0

    approval_path = tmp_path / "approval.json"
    approval_path.write_text(json.dumps(approval_for(payload())))
    assert (
        gate.main(
            [
                "verify",
                "--payload",
                str(body_path),
                "--approval",
                str(approval_path),
                "--now",
                "2026-10-20T09:30:00Z",
            ]
        )
        == 0
    )

    tampered = payload(sequence=123456790)
    body_path.write_text(json.dumps(tampered))
    assert (
        gate.main(
            [
                "verify",
                "--payload",
                str(body_path),
                "--approval",
                str(approval_path),
                "--now",
                "2026-10-20T09:30:00Z",
            ]
        )
        == 1
    )


def test_gate_cli_approve_never_writes_a_refused_approval(tmp_path):
    body_path = tmp_path / "payload.json"
    body_path.write_text(json.dumps(payload()))
    sim_path = tmp_path / "simulation.json"
    # Simulation of a *different* payload: the CLI must refuse to record it.
    sim_path.write_text(
        json.dumps(
            {
                "payload_sha256": digest_of(payload(sequence=999)),
                "network": MAINNET,
                "success": True,
                "fee_stroops": 1500,
            }
        )
    )
    out = tmp_path / "approval.json"
    rc = gate.main(
        [
            "approve",
            "--payload",
            str(body_path),
            "--simulation",
            str(sim_path),
            "--requested-by",
            "ops-bot",
            "--reviewer",
            "alice",
            "--reviewer",
            "bob",
            "--reviewed-at",
            "2026-10-20T09:00:00Z",
            "--expires-at",
            "2026-10-20T10:00:00Z",
            "--out",
            str(out),
        ]
    )
    assert rc == 1 and not out.exists()
