"""Tests for the postmortem process checks (#528)."""

from __future__ import annotations

import datetime as dt
from pathlib import Path

from services.incident_review.check_postmortems import (
    POSTMORTEM_DIR,
    REQUIRED_FIELDS,
    REQUIRED_SECTIONS,
    check_index,
    check_overdue,
    check_postmortems,
    load_postmortems,
    overdue_actions,
    parse_postmortem,
    report,
)

REPO = Path(__file__).resolve().parents[3]
DIR = REPO / POSTMORTEM_DIR
TEMPLATE = DIR / "TEMPLATE.md"
TODAY = dt.date(2026, 9, 28)

GOOD = """# Postmortem: something broke

| Field | Value |
|---|---|
| **Incident ID** | `INC-20260812-99` |
| **Date** | 2026-08-12 |
| **Severity** | P1 |
| **Status** | Final |
| **Incident commander** | `oracle-oncall` |
| **Duration** | 10 min |
| **Runbook entries used** | RB-02 |
| **Postmortem owner** | `oracle-oncall` |

## 1. Summary
It broke.

## 2. Impact
None.

## 3. Timeline
| Time (UTC) | Event | Source of truth |
|---|---|---|
| 10:00 | broke | alert |

## 4. Root cause
A cause.

## 5. What went wrong
A gap.

## 6. Action items
| # | Action | Owner | Due | Issue | Status |
|---|---|---|---|---|---|
| 1 | Fix the thing | `oracle-oncall` | 2026-10-01 | #123 | Open |

## 7. Detection and response assessment
Fine.

## 8. Review and sign-off
| Reviewer role | What they checked | Date |
|---|---|---|
| `oracle-secondary` | all | 2026-08-14 |
"""


def _write(root: Path, body: str, name: str = "2026-08-12-x.md") -> Path:
    d = root / POSTMORTEM_DIR
    d.mkdir(parents=True, exist_ok=True)
    (d / name).write_text(body)
    return d / name


def _root_with(body: str, tmp_path: Path, index_body: str = "") -> Path:
    root = tmp_path / "repo"
    _write(root, body)
    (root / POSTMORTEM_DIR / "README.md").write_text(
        index_body or "# Incidents\n\n| x | [2026-08-12-x.md](2026-08-12-x.md) |\n"
    )
    return root


# ── The committed postmortems satisfy the process ──────────────────────────────

def test_committed_postmortems_pass_the_check():
    assert check_postmortems(REPO, TODAY) == []


def test_template_declares_every_required_section_and_field():
    text = TEMPLATE.read_text()
    sections = [ln for ln in text.splitlines() if ln.startswith("## ")]
    names = {ln.lstrip("# ").split(". ", 1)[-1].strip() for ln in sections}
    assert set(REQUIRED_SECTIONS) <= names
    for field in REQUIRED_FIELDS:
        assert f"**{field}**" in text


def test_every_committed_postmortem_is_indexed():
    names = {pm.path for pm in load_postmortems(REPO)}
    assert names
    assert check_index(REPO, load_postmortems(REPO)) == []


def test_committed_postmortems_have_owners_issues_and_due_dates():
    for pm in load_postmortems(REPO):
        assert pm.actions, pm.path
        for a in pm.actions:
            assert a.owner and a.due and a.issue.startswith("#") and a.action


# ── Parsing ──────────────────────────────────────────────────────────────────

def test_parses_fields_sections_and_actions():
    pm = parse_postmortem(Path("2026-08-12-x.md"), GOOD)
    assert pm.fields["Severity"] == "P1"
    assert pm.fields["Incident commander"] == "`oracle-oncall`"
    assert "Action items" in pm.sections
    assert len(pm.actions) == 1
    assert pm.actions[0].due == "2026-10-01"
    assert pm.actions[0].issue == "#123"


def test_numbered_tables_outside_action_items_are_ignored():
    body = GOOD.replace("| 10:00 | broke | alert |", "| 10:00 | broke | alert |\n| 10:05 | more | on-chain |")
    pm = parse_postmortem(Path("x.md"), body)
    assert len(pm.actions) == 1


def test_timeline_header_row_is_not_an_action_item():
    pm = parse_postmortem(Path("x.md"), GOOD)
    assert all(a.number == 1 for a in pm.actions)


# ── Overdue follow-up ────────────────────────────────────────────────────────

def test_open_item_past_due_is_overdue():
    item = parse_postmortem(Path("x.md"), GOOD).actions[0]
    assert item.is_overdue(dt.date(2026, 10, 2))
    assert not item.is_overdue(dt.date(2026, 9, 30))


def test_done_and_wont_fix_items_are_never_overdue():
    base = parse_postmortem(Path("x.md"), GOOD).actions[0]
    for status in ("Done", "Won't fix (superseded)"):
        closed = type(base)(**{**base.__dict__, "status": status})
        assert not closed.is_overdue(dt.date(2030, 1, 1))


def test_overdue_is_surfaced_across_postmortems(tmp_path):
    root = _root_with(GOOD, tmp_path)
    assert check_overdue(root, TODAY) == []
    later = check_overdue(root, dt.date(2026, 11, 1))
    assert len(later) == 1
    assert later[0].owner == "`oracle-oncall`"
    assert len(overdue_actions(load_postmortems(root), dt.date(2026, 11, 1))) == 1


def test_strict_overdue_is_reported_but_not_a_structural_error(tmp_path):
    root = _root_with(GOOD, tmp_path)
    assert check_postmortems(root, dt.date(2026, 11, 1)) == []


def test_unparseable_due_date_is_a_structural_error(tmp_path):
    root = _root_with(GOOD.replace("2026-10-01", "next month"), tmp_path)
    errors = check_postmortems(root, TODAY)
    assert any("not YYYY-MM-DD" in e for e in errors)


# ── The gate fails on a bad postmortem ───────────────────────────────────────

def test_missing_section_is_reported(tmp_path):
    body = GOOD.replace("## 5. What went wrong", "## 5. Other")
    errors = check_postmortems(_root_with(body, tmp_path), TODAY)
    assert any("missing required section 'What went wrong'" in e for e in errors)


def test_missing_header_field_is_reported(tmp_path):
    body = GOOD.replace("| **Postmortem owner** | `oracle-oncall` |\n", "")
    errors = check_postmortems(_root_with(body, tmp_path), TODAY)
    assert any("missing header field 'Postmortem owner'" in e for e in errors)


def test_template_placeholder_is_reported(tmp_path):
    body = GOOD.replace("| **Duration** | 10 min |", "| **Duration** | YYYY-MM-DD |")
    errors = check_postmortems(_root_with(body, tmp_path), TODAY)
    assert any("placeholder" in e for e in errors)


def test_action_item_without_owner_issue_or_due_is_reported(tmp_path):
    body = GOOD.replace("| 1 | Fix the thing | `oracle-oncall` | 2026-10-01 | #123 | Open |",
                        "| 1 | Fix the thing |  |  |  | Open |")
    errors = check_postmortems(_root_with(body, tmp_path), TODAY)
    assert any("has no owner" in e for e in errors)
    assert any("not YYYY-MM-DD" in e for e in errors)
    assert any("has no tracking issue" in e for e in errors)


def test_bad_status_is_reported(tmp_path):
    body = GOOD.replace("#123 | Open |", "#123 | maybe |")
    errors = check_postmortems(_root_with(body, tmp_path), TODAY)
    assert any("is not one of" in e for e in errors)


def test_postmortem_without_action_items_is_reported(tmp_path):
    body = GOOD.replace("| 1 | Fix the thing | `oracle-oncall` | 2026-10-01 | #123 | Open |", "")
    errors = check_postmortems(_root_with(body, tmp_path), TODAY)
    assert any("no action items" in e for e in errors)


def test_unindexed_postmortem_is_reported(tmp_path):
    root = _root_with(GOOD, tmp_path, index_body="# Incidents\n\nNothing here yet.\n")
    errors = check_postmortems(root, TODAY)
    assert any("is not indexed" in e for e in errors)


def test_index_pointing_at_a_missing_file_is_reported(tmp_path):
    root = _root_with(GOOD, tmp_path, index_body="# Incidents\n\n[2020-01-01-gone.md](2020-01-01-gone.md)\n")
    errors = check_postmortems(root, TODAY)
    assert any("index links unknown postmortem" in e for e in errors)


def test_missing_index_is_reported(tmp_path):
    root = tmp_path / "repo"
    _write(root, GOOD)
    errors = check_postmortems(root, TODAY)
    assert any("index is missing" in e for e in errors)


def test_template_itself_is_not_parsed_as_a_postmortem():
    # TEMPLATE.md does not match the dated filename pattern.
    assert all(pm.path != "TEMPLATE.md" for pm in load_postmortems(REPO))


def test_report_lists_every_postmortem_with_overdue_counts():
    out = report(REPO, TODAY)
    for pm in load_postmortems(REPO):
        assert pm.path in out
