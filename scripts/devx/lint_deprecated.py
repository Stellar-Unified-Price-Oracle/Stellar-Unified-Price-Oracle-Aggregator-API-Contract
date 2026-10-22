#!/usr/bin/env python3
"""Flag deprecated contract usage and enforce the deprecation window (issue #538).

- Every entry in docs/migrations/deprecations.json must have since/removal dates,
  a replacement and an existing migration guide.
- The window (removal - since) must be >= window_days.
- A function still present in the interface after its removal date fails CI.
- Any reference to a deprecated function under examples/ (or paths given as args) is flagged.
"""
import datetime as dt, json, pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
cfg = json.loads((ROOT / "docs/migrations/deprecations.json").read_text())
iface = json.loads((ROOT / "interface/interface.json").read_text())["functions"]
roots = [pathlib.Path(a) for a in sys.argv[1:]] or [ROOT / "examples"]
today = dt.date.today()
errs = []

for d in cfg["deprecations"]:
    fn = d["function"]
    since, removal = dt.date.fromisoformat(d["since"]), dt.date.fromisoformat(d["removal"])
    if (removal - since).days < cfg["window_days"]:
        errs.append(f"{fn}: deprecation window shorter than {cfg['window_days']} days")
    if not (ROOT / "docs/migrations" / d["guide"]).exists():
        errs.append(f"{fn}: migration guide {d['guide']} missing")
    if today > removal and fn in iface:
        errs.append(f"{fn}: removal date {removal} passed but function still exported")
    pat = re.compile(rf"""["'`]{re.escape(fn)}["'`]""")
    for r in roots:
        for f in r.rglob("*"):
            if f.is_file() and "node_modules" not in f.parts and f.suffix in {".js", ".ts", ".py", ".rs"}:
                for i, line in enumerate(f.read_text(errors="ignore").splitlines(), 1):
                    if pat.search(line):
                        errs.append(f"{f.relative_to(ROOT)}:{i}: deprecated `{fn}` (use `{d['replacement']}`, see docs/migrations/{d['guide']})")

if errs:
    print("\n".join(errs), file=sys.stderr); sys.exit(1)
print(f"deprecation lint OK ({len(cfg['deprecations'])} deprecations)")
