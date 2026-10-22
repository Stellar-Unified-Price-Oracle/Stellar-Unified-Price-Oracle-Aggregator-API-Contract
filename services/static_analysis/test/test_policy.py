"""Tests for the SAST / advisory / baseline policy gate (#500)."""
from __future__ import annotations

import json
from datetime import date, timedelta
from pathlib import Path

import pytest

from services.static_analysis.gate import gate
from services.static_analysis.policy import (
    MAX_ALLOWLIST_DAYS,
    AllowlistEntry,
    Finding,
    UnsafeBaseline,
    baseline_digest,
    check_advisories,
    check_unsafe,
    exceedances,
    filter_allowlisted,
    load_allowlist,
    load_baseline,
    parse_cargo_audit,
    rebaseline,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
SHIPPED_BASELINE = REPO_ROOT / "config" / "security-baseline.json"
SHIPPED_ALLOWLIST = REPO_ROOT / "config" / "security-allowlist.json"
CONTRACTS = REPO_ROOT / "contracts"

TODAY = date(2026, 1, 1)


def _soon(days: int = 30) -> str:
    return (TODAY + timedelta(days=days)).isoformat()


def _entry(**kwargs) -> AllowlistEntry:
    base = dict(
        id="SAST-RUST-UNWRAP",
        kind="sast",
        owner="alice",
        expires=_soon(),
        reason="known false positive",
        paths=("price-oracle/src/lib.rs",),
    )
    base.update(kwargs)
    return AllowlistEntry(**base)  # type: ignore[arg-type]


def _finding(path: str = "price-oracle/src/lib.rs", line: int = 10) -> Finding:
    return Finding(
        id="SAST-RUST-UNWRAP",
        kind="sast",
        path=path,
        line=line,
        message=".unwrap()",
    )


# -- allowlist: owner, expiry, scope ----------------------------------------


def test_a_valid_entry_has_no_errors():
    assert _entry().errors(TODAY) == []


def test_entry_without_owner_is_rejected():
    errors = _entry(owner="  ").errors(TODAY)
    assert any("no owner" in e for e in errors)


def test_entry_without_expiry_is_rejected():
    errors = _entry(expires="").errors(TODAY)
    assert any("ISO date" in e for e in errors)


def test_expired_entry_is_rejected():
    errors = _entry(expires=(TODAY - timedelta(days=1)).isoformat()).errors(TODAY)
    assert any("expired" in e for e in errors)


def test_entry_must_be_time_boxed():
    errors = _entry(expires=_soon(MAX_ALLOWLIST_DAYS + 10)).errors(TODAY)
    assert any("time-boxed" in e for e in errors)


def test_blanket_path_scope_is_rejected_as_a_blanket_suppression():
    errors = _entry(paths=("**",)).errors(TODAY)
    assert any("blanket path scope" in e for e in errors)
    assert _entry(paths=()).errors(TODAY)  # no scope matches nothing


def test_entry_scope_is_honoured():
    assert _entry().matches("SAST-RUST-UNWRAP", "price-oracle/src/lib.rs")
    assert not _entry().matches("SAST-RUST-UNWRAP", "price-oracle/src/other.rs")
    assert not _entry().matches("SAST-RUST-EXPECT", "price-oracle/src/lib.rs")


def test_entry_without_reason_is_rejected():
    assert any("no reason" in e for e in _entry(reason="").errors(TODAY))


def test_unknown_kind_is_rejected():
    assert any("unknown kind" in e for e in _entry(kind="mystery").errors(TODAY))


# -- allowlist file loading --------------------------------------------------


def test_missing_allowlist_file_is_an_error_not_an_empty_allowlist(tmp_path: Path):
    entries, errors = load_allowlist(tmp_path / "nope.json", TODAY)
    assert entries == []
    assert errors and "missing" in errors[0]


def test_malformed_allowlist_file_is_an_error(tmp_path: Path):
    path = tmp_path / "a.json"
    path.write_text("{not json", encoding="utf-8")
    entries, errors = load_allowlist(path, TODAY)
    assert entries == []
    assert any("not valid JSON" in e for e in errors)


def test_expired_entry_in_file_fails_the_load(tmp_path: Path):
    path = tmp_path / "a.json"
    path.write_text(
        json.dumps({"entries": [_entry(expires="2020-01-01").to_dict()]}), encoding="utf-8"
    )
    entries, errors = load_allowlist(path, TODAY)
    assert len(entries) == 1
    assert any("expired" in e for e in errors)


def test_shipped_allowlist_is_valid():
    entries, errors = load_allowlist(SHIPPED_ALLOWLIST)
    assert errors == [], errors
    for entry in entries:
        assert entry.owner and entry.expires and entry.paths


# -- allowlisting findings ---------------------------------------------------


def test_allowlisted_finding_is_split_out_and_not_reported_as_new():
    new, allowed = filter_allowlisted([_finding()], [_entry()])
    assert new == []
    assert len(allowed) == 1


def test_finding_outside_the_allowlist_scope_still_fails():
    new, allowed = filter_allowlisted([_finding(path="price-oracle/src/zzz.rs")], [_entry()])
    assert len(new) == 1 and allowed == []


# -- baseline: cannot grow without review -----------------------------------


def test_baseline_digest_changes_when_counts_change():
    a = baseline_digest({"X:1": 1})
    b = baseline_digest({"X:1": 2})
    assert a != b


def test_edited_counts_without_reapproval_fail():
    baseline = UnsafeBaseline(counts={"X:1": 1}, approved_sha256=baseline_digest({"X:1": 1}))
    tampered = UnsafeBaseline(
        counts={"X:1": 5}, approved_sha256=baseline.approved_sha256
    )
    assert tampered.errors() != []
    assert any("edited without re-approval" in e for e in tampered.errors())


def test_rebaseline_requires_a_named_reviewer():
    with pytest.raises(ValueError):
        rebaseline({"X:1": 1}, "  ")


def test_rebaseline_records_the_reviewer_and_a_matching_digest():
    b = rebaseline({"X:1": 3}, "alice", TODAY)
    assert b.approved_by == "alice"
    assert b.approved_on == TODAY.isoformat()
    assert b.errors() == []


def test_unsafe_addition_beyond_the_baseline_fails():
    baseline = rebaseline({"SAST-RUST-UNWRAP:a.rs": 2}, "alice", TODAY)
    new, errors = check_unsafe({"SAST-RUST-UNWRAP:a.rs": 3}, baseline)
    assert not errors
    assert len(new) == 1 and "baseline allows 2" in new[0]


def test_new_file_with_unsafe_code_fails_even_at_the_same_total():
    baseline = rebaseline({"SAST-RUST-UNWRAP:a.rs": 2}, "alice", TODAY)
    new, _ = check_unsafe({"SAST-RUST-UNWRAP:a.rs": 1, "SAST-RUST-UNWRAP:b.rs": 1}, baseline)
    assert any("b.rs" in item for item in new)


def test_baseline_drift_downward_also_fails():
    baseline = rebaseline({"SAST-RUST-UNWRAP:a.rs": 5}, "alice", TODAY)
    _, errors = check_unsafe({"SAST-RUST-UNWRAP:a.rs": 2}, baseline)
    assert any("tighten the baseline" in e for e in errors)


def test_missing_baseline_fails():
    new, errors = check_unsafe({}, None)
    assert new == [] and errors


def test_exceedances_are_the_findings_after_the_accepted_ones():
    baseline = rebaseline({"SAST-RUST-UNWRAP:a.rs": 1}, "alice", TODAY)
    findings = [
        _finding(path="a.rs", line=1),
        _finding(path="a.rs", line=2),
        _finding(path="a.rs", line=3),
    ]
    new = exceedances(findings, baseline)
    assert [f.line for f in new] == [2, 3]


def test_shipped_baseline_matches_the_repository():
    baseline, errors = load_baseline(SHIPPED_BASELINE)
    assert errors == [], errors
    assert baseline is not None and baseline.approved_by
    assert baseline.counts  # not empty


# -- dependency advisories ---------------------------------------------------

AUDIT_JSON = {
    "vulnerabilities": {
        "list": [
            {
                "id": "RUSTSEC-2024-0001",
                "package": "smallvec",
                "title": "buffer overflow",
                "versions": {"patched": [">=1.6.1"]},
            },
            {
                "id": "RUSTSEC-2024-9999",
                "package": "unfixable-crate",
                "title": "no patch available",
                "versions": {"patched": []},
            },
        ]
    }
}


def test_cargo_audit_json_is_parsed():
    advisories = parse_cargo_audit(AUDIT_JSON)
    assert [a.id for a in advisories] == ["RUSTSEC-2024-0001", "RUSTSEC-2024-9999"]
    assert advisories[0].has_fix is True
    assert advisories[1].has_fix is False


def test_a_deliberately_introduced_vulnerable_dependency_fails_the_gate():
    advisories = parse_cargo_audit(AUDIT_JSON)
    new, allowed, notes = check_advisories(advisories, [])
    assert [a.id for a in new] == ["RUSTSEC-2024-0001", "RUSTSEC-2024-9999"]
    assert allowed == []
    # The unfixable one must be explicitly acknowledged, never silently ignored.
    assert any("no patched version" in n for n in notes)


def test_advisory_with_no_fix_needs_an_owned_time_boxed_allowlist_entry():
    advisories = parse_cargo_audit(AUDIT_JSON)
    entry = AllowlistEntry(
        id="RUSTSEC-2024-9999",
        kind="advisory",
        owner="bob",
        expires=_soon(60),
        reason="no fix upstream; pinned until 2.0",
        paths=("crates/unfixable-crate",),
    )
    new, allowed, notes = check_advisories(advisories, [entry])
    assert [a.id for a in new] == ["RUSTSEC-2024-0001"]  # the fixable one still fails
    assert [a.id for a in allowed] == ["RUSTSEC-2024-9999"]
    assert not notes
    assert entry.errors(TODAY) == []


def test_advisory_allowlist_does_not_cover_other_crates():
    advisories = parse_cargo_audit(AUDIT_JSON)
    entry = AllowlistEntry(
        id="RUSTSEC-2024-9999",
        kind="advisory",
        owner="bob",
        expires=_soon(),
        reason="x",
        paths=("crates/some-other-crate",),
    )
    new, _, _ = check_advisories(advisories, [entry])
    assert "RUSTSEC-2024-9999" in [a.id for a in new]


def test_advisory_without_allowlist_entry_fails_the_full_gate(tmp_path: Path):
    baseline = tmp_path / "baseline.json"
    baseline.write_text(
        json.dumps(rebaseline({}, "alice", TODAY).to_dict()), encoding="utf-8"
    )
    allowlist = tmp_path / "allowlist.json"
    allowlist.write_text(json.dumps({"entries": []}), encoding="utf-8")
    empty = tmp_path / "empty"
    empty.mkdir()
    result = gate(
        root=empty,
        baseline_path=baseline,
        allowlist_path=allowlist,
        advisories=parse_cargo_audit(AUDIT_JSON),
    )
    assert result["ok"] is False
    assert any("RUSTSEC-2024-0001" in e for e in result["errors"])


# -- the gate over the real repository --------------------------------------


def test_gate_is_green_on_the_untouched_repository():
    result = gate(root=CONTRACTS, allowlist_path=SHIPPED_ALLOWLIST, baseline_path=SHIPPED_BASELINE)
    assert result["ok"] is True, result["errors"]
    assert result["new_findings"] == []
    assert result["errors"] == []


def test_gate_produces_a_human_readable_report():
    result = gate(root=CONTRACTS, allowlist_path=SHIPPED_ALLOWLIST, baseline_path=SHIPPED_BASELINE)
    report = result["report"]
    assert report.startswith("# Static analysis / SAST report")
    assert "## Findings by rule" in report
    assert "## Unsafe-code baseline" in report
    assert "## Dependency advisories" in report
