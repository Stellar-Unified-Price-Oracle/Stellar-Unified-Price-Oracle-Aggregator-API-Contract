"""Secret scanning for the working tree, staged files and full history (#502).

A single committed key can compromise deployment or bridge integrations, and
git history makes removal painful. Prevention at commit time is far cheaper
than remediation, so this scanner runs three ways:

* **pre-commit** (`.husky/pre-commit`) over the *staged* files, so a leak is
  caught before it is ever written;
* **CI** over the whole tree — source, config, CI definitions, fixtures and
  docs, not only source — failing on any new detection;
* **history** over every blob ever committed, reporting material that is
  already in the log and therefore needs rotation rather than a rebase.

False positives are handled by an allowlist with the same discipline as the
static-analysis one (#500): an entry is scoped to a path, carries an owner and
an expiry, and cannot blanket-suppress the tree. Independently of the
allowlist, obviously-fake values (`CHANGEME`, `example`, `<your-token>`,
`${VAR}`, `os.environ[...]`, …) are not reported at all, so legitimate test
material does not need an entry in the first place.
"""
from __future__ import annotations

import argparse
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

from services.static_analysis.policy import AllowlistEntry, load_allowlist

#: Directories never scanned (build output, dependencies, VCS internals).
SKIP_DIRS = (".git", "target", "node_modules", "vendor", "__pycache__", ".pytest_cache")

#: Suffixes treated as binary and skipped.
BINARY_SUFFIXES = (
    ".wasm", ".png", ".jpg", ".jpeg", ".gif", ".pdf", ".zip", ".gz", ".ico", ".so",
)


@dataclass(frozen=True)
class SecretRule:
    """One secret pattern."""

    id: str
    pattern: re.Pattern
    description: str


SECRET_RULES: Tuple[SecretRule, ...] = (
    SecretRule(
        "SECRET-AWS-ACCESS-KEY-ID",
        re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
        "AWS access key id",
    ),
    SecretRule(
        "SECRET-GITHUB-TOKEN",
        re.compile(r"\bgh[pousr]_[A-Za-z0-9]{30,}\b"),
        "GitHub personal access / OAuth / app token",
    ),
    SecretRule(
        "SECRET-GITHUB-FINE-GRAINED-PAT",
        re.compile(r"\bgithub_pat_[A-Za-z0-9_]{40,}\b"),
        "GitHub fine-grained personal access token",
    ),
    SecretRule(
        "SECRET-SLACK-TOKEN",
        re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b"),
        "Slack token",
    ),
    SecretRule(
        "SECRET-PRIVATE-KEY-BLOCK",
        re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY"),
        "PEM private key",
    ),
    SecretRule(
        "SECRET-SOROBAN-SECRET-SEED",
        re.compile(r"\bS[A-Z2-7]{55}\b"),
        "Stellar secret seed (account key)",
    ),
    SecretRule(
        "SECRET-GENERIC-ASSIGNMENT",
        re.compile(
            r"(?i)\b(api[_-]?key|secret[_-]?key|secret|password|passwd|token|private[_-]?key)"
            r"\s*[:=]\s*[\"'][^\"'\s]{12,}[\"']"
        ),
        "credential assigned a literal value",
    ),
)

#: Substrings that mark a match as a placeholder, example or environment
#: lookup rather than a live credential. Checked against the matched text.
PLACEHOLDER_MARKERS: Tuple[str, ...] = (
    "example",
    "changeme",
    "change-me",
    "change_me",
    "your",
    "yours",
    "dummy",
    "placeholder",
    "redacted",
    "notarealkey",
    "fake",
    "sample",
    "test",
    "todo",
    "replace",
    "insert",
    "xxxx",
    "<",
    "${",
    "$(",
    "{{",
    "%(",
    "os.environ",
    "process.env",
    "env::",
    "secretref",
    "vault:",
    "arn:aws",
)


def _is_placeholder(matched: str, line: str) -> bool:
    lowered = (matched + " " + line).lower()
    return any(marker in lowered for marker in PLACEHOLDER_MARKERS)


def scan_text(text: str, path: str, rules: Sequence[SecretRule] = SECRET_RULES) -> List["SecretFinding"]:
    """Scans one blob of text, skipping placeholders and comments-free noise."""
    findings: List["SecretFinding"] = []
    for lineno, line in enumerate(text.splitlines(), start=1):
        if len(line) > 4096:
            continue
        for rule in rules:
            match = rule.pattern.search(line)
            if not match:
                continue
            if _is_placeholder(match.group(0), line):
                continue
            findings.append(
                SecretFinding(
                    id=rule.id,
                    path=path,
                    line=lineno,
                    description=rule.description,
                    redacted=_redact(match.group(0)),
                )
            )
    return findings


def _redact(matched: str) -> str:
    """Keeps enough of the match to recognise it, never enough to use it."""
    if len(matched) <= 8:
        return "*" * len(matched)
    return matched[:4] + "*" * (len(matched) - 8) + matched[-4:]


@dataclass(frozen=True)
class SecretFinding:
    """One detected secret. The matched value is redacted by construction."""

    id: str
    path: str
    line: int
    description: str
    redacted: str
    commit: str = ""

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "path": self.path,
            "line": self.line,
            "description": self.description,
            "redacted": self.redacted,
            "commit": self.commit,
        }


def _iter_files(roots: Sequence[Path]) -> Iterable[Path]:
    for root in roots:
        if root.is_file():
            yield root
            continue
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            if set(path.parts) & set(SKIP_DIRS):
                continue
            if path.suffix.lower() in BINARY_SUFFIXES:
                continue
            yield path


def scan_tree(roots: Sequence[Path], rules: Sequence[SecretRule] = SECRET_RULES) -> List[SecretFinding]:
    """Scans every text file under `roots` (source, config, CI, fixtures)."""
    findings: List[SecretFinding] = []
    for path in _iter_files(roots):
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        findings.extend(scan_text(text, str(path), rules))
    return findings


def scan_staged(repo: Path = Path(".")) -> List[SecretFinding]:
    """Scans only the files about to be committed."""
    proc = subprocess.run(
        ["git", "diff", "--cached", "--name-only", "--diff-filter=ACM"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=True,
    )
    names = [line.strip() for line in proc.stdout.splitlines() if line.strip()]
    findings: List[SecretFinding] = []
    for name in names:
        path = repo / name
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        findings.extend(scan_text(text, name))
    return findings


def _git(args: Sequence[str], repo: Path) -> str:
    proc = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=False
    )
    return proc.stdout


def history_blobs(repo: Path = Path(".")) -> Dict[str, str]:
    """Maps blob sha -> path for every blob ever committed on any ref."""
    objects = _git(["rev-list", "--objects", "--all"], repo)
    blobs: Dict[str, str] = {}
    for line in objects.splitlines():
        parts = line.split(" ", 1)
        if len(parts) != 2:
            continue
        sha, name = parts
        if name.endswith(BINARY_SUFFIXES):
            continue
        blobs[sha] = name
    return blobs


def _blob_contents(repo: Path, shas: Sequence[str]) -> Dict[str, str]:
    """Streams `git cat-file --batch` over `shas` (one process, not N).

    Binary blobs are decoded leniently rather than aborting the scan: a
    non-UTF-8 blob is not a place a credential can hide in a form we can read,
    and one bad object must not stop the history report.
    """
    if not shas:
        return {}
    proc = subprocess.run(
        ["git", "cat-file", "--batch"],
        cwd=repo,
        input=("\n".join(shas) + "\n").encode("utf-8"),
        capture_output=True,
        check=False,
    )
    contents: Dict[str, str] = {}
    current: Optional[str] = None
    remaining = 0
    for raw_line in proc.stdout.split(b"\n"):
        if current is None:
            parts = raw_line.split(b" ", 2)
            if len(parts) >= 2 and parts[1] == b"blob":
                current = parts[0].decode("utf-8", "replace")
                size = int(parts[2]) if len(parts) > 2 and parts[2].isdigit() else 0
                remaining = size
                contents[current] = ""
            continue
        if remaining <= 0:
            current = None
            continue
        text = raw_line.decode("utf-8", "replace")
        contents[current] += text + "\n"
        remaining -= len(raw_line) + 1
    return contents


def _introducing_commit(repo: Path, blob: str) -> str:
    """The first commit that introduced `blob` (short sha, or '')."""
    out = _git(["log", "--all", "--find-object=" + blob, "--format=%h", "--reverse"], repo)
    return out.splitlines()[0] if out.strip() else ""


def scan_history(
    repo: Path = Path("."),
    rules: Sequence[SecretRule] = SECRET_RULES,
    limit: Optional[int] = None,
) -> List[SecretFinding]:
    """Scans every blob ever committed, and reports findings with their commit.

    Already-committed material is reported, not "fixed": the remediation for a
    leak in history is rotation (see `docs/secret-scanning.md` §5), which a
    rebase alone cannot achieve.
    """
    blobs = history_blobs(repo)
    shas = list(blobs)
    if limit:
        shas = shas[:limit]
    contents = _blob_contents(repo, shas)
    findings: List[SecretFinding] = []
    for sha, text in contents.items():
        path = blobs.get(sha, sha)
        for finding in scan_text(text, path, rules):
            findings.append(
                SecretFinding(
                    id=finding.id,
                    path=finding.path,
                    line=finding.line,
                    description=finding.description,
                    redacted=finding.redacted,
                    commit=_introducing_commit(repo, sha),
                )
            )
    return findings


def filter_findings(
    findings: Sequence[SecretFinding], allowlist: Sequence[AllowlistEntry]
) -> Tuple[List[SecretFinding], List[SecretFinding]]:
    """Splits into `(new, allowlisted)` using the shared path-scoped allowlist."""
    new: List[SecretFinding] = []
    allowed: List[SecretFinding] = []
    for finding in findings:
        if any(e.matches(finding.id, finding.path) for e in allowlist):
            allowed.append(finding)
        else:
            new.append(finding)
    return new, allowed


def render_report(
    findings: Sequence[SecretFinding],
    allowlisted: Sequence[SecretFinding],
    history: Sequence[SecretFinding] = (),
) -> str:
    """Human-readable report attached to every CI run."""
    lines: List[str] = ["# Secret scan report", ""]
    lines.append(f"- working-tree findings: {len(findings)}")
    lines.append(f"- allowlisted (owned, unexpired): {len(allowlisted)}")
    lines.append(f"- historical findings (rotation candidates): {len(history)}")
    lines.append("")
    for title, items in (
        ("Working tree", findings),
        ("Allowlisted", allowlisted),
        ("History", history),
    ):
        lines.append(f"## {title}")
        if not items:
            lines.append("- none")
        for f in items:
            where = f"{f.path}:{f.line}"
            if f.commit:
                where = f"{where} (introduced in {f.commit})"
            lines.append(f"- `{f.id}` {where} — {f.description} — `{f.redacted}`")
        lines.append("")
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Secret scanner (#502)")
    parser.add_argument("paths", nargs="*", default=["."], help="paths to scan")
    parser.add_argument(
        "--allowlist",
        default=str(Path("config") / "secret-scan-allowlist.json"),
    )
    parser.add_argument("--staged", action="store_true", help="scan staged files only")
    parser.add_argument("--history", action="store_true", help="also scan git history")
    parser.add_argument("--history-limit", type=int, default=None)
    parser.add_argument("--report", default=None, help="write a markdown report here")
    args = parser.parse_args(argv)

    if args.staged:
        raw = scan_staged(Path("."))
    else:
        raw = scan_tree([Path(p) for p in (args.paths or ["."])])

    allowlist, allowlist_errors = load_allowlist(Path(args.allowlist))
    new, allowed = filter_findings(raw, allowlist)

    history_findings: List[SecretFinding] = []
    if args.history:
        history_findings = scan_history(Path("."), limit=args.history_limit)
        history_new, history_allowed = filter_findings(history_findings, allowlist)
        history_findings = history_new
        allowed.extend(history_allowed)

    if args.report:
        report_path = Path(args.report)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(render_report(new, allowed, history_findings), encoding="utf-8")
        print(f"report written to {report_path}")

    for error in allowlist_errors:
        print(f"::error::{error}")
    for finding in new:
        print(f"::error::secret detected: {finding.id} at {finding.path}:{finding.line}")
    for finding in history_findings:
        print(
            f"::error::secret in history: {finding.id} at {finding.path}:{finding.line} "
            f"(commit {finding.commit or 'unknown'}) — rotate, do not just rebase"
        )
    print(
        f"{len(new)} new secret(s), {len(allowed)} allowlisted, "
        f"{len(history_findings)} in history"
    )
    return 1 if (new or allowlist_errors or history_findings) else 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
