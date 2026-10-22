"""Source-level SAST pass over the contract sources (#500).

A small, dependency-free rule set aimed at the constructs that actually matter
in a `no_std` Soroban contract: raw `unsafe`, unchecked indexing, infallible
`unwrap`/`expect` on a value the contract does not control, and a handful of
arithmetic patterns that silently truncate in fixed-point price math.

The pass is deterministic and side-effect free, so its output can be baselined
and diffed. It is intentionally narrow: a SAST tool that cries wolf gets
blanket-suppressed, which is the failure mode the issue calls out.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

from services.static_analysis.policy import Finding

#: Directories never scanned: build output, vendored code, and the test
#: harnesses, whose `unwrap`s are deliberate.
DEFAULT_EXCLUDE_DIRS = ("target", "node_modules", ".git", "vendor", "fuzz")

#: Files whose name marks them as test code (`unwrap` there is not a finding).
TEST_FILE_SUFFIXES = ("_tests.rs", "_test.rs", "test.rs", "tests.rs")


@dataclass(frozen=True)
class Rule:
    """One SAST rule: an id, a severity, a pattern and an explanation."""

    id: str
    severity: str
    pattern: re.Pattern
    message: str


RULES: Tuple[Rule, ...] = (
    Rule(
        id="SAST-RUST-UNSAFE-BLOCK",
        severity="high",
        pattern=re.compile(r"(?<![\w:])unsafe\s*\{|(?<![\w:])unsafe\s+fn\b"),
        message="raw `unsafe` block or `unsafe fn` in contract code",
    ),
    Rule(
        id="SAST-RUST-UNWRAP-UNCHECKED",
        severity="high",
        pattern=re.compile(r"\.unwrap_unchecked\s*\("),
        message="`.unwrap_unchecked()` bypasses bounds and validity checks",
    ),
    Rule(
        id="SAST-RUST-UNWRAP",
        severity="medium",
        pattern=re.compile(r"\.unwrap\s*\(\s*\)"),
        message="`.unwrap()` panics; a contract must surface a typed error instead",
    ),
    Rule(
        id="SAST-RUST-EXPECT",
        severity="medium",
        pattern=re.compile(r"\.expect\s*\("),
        message="`.expect()` panics; a contract must surface a typed error instead",
    ),
    Rule(
        id="SAST-RUST-SLAB-PANIC",
        severity="low",
        pattern=re.compile(r"\b(panic!|unreachable!|todo!|unimplemented!)\s*\("),
        message="explicit panic macro; use `panic_with_error!` with an ErrorCode",
    ),
    Rule(
        id="SAST-RUST-NARROWING-CAST",
        severity="low",
        pattern=re.compile(r"\bas\s+(u8|u16|u32|u64)\s*[,;)\]}]"),
        message="narrowing cast can truncate a price or ledger value",
    ),
)


def is_test_path(path: str) -> bool:
    name = Path(path).name
    return name.endswith(TEST_FILE_SUFFIXES) or name == "prop_tests.rs"


def iter_source_files(
    root: Path, excludes: Sequence[str] = DEFAULT_EXCLUDE_DIRS, include_tests: bool = False
) -> Iterable[Path]:
    """Yields scannable source files, skipping build output and test code."""
    for path in sorted(root.rglob("*.rs")):
        rel_parts = set(path.relative_to(root).parts[:-1])
        if rel_parts & set(excludes):
            continue
        if not include_tests and is_test_path(str(path.relative_to(root))):
            continue
        yield path


def scan_text(text: str, path: str, rules: Sequence[Rule] = RULES) -> List[Finding]:
    """Runs `rules` over one file's contents."""
    findings: List[Finding] = []
    for lineno, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("//") or stripped.startswith("*"):
            continue  # commented-out code and doc prose
        for rule in rules:
            if rule.pattern.search(line):
                findings.append(
                    Finding(
                        id=rule.id,
                        kind="sast",
                        path=path,
                        line=lineno,
                        message=rule.message,
                    )
                )
    return findings


def scan_file(path: Path, rel: str, rules: Sequence[Rule] = RULES) -> List[Finding]:
    return scan_text(path.read_text(encoding="utf-8", errors="replace"), rel, rules)


def scan_tree(
    root: Path,
    excludes: Sequence[str] = DEFAULT_EXCLUDE_DIRS,
    include_tests: bool = False,
    rules: Sequence[Rule] = RULES,
) -> List[Finding]:
    """Scans every source file under `root`."""
    findings: List[Finding] = []
    for path in iter_source_files(root, excludes, include_tests):
        findings.extend(scan_file(path, str(path.relative_to(root)), rules))
    return findings


def unsafe_counts(findings: Sequence[Finding]) -> Dict[str, int]:
    """Per-``rule:file`` counts — the shape the CI baseline is stored in.

    The unsafe rules (`SAST-RUST-UNSAFE-BLOCK`, `SAST-RUST-UNWRAP-UNCHECKED`)
    and the panic-prone ones (`UNWRAP` / `EXPECT` / `SLAB-PANIC`) share the
    baseline because they share the review that produced it: any of them going
    up is unsafe-code creep, and any of them going down means the baseline
    should be tightened.
    """
    counts: Dict[str, int] = {}
    for finding in findings:
        key = f"{finding.id}:{finding.path}"
        counts[key] = counts.get(key, 0) + 1
    return counts


def findings_by_id(findings: Sequence[Finding]) -> Dict[str, int]:
    counts: Dict[str, int] = {}
    for finding in findings:
        counts[finding.id] = counts.get(finding.id, 0) + 1
    return counts


def render_report(
    findings: Sequence[Finding],
    advisories: Sequence,
    new_findings: Sequence[Finding],
    allowlisted: Sequence[Finding],
    notes: Sequence[str],
    counts: Dict[str, int],
) -> str:
    """Human-readable report attached to every CI run."""
    lines: List[str] = ["# Static analysis / SAST report", ""]
    lines.append(f"- findings scanned: {len(findings)}")
    lines.append(f"- new (gate-failing) findings: {len(new_findings)}")
    lines.append(f"- allowlisted findings: {len(allowlisted)}")
    lines.append(f"- dependency advisories: {len(advisories)}")
    lines.append("")
    lines.append("## Findings by rule")
    for rule_id, n in sorted(findings_by_id(findings).items()):
        lines.append(f"- `{rule_id}`: {n}")
    lines.append("")
    lines.append("## Unsafe-code baseline")
    if not counts:
        lines.append("- (no unsafe constructs)")
    for key, n in sorted(counts.items()):
        lines.append(f"- `{key}`: {n}")
    lines.append("")
    lines.append("## New findings")
    if not new_findings:
        lines.append("- none")
    for finding in new_findings:
        lines.append(f"- `{finding.id}` {finding.path}:{finding.line} — {finding.message}")
    lines.append("")
    lines.append("## Allowlisted findings")
    if not allowlisted:
        lines.append("- none")
    for finding in allowlisted:
        lines.append(f"- `{finding.id}` {finding.path}:{finding.line}")
    lines.append("")
    lines.append("## Dependency advisories")
    if not advisories:
        lines.append("- none")
    for advisory in advisories:
        state = "fix available" if advisory.has_fix else "NO FIX"
        lines.append(f"- `{advisory.id}` {advisory.crate}: {advisory.title} ({state})")
    lines.append("")
    lines.append("## Notes")
    if not notes:
        lines.append("- none")
    for note in notes:
        lines.append(f"- {note}")
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Source-level SAST pass (#500)")
    parser.add_argument("root", nargs="?", default="contracts", help="directory to scan")
    parser.add_argument("--json", action="store_true", help="emit findings as JSON")
    parser.add_argument("--include-tests", action="store_true")
    args = parser.parse_args(argv)

    findings = scan_tree(Path(args.root), include_tests=args.include_tests)
    if args.json:
        json.dump([f.to_dict() for f in findings], sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
    else:
        by_rule = findings_by_id(findings)
        print(f"{len(findings)} finding(s) under {args.root}")
        for rule_id, n in sorted(by_rule.items()):
            print(f"  {rule_id}: {n}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
