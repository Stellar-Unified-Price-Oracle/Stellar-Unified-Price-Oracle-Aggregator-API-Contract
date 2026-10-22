"""Tests for production incident evidence redaction (#687)."""

import json

import pytest

from services.evidence_redaction import fixtures, gate
from services.evidence_redaction.redact import (
    RULES,
    RedactionDisabled,
    Redactor,
    main as redact_main,
    redact_text,
    render_report,
)
from services.secret_scan.scanner import scan_text

SEED = fixtures.synthetic_seed()
JWT = fixtures.synthetic_jwt()
BASIC = fixtures.synthetic_basic_auth()
DB_PASSWORD = "hunter2-7"


def redact(text):
    return redact_text(text)


# -- one rule at a time ----------------------------------------------------


def test_stellar_seed_is_removed_wherever_it_appears():
    result = redact(f"admin key {SEED} and again {SEED}")
    assert SEED not in result.text
    assert result.text.count("[REDACTED:stellar-seed]") == 2
    assert result.counts == {"stellar-seed": 2}


def test_fixtures_are_realistic_enough_for_the_repo_secret_scanner():
    # The synthetic seed is generated at runtime precisely because it is a
    # valid seed shape: this is the scanner the pre-commit hook runs.
    findings = scan_text(f"key={SEED}", "<synthetic>")
    assert [f.id for f in findings] == ["SECRET-SOROBAN-SECRET-SEED"]


def test_authorization_headers_keep_their_structure():
    result = redact(f"Authorization: {BASIC}")
    assert "Authorization: Basic [REDACTED:auth-header]" == result.text
    assert BASIC.split(" ", 1)[1] not in result.text


def test_bearer_token_is_removed_but_the_scheme_survives():
    result = redact(f"Authorization: Bearer {JWT}")
    assert JWT not in result.text
    assert result.text == "Authorization: Bearer [REDACTED:auth-header]"


def test_credential_flags_keep_the_flag_name():
    result = redact(f"oracle-cli add-source --admin-key {SEED} --name chainlink")
    assert f"--admin-key [REDACTED:key-flag]" in result.text
    assert "--name chainlink" in result.text
    assert SEED not in result.text


def test_environment_assignments_keep_the_variable_name():
    result = redact(f"export ORACLE_SOURCE_KEY={SEED}")
    assert result.text.startswith("export ORACLE_SOURCE_KEY=[REDACTED:")


def test_json_credentials_keep_the_field_name():
    body = json.dumps({"level": "error", "api_key": SEED, "msg": "rpc rejected"})
    result = redact(body)
    payload = json.loads(result.text)
    assert payload["msg"] == "rpc rejected"
    assert payload["api_key"] == "[REDACTED:json-field]"
    assert SEED not in result.text


def test_url_password_is_removed_but_user_and_host_survive():
    result = redact(f"postgres://oracle:{DB_PASSWORD}@db.internal:5432/oracle")
    assert result.text == "postgres://oracle:[REDACTED:url-userinfo]@db.internal:5432/oracle"


def test_pem_block_is_removed_entirely():
    # Headers come from the fixture module: a committed literal would be
    # flagged by this repository's own scanner.
    pem = "\n".join([fixtures.PEM_BEGIN, fixtures.synthetic_pem_body(), fixtures.PEM_END])
    result = redact(f"key follows:\n{pem}\nafter")
    assert fixtures.PEM_BEGIN not in result.text and "synthetic-pem" not in result.text
    assert result.text == "key follows:\n[REDACTED:pem-private-key]\nafter"


def test_public_chain_data_is_not_redacted():
    text = f"admin={fixtures.OPERATOR_ACCOUNT} contract={fixtures.CONTRACT_ID}"
    assert redact(text).text == text


def test_unanchored_seed_inside_a_word_is_ignored():
    # `\b` keeps a longer base32 run from being truncated into a false seed.
    text = "X" + SEED
    assert SEED in redact(text).text


# -- whole-document behaviour ---------------------------------------------


def test_incident_bundle_loses_every_secret():
    log = fixtures.synthetic_incident_log()
    result = redact(log)
    for secret in fixtures.synthetic_secrets():
        assert secret not in result.text
    assert result.redacted == 10


def test_redaction_is_idempotent():
    log = fixtures.synthetic_incident_log()
    once = redact(log).text
    twice = redact(once)
    assert twice.text == once
    assert twice.redacted == 0


def test_findings_point_at_the_source_lines():
    result = redact("clean line\nAuthorization: Bearer " + JWT + "\n")
    finding = result.findings[0]
    assert finding.rule_id == "auth-header" and finding.line == 2


def test_report_quotes_no_secret_material():
    log = fixtures.synthetic_incident_log()
    result = redact(log)
    report = render_report(result, "incident.log")
    for secret in fixtures.synthetic_secrets():
        assert secret not in report
    assert "| stellar-seed |" in report


def test_every_rule_is_exercised_by_the_bundle():
    result = redact(fixtures.synthetic_incident_log())
    assert set(result.counts) == {rule.id for rule in RULES}
    assert all(count > 0 for count in result.counts.values())


# -- fail closed -----------------------------------------------------------


def test_disabling_redaction_raises_instead_of_returning_input():
    with pytest.raises(RedactionDisabled):
        Redactor(enabled=False)


def test_redact_text_refuses_to_pass_through():
    with pytest.raises(RedactionDisabled):
        redact_text("Authorization: Bearer " + JWT, enabled=False)


def test_cli_refuses_no_redact_and_writes_nothing(tmp_path):
    source = tmp_path / "raw.log"
    source.write_text(fixtures.synthetic_incident_log())
    out = tmp_path / "out.log"
    assert redact_main([str(source), "--no-redact", "--out", str(out)]) == 2
    assert not out.exists()


def test_cli_check_fails_closed_on_unredacted_evidence(tmp_path):
    source = tmp_path / "raw.log"
    source.write_text(fixtures.synthetic_incident_log())
    assert redact_main([str(source), "--check"]) == 1


def test_cli_writes_redacted_artifact_and_report(tmp_path):
    source = tmp_path / "raw.log"
    source.write_text(fixtures.synthetic_incident_log())
    out = tmp_path / "out.log"
    report = tmp_path / "report.md"
    assert redact_main([str(source), "--out", str(out), "--report", str(report)]) == 0
    redacted = out.read_text()
    for secret in fixtures.synthetic_secrets():
        assert secret not in redacted
    assert redact_main([str(out), "--check"]) == 0
    for secret in fixtures.synthetic_secrets():
        assert secret not in report.read_text()


def test_cli_redacts_stdin(monkeypatch, capsys):
    monkeypatch.setattr("sys.stdin", __import__("io").StringIO(f"token {SEED}\n"))
    assert redact_main([]) == 0
    captured = capsys.readouterr().out
    assert SEED not in captured and "[REDACTED:stellar-seed]" in captured


# -- gate ------------------------------------------------------------------


def test_gate_passes_and_reports(tmp_path):
    checks = gate.run()
    failed = [c.line() for c in checks if not c.ok]
    assert not failed, "\n".join(failed)
    report = tmp_path / "redaction.md"
    assert gate.main(["--report", str(report)]) == 0
    text = report.read_text()
    assert "passed: 20" in text
    assert SEED not in text and JWT not in text and DB_PASSWORD not in text
