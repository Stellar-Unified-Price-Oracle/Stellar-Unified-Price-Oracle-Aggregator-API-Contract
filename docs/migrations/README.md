# Migration Guides

Every breaking change to the contract interface ships with a versioned guide in this directory.

## Policy

1. **What counts as breaking** — removing/renaming a function, changing a parameter or return type,
   renumbering/removing an error code, *or* a silent behavioural change (e.g. different rounding,
   stricter validation) even when the signature is unchanged.
2. **Guide required** — a PR with a breaking change must add `vX.Y.Z.md` (copy [`TEMPLATE.md`](TEMPLATE.md))
   and register it in [`index.json`](index.json). `scripts/devx/gen_interface.py --check` fails CI
   when the interface breaks without a registered guide for the new version.
3. **Deprecation window** — a function is deprecated before removal, for at least `window_days` (90)
   as declared in [`deprecations.json`](deprecations.json), with a stated removal date.
   `scripts/devx/lint_deprecated.py` fails CI if the window is too short, the guide is missing, or the
   function is still exported after its removal date.
4. **Linter** — the same script flags deprecated usage in `examples/` (consumer templates); consumers can
   run it against their own code: `python3 scripts/devx/lint_deprecated.py path/to/src`.
5. **Examples compile** — before/after snippets that live in `examples/` are built by the `templates` CI job.

## Deprecation entry format

```json
{ "function": "old_fn", "since": "2026-10-01", "removal": "2027-01-01",
  "replacement": "new_fn", "guide": "v0.2.0.md" }
```

Deprecated functions are also marked with a `deprecated` field in the published `interface.json`.

## Guides

| Version | Guide |
|---|---|
| 0.1.0 | [v0.1.0.md](v0.1.0.md) — initial published interface (baseline) |
