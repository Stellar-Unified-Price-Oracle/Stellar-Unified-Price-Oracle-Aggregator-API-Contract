"""Tests for the source-level SAST rules themselves (#500)."""
from __future__ import annotations

import json
from datetime import date
from pathlib import Path

from services.static_analysis.gate import gate
from services.static_analysis.policy import rebaseline
from services.static_analysis.sast import (
    RULES,
    is_test_path,
    iter_source_files,
    scan_text,
    scan_tree,
    unsafe_counts,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
CONTRACTS = REPO_ROOT / "contracts"
SHIPPED_BASELINE = REPO_ROOT / "config" / "security-baseline.json"
SHIPPED_ALLOWLIST = REPO_ROOT / "config" / "security-allowlist.json"
TODAY = date(2026, 1, 1)


def _ids(text: str):
    return sorted({f.id for f in scan_text(text, "x.rs")})


def test_unsafe_block_is_high_severity_and_detected():
    findings = scan_text("fn f() { unsafe { core::ptr::read(p) } }", "x.rs")
    assert [f.id for f in findings] == ["SAST-RUST-UNSAFE-BLOCK"]
    assert findings[0].line == 1
    rule = next(r for r in RULES if r.id == "SAST-RUST-UNSAFE-BLOCK")
    assert rule.severity == "high"


def test_unsafe_fn_is_detected():
    assert "SAST-RUST-UNSAFE-BLOCK" in _ids("unsafe fn f() -> u8 { 1 }")


def test_unwrap_unchecked_is_high_severity():
    findings = scan_text("let v = x.unwrap_unchecked();", "x.rs")
    assert [f.id for f in findings] == ["SAST-RUST-UNWRAP-UNCHECKED"]


def test_unwrap_and_expect_are_flagged_separately():
    assert _ids("a.unwrap(); b.expect(\"boom\");") == [
        "SAST-RUST-EXPECT",
        "SAST-RUST-UNWRAP",
    ]


def test_panic_macros_are_flagged():
    assert "SAST-RUST-SLAB-PANIC" in _ids("panic!(\"nope\");")


def test_narrowing_cast_is_flagged_but_widening_is_not():
    assert "SAST-RUST-NARROWING-CAST" in _ids("let y = x as u32;")
    assert "SAST-RUST-NARROWING-CAST" not in _ids("let y = x as i128;")


def test_safe_code_produces_no_findings():
    assert scan_text("pub fn add(a: i128, b: i128) -> i128 { a + b }", "x.rs") == []


def test_commented_out_code_is_not_a_finding():
    assert scan_text("// let v = x.unwrap();", "x.rs") == []
    assert scan_text("    // unsafe { core::ptr::null_mut() }", "x.rs") == []


def test_soroban_panic_with_error_is_not_flagged_as_a_raw_panic():
    # The codebase's idiomatic failure path must not trip the SAST rule.
    assert scan_text('panic_with_error!(env, ErrorCode::NoData);', "x.rs") == []


def test_test_files_are_excluded_from_the_scan():
    assert is_test_path("price-oracle/src/test.rs")
    assert is_test_path("price-oracle/src/chaos_tests.rs")
    assert not is_test_path("price-oracle/src/prices.rs")


def test_scan_skips_test_and_build_directories(tmp_path: Path):
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "lib.rs").write_text("a.unwrap();\n", encoding="utf-8")
    (tmp_path / "src" / "unit_tests.rs").write_text("a.unwrap();\n", encoding="utf-8")
    (tmp_path / "target").mkdir()
    (tmp_path / "target" / "gen.rs").write_text("a.unwrap();\n", encoding="utf-8")
    files = [str(p.relative_to(tmp_path)) for p in iter_source_files(tmp_path)]
    assert files == ["src/lib.rs"]
    assert len(scan_tree(tmp_path)) == 1


def test_counts_are_keyed_by_rule_and_file():
    findings = scan_text("a.unwrap();\nb.unwrap();\n", "src/x.rs")
    assert unsafe_counts(findings) == {"SAST-RUST-UNWRAP:src/x.rs": 2}


# -- the gate reacts to a newly introduced unsafe construct ------------------


def _write_baseline(tmp_path: Path, counts):
    path = tmp_path / "baseline.json"
    path.write_text(json.dumps(rebaseline(counts, "alice", TODAY).to_dict()), encoding="utf-8")
    return path


def test_new_unsafe_code_in_a_baselined_file_fails_the_gate(tmp_path: Path):
    src = tmp_path / "contracts"
    src.mkdir()
    (src / "lib.rs").write_text("a.unwrap();\n", encoding="utf-8")
    baseline = _write_baseline(tmp_path, {"SAST-RUST-UNWRAP:lib.rs": 1})
    allowlist = tmp_path / "allowlist.json"
    allowlist.write_text(json.dumps({"entries": []}), encoding="utf-8")

    clean = gate(root=src, baseline_path=baseline, allowlist_path=allowlist)
    assert clean["ok"] is True, clean["errors"]

    (src / "lib.rs").write_text("a.unwrap();\nb.unwrap();\n", encoding="utf-8")
    dirty = gate(root=src, baseline_path=baseline, allowlist_path=allowlist)
    assert dirty["ok"] is False
    assert any("lib.rs:2" in e for e in dirty["errors"])
    assert any("unsafe addition beyond baseline" in e for e in dirty["errors"])


def test_a_new_file_containing_unsafe_code_fails_the_gate(tmp_path: Path):
    src = tmp_path / "contracts"
    src.mkdir()
    (src / "lib.rs").write_text("", encoding="utf-8")
    baseline = _write_baseline(tmp_path, {})
    allowlist = tmp_path / "allowlist.json"
    allowlist.write_text(json.dumps({"entries": []}), encoding="utf-8")
    (src / "evil.rs").write_text("unsafe { core::ptr::null_mut() };\n", encoding="utf-8")
    result = gate(root=src, baseline_path=baseline, allowlist_path=allowlist)
    assert result["ok"] is False
    assert any("SAST-RUST-UNSAFE-BLOCK" in e for e in result["errors"])


def test_allowlisting_a_specific_file_keeps_scanning_everywhere_else(tmp_path: Path):
    src = tmp_path / "contracts"
    src.mkdir()
    (src / "lib.rs").write_text("a.unwrap();\nb.unwrap();\n", encoding="utf-8")
    (src / "other.rs").write_text("c.unwrap();\n", encoding="utf-8")
    baseline = _write_baseline(tmp_path, {})
    allowlist = tmp_path / "allowlist.json"
    allowlist.write_text(
        json.dumps(
            {
                "entries": [
                    {
                        "id": "SAST-RUST-UNWRAP",
                        "kind": "sast",
                        "owner": "alice",
                        "expires": "2026-06-01",
                        "reason": "reviewed false positive in lib.rs only",
                        "paths": ["lib.rs"],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    result = gate(root=src, baseline_path=baseline, allowlist_path=allowlist)
    # lib.rs is allowlisted; other.rs is not, so the gate still fails.
    assert result["ok"] is False
    assert any("other.rs" in e for e in result["errors"])
    assert not any("lib.rs" in e for e in result["errors"])
    assert len(result["allowlisted"]) == 2


def test_removing_baselined_code_requires_tightening_the_baseline():
    # Shipped baseline is a snapshot of the tree; shrinking the tree without
    # re-baselining is itself an error, so the baseline cannot hide drift.
    baseline, errors = __import__(
        "services.static_analysis.policy", fromlist=["load_baseline"]
    ).load_baseline(SHIPPED_BASELINE)
    assert errors == []
    counts = unsafe_counts(scan_tree(CONTRACTS))
    drifted = {k: v + 5 for k, v in counts.items()}
    from services.static_analysis.policy import check_unsafe

    new, errs = check_unsafe(drifted, baseline)
    assert new and all("baseline allows" in item for item in new)
