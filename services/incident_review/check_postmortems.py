"""Postmortem process checks (#528).

A postmortem that nobody can find, or whose action items have no owners, is a
document, not a process. This module enforces the parts of the process that can
be checked mechanically:

* every postmortem carries the template's required sections and header fields,
  with no template placeholders left in;
* every action item has an owner, a due date, a tracking issue and a status;
* the index in ``README.md`` lists every postmortem file and no phantom entry;
* overdue action items are surfaced, so follow-up cannot quietly die.

Markdown is parsed with the same light-weight line scanners the rest of the
off-chain tooling uses; there is no third-party Python dependency.
"""

from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

POSTMORTEM_DIR = Path("docs/incident-management")
INDEX = POSTMORTEM_DIR / "README.md"
TEMPLATE = POSTMORTEM_DIR / "TEMPLATE.md"

# Sections every postmortem must contain.
REQUIRED_SECTIONS: Tuple[str, ...] = (
    "Summary",
    "Impact",
    "Timeline",
    "Root cause",
    "What went wrong",
    "Action items",
    "Detection and response assessment",
    "Review and sign-off",
)

# Header fields every postmortem must fill in.
REQUIRED_FIELDS: Tuple[str, ...] = (
    "Incident ID",
    "Date",
    "Severity",
    "Status",
    "Incident commander",
    "Duration",
    "Runbook entries used",
    "Postmortem owner",
)

# Permitted action-item statuses; "Won't fix" carries its rationale in the row.
STATUSES: Tuple[str, ...] = ("Open", "In progress", "Done", "Won't fix")

_FILENAME_RE = re.compile(r"^(?P<date>\d{4}-\d{2}-\d{2})-(?P<slug>[a-z0-9-]+)\.md$")
_HEADING_RE = re.compile(r"^##\s+(?:\d+\.\s+)?(?P<name>.+?)\s*$")
_FIELD_ROW_RE = re.compile(r"^\|\s*\*\*(?P<name>[A-Za-z ]+)\*\*\s*\|\s*(?P<value>.*?)\s*\|\s*$")
_ACTION_ROW_RE = re.compile(r"^\|\s*(?P<num>\d+)\s*\|(?P<rest>.*)\|\s*$")
_ISSUE_RE = re.compile(r"#\d+")
_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
_INDEX_LINK_RE = re.compile(r"\]\((?P<path>\d{4}-\d{2}-\d{2}-[a-z0-9-]+\.md)\)")

# Placeholders from the template; a real postmortem must replace every one.
_PLACEHOLDERS = ("YYYY-MM-DD", "INC-YYYYMMDD-NN", "P0 / P1 / P2 / P3", "TBD", "<short title>")


@dataclass(frozen=True)
class ActionItem:
    """One row of a postmortem's action-item table."""

    postmortem: str
    number: int
    action: str
    owner: str
    due: str
    issue: str
    status: str

    @property
    def closed(self) -> bool:
        return self.status == "Done" or self.status.startswith("Won't fix")

    def is_overdue(self, today: dt.date) -> bool:
        """True when the due date has passed and the item is not closed."""
        if self.closed:
            return False
        try:
            return dt.date.fromisoformat(self.due) < today
        except ValueError:
            return False


@dataclass(frozen=True)
class Postmortem:
    """A parsed postmortem file."""

    path: str
    fields: Dict[str, str]
    sections: Tuple[str, ...]
    actions: Tuple[ActionItem, ...]


def parse_postmortem(path: Path, text: str) -> Postmortem:
    """Extracts the header fields, section titles and action items.

    Action rows are only read inside the ``## 6. Action items`` section, so a
    numbered table elsewhere in the document cannot be mistaken for one.
    """
    fields: Dict[str, str] = {}
    sections: List[str] = []
    actions: List[ActionItem] = []
    in_actions = False

    for line in text.splitlines():
        field = _FIELD_ROW_RE.match(line.strip())
        if field:
            fields[field.group("name").strip()] = field.group("value").strip()
            continue

        heading = _HEADING_RE.match(line)
        if heading:
            name = heading.group("name")
            sections.append(name)
            in_actions = name == "Action items"
            continue

        if not in_actions:
            continue
        row = _ACTION_ROW_RE.match(line.strip())
        if not row:
            continue
        cells = [c.strip() for c in row.group("rest").split("|")]
        if len(cells) < 5 or set("".join(cells)) <= set("-: "):
            continue  # header or separator row
        action, owner, due, issue, status = cells[:5]
        actions.append(
            ActionItem(
                postmortem=path.name,
                number=int(row.group("num")),
                action=action,
                owner=owner,
                due=due,
                issue=issue,
                status=status,
            )
        )
    return Postmortem(path.name, fields, tuple(sections), tuple(actions))


def load_postmortems(root: Path) -> List[Postmortem]:
    """Loads every dated postmortem in the directory, in filename order."""
    directory = root / POSTMORTEM_DIR
    if not directory.is_dir():
        return []
    out: List[Postmortem] = []
    for path in sorted(directory.glob("*.md")):
        if _FILENAME_RE.match(path.name):
            out.append(parse_postmortem(path, path.read_text()))
    return out


def _action_problems(item: ActionItem) -> List[str]:
    """Owner / due date / issue / status problems for one action item."""
    problems: List[str] = []
    if not item.action:
        problems.append("has no action text")
    if not item.owner:
        problems.append("has no owner")
    if not _DATE_RE.match(item.due):
        problems.append(f"due date {item.due!r} is not YYYY-MM-DD")
    if not _ISSUE_RE.search(item.issue):
        problems.append(f"has no tracking issue (got {item.issue!r})")
    if not any(item.status == s or item.status.startswith(s) for s in STATUSES):
        problems.append(f"status {item.status!r} is not one of {list(STATUSES)}")
    return problems


def check_postmortem(pm: Postmortem) -> List[str]:
    """Required sections, header fields and action-item problems for one file."""
    errors: List[str] = []
    for name in REQUIRED_SECTIONS:
        if name not in pm.sections:
            errors.append(f"{pm.path}: missing required section '{name}'")
    for name in REQUIRED_FIELDS:
        value = pm.fields.get(name, "")
        if not value:
            errors.append(f"{pm.path}: missing header field '{name}'")
        elif any(p in value for p in _PLACEHOLDERS):
            errors.append(f"{pm.path}: header field '{name}' still holds the placeholder {value!r}")
    if not pm.actions:
        errors.append(f"{pm.path}: no action items (an incident always yields at least one)")
    for item in pm.actions:
        errors.extend(f"{pm.path}: action {item.number}: {msg}" for msg in _action_problems(item))
    return errors


def check_index(root: Path, postmortems: Sequence[Postmortem]) -> List[str]:
    """The index must list every postmortem, and nothing that does not exist."""
    index = root / INDEX
    if not index.is_file():
        return [f"{INDEX.as_posix()}: index is missing"]
    linked = {m.group("path") for m in _INDEX_LINK_RE.finditer(index.read_text())}
    names = {pm.path for pm in postmortems}
    errors = [f"{INDEX.as_posix()}: postmortem {n} is not indexed" for n in sorted(names - linked)]
    errors += [f"{INDEX.as_posix()}: index links unknown postmortem {n}" for n in sorted(linked - names)]
    return errors


def overdue_actions(postmortems: Sequence[Postmortem], today: dt.date) -> List[ActionItem]:
    """Every action item past its due date that is not closed."""
    return [a for pm in postmortems for a in pm.actions if a.is_overdue(today)]


def check_postmortems(root: Path, today: Optional[dt.date] = None) -> List[str]:
    """Structural check: required sections, header fields, action items, index.

    Overdue follow-up is deliberately *not* an error here. A stale action item
    is a real finding, but failing CI on it would train everyone to ignore the
    gate, and the item cannot be fixed from inside the repository. Overdue items
    are surfaced by :func:`overdue_actions`, printed by ``--report`` and
    returned by ``--strict-overdue``, which is what the monthly follow-up job
    runs.
    """
    postmortems = load_postmortems(root)
    errors: List[str] = []
    for pm in postmortems:
        errors.extend(check_postmortem(pm))
    errors.extend(check_index(root, postmortems))
    return errors


def check_overdue(root: Path, today: Optional[dt.date] = None) -> List[ActionItem]:
    """Action items past due and not yet closed, across every postmortem."""
    return overdue_actions(load_postmortems(root), today or dt.date.today())


def report(root: Path, today: Optional[dt.date] = None) -> str:
    """Markdown summary of postmortems, action items and overdue follow-up."""
    today = today or dt.date.today()
    postmortems = load_postmortems(root)
    if not postmortems:
        return "# Postmortem report\n\nNo postmortems filed.\n"
    rows = [
        "# Postmortem report",
        "",
        "| Date | Postmortem | Severity | Status | Actions | Overdue |",
        "|---|---|---|---|---|---|",
    ]
    for pm in postmortems:
        date = _FILENAME_RE.match(pm.path).group("date")
        overdue = sum(1 for a in pm.actions if a.is_overdue(today))
        rows.append(
            f"| {date} | [{pm.path}]({pm.path}) | {pm.fields.get('Severity', '')} "
            f"| {pm.fields.get('Status', '')} | {len(pm.actions)} | {overdue} |"
        )
    return "\n".join(rows) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--root", type=Path, default=Path("."), help="repository root")
    p.add_argument("--report", action="store_true", help="print the postmortem report")
    p.add_argument(
        "--strict-overdue",
        action="store_true",
        help="also fail when an action item is past due (used by the follow-up job)",
    )
    p.add_argument(
        "--today",
        type=dt.date.fromisoformat,
        default=dt.date.today(),
        help="evaluate overdue action items as of this date (YYYY-MM-DD)",
    )
    args = p.parse_args(argv)

    if args.report:
        print(report(args.root, args.today))
        return 0

    errors = check_postmortems(args.root, args.today)
    overdue = check_overdue(args.root, args.today)
    for e in errors:
        print(f"postmortem process: {e}", file=sys.stderr)
    for item in overdue:
        print(
            f"postmortem follow-up: {item.postmortem} action {item.number} is overdue "
            f"(due {item.due}, owner {item.owner or 'unassigned'}, issue {item.issue})",
            file=sys.stderr,
        )
    if errors or (overdue and args.strict_overdue):
        return 1
    print("postmortem process: structure, index and follow-up all OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
