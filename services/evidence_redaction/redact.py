"""Redaction filter for logs and incident artifacts (#687).

An incident is when operators are most likely to paste raw CLI output, RPC
errors and environment dumps into a ticket. That output is also where a
Stellar secret seed, an ``Authorization`` header or an ``--admin-key`` value is
most likely to be sitting. Once it is in a log store or an artifact bucket, a
rotation-only remediation is the best that is left.

So the artifact pipeline runs through one filter, and the filter **cannot** be
run in pass-through mode:

* ``Redactor(enabled=False)`` raises instead of returning the input, and the
  CLI exits non-zero before emitting anything. A pipeline that cannot redact
  fails closed rather than publishing raw evidence;
* matching is structural — prefixes such as ``Authorization:`` or
  ``--admin-key`` are kept and only the credential is replaced, so the artifact
  stays useful for diagnosis;
* the report contains rule ids, line numbers and counts, never matched values,
  so the redaction report cannot become the leak;
* public data is deliberately not redacted. Stellar account addresses (``G…``)
  and contract ids (``C…``) are public chain data and are the substance of an
  incident timeline; only secret seeds (``S…``) are credentials.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Dict, Iterable, List, Optional, Sequence, Tuple

#: Fixed marker vocabulary. A marker names *what* was removed, never the value.
MARKER = "[REDACTED:{rule}]"


class RedactionDisabled(RuntimeError):
    """Raised when redaction is disabled. Never returns unredacted text."""


@dataclass(frozen=True)
class RedactionRule:
    """One credential shape and how to replace it."""

    id: str
    pattern: re.Pattern
    replace: Callable[[re.Match], str]
    description: str


@dataclass(frozen=True)
class Finding:
    """A redaction, located but not quoted."""

    rule_id: str
    line: int
    count: int

    def to_dict(self) -> dict:
        return {"rule": self.rule_id, "line": self.line, "count": self.count}


@dataclass(frozen=True)
class RedactionResult:
    """Redacted text plus the bounded record of what was removed."""

    text: str
    findings: Tuple[Finding, ...]
    counts: Dict[str, int]

    @property
    def redacted(self) -> int:
        return sum(self.counts.values())

    def to_dict(self) -> dict:
        return {"redacted": self.redacted, "by_rule": dict(sorted(self.counts.items()))}


def _marker(rule_id: str) -> str:
    return MARKER.format(rule=rule_id)


def _auth_header(match: re.Match) -> str:
    # group(2) carries the scheme and its separator, when there is one.
    return f"{match.group(1)}{match.group(2)}{_marker('auth-header')}"


def _keep_prefix(group: int, rule_id: str) -> Callable[[re.Match], str]:
    return lambda match: match.group(group) + _marker(rule_id)


#: Order matters: structural rules run first so the surrounding label is kept
#: (``Authorization: Bearer [REDACTED:auth-header]``), and the bare-seed rule
#: runs last as the catch-all for a value with no label at all. A value is
#: attributed to the first rule that matches it, so the counts never double.
RULES: Tuple[RedactionRule, ...] = (
    RedactionRule(
        "pem-private-key",
        re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----"),
        lambda match: _marker("pem-private-key"),
        "PEM private key block",
    ),
    RedactionRule(
        "auth-header",
        re.compile(
            r"(?i)(\bauthorization\s*[:=]\s*)((?:(?:bearer|basic|token|apikey)\s+)?)"
            # The value may be neither an existing marker nor a bare scheme
            # word: otherwise redaction would not be idempotent, and re-running
            # the filter (or --check on an artifact) would mangle the marker.
            r"(?!\[REDACTED:)(?!(?:bearer|basic|token|apikey)\b)([^\s\"',;]+)"
        ),
        _auth_header,
        "Authorization header value",
    ),
    RedactionRule(
        "key-flag",
        # --admin-key, --source-key, --signer-secret, --api-token, ...
        re.compile(
            r"(?i)(--[a-z0-9-]*(?:key|secret|token|password|passwd)[a-z0-9-]*[=\s]+)"
            r"(?!\[REDACTED:)(\S+)"
        ),
        _keep_prefix(1, "key-flag"),
        "credential passed as a CLI flag",
    ),
    RedactionRule(
        "env-assignment",
        re.compile(
            # KEY is deliberately broad: ORACLE_SOURCE_KEY holds an identity
            # name, not a seed, but a credential nevertheless.
            r"(?i)\b([A-Z0-9_]*(?:SECRET|TOKEN|PASSWORD|PASSWD|KEY|CREDENTIAL)[A-Z0-9_]*\s*=\s*)"
            r"(?!\[REDACTED:)(\S+)"
        ),
        _keep_prefix(1, "env-assignment"),
        "credential assigned to an environment variable",
    ),
    RedactionRule(
        "json-field",
        re.compile(
            r"(?i)(\"[a-z0-9_-]*(?:secret|seed|token|password|key|authorization|credential)"
            r"[a-z0-9_-]*\"\s*:\s*\")(?!\[REDACTED:)([^\"]*)(\")"
        ),
        lambda match: match.group(1) + _marker("json-field") + match.group(3),
        "credential in a JSON field",
    ),
    RedactionRule(
        "url-userinfo",
        re.compile(r"(?i)(\b[a-z][a-z0-9+.-]*://[^/\s:@]+:)(?!\[REDACTED:)([^@\s/]+)(@)"),
        lambda match: match.group(1) + _marker("url-userinfo") + match.group(3),
        "password embedded in a URL",
    ),
    RedactionRule(
        "jwt",
        re.compile(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b"),
        lambda match: _marker("jwt"),
        "JSON Web Token",
    ),
    RedactionRule(
        "stellar-seed",
        # A Stellar secret seed is exactly 56 base32 characters starting with S.
        re.compile(r"\bS[A-Z2-7]{55}\b"),
        lambda match: _marker("stellar-seed"),
        "Stellar secret seed (account key)",
    ),
)

RULES_BY_ID = {rule.id: rule for rule in RULES}


@dataclass
class Redactor:
    """Applies every rule, or refuses to run."""

    enabled: bool = True
    rules: Sequence[RedactionRule] = field(default=RULES)

    def __post_init__(self) -> None:
        # Fail closed at construction: there is no supported pass-through.
        if not self.enabled:
            raise RedactionDisabled(
                "redaction cannot be disabled: incident artifacts are never emitted raw"
            )

    def redact(self, text: str) -> RedactionResult:
        if not self.enabled:  # pragma: no cover - guarded in __post_init__
            raise RedactionDisabled("redaction is disabled")
        if not isinstance(text, str):
            raise TypeError("redact expects text")

        findings: List[Finding] = []
        counts: Dict[str, int] = {}
        for rule in self.rules:
            matches = list(rule.pattern.finditer(text))
            if not matches:
                continue
            counts[rule.id] = counts.get(rule.id, 0) + len(matches)
            for match in matches:
                findings.append(
                    Finding(
                        rule_id=rule.id,
                        line=text.count("\n", 0, match.start()) + 1,
                        count=1,
                    )
                )
            text = rule.pattern.sub(rule.replace, text)
        findings.sort(key=lambda f: (f.line, f.rule_id))
        return RedactionResult(text=text, findings=tuple(findings), counts=counts)


def redact_text(text: str, enabled: bool = True) -> RedactionResult:
    """Convenience wrapper. Raises `RedactionDisabled` when disabled."""
    return Redactor(enabled=enabled).redact(text)


def redact_stream(chunks: Iterable[str]) -> RedactionResult:
    """Redacts a stream as one document, so a rule spanning lines still matches."""
    text = "".join(chunks)
    return Redactor().redact(text)


def render_report(result: RedactionResult, source: str = "stdin") -> str:
    """Markdown report: rule ids, line numbers and counts only."""
    lines = [
        "# Evidence redaction report",
        "",
        f"- source: `{source}`",
        f"- values redacted: {result.redacted}",
        "",
        "| Rule | Line | Count |",
        "|---|---|---|",
    ]
    for finding in result.findings:
        lines.append(f"| {finding.rule_id} | {finding.line} | {finding.count} |")
    if not result.findings:
        lines.append("| — | — | 0 |")
    return "\n".join(lines) + "\n"


def _read_inputs(paths: Sequence[str]) -> List[Tuple[str, str]]:
    if not paths:
        return [("<stdin>", sys.stdin.read())]
    documents: List[Tuple[str, str]] = []
    for raw in paths:
        if raw == "-":
            documents.append(("<stdin>", sys.stdin.read()))
            continue
        path = Path(raw)
        documents.append((str(path), path.read_text(encoding="utf-8", errors="replace")))
    return documents


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Redact secrets from logs and incident artifacts (#687)")
    parser.add_argument("paths", nargs="*", help="files to redact ('-' for stdin)")
    parser.add_argument("--out", type=Path, default=None, help="write redacted text here")
    parser.add_argument("--report", type=Path, default=None, help="write a redaction report here")
    parser.add_argument(
        "--check",
        action="store_true",
        help="emit nothing and fail if a secret is still present (for already-redacted artifacts)",
    )
    parser.add_argument(
        "--no-redact",
        action="store_true",
        help="refused: kept only to make the fail-closed behaviour explicit",
    )
    args = parser.parse_args(argv)

    if args.no_redact:
        # Fail closed. No output is produced, not even the input.
        print(
            "FATAL: --no-redact is not supported; evidence artifacts are never emitted raw",
            file=sys.stderr,
        )
        return 2

    total = 0
    outputs: List[str] = []
    reports: List[str] = []
    for name, text in _read_inputs(args.paths):
        result = redact_text(text)
        total += result.redacted
        outputs.append(result.text)
        reports.append(render_report(result, name))

    if args.check:
        print(f"{total} secret(s) still present")
        return 1 if total else 0

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text("".join(outputs), encoding="utf-8")
        print(f"redacted artifact written to {args.out} ({total} value(s) removed)")
    else:
        sys.stdout.write("".join(outputs))

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text("\n".join(reports), encoding="utf-8")
        print(f"report written to {args.report}", file=sys.stderr)

    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
