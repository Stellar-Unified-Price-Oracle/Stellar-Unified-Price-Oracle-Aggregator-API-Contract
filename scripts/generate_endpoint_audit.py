#!/usr/bin/env python3
"""Generate an endpoint surface audit table from interface/interface.json."""
from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INTERFACE_JSON = ROOT / "interface" / "interface.json"
LIB_RS = ROOT / "contracts" / "price-oracle" / "src" / "lib.rs"
AUTH_MATRIX = ROOT / "docs" / "security" / "endpoint-authority-matrix.md"
SRC_DIR = ROOT / "contracts" / "price-oracle" / "src"
OUT_MD = ROOT / "docs" / "endpoint-surface-audit.md"

MODULE_CALL_RE = re.compile(r"(?:crate::)?([a-z_][a-z0-9_]*)::([a-z_][a-z0-9_]*)\s*\(")
ISSUE_RE = re.compile(r"#(\d+)")
PUB_FN_RE = re.compile(r"^(\s*)pub fn ([a-z_][a-z0-9_]*)\s*\(")


def parse_auth_matrix() -> dict[str, str]:
    authority: dict[str, str] = {}
    if not AUTH_MATRIX.exists():
        return authority
    for line in AUTH_MATRIX.read_text(encoding="utf-8").splitlines():
        m = re.match(r"^\|\s*`([a-z_][a-z0-9_]*)`\s*\|\s*`([^`]+)`\s*\|", line)
        if m:
            authority[m.group(1)] = m.group(2)
    return authority


def module_issue_map() -> dict[str, str]:
    issues: dict[str, str] = {}
    for rs in sorted(SRC_DIR.glob("*.rs")):
        if rs.name.endswith("_tests.rs"):
            continue
        head = "\n".join(rs.read_text(encoding="utf-8").splitlines()[:20])
        found = sorted(set(ISSUE_RE.findall(head)), key=int)
        if found:
            issues[rs.stem] = ", ".join(f"#{n}" for n in found[:4])
    return issues


def parse_lib_endpoints() -> dict[str, dict[str, str]]:
    lines = LIB_RS.read_text(encoding="utf-8").splitlines()
    endpoints: dict[str, dict[str, str]] = {}
    mod_issues = module_issue_map()
    i = 0
    while i < len(lines):
        m = PUB_FN_RE.match(lines[i])
        if not m:
            i += 1
            continue
        fn_name = m.group(2)
        doc_lines = []
        j = i - 1
        while j >= 0 and (
            lines[j].lstrip().startswith("///")
            or lines[j].lstrip().startswith("#[")
            or lines[j].strip() == ""
        ):
            if lines[j].lstrip().startswith("///"):
                doc_lines.append(lines[j])
            j -= 1
        doc_text = "\n".join(reversed(doc_lines))

        body_lines = []
        depth = 0
        opened = False
        k = i
        while k < len(lines):
            line = lines[k]
            body_lines.append(line)
            for ch in line:
                if ch == "{":
                    depth += 1
                    opened = True
                elif ch == "}":
                    depth -= 1
            if opened and depth == 0:
                break
            k += 1
        body = "\n".join(body_lines)

        mods = [
            mod
            for mod, _ in MODULE_CALL_RE.findall(body)
            if mod not in {"soroban_sdk", "storage", "events", "types", "errors", "error"}
            and (SRC_DIR / f"{mod}.rs").exists()
        ]
        if not mods:
            mods = [
                mod
                for mod, _ in MODULE_CALL_RE.findall(body)
                if (SRC_DIR / f"{mod}.rs").exists()
            ]
        module = mods[0] if mods else "lib"

        mutates_direct = any(
            tok in body
            for tok in (".set(", ".remove(", ".extend_ttl(", ".update_current_contract_wasm(")
        )
        prefix = fn_name.split("_", 1)[0]
        is_read_name = prefix in {
            "get",
            "is",
            "has",
            "check",
            "verify",
            "list",
            "count",
            "compute",
            "estimate",
            "preview",
            "simulate",
            "explain",
            "export",
            "version",
            "decimals",
            "resolution",
            "description",
            "assets",
            "lastprice",
            "price",
            "prices",
            "twap",
        }

        doc_issues = sorted(set(ISSUE_RE.findall(doc_text + "\n" + body)), key=int)
        if doc_issues:
            orig_issue = ", ".join(f"#{n}" for n in doc_issues[:3])
        elif module in mod_issues:
            orig_issue = mod_issues[module]
        else:
            orig_issue = "—"

        endpoints[fn_name] = {
            "module": module,
            "mutates_direct": "yes" if mutates_direct else ("no" if is_read_name else "yes (delegated)"),
            "body": body,
            "orig_issue": orig_issue,
        }
        i = k + 1
    return endpoints


def build_rows() -> list[tuple[str, str, str, str, str]]:
    iface = json.loads(INTERFACE_JSON.read_text(encoding="utf-8"))
    functions = iface.get("functions", {})
    auth_matrix = parse_auth_matrix()
    lib_info = parse_lib_endpoints()
    rows: list[tuple[str, str, str, str, str]] = []

    for endpoint in functions:
        info = lib_info.get(endpoint, {})
        module = info.get("module", "lib")
        body = info.get("body", "")
        auth = auth_matrix.get(endpoint)
        if auth is None:
            if ".require_auth" in body or "get_admin" in body or "check_role" in body:
                requires_auth = "yes"
            elif info.get("mutates_direct", "yes").startswith("no"):
                requires_auth = "no (public-read)"
            else:
                requires_auth = "delegated / review"
        elif auth == "public-read":
            requires_auth = "no (`public-read`)"
        elif auth == "public":
            requires_auth = "no (`public`)"
        else:
            requires_auth = f"yes (`{auth}`)"

        if auth == "public-read":
            mutates = "no"
        else:
            mutates = info.get("mutates_direct", "unknown")

        orig_issue = info.get("orig_issue", "—")
        rows.append((endpoint, module, mutates, requires_auth, orig_issue))
    return rows


def main() -> None:
    rows = build_rows()
    lines = [
        "# Endpoint Surface Audit (`interface/interface.json`)",
        "",
        f"Generated from `interface/interface.json` ({len(rows)} public endpoints), cross-referenced with `contracts/price-oracle/src/lib.rs` and [docs/security/endpoint-authority-matrix.md](security/endpoint-authority-matrix.md).",
        "",
        "| Endpoint | Module | Mutates state? | Requires auth? | Original issue |",
        "|---|---|---|---|---|",
    ]
    for endpoint, module, mutates, auth, issue in rows:
        lines.append(f"| `{endpoint}` | `{module}` | {mutates} | {auth} | {issue} |")
    lines.append("")
    OUT_MD.write_text("\n".join(lines), encoding="utf-8")
    print(f"Wrote {len(rows)} rows to {OUT_MD}")


if __name__ == "__main__":
    main()
