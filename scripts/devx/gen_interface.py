#!/usr/bin/env python3
"""Generate the contract interface artifact from the #[contractimpl] block (issue #535).

Usage:
  gen_interface.py            regenerate interface/interface.json
  gen_interface.py --check    CI: fail if the artifact is stale or semver rules are violated
                              (compared against the interface on BASE_REF, default origin/main)
Semver rules (0.x treats minor as major):
  removed fn / changed signature -> MAJOR bump required
  added fn                       -> MINOR bump required
"""
import json, os, re, subprocess, sys, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
LIB = ROOT / "contracts/price-oracle/src/lib.rs"
PKG = ROOT / "interface/package.json"
OUT = ROOT / "interface/interface.json"
DEPRECATIONS = ROOT / "docs/migrations/deprecations.json"


def split_params(s):
    out, depth, cur = [], 0, ""
    for ch in s:
        depth += ch in "<([" ; depth -= ch in ">)]"
        if ch == "," and depth == 0:
            out.append(cur); cur = ""
        else:
            cur += ch
    return [" ".join(p.split()) for p in out + [cur] if p.strip()]


def clean(t):
    return t.replace("crate::types::", "").replace("soroban_sdk::", "")


def extract(src):
    impl = src.split("#[contractimpl]", 1)[1]
    fns = {}
    for m in re.finditer(r"\n    pub fn (\w+)\s*\((.*?)\)\s*(->\s*([^{]+?))?\s*\{", impl, re.S):
        params = [clean(p) for p in split_params(m[2]) if not re.match(r"_?env\s*:", p)]
        fns[m[1]] = {"params": params, "returns": clean(" ".join(m[4].split())) if m[4] else "()"}
    return dict(sorted(fns.items()))


def build():
    pkg = json.loads(PKG.read_text())
    deps = json.loads(DEPRECATIONS.read_text())["deprecations"] if DEPRECATIONS.exists() else []
    fns = extract(LIB.read_text())
    for d in deps:
        if d["function"] in fns:
            fns[d["function"]]["deprecated"] = {k: d[k] for k in ("since", "removal", "replacement", "guide")}
    return {"name": pkg["name"], "version": pkg["version"], "contract": "price-oracle", "functions": fns}


def ver(v):
    return tuple(int(x) for x in v.split("."))


def main():
    art = build()
    text = json.dumps(art, indent=2) + "\n"
    if "--check" not in sys.argv:
        OUT.write_text(text); print(f"interface {art['version']}: {len(art['functions'])} functions"); return
    errs = []
    if not OUT.exists() or OUT.read_text() != text:
        errs.append("interface/interface.json is stale; run scripts/devx/gen_interface.py")
    base = os.environ.get("BASE_REF", "origin/main")
    try:
        old = json.loads(subprocess.check_output(["git", "show", f"{base}:interface/interface.json"], cwd=ROOT, stderr=subprocess.DEVNULL))
    except Exception:
        old = None
    if old:
        o, n = old["functions"], art["functions"]
        strip = lambda f: {k: v for k, v in f.items() if k != "deprecated"}
        breaking = [f for f in o if f not in n or strip(o[f]) != strip(n[f])]
        added = [f for f in n if f not in o]
        ov, nv = ver(old["version"]), ver(art["version"])
        major = (lambda v: v[:2]) if ov[0] == 0 else (lambda v: v[:1])
        if breaking and not major(nv) > major(ov):
            errs.append(f"breaking interface change {breaking} requires a major bump (was {old['version']})")
        if breaking:
            vers = json.loads((ROOT / "docs/migrations/index.json").read_text())
            if art["version"] not in vers:
                errs.append(f"breaking change needs a migration guide registered for {art['version']} in docs/migrations/index.json")
        if added and not nv > ov:
            errs.append(f"new functions {added} require a version bump (was {old['version']})")
    versions = json.loads((ROOT / "interface/versions.json").read_text())
    if art["version"] not in versions:
        errs.append(f"interface/versions.json has no entry for {art['version']}")
    if errs:
        print("\n".join(errs), file=sys.stderr); sys.exit(1)
    print(f"interface {art['version']} OK")


if __name__ == "__main__":
    main()
