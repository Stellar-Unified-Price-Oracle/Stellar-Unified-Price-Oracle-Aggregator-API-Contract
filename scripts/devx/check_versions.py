#!/usr/bin/env python3
"""Single version source of truth check (issue #835).

`interface/package.json` is the canonical version. Every other place that
declares the released version must agree with it:

  - interface/versions.json  (must have an entry for the version)
  - contracts/price-oracle/Cargo.toml
  - contracts/minimal-oracle/Cargo.toml
  - package.json (root, when it declares a version)
  - any SDK package.json under sdk/ or packages/ (when present)

Usage:
  check_versions.py            print the canonical version and exit 0
  check_versions.py --check    CI: fail on any mismatch
"""
import json, re, sys, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
CANONICAL = ROOT / "interface/package.json"
VERSIONS = ROOT / "interface/versions.json"
CRATES = [
    ROOT / "contracts/price-oracle/Cargo.toml",
    ROOT / "contracts/minimal-oracle/Cargo.toml",
]
SDK_DIRS = [ROOT / "sdk", ROOT / "packages"]


def canonical_version():
    return json.loads(CANONICAL.read_text())["version"]


def crate_version(path):
    m = re.search(r'^version\s*=\s*"([^"]+)"', path.read_text(), re.M)
    return m[1] if m else None


def sdk_packages():
    out = []
    for d in SDK_DIRS:
        if d.is_dir():
            out.extend(sorted(d.glob("*/package.json")))
    return out


def collect():
    """Return [(label, version)] for every declared version."""
    found = [("interface/package.json", canonical_version())]
    root_pkg = ROOT / "package.json"
    if root_pkg.exists():
        v = json.loads(root_pkg.read_text()).get("version")
        if v:
            found.append(("package.json", v))
    for c in CRATES:
        if c.exists():
            found.append((str(c.relative_to(ROOT)), crate_version(c)))
    for p in sdk_packages():
        found.append((str(p.relative_to(ROOT)), json.loads(p.read_text()).get("version")))
    return found


def main():
    want = canonical_version()
    if "--check" not in sys.argv:
        print(want)
        return
    errs = []
    for label, got in collect():
        if got != want:
            errs.append(f"{label} declares {got!r}, expected {want!r} (from interface/package.json)")
    versions = json.loads(VERSIONS.read_text())
    if want not in versions:
        errs.append(f"interface/versions.json has no entry for {want}")
    if errs:
        print("\n".join(errs), file=sys.stderr)
        sys.exit(1)
    print(f"version {want} consistent")


if __name__ == "__main__":
    main()
