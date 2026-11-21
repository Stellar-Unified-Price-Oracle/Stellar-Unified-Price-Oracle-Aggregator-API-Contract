#!/usr/bin/env python3
"""Docs freshness gate (#540): dead links, broken snippets, stale interfaces.

    python3 scripts/docs_freshness.py            # links (local), snippets, interface, nav, status
    python3 scripts/docs_freshness.py --external # also HEAD-check http(s) links

Failure classes and how to fix each are in docs/docs-freshness.md.
Exit code is non-zero if any check fails.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = ROOT / "docs" / "link-allowlist.txt"
BASELINE = ROOT / "docs" / "docs-freshness-baseline.txt"
LIB_RS = ROOT / "contracts" / "price-oracle" / "src" / "lib.rs"
CARGO_TOML = ROOT / "contracts" / "price-oracle" / "Cargo.toml"
SKIP_DIRS = {"node_modules", "target", ".git", "fuzz", "test_snapshots", "hermetic-artifacts"}

LINK_RE = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
FENCE_RE = re.compile(r"^```([^\n`]*)\n(.*?)^```", re.M | re.S)
CLIENT_CALL_RE = re.compile(r"\bclient\.(?:try_)?([a-z_][a-z0-9_]*)\s*\(")
INVOKE_RE = re.compile(r"invoke\b[^\n]*?--\s+([a-z_][a-z0-9_]*)")
PLACEHOLDER_RE = re.compile(r"<[A-Za-z0-9_\-. /:|]+>")
VERSION_MARK_RE = re.compile(r"<!--\s*contract-version:\s*([0-9.]+)\s*-->")


def markdown_files(root: Path = ROOT) -> list[Path]:
    return sorted(p for p in root.rglob("*.md") if not SKIP_DIRS & set(p.relative_to(root).parts))


def load_allowlist(path: Path = ALLOWLIST) -> set[str]:
    if not path.exists():
        return set()
    out = set()
    for line in path.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            out.add(line)
    return out


def strip_code(text: str) -> str:
    return FENCE_RE.sub("", text)


def check_links(md: Path, allow: set[str], external: bool) -> list[str]:
    errors = []
    for target in LINK_RE.findall(strip_code(md.read_text(encoding="utf-8"))):
        if target.startswith(("mailto:", "#")) or target in allow:
            continue
        if any(target.startswith(a.rstrip("*")) for a in allow if a.endswith("*")):
            continue
        if target.startswith(("http://", "https://")):
            if external and not url_ok(target):
                errors.append(f"{md.relative_to(ROOT)}: dead link {target}")
            continue
        path = target.split("#", 1)[0]
        if path and not (md.parent / path).exists():
            errors.append(f"{md.relative_to(ROOT)}: missing file {target}")
    return errors


def url_ok(url: str) -> bool:
    for method in ("HEAD", "GET"):
        try:
            req = urllib.request.Request(url, method=method, headers={"User-Agent": "docs-freshness"})
            with urllib.request.urlopen(req, timeout=15) as r:
                if r.status < 400:
                    return True
        except Exception:
            continue
    return False


def check_snippets(md: Path) -> list[str]:
    """json / python / bash blocks always; rust blocks tagged `rust,compile`."""
    errors = []
    rel = md.relative_to(ROOT)
    for info, body in FENCE_RE.findall(md.read_text(encoding="utf-8")):
        tags = {t.strip() for t in info.split(",")}
        lang = info.split(",")[0].strip()
        if "ignore" in tags or "no-check" in tags:
            continue
        try:
            if lang == "json":
                json.loads(body)
            elif lang == "python":
                compile(body, str(rel), "exec")
            elif lang in ("bash", "sh"):
                # `<placeholder>` is the documented convention for values the
                # reader substitutes; treat it as a word, not a redirect.
                script = PLACEHOLDER_RE.sub("PLACEHOLDER", body)
                r = subprocess.run(["bash", "-n"], input=script, text=True, capture_output=True)
                if r.returncode:
                    raise ValueError(r.stderr.strip())
            elif lang == "rust" and "compile" in tags:
                compile_rust(body)
        except Exception as e:  # noqa: BLE001
            errors.append(f"{rel}: {lang} snippet does not compile: {str(e).splitlines()[0] if str(e) else e}")
    return errors


def compile_rust(body: str) -> None:
    with tempfile.TemporaryDirectory() as d:
        src = Path(d) / "snippet.rs"
        src.write_text("#![allow(dead_code, unused)]\n" + body)
        r = subprocess.run(
            ["rustc", "--edition", "2021", "--crate-type", "lib", "--emit", "metadata",
             "-o", str(Path(d) / "out"), str(src)],
            capture_output=True, text=True,
        )
        if r.returncode:
            raise ValueError(r.stderr.strip())


def contract_endpoints(lib_rs: Path = LIB_RS) -> set[str]:
    return set(re.findall(r"^\s*pub fn ([a-z_][a-z0-9_]*)\s*\(", lib_rs.read_text(), re.M))


def contract_version(cargo: Path = CARGO_TOML) -> str:
    return re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.M).group(1)


def check_interface(md: Path, endpoints: set[str], version: str) -> list[str]:
    errors = []
    rel = md.relative_to(ROOT)
    text = md.read_text(encoding="utf-8")
    for info, body in FENCE_RE.findall(text):
        if "ignore" in info:
            continue
        for fn in set(CLIENT_CALL_RE.findall(body)) | set(INVOKE_RE.findall(body)):
            if fn not in endpoints:
                errors.append(f"{rel}: example calls `{fn}`, which is not a contract endpoint")
    for pinned in VERSION_MARK_RE.findall(text):
        if pinned != version:
            errors.append(f"{rel}: examples pinned to contract {pinned}, current is {version}")
    return errors


SUMMARY_RE = re.compile(r"^#\s+(.+?)\s*$", re.M)
STATUS_RE = re.compile(r"^<!--\s*status:\s*(shipped|experimental|not deployed)\s*-->\s*$", re.M)
STATUS_VALUES = ("shipped", "experimental", "not deployed")
# Docs that describe the repo itself rather than a contract feature.
STATUS_EXEMPT = {"SUMMARY.md", "README.md", "TEMPLATE.md"}


def module_status(lib_rs: Path = LIB_RS) -> dict[str, str]:
    """#852: map each contract module to its deployment status.

    A module is `shipped` when `lib.rs` declares it unconditionally, and
    `experimental` when it is only declared behind a `#[cfg(feature = ...)]`
    gate. Modules that are not declared at all are `not deployed`.
    """
    status: dict[str, str] = {}
    gated = False
    for line in lib_rs.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("#[cfg("):
            gated = True
            continue
        m = re.match(r"^(?:pub )?mod ([a-z_][a-z0-9_]*);", stripped)
        if m:
            status[m.group(1)] = "experimental" if gated else "shipped"
            gated = False
        elif stripped and not stripped.startswith("//"):
            gated = False
    return status


def check_status(md: Path, modules: dict[str, str]) -> list[str]:
    """#852: every feature doc must declare its status badge on line 1."""
    if md.name in STATUS_EXEMPT:
        return []
    rel = md.relative_to(ROOT)
    text = md.read_text(encoding="utf-8")
    m = STATUS_RE.search(text)
    if not m:
        return [f"{rel}: missing status badge (one of: {', '.join(STATUS_VALUES)})"]
    declared = m.group(1)
    stem = md.stem.replace("-", "_")
    expected = modules.get(stem)
    if expected is not None and expected != declared:
        return [f"{rel}: status `{declared}` but module `{stem}` is `{expected}`"]
    return []


def check_nav(docs: Path = ROOT / "docs") -> list[str]:
    """#850: every docs page must be reachable from the mdBook nav.

    The nav is `docs/SUMMARY.md`; a page is reachable when it is linked from it
    (directly or through a nested section). Pages that are not markdown (JSON,
    CSV, SQL, ...) are data, not pages, and are not required to be listed.
    """
    summary = docs / "SUMMARY.md"
    if not summary.exists():
        return [f"docs/SUMMARY.md: missing mdBook nav"]
    listed = set()
    for target in LINK_RE.findall(strip_code(summary.read_text(encoding="utf-8"))):
        path = target.split("#", 1)[0]
        if path and not path.startswith(("http://", "https://", "mailto:")):
            listed.add((docs / path).resolve())
    errors = []
    for md in sorted(docs.rglob("*.md")):
        if md.name == "SUMMARY.md" or md.resolve() in listed:
            continue
        errors.append(f"{md.relative_to(ROOT)}: orphan page, not listed in docs/SUMMARY.md")
    return errors


def run(files: list[Path], external: bool) -> list[str]:
    allow = load_allowlist()
    endpoints = contract_endpoints()
    version = contract_version()
    modules = module_status()
    errors: list[str] = []
    for md in files:
        errors += check_links(md, allow, external)
        errors += check_snippets(md)
        errors += check_interface(md, endpoints, version)
        errors += check_status(md, modules)
    if not files or any("docs" in f.parts for f in files):
        errors += check_nav()
    return errors


def main() -> int:
    p = argparse.ArgumentParser(description="Docs freshness gate")
    p.add_argument("--external", action="store_true", help="also check http(s) links")
    p.add_argument("files", nargs="*", type=Path)
    a = p.parse_args()
    files = [f.resolve() for f in a.files] or markdown_files()
    errors = run(files, a.external)
    # Known, tracked problems predating the gate. New problems fail; so does a
    # baseline entry that no longer reproduces (delete it — the baseline only shrinks).
    baseline = load_allowlist(BASELINE) if not a.files else set()
    stale = sorted(baseline - set(errors))
    errors = [e for e in errors if e not in baseline]
    errors += [f"baseline entry fixed, remove it from docs/docs-freshness-baseline.txt: {b}" for b in stale]
    for e in errors:
        print(f"::error::{e}")
    print(f"docs-freshness: {len(files)} files, {len(errors)} problems (see docs/docs-freshness.md)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
