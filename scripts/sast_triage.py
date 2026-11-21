#!/usr/bin/env python3
"""Per-finding triage of the source-level SAST gate (`services.static_analysis.sast`).

Every finding gets exactly one disposition:

* ``false-positive`` - the rule matched code that cannot ship or cannot misbehave
  (test-only / kani-only code, bool->int, enum discriminants, widening casts).
* ``accept``         - production code where the panic or truncation is
  unreachable by construction; the reason names the invariant.
* ``fix``            - production code that should surface a typed error instead.
* ``review``         - the heuristics could not decide; a human must pick one.

The classifier is deliberately small and explainable: each disposition carries a
stable reason code, so a reviewer can audit the *rule*, not 300 lines.

Usage:
    python3 scripts/sast_triage.py            # (re)write docs/security/sast-triage.md
    python3 scripts/sast_triage.py --check    # exit 1 if the checked-in report is stale
"""
from __future__ import annotations

import re
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from services.static_analysis.sast import scan_tree  # noqa: E402

CONTRACTS = ROOT / "contracts"
OUT = ROOT / "docs" / "security" / "sast-triage.md"

TEST_ATTR = re.compile(r"#\[cfg\((?:test|kani)\)\]|#\[test\]|#\[kani::proof\]|proptest!")
ITEM_OPEN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:mod|fn|impl)\b")

# --- reason codes ----------------------------------------------------------
R_TEST = "test-only: inside #[cfg(test)] / #[cfg(kani)] / #[test]"
R_BOOL = "bool-to-int cast (0/1), cannot truncate"
R_ENUM = "fieldless enum discriminant, fits the target type"
R_MASK = "value masked, reduced modulo, or clamped before the cast"
R_LEN32 = "`len()` is a u32 in soroban-sdk collections / usize on wasm32; lossless"
R_WIDEN = "widening cast (u8/u16/u32 -> u64), cannot truncate"
R_BYTE = "u8 element widened to u32/u64, cannot truncate"
R_LOOP = "index comes from a loop bounded by the collection length"
R_INIT = "admin key is written by `init`, which every entrypoint requires first"
R_HAS = "presence checked with `has` immediately above; the `get` cannot be `None`"
R_JUSTSET = "key was written earlier in the same function (`init`)"


def test_ranges(lines: List[str]) -> List[Tuple[int, int]]:
    """Line ranges (1-based, inclusive) of test/kani-only items."""
    ranges: List[Tuple[int, int]] = []
    i = 0
    while i < len(lines):
        if TEST_ATTR.search(lines[i]):
            j = i
            while j < len(lines) and "{" not in lines[j] and ";" not in lines[j]:
                j += 1
            if j < len(lines) and "{" in lines[j]:
                depth, k = 0, j
                while k < len(lines):
                    depth += lines[k].count("{") - lines[k].count("}")
                    if depth <= 0 and k >= j:
                        break
                    k += 1
                ranges.append((i + 1, k + 1))
                i = k
        i += 1
    return ranges


def in_ranges(line: int, ranges: List[Tuple[int, int]]) -> bool:
    return any(a <= line <= b for a, b in ranges)


def classify_cast(code: str) -> Optional[Tuple[str, str]]:
    stmt = code.strip()
    m = re.search(r"([^\s(]+|\([^()]*\)|\([^()]*\([^()]*\)[^()]*\))\s+as\s+(u8|u16|u32|u64)\b", stmt)
    dst = m.group(2) if m else ""
    if re.search(r"%\s*\d+\)\s*as\s+(u8|u16|u32|u64)", stmt) or re.search(r"&\s*0x[0-9a-fA-F]+\)\s*as", stmt):
        return "accept", R_MASK
    if re.search(r"\.min\([^)]*(?:MAX|\d)[^)]*\)\)?\s*as\s+(u8|u16|u32|u64)", stmt):
        return "accept", R_MASK
    if re.search(r"(?:get_unchecked|get|set)\(\s*\d+\s*\)\s*as\s+(u32|u64)\b|\(\s*\*\s*\w*byte\w*\s+as\s+u64\)", stmt):
        return "false-positive", R_BYTE
    if re.search(r"(?:get_unchecked|get|set|publish)\(\s*\(?\s*(?:i|j|k|idx)\s+as\s+u32", stmt):
        return "accept", R_LOOP
    if re.search(r"(?:len|count)\(\)\s*as\s+(u32|u64)", stmt):
        return "accept", R_LEN32 if dst == "u32" else R_WIDEN
    if re.search(r"\)\s*!=\s*0\)\s*as\s+u8|==\s*[^)]+\)\s*as\s+u8|\bis_\w+\(\)\s*as\s+u8", stmt):
        return "false-positive", R_BOOL
    if re.search(r"\b[A-Z][A-Za-z0-9]*::[A-Z][A-Za-z0-9]*\s+as\s+(u8|u16|u32|u64)\b", stmt):
        return "false-positive", R_ENUM
    if re.search(r"\b\w*(?:severity|kind|mode|tier|status|level|method|regime|category|health|interpretation)\s+as\s+(u8|u16|u32|u64)\b", stmt):
        return "false-positive", R_ENUM
    if dst == "u64" and re.search(r"\b\w+\s+as\s+u64\b", stmt) and not re.search(r"\bu128\b|\bi128\b|\bi64\b", stmt):
        # `x as u64` where x is a u32/u8 field (all duration/period/ledger fields are u32).
        if re.search(r"\b(?:plan_duration|ledger|period|window|interval|ttl|n|count|len|step|round|index|i|idx|num\w*|denom\w*)\s+as\s+u64\b", stmt):
            return "false-positive", R_WIDEN
    return None


def classify(rule: str, path: str, line: int, code: str, tests: List[Tuple[int, int]], before: str = "") -> Tuple[str, str]:
    if in_ranges(line, tests):
        return "false-positive", R_TEST
    if rule == "SAST-RUST-NARROWING-CAST":
        c = classify_cast(code)
        return c if c else ("review", "cast source type is not visible from the line; confirm the value range")
    s = code.strip()
    if rule in ("SAST-RUST-UNWRAP", "SAST-RUST-EXPECT"):
        if re.search(r"\.get\(&?\w+\)\s*\.(unwrap|expect)\(", s) and re.search(r"\.has\(&?\w+\)", before[-600:]):
            return "accept", R_HAS
        if "DataKey::Admin" in s:
            if re.search(r"\.set\(\s*&DataKey::Admin\b", before):
                return "accept", R_JUSTSET
            return "fix", "return a typed `NotInitialized` error instead of panicking; admin is only guaranteed after `init`"
        if re.search(r"\.get\((?:i|n_sources\s*-\s*1|idx|index)\)\s*\.(unwrap|expect)", s):
            return "accept", R_LOOP
        if re.search(r"storage\(\)\s*\.\s*(temporary|persistent|instance)\(\)\s*\.get\([^)]*\)\s*\.unwrap\(\)", s) and not re.search(r"\.has\(", before[-600:]):
            return "fix", "guard with `has`/`match` and return `ErrorCode::NoData`; an expired entry would trap the transaction"
    return "review", "no rule matched; decide fix, accept, or false positive"


def collect() -> List[Dict[str, object]]:
    findings = scan_tree(CONTRACTS)
    cache: Dict[str, List[str]] = {}
    ranges: Dict[str, List[Tuple[int, int]]] = {}
    out: List[Dict[str, object]] = []
    for f in sorted(findings, key=lambda x: (x.path, x.line, x.id)):
        full = CONTRACTS / f.path
        lines = cache.setdefault(f.path, full.read_text(encoding="utf-8").splitlines())
        rng = ranges.setdefault(f.path, test_ranges(lines))
        code = lines[f.line - 1] if 0 < f.line <= len(lines) else ""
        before = "\n".join(lines[max(0, f.line - 81) : f.line - 1])
        disp, reason = classify(f.id, f.path, f.line, code, rng, before)
        out.append({"path": f.path, "line": f.line, "rule": f.id, "code": code.strip(), "disp": disp, "reason": reason})
    return out


def render() -> str:
    rows = collect()
    by_disp = Counter(r["disp"] for r in rows)
    by_rule: Dict[str, Counter] = defaultdict(Counter)
    for r in rows:
        by_rule[str(r["rule"])][str(r["disp"])] += 1
    out = [
        "# SAST per-finding triage",
        "",
        "Generated by `scripts/sast_triage.py` from `python -m services.static_analysis.sast`.",
        "Do not edit by hand; change the classifier or the code, then re-run.",
        "",
        "Dispositions: **fix** (production code should return a typed error), **accept** (unreachable by",
        "construction; reason names the invariant), **false-positive** (test/kani-only code or a cast",
        "that cannot truncate), **review** (heuristics cannot decide; needs a human).",
        "",
        f"Total findings: **{len(rows)}**",
        "",
        "| Disposition | Findings |",
        "|---|---:|",
    ]
    for d in ("fix", "accept", "false-positive", "review"):
        out.append(f"| {d} | {by_disp.get(d, 0)} |")
    out += ["", "| Rule | fix | accept | false-positive | review |", "|---|---:|---:|---:|---:|"]
    for rule in sorted(by_rule):
        c = by_rule[rule]
        out.append(f"| `{rule}` | {c['fix']} | {c['accept']} | {c['false-positive']} | {c['review']} |")
    out += [
        "",
        "## Re-baseline",
        "",
        "`config/security-baseline.json` is stale against this scan. Re-baselining needs a named reviewer",
        "(`python -m services.static_analysis.gate --skip-advisories --rebaseline <handle>`), so it is not",
        "done here: apply the `fix` items, resolve the `review` items, then re-baseline in the same change.",
        "",
    ]
    for d, title in (("fix", "Fix"), ("review", "Needs review"), ("accept", "Accept with justification"), ("false-positive", "False positives")):
        sel = [r for r in rows if r["disp"] == d]
        out += [f"## {title} ({len(sel)})", ""]
        if not sel:
            out += ["None.", ""]
            continue
        out += ["| Location | Rule | Code | Reason |", "|---|---|---|---|"]
        for r in sel:
            code = str(r["code"]).replace("|", "\\|")
            if len(code) > 90:
                code = code[:87] + "..."
            out.append(f"| `{r['path']}:{r['line']}` | `{str(r['rule'])[10:]}` | `{code}` | {r['reason']} |")
        out.append("")
    return "\n".join(out)


def main() -> int:
    report = render()
    if "--check" in sys.argv:
        if not OUT.exists() or OUT.read_text(encoding="utf-8") != report:
            print(f"{OUT.relative_to(ROOT)} is stale; run scripts/sast_triage.py", file=sys.stderr)
            return 1
        return 0
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(report, encoding="utf-8")
    print(f"Wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
