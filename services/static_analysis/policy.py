"""Baseline + allowlist policy for the static-analysis gate (#500).

Advisories, unsafe-code creep and SAST findings are silent regressions unless
something fails on them. This module encodes the two escape hatches that make
such a gate survivable — and time-boxes both:

* :class:`AllowlistEntry` — an acknowledged finding that CI will not fail on.
  Every entry must name an **owner**, an **expiry date**, a reason, and the
  **paths** it is scoped to. An entry that covers the whole tree (``**``) is
  rejected: a blanket suppression is exactly the "false positives blanket-
  suppressed" failure mode the issue rules out. Expired entries are a hard
  error, so an allowlist cannot rot into a permanent mute.
* :class:`UnsafeBaseline` — the accepted per-(rule, file) count of `unsafe`
  constructs. The file carries a ``approved_sha256`` over its own canonical
  contents, so editing the counts without deliberately re-approving them fails
  the gate. The baseline can therefore only grow through an explicit,
  reviewable :func:`rebaseline` that records who approved it and when.

See ``docs/static-analysis.md`` for the triage and re-baseline process.
"""
from __future__ import annotations

import fnmatch
import hashlib
import json
from dataclasses import dataclass
from datetime import date, datetime
from pathlib import Path
from typing import Iterable, List, Mapping, Optional, Sequence, Tuple

#: Longest an allowlist entry may live, in days. Entries must be re-reviewed
#: at least this often, so an acknowledged risk cannot become permanent.
MAX_ALLOWLIST_DAYS = 180

#: Kinds of finding the allowlist may cover. `secret` is the secret-scanning
#: allowlist (#502), which reuses the same owner/expiry/scope rules.
ALLOWLIST_KINDS = ("advisory", "unsafe", "sast", "secret")


def _parse_date(value: str) -> date:
    return datetime.strptime(value, "%Y-%m-%d").date()


@dataclass(frozen=True)
class AllowlistEntry:
    """One acknowledged finding that CI will not fail on."""

    id: str
    kind: str
    owner: str
    expires: str
    reason: str
    paths: Tuple[str, ...] = ()

    def matches(self, finding_id: str, path: str) -> bool:
        """True when this entry covers `finding_id` at `path`.

        The path scope is mandatory: an entry with no `paths` matches nothing,
        and a bare ``**`` is rejected by :meth:`errors`.
        """
        if self.id != finding_id:
            return False
        return any(fnmatch.fnmatch(path, pattern) for pattern in self.paths)

    def errors(self, today: Optional[date] = None) -> List[str]:
        """Reasons this entry is invalid (empty == usable)."""
        today = today or date.today()
        errors: List[str] = []
        if self.kind not in ALLOWLIST_KINDS:
            errors.append(f"{self.id}: unknown kind '{self.kind}'")
        if not self.owner.strip():
            errors.append(f"{self.id}: no owner")
        if not self.reason.strip():
            errors.append(f"{self.id}: no reason")
        if not self.paths:
            errors.append(f"{self.id}: no path scope")
        elif any(p in ("**", "*") for p in self.paths):
            errors.append(
                f"{self.id}: blanket path scope {self.paths} — a false positive must "
                "be tracked at its location, not suppressed repo-wide"
            )
        try:
            expiry = _parse_date(self.expires)
        except (TypeError, ValueError):
            errors.append(f"{self.id}: expiry must be an ISO date (YYYY-MM-DD)")
            return errors
        if expiry < today:
            errors.append(
                f"{self.id}: expired on {self.expires} — re-review or remove the entry"
            )
        elif (expiry - today).days > MAX_ALLOWLIST_DAYS:
            errors.append(
                f"{self.id}: expiry {self.expires} is more than {MAX_ALLOWLIST_DAYS} "
                "days out — an allowlist entry must be time-boxed"
            )
        return errors

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "kind": self.kind,
            "owner": self.owner,
            "expires": self.expires,
            "reason": self.reason,
            "paths": list(self.paths),
        }


def load_allowlist(path: Path, today: Optional[date] = None) -> Tuple[List[AllowlistEntry], List[str]]:
    """Reads the allowlist file and returns `(entries, errors)`.

    A malformed file is an error, never an empty allowlist: silently ignoring
    a broken allowlist would turn every acknowledged finding into a build break
    (noisy) or, worse, into an unnoticed suppression.
    """
    errors: List[str] = []
    if not path.exists():
        return [], [f"allowlist file {path} is missing"]
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return [], [f"allowlist file {path} is not valid JSON: {exc}"]
    entries: List[AllowlistEntry] = []
    for item in raw.get("entries", []):
        try:
            entry = AllowlistEntry(
                id=str(item["id"]),
                kind=str(item["kind"]),
                owner=str(item.get("owner", "")),
                expires=str(item.get("expires", "")),
                reason=str(item.get("reason", "")),
                paths=tuple(str(p) for p in item.get("paths", ())),
            )
        except KeyError as exc:
            errors.append(f"allowlist entry missing field {exc}")
            continue
        entries.append(entry)
    for entry in entries:
        errors.extend(entry.errors(today))
    return entries, errors


def filter_allowlisted(
    findings: Sequence["Finding"], allowlist: Iterable[AllowlistEntry]
) -> Tuple[List["Finding"], List["Finding"]]:
    """Splits `findings` into `(new, allowlisted)`."""
    entries = list(allowlist)
    new: List["Finding"] = []
    allowed: List["Finding"] = []
    for finding in findings:
        if any(e.matches(finding.id, finding.path) for e in entries):
            allowed.append(finding)
        else:
            new.append(finding)
    return new, allowed


# ---------------------------------------------------------------------------
# Unsafe-code baseline
# ---------------------------------------------------------------------------


def baseline_digest(counts: Mapping[str, int]) -> str:
    """SHA-256 over the canonical serialization of the per-(rule, file) counts.

    The digest is what makes baseline growth a *reviewed* act: editing the
    counts changes the digest, and the committed file must carry the new digest
    or the gate fails.
    """
    canonical = json.dumps(dict(sorted(counts.items())), separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


@dataclass(frozen=True)
class UnsafeBaseline:
    """Accepted `unsafe`-code counts, with the digest that approves them."""

    counts: Mapping[str, int]
    approved_sha256: str
    approved_by: str = ""
    approved_on: str = ""

    def digest(self) -> str:
        return baseline_digest(self.counts)

    def errors(self) -> List[str]:
        errors: List[str] = []
        if not self.approved_sha256:
            errors.append("baseline has no approved_sha256 — re-approve it deliberately")
        elif self.approved_sha256 != self.digest():
            errors.append(
                "baseline counts were edited without re-approval: approved_sha256 "
                f"{self.approved_sha256[:12]}… does not match the digest of the "
                f"committed counts ({self.digest()[:12]}…) — run "
                "`python -m services.static_analysis.policy rebaseline` and commit "
                "the result for review"
            )
        for key, n in sorted(self.counts.items()):
            if n < 0:
                errors.append(f"baseline entry {key} has a negative count")
        return errors

    def to_dict(self) -> dict:
        return {
            "approved_sha256": self.approved_sha256,
            "approved_by": self.approved_by,
            "approved_on": self.approved_on,
            "counts": dict(sorted(self.counts.items())),
        }


def load_baseline(path: Path) -> Tuple[Optional[UnsafeBaseline], List[str]]:
    if not path.exists():
        return None, [f"baseline file {path} is missing"]
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return None, [f"baseline file {path} is not valid JSON: {exc}"]
    counts = {str(k): int(v) for k, v in raw.get("counts", {}).items()}
    baseline = UnsafeBaseline(
        counts=counts,
        approved_sha256=str(raw.get("approved_sha256", "")),
        approved_by=str(raw.get("approved_by", "")),
        approved_on=str(raw.get("approved_on", "")),
    )
    return baseline, baseline.errors()


def rebaseline(counts: Mapping[str, int], reviewer: str, today: Optional[date] = None) -> UnsafeBaseline:
    """Builds a *newly approved* baseline, recording who approved it and when.

    This is the only way the baseline grows. The result is a normal file
    change: it shows up in review as `approved_by` / `approved_on` plus the new
    digest, so a silent baseline bump is not possible.
    """
    today = today or date.today()
    if not reviewer.strip():
        raise ValueError("re-baselining requires a named reviewer")
    return UnsafeBaseline(
        counts=dict(counts),
        approved_sha256=baseline_digest(counts),
        approved_by=reviewer,
        approved_on=today.isoformat(),
    )


def check_unsafe(
    counts: Mapping[str, int], baseline: Optional[UnsafeBaseline]
) -> Tuple[List[str], List[str]]:
    """Compares observed `unsafe` counts with the baseline.

    Returns `(new_findings, errors)`:

    * a file whose count exceeds the baseline is *unsafe-code creep* and fails;
    * a baseline entry that no longer matches reality is *drift in the other
      direction* and also fails, because a baseline that is never tightened
      hides regressions in everything it covers.
    """
    errors: List[str] = []
    if baseline is None:
        return [], ["no unsafe baseline to compare against"]
    errors.extend(baseline.errors())
    new: List[str] = []
    for key in sorted(set(counts) | set(baseline.counts)):
        observed = counts.get(key, 0)
        accepted = baseline.counts.get(key, 0)
        if observed > accepted:
            new.append(f"{key}: {observed} unsafe construct(s), baseline allows {accepted}")
        elif observed < accepted:
            errors.append(
                f"{key}: baseline allows {accepted} but only {observed} found — "
                "tighten the baseline (do not leave slack)"
            )
    return new, errors


# ---------------------------------------------------------------------------
# Findings and dependency advisories
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Finding:
    """One static-analysis result, as consumed by the allowlist."""

    id: str
    kind: str  # "unsafe" | "sast" | "advisory"
    path: str
    line: int
    message: str

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "kind": self.kind,
            "path": self.path,
            "line": self.line,
            "message": self.message,
        }


@dataclass(frozen=True)
class Advisory:
    """A RustSec advisory for a crate in the dependency graph."""

    id: str
    crate: str
    title: str = ""
    patched_versions: str = ""

    @property
    def has_fix(self) -> bool:
        """True when the advisory names at least one patched version."""
        return bool(self.patched_versions.strip()) and self.patched_versions.strip() not in (
            "<none>",
            "unaffected",
        )

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "crate": self.crate,
            "title": self.title,
            "patched_versions": self.patched_versions,
            "has_fix": self.has_fix,
        }


def parse_cargo_audit(raw: Mapping[str, object]) -> List[Advisory]:
    """Parses `cargo audit --json` output into advisories.

    A deliberately introduced vulnerable dependency therefore reaches the gate
    as a normal, non-allowlisted advisory and fails CI — that is the
    regression test for this issue.
    """
    advisories: List[Advisory] = []
    vulns = raw.get("vulnerabilities", {}) if isinstance(raw, Mapping) else {}
    for item in (vulns or {}).get("list", []):  # type: ignore[union-attr]
        patched = item.get("versions", {}).get("patched", []) or []
        advisories.append(
            Advisory(
                id=str(item.get("id", "")),
                crate=str(item.get("package", "")),
                title=str(item.get("title", "")),
                patched_versions=", ".join(str(p) for p in patched),
            )
        )
    return sorted(advisories, key=lambda a: a.id)


def check_advisories(
    advisories: Sequence[Advisory], allowlist: Iterable[AllowlistEntry]
) -> Tuple[List[Advisory], List[Advisory], List[str]]:
    """Returns `(new, allowlisted, notes)`.

    An advisory that is *not* allowlisted fails the gate. An advisory with no
    patched version can only be acknowledged with a time-boxed, owned allowlist
    entry (enforced by :class:`AllowlistEntry`), so "no fix available" is never
    a silent skip.
    """
    entries = [e for e in allowlist if e.kind == "advisory"]
    new: List[Advisory] = []
    allowed: List[Advisory] = []
    notes: List[str] = []
    for advisory in advisories:
        if any(e.matches(advisory.id, f"crates/{advisory.crate}") for e in entries):
            allowed.append(advisory)
            continue
        new.append(advisory)
        if not advisory.has_fix:
            notes.append(
                f"{advisory.id} ({advisory.crate}) has no patched version: add an "
                "owned, time-boxed allowlist entry or pin a fixed dependency"
            )
    return new, allowed, notes


def exceedances(findings: Sequence["Finding"], baseline: Optional["UnsafeBaseline"]) -> List["Finding"]:
    """Returns the findings that are *beyond* the baseline.

    The baseline is per ``rule:file``, so "one more `.unwrap()` in
    ``price_bounds.rs``" is a locatable finding rather than an anonymous count
    that grew. Order is the scan order, so the first N occurrences of a rule in
    a file are the accepted ones and everything after them is new.
    """
    if baseline is None:
        return list(findings)
    remaining = {k: v for k, v in baseline.counts.items() if v > 0}
    new: List["Finding"] = []
    for finding in findings:
        key = f"{finding.id}:{finding.path}"
        if remaining.get(key, 0) > 0:
            remaining[key] -= 1
        else:
            new.append(finding)
    return new


def effective_baseline(
    baseline: Optional["UnsafeBaseline"], allowlist: Sequence[AllowlistEntry]
) -> Optional["UnsafeBaseline"]:
    """The baseline with allowlisted ``rule:file`` pairs removed.

    A pair covered by an owned, unexpired allowlist entry is out of scope for
    the count comparison in both directions: it must not read as "unsafe
    addition beyond baseline" *and* it must not read as baseline drift. Call
    :meth:`UnsafeBaseline.errors` on the original baseline first, so the
    digest check still covers the committed file.
    """
    if baseline is None:
        return None
    kept = {}
    for key, n in baseline.counts.items():
        rule_id, _, path = key.partition(":")
        if any(e.matches(rule_id, path) for e in allowlist):
            continue
        kept[key] = n
    return UnsafeBaseline(
        counts=kept,
        approved_sha256=baseline.approved_sha256,
        approved_by=baseline.approved_by,
        approved_on=baseline.approved_on,
    )
