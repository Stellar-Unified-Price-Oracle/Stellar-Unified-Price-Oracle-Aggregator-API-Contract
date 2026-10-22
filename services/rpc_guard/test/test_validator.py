"""Tests for the RPC response trust boundary (#684)."""

import json

import pytest

from services.rpc_guard import gate
from services.rpc_guard.validator import (
    MAX_DETAIL,
    NetworkPolicy,
    RpcErrorCode,
    RpcGuard,
    RpcValidationError,
)

H = "a" * 64
NEXT_H = "b" * 64


def network(**over):
    payload = {"passphrase": NetworkPolicy.testnet().passphrase, "protocolVersion": 23}
    payload.update(over)
    return payload


def ledger(**over):
    payload = {
        "sequence": 1000,
        "hash": H,
        "protocolVersion": 23,
        "closedAt": "2026-10-20T10:00:00Z",
    }
    payload.update(over)
    return payload


def transaction(**over):
    payload = {
        "hash": "c" * 64,
        "status": "SUCCESS",
        "ledger": 1000,
        "successful": True,
        "envelopeXdr": "envelope",
        "resultXdr": "result",
        "resultMetaXdr": "meta",
        "createdAt": 1792490405,
        "feeCharged": 100,
    }
    payload.update(over)
    return payload


def guard() -> RpcGuard:
    return RpcGuard(NetworkPolicy.testnet())


def ready() -> RpcGuard:
    g = guard()
    g.accept_network(network())
    g.accept_ledger(ledger())
    return g


def rejects(code, fn):
    with pytest.raises(RpcValidationError) as excinfo:
        fn()
    assert excinfo.value.code is code
    return excinfo.value


# -- ordering: no operational decision on an unvalidated context ----------


def test_ledger_requires_a_validated_network():
    rejects(RpcErrorCode.NETWORK_UNVALIDATED, lambda: guard().accept_ledger(ledger()))


def test_transaction_requires_a_validated_ledger():
    g = guard()
    g.accept_network(network())
    rejects(RpcErrorCode.LEDGER_UNVALIDATED, lambda: g.accept_transaction(transaction()))


# -- network ---------------------------------------------------------------


def test_matching_network_is_accepted():
    state = guard().accept_network(network())
    assert state.network_id == "testnet"
    assert state.passphrase_matched is True
    assert state.protocol_version == 23


def test_foreign_network_is_rejected():
    rejects(
        RpcErrorCode.NETWORK_MISMATCH,
        lambda: guard().accept_network(network(passphrase=NetworkPolicy.mainnet().passphrase)),
    )


@pytest.mark.parametrize("version", [21, 28, 99])
def test_protocol_outside_the_supported_window_is_rejected(version):
    rejects(
        RpcErrorCode.PROTOCOL_UNSUPPORTED,
        lambda: guard().accept_network(network(protocolVersion=version)),
    )


@pytest.mark.parametrize("value", [True, "23", 23.0, None])
def test_non_integer_protocol_version_is_malformed(value):
    rejects(
        RpcErrorCode.MALFORMED_RESPONSE,
        lambda: guard().accept_network(network(protocolVersion=value)),
    )


def test_json_rpc_error_is_not_read_as_data():
    error = rejects(
        RpcErrorCode.RPC_ERROR,
        lambda: guard().accept_network({"error": {"code": -32602, "message": "bad params"}}),
    )
    assert "-32602" in error.detail


# -- ledger ----------------------------------------------------------------


def test_valid_ledger_is_accepted():
    state = ready().ledger
    assert state.sequence == 1000
    assert state.close_time == 1792490400


def test_ledger_hash_must_be_64_hex_characters():
    g = guard()
    g.accept_network(network())
    for bad in ("deadbeef", "z" * 64, ""):
        rejects(RpcErrorCode.LEDGER_HASH, lambda bad=bad: g.accept_ledger(ledger(hash=bad)))


def test_ledger_protocol_must_match_the_negotiated_network():
    g = guard()
    g.accept_network(network())
    rejects(
        RpcErrorCode.PROTOCOL_UNSUPPORTED,
        lambda: g.accept_ledger(ledger(protocolVersion=24)),
    )


def test_ledger_close_time_cannot_move_backwards():
    g = ready()
    rejects(
        RpcErrorCode.LEDGER_TIME,
        lambda: g.accept_ledger(ledger(sequence=1001, hash=NEXT_H, closedAt="2026-10-20T09:59:00Z")),
    )


def test_ledger_cannot_regress():
    g = ready()
    rejects(RpcErrorCode.LEDGER_REGRESSION, lambda: g.accept_ledger(ledger(sequence=999)))


def test_same_height_different_hash_is_a_reorg_not_a_merge():
    g = ready()
    rejects(
        RpcErrorCode.LEDGER_REORG,
        lambda: g.accept_ledger(ledger(hash="d" * 64)),
    )
    # The rejection must not corrupt the accepted state: a re-read still works.
    assert g.ledger.hash == H


def test_repeated_read_of_the_same_ledger_is_idempotent():
    g = ready()
    assert g.accept_ledger(ledger()).sequence == 1000
    assert g.rejections == []


def test_ledger_without_a_timezone_is_rejected():
    g = guard()
    g.accept_network(network())
    rejects(
        RpcErrorCode.LEDGER_TIME,
        lambda: g.accept_ledger(ledger(closedAt="2026-10-20T10:00:00")),
    )


# -- transaction -----------------------------------------------------------


def test_successful_transaction_is_accepted():
    tx = ready().accept_transaction(transaction(), expected_hash="c" * 64)
    assert tx.status == "SUCCESS" and tx.successful is True
    assert tx.ledger == 1000


def test_failed_transaction_is_a_valid_response():
    tx = ready().accept_transaction(transaction(status="FAILED", successful=False))
    assert tx.successful is False


def test_not_found_is_a_rejection_not_a_result():
    rejects(
        RpcErrorCode.TX_STATUS,
        lambda: ready().accept_transaction({"status": "NOT_FOUND"}),
    )


def test_transaction_from_another_submission_is_rejected():
    rejects(
        RpcErrorCode.TX_HASH,
        lambda: ready().accept_transaction(transaction(hash="e" * 64), expected_hash="c" * 64),
    )


def test_transaction_beyond_the_validated_head_is_rejected():
    rejects(RpcErrorCode.TX_LEDGER, lambda: ready().accept_transaction(transaction(ledger=1005)))


def test_transaction_beyond_the_age_window_is_rejected():
    policy = NetworkPolicy.testnet(max_tx_age_ledgers=2)
    g = RpcGuard(policy)
    g.accept_network(network())
    g.accept_ledger(ledger(sequence=1000))
    rejects(RpcErrorCode.TX_LEDGER, lambda: g.accept_transaction(transaction(ledger=900, createdAt=1792490400)))


def test_status_and_result_flag_must_agree():
    rejects(
        RpcErrorCode.TX_STATUS,
        lambda: ready().accept_transaction(transaction(successful=False)),
    )


@pytest.mark.parametrize("missing", ["envelopeXdr", "resultXdr", "resultMetaXdr"])
def test_result_without_its_envelope_or_meta_is_rejected(missing):
    payload = transaction()
    del payload[missing]
    rejects(RpcErrorCode.TX_ENVELOPE, lambda: ready().accept_transaction(payload))


def test_transaction_time_must_be_near_the_ledger_close():
    rejects(
        RpcErrorCode.TX_TIME,
        lambda: ready().accept_transaction(transaction(createdAt=1792490000)),
    )


def test_negative_fee_is_rejected():
    rejects(
        RpcErrorCode.MALFORMED_RESPONSE,
        lambda: ready().accept_transaction(transaction(feeCharged=-1)),
    )


# -- observability ---------------------------------------------------------


def test_rejections_are_recorded_and_reported():
    seen = []
    g = RpcGuard(NetworkPolicy.testnet(), on_reject=seen.append)
    with pytest.raises(RpcValidationError):
        g.accept_network(network(passphrase="nope"))
    assert len(seen) == 1 and seen[0].code is RpcErrorCode.NETWORK_MISMATCH
    assert g.summary()["rejections"] == {"network_mismatch": 1}
    assert g.network is None and g.ledger is None


def test_summary_is_bounded_json():
    g = ready()
    summary = g.summary()
    assert json.dumps(summary)
    assert summary["network"] == "testnet" and summary["ledger"] == 1000


def test_error_detail_is_bounded_and_sanitised():
    error = RpcValidationError(RpcErrorCode.MALFORMED_RESPONSE, "field\nname", "x" * 4096 + "\x00")
    assert len(error.detail) <= MAX_DETAIL
    assert "\n" not in error.field and "\x00" not in error.detail
    assert len(str(error)) < MAX_DETAIL + 64


def test_details_never_echo_the_rejected_value():
    # A hostile endpoint could try to smuggle log lines through a field value;
    # the message must describe the failure without repeating it.
    injected = "INJECTED-LOG-LINE-9c1f"
    error = rejects(
        RpcErrorCode.NETWORK_MISMATCH,
        lambda: guard().accept_network(network(passphrase=injected)),
    )
    assert injected not in str(error)


# -- gate corpus -----------------------------------------------------------


def test_every_fixture_case_behaves_as_documented():
    results = gate.run()
    failed = [r.line() for r in results if not r.ok]
    assert not failed, "\n".join(failed)
    assert len(results) == 19


def test_corpus_exercises_the_rejection_classes_that_matter():
    codes = {case["expect"] for case in gate.load_manifest()["cases"]}
    required = {
        "network_mismatch",
        "protocol_unsupported",
        "ledger_hash",
        "ledger_time",
        "ledger_regression",
        "ledger_reorg",
        "tx_status",
        "tx_hash",
        "tx_ledger",
        "tx_envelope",
        "tx_time",
        "rpc_error",
        "malformed_response",
    }
    assert required <= codes, sorted(required - codes)


def test_gate_report_is_generated(tmp_path):
    report = tmp_path / "rpc-boundary.md"
    assert gate.main(["--report", str(report)]) == 0
    text = report.read_text()
    assert "# RPC trust-boundary gate" in text and "passed: 19" in text
