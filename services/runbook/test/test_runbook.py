"""Tests for the runbook coverage checker (#527).

The gate is only worth having if it can fail, so most of these tests feed it a
deliberately broken runbook and assert the specific problem is reported.
"""

from __future__ import annotations

import shutil
from pathlib import Path

from services.runbook.check_runbook import (
    CADENCES,
    REQUIRED_RULE_FILES,
    REQUIRED_FIELDS,
    ROLES,
    AlertRule,
    alert_routing_table,
    check_runbook,
    load_alerts,
    parse_alert_map,
    parse_alert_rules,
    parse_runbook,
    rule_files,
)

REPO = Path(__file__).resolve().parents[3]
RUNBOOK_MD = REPO / "docs" / "runbook.md"
RULES_V2 = REPO / "docs" / "monitoring" / "alerts-v2.yml"
RULES_V1 = REPO / "docs" / "monitoring" / "alerts.yml"


def _copy_repo(tmp_path: Path) -> Path:
    """Copies the runbook and both rule files into an isolated repo layout."""
    root = tmp_path / "repo"
    (root / "docs" / "monitoring").mkdir(parents=True)
    shutil.copy(RUNBOOK_MD, root / "docs" / "runbook.md")
    for rule in (RULES_V1, RULES_V2):
        shutil.copy(rule, root / "docs" / "monitoring" / rule.name)
    return root


def _errors_for(markdown: str, tmp_path: Path) -> list:
    """Runs the gate against a mutated runbook in a copy of the repo layout."""
    root = _copy_repo(tmp_path)
    (root / "docs" / "runbook.md").write_text(markdown)
    return check_runbook(root)


# ── The committed runbook and rules are consistent ────────────────────────────

def test_committed_runbook_covers_every_paging_alert():
    assert check_runbook(REPO) == []


def test_every_paging_alert_is_mapped_and_entries_are_unique():
    entries = parse_runbook(RUNBOOK_MD.read_text())
    ids = [e.entry_id for e in entries]
    assert ids == sorted(ids, key=lambda i: int(i.split("-")[1]))
    assert len(ids) == len(set(ids))

    paging = {a.name for a in load_alerts(REPO) if a.pages}
    mapped = {name for name, _ in parse_alert_map(RUNBOOK_MD.read_text())}
    assert paging == mapped


def test_every_entry_has_all_required_fields():
    for entry in parse_runbook(RUNBOOK_MD.read_text()):
        for field in REQUIRED_FIELDS:
            assert entry.fields.get(field, "").strip(), f"{entry.entry_id} missing {field}"


def test_owners_and_cadences_are_known():
    for entry in parse_runbook(RUNBOOK_MD.read_text()):
        assert any(f"`{role}`" in entry.fields["Owner"] for role in ROLES)
        assert any(c.lower() in entry.fields["Review cadence"].lower() for c in CADENCES)


def test_routing_table_links_every_paging_alert():
    routing = alert_routing_table(REPO)
    paging = {a.name for a in load_alerts(REPO) if a.pages}
    assert paging <= set(routing)
    for alert, link in routing.items():
        assert link["url"].startswith("docs/runbook.md#rb-")
        assert link["entry"] and link["owner"]


def test_paging_classification_follows_severity_and_sla_class():
    assert AlertRule("A", "f", severity="critical", sla_class="P1").pages
    assert AlertRule("B", "f", severity="warning", sla_class="P0").pages
    assert not AlertRule("C", "f", severity="warning", sla_class="P3").pages
    assert not AlertRule("D", "f", severity="warning", sla_class="governance").pages


def test_alert_labels_do_not_leak_between_blocks():
    text = (
        "groups:\n"
        "  - name: g\n"
        "    rules:\n"
        "      - alert: First\n"
        "        labels:\n"
        "          severity: critical\n"
        "      - alert: Second\n"
        "        labels:\n"
        "          severity: warning\n"
    )
    alerts = parse_alert_rules(text, "f.yml")
    assert [(a.name, a.severity) for a in alerts] == [("First", "critical"), ("Second", "warning")]


# ── The gate fails on drift ───────────────────────────────────────────────────

def test_gate_fails_when_a_paging_alert_has_no_entry(tmp_path):
    errors = _errors_for("# Runbook\n", tmp_path)
    assert any("has no runbook entry" in e for e in errors)


def test_gate_fails_when_a_required_field_is_emptied(tmp_path):
    lines = RUNBOOK_MD.read_text().splitlines(keepends=True)
    kept = [ln for ln in lines if not ln.startswith("**Resolution:** `get_sources()`")]
    assert len(kept) == len(lines) - 1, "fixture drift: RB-01 resolution line not found"
    errors = _errors_for("".join(kept), tmp_path)
    assert any("RB-01" in e and "'Resolution'" in e for e in errors)


def test_gate_fails_on_unknown_owner_and_cadence(tmp_path):
    broken = RUNBOOK_MD.read_text().replace("**Owner:** `risk`", "**Owner:** `someone-else`")
    broken = broken.replace("**Review cadence:** Quarterly.\n", "**Review cadence:** Eventually.\n", 1)
    errors = _errors_for(broken, tmp_path)
    assert any("is not a known role" in e for e in errors)
    assert any("names no known period" in e for e in errors)


def test_gate_fails_on_orphan_entry(tmp_path):
    broken = RUNBOOK_MD.read_text() + (
        "\n## RB-99 — Entry for an alert that does not exist\n\n"
        "**Alerts:** `NoSuchAlert` (`alerts-v2.yml`) — severity `critical`\n"
        "**Meaning:** x\n**First check:** x\n**Mitigation:** x\n**Escalation:** x\n"
        "**Resolution:** x\n**Owner:** `oracle-oncall`\n**Review cadence:** Quarterly\n"
    )
    errors = _errors_for(broken, tmp_path)
    assert any("RB-99" in e and "orphan" in e for e in errors)


def test_gate_fails_when_a_paging_alert_leaves_the_table(tmp_path):
    broken = RUNBOOK_MD.read_text().replace(
        "| `OracleAdminChanged` | `alerts-v2.yml`, `alerts.yml` | RB-07 |\n", ""
    )
    errors = _errors_for(broken, tmp_path)
    assert any("missing from the alert -> entry table" in e for e in errors)


def test_gate_rejects_a_table_row_for_a_non_paging_alert(tmp_path):
    broken = RUNBOOK_MD.read_text().replace(
        "| `OracleAdminChanged` | `alerts-v2.yml`, `alerts.yml` | RB-07 |",
        "| `OracleAdminChanged` | `alerts-v2.yml`, `alerts.yml` | RB-07 |\n"
        "| `OraclePriceSpike` | `alerts.yml` | RB-07 |",
    )
    errors = _errors_for(broken, tmp_path)
    assert any("non-paging alert OraclePriceSpike" in e for e in errors)


def test_parse_alert_map_ignores_the_ownership_table():
    names = {name for name, _ in parse_alert_map(RUNBOOK_MD.read_text())}
    assert "oracle-oncall" not in names
    assert "OracleAdminChanged" in names


def test_field_body_stops_at_a_blank_line():
    entries = parse_runbook(
        "## RB-01 — T\n\n**Owner:** `oracle-oncall`\nmore owner text\n\nstray paragraph\n"
    )
    assert entries[0].fields["Owner"] == "`oracle-oncall` more owner text"
    assert "stray paragraph" not in entries[0].fields["Owner"]


def test_a_newly_added_alert_file_is_covered_automatically(tmp_path):
    """A new rule file must not be able to escape runbook coverage."""
    root = _copy_repo(tmp_path)
    (root / "docs" / "monitoring" / "alerts-extra.yml").write_text(
        "groups:\n  - name: g\n    rules:\n      - alert: BrandNewPage\n"
        "        labels:\n          severity: critical\n"
    )
    errors = check_runbook(root)
    assert any("BrandNewPage" in e and "has no runbook entry" in e for e in errors)


def test_promtool_test_files_are_not_treated_as_rule_files():
    names = {p.name for p in rule_files(REPO)}
    assert "alerts-v2.yml" in names
    assert not any(n.endswith("_test.yml") for n in names)


def test_every_required_rule_file_is_present():
    assert [p.name for p in rule_files(REPO)] == sorted(
        p.name for p in rule_files(REPO)
    )
    for rel in REQUIRED_RULE_FILES:
        assert (REPO / rel).is_file(), rel


def test_check_runbook_reports_a_missing_rule_file(tmp_path):
    root = tmp_path / "repo"
    (root / "docs").mkdir(parents=True)
    shutil.copy(RUNBOOK_MD, root / "docs" / "runbook.md")
    # No rule files at all: entries become orphans rather than silently passing,
    # and the missing rule files are reported by name.
    errors = check_runbook(root)
    assert errors
    assert any("is missing" in e for e in errors)
    assert any("orphan" in e for e in errors)
