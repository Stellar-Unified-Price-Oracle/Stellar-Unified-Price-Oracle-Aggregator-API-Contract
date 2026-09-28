"""Runbook coverage checker (#527).

An alert that pages without a runbook entry is an alert that pages into the
void, so the mapping is enforced rather than documented and forgotten. This
module parses the alert rules in ``docs/monitoring/*.yml`` and the entries in
``docs/runbook.md`` and fails CI when the two disagree.

Checked properties:

* every paging alert (``severity: critical`` or SLA class ``P0``/``P1``) has a
  runbook entry that names both the alert and the rule file it comes from;
* every entry has all required fields, non-empty;
* every entry names a known on-call role as owner and a review cadence;
* every entry is referenced by at least one real alert (no orphan entries);
* the ``Alert -> entry`` table lists exactly the paging alerts, mapped to
  entries that exist.

The alert YAML is parsed with a small block scanner rather than PyYAML: the
repository has no third-party Python dependency, and the rule files only need
``- alert:``, ``severity:`` and ``sla_class:``.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

# Fields every entry must define, in the order the template presents them.
REQUIRED_FIELDS: Tuple[str, ...] = (
    "Alerts",
    "Meaning",
    "First check",
    "Mitigation",
    "Escalation",
    "Resolution",
    "Owner",
    "Review cadence",
)

# Escalation/ownership roles, defined in docs/runbook.md.
ROLES: Tuple[str, ...] = (
    "oracle-oncall",
    "oracle-secondary",
    "security",
    "governance",
    "source-onboarding",
    "core-contracts",
    "risk",
)

# A review cadence must name one of these periods.
CADENCES: Tuple[str, ...] = ("Weekly", "Monthly", "Quarterly")

# SLA classes that page, per docs/SLA.md §5.
PAGING_SLA_CLASSES = ("P0", "P1")

# Rule files that must exist. Any other ``alerts*.yml`` in the monitoring
# directory is picked up automatically, so a new alert file cannot silently
# escape runbook coverage.
REQUIRED_RULE_FILES: Tuple[Path, ...] = (
    Path("docs/monitoring/alerts-v2.yml"),
    Path("docs/monitoring/alerts.yml"),
)

RULE_FILE_GLOB = "alerts*.yml"
RULE_FILE_EXCLUDE = "_test.yml"  # promtool unit tests, not rule files

RUNBOOK = Path("docs/runbook.md")

_ENTRY_RE = re.compile(r"^##\s+(RB-\d+)\s+—\s+(.*)$")
_FIELD_RE = re.compile(r"^\*\*(?P<name>[A-Za-z ]+):\*\*\s*(?P<value>.*)$")
_ALERT_RE = re.compile(r"^(\s*)-\s+alert:\s*(?P<name>[A-Za-z0-9_]+)\s*$")
_LABEL_RE = re.compile(r"^\s+(?P<key>severity|sla_class):\s*(?P<value>\S+)\s*$")
_TABLE_ROW_RE = re.compile(r"^\|\s*`(?P<alert>[A-Za-z0-9_]+)`\s*\|(?P<rest>.*)\|\s*$")
_TABLE_ENTRY_RE = re.compile(r"RB-\d+")
_ALERT_NAME_RE = re.compile(r"`([A-Za-z0-9_]+)`")
# Lines that end a field body: headings, horizontal rules and tables.
_STRUCTURE_RE = re.compile(r"^(#{1,6}\s|-{3,}\s*$|\|)")


@dataclass(frozen=True)
class AlertRule:
    """One ``- alert:`` block from a rule file."""

    name: str
    rule_file: str
    severity: str = ""
    sla_class: str = ""

    @property
    def pages(self) -> bool:
        """True when the alert pages a human (SLA §5 P0/P1)."""
        return self.severity == "critical" or self.sla_class in PAGING_SLA_CLASSES


@dataclass(frozen=True)
class RunbookEntry:
    """One ``## RB-xx`` section of docs/runbook.md."""

    entry_id: str
    title: str
    fields: Dict[str, str]

    def mentions_alert(self, name: str, rule_file: str) -> bool:
        """True when the Alerts field names ``name`` from ``rule_file``."""
        body = self.fields.get("Alerts", "")
        return name in _ALERT_NAME_RE.findall(body) and rule_file in body


def parse_alert_rules(text: str, rule_file: str) -> List[AlertRule]:
    """Extracts every alert block from a Prometheus rule file.

    Blocks are delimited by the ``- alert:`` keys, so one alert's labels can
    never leak into the next one.
    """
    blocks: List[List[str]] = []
    for line in text.splitlines():
        if _ALERT_RE.match(line):
            blocks.append([line])
        elif blocks:
            blocks[-1].append(line)

    alerts: List[AlertRule] = []
    for block in blocks:
        name = _ALERT_RE.match(block[0]).group("name")
        severity = sla_class = ""
        for line in block[1:]:
            m = _LABEL_RE.match(line)
            if not m:
                continue
            if m.group("key") == "severity":
                severity = m.group("value")
            else:
                sla_class = m.group("value")
        alerts.append(AlertRule(name, rule_file, severity, sla_class))
    return alerts


def parse_runbook(markdown: str) -> List[RunbookEntry]:
    """Extracts every runbook entry with its required fields.

    Field values wrap over several lines, so a field body is the run of
    non-blank lines up to the next bold field or the next entry heading. A
    blank line ends the field, which keeps a trailing entry from swallowing the
    tables that follow it.
    """
    entries: List[RunbookEntry] = []
    entry_id: Optional[str] = None
    title = ""
    fields: Dict[str, List[str]] = {}
    last = ""

    def flush() -> None:
        if entry_id is not None:
            body = {k: " ".join(x for x in v if x).strip() for k, v in fields.items()}
            entries.append(RunbookEntry(entry_id, title, body))

    for line in markdown.splitlines():
        heading = _ENTRY_RE.match(line)
        if heading:
            flush()
            entry_id, title, fields = heading.group(1), heading.group(2), {}
            continue
        if entry_id is None:
            continue
        field = _FIELD_RE.match(line)
        if field:
            fields.setdefault(field.group("name").strip(), []).append(field.group("value"))
            last = field.group("name").strip()
        elif not line.strip() or _STRUCTURE_RE.match(line):
            # A blank line or a heading/rule/table closes the current field so a
            # trailing entry cannot absorb the sections that follow it.
            last = ""
        elif last:
            fields[last].append(line.strip())
    flush()
    return entries


def parse_alert_map(markdown: str) -> List[Tuple[str, str]]:
    """Extracts the ``Alert -> entry`` table as ``(alert, entry_id)`` pairs.

    Only the rows of the ``## Alert -> entry map`` section are read, so the
    ownership table further down (which also has a backticked first column)
    cannot be mistaken for an alert mapping.
    """
    section = markdown.split("## Alert -> entry map", 1)
    if len(section) < 2:
        return []
    body = section[1].split("\n## ", 1)[0]
    rows: List[Tuple[str, str]] = []
    for line in body.splitlines():
        m = _TABLE_ROW_RE.match(line.strip())
        if m:
            entry = _TABLE_ENTRY_RE.search(m.group("rest"))
            if entry:
                rows.append((m.group("alert"), entry.group(0)))
    return rows


def rule_files(root: Path) -> List[Path]:
    """Every Prometheus rule file in ``docs/monitoring``.

    Discovered rather than hard-coded, so adding an alert file without a
    runbook entry fails this gate instead of escaping it.
    """
    directory = root / Path("docs/monitoring")
    if not directory.is_dir():
        return []
    return sorted(
        p for p in directory.glob(RULE_FILE_GLOB) if not p.name.endswith(RULE_FILE_EXCLUDE)
    )


def load_alerts(root: Path) -> List[AlertRule]:
    """Loads every rule file, skipping any that is not present."""
    alerts: List[AlertRule] = []
    for path in rule_files(root):
        alerts.extend(parse_alert_rules(path.read_text(), path.name))
    return alerts


def check_runbook(root: Path) -> List[str]:
    """Returns human-readable problems with the runbook. Empty means healthy."""
    markdown = (root / RUNBOOK).read_text()
    entries = parse_runbook(markdown)
    alerts = load_alerts(root)
    paging = [a for a in alerts if a.pages]
    by_id = {e.entry_id: e for e in entries}
    errors: List[str] = []

    # 0. The rule files the runbook claims to cover must actually exist.
    for rel in REQUIRED_RULE_FILES:
        if not (root / rel).is_file():
            errors.append(f"rule file {rel.as_posix()} is missing")

    # 1. Coverage: every paging alert has a complete entry naming it.
    for alert in paging:
        covering = [e for e in entries if e.mentions_alert(alert.name, alert.rule_file)]
        if not covering:
            errors.append(f"paging alert {alert.name} ({alert.rule_file}) has no runbook entry")
            continue
        for entry in covering:
            errors.extend(f"{entry.entry_id}: {msg}" for msg in _field_problems(entry))

    # 2. No orphan entries: every entry is referenced by a real alert.
    known = {a.name for a in alerts}
    for entry in entries:
        if not any(n in known for n in _ALERT_NAME_RE.findall(entry.fields.get("Alerts", ""))):
            errors.append(f"{entry.entry_id}: entry references no known alert (orphan)")

    # 3. The alert -> entry table agrees with the paging set.
    table = dict(parse_alert_map(markdown))
    for alert in paging:
        entry_id = table.get(alert.name)
        if entry_id is None:
            errors.append(f"paging alert {alert.name} missing from the alert -> entry table")
        elif entry_id not in by_id:
            errors.append(f"alert -> entry table maps {alert.name} to unknown {entry_id}")
    for alert_name, entry_id in table.items():
        if entry_id not in by_id:
            continue
        if alert_name not in known:
            errors.append(f"alert -> entry table maps unknown alert {alert_name}")
        elif not any(a.name == alert_name and a.pages for a in alerts):
            errors.append(f"alert -> entry table maps non-paging alert {alert_name} ({entry_id})")
    return errors


def _field_problems(entry: RunbookEntry) -> List[str]:
    """Required-field, owner and cadence problems for a single entry."""
    problems: List[str] = []
    for name in REQUIRED_FIELDS:
        if not entry.fields.get(name, "").strip():
            problems.append(f"required field '{name}' is missing or empty")
    owner = entry.fields.get("Owner", "")
    if owner and not any(f"`{role}`" in owner for role in ROLES):
        problems.append(f"owner '{owner}' is not a known role {list(ROLES)}")
    cadence = entry.fields.get("Review cadence", "")
    if cadence and not any(c.lower() in cadence.lower() for c in CADENCES):
        problems.append(f"review cadence '{cadence}' names no known period {list(CADENCES)}")
    return problems


def alert_routing_table(root: Path) -> Dict[str, Dict[str, str]]:
    """Builds the machine-readable ``{alert: {entry, url, owner, ...}}`` map.

    This is what the alert payload links from: a router, or an Alertmanager
    ``runbook_url`` templating step, reads it instead of a responder having to
    remember which anchor belongs to which alert.
    """
    entries = {e.entry_id: e for e in parse_runbook((root / RUNBOOK).read_text())}
    routing: Dict[str, Dict[str, str]] = {}
    for alert_name, entry_id in parse_alert_map((root / RUNBOOK).read_text()):
        entry = entries.get(entry_id)
        if entry is None:
            continue
        routing[alert_name] = {
            "entry": entry_id,
            "title": entry.title,
            "url": f"{RUNBOOK.as_posix()}#{entry_id.lower()}",
            "owner": entry.fields.get("Owner", ""),
            "review_cadence": entry.fields.get("Review cadence", ""),
        }
    return routing


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--root", type=Path, default=Path("."), help="repository root")
    p.add_argument(
        "--routing",
        action="store_true",
        help="print the machine-readable alert -> runbook routing table as JSON",
    )
    args = p.parse_args(argv)

    if args.routing:
        print(json.dumps(alert_routing_table(args.root), indent=2, sort_keys=True))
        return 0

    errors = check_runbook(args.root)
    for e in errors:
        print(f"runbook coverage: {e}", file=sys.stderr)
    if errors:
        return 1
    print("runbook coverage: every paging alert has a complete entry")
    return 0


if __name__ == "__main__":
    sys.exit(main())
