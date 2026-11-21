# #835 Versioning and changelog automation

## Built in this change
- `scripts/devx/check_versions.py` — single source of truth guard. `interface/package.json`
  is canonical; the script verifies `interface/versions.json`, both contract `Cargo.toml`
  versions, the root `package.json` (when versioned) and any `sdk/*/package.json` or
  `packages/*/package.json` agree with it. `--check` exits non-zero on any mismatch.

## Remaining work
1. **CI wiring** — add a step to `.github/workflows/devx.yml`:
   `python3 scripts/devx/check_versions.py --check` (next to the existing
   `gen_interface.py --check` / `gen_error_registry.py --check` steps).
2. **Conventional Commits linting** — add `commitlint` (`@commitlint/config-conventional`)
   plus a `commit-msg` husky hook in `.husky/`, and a `commitlint` job in CI that lints the
   PR's commit range (`npx commitlint --from origin/main --to HEAD`).
3. **Changelog generation** — add `cliff.toml` and a `release` workflow that runs
   `git-cliff --tag vX.Y.Z -o CHANGELOG.md` on tag push, commits the changelog, and
   creates the GitHub release. Alternative: `release-please` with
   `release-please-config.json` + `.release-please-manifest.json` keyed on
   `interface/package.json`.
4. **Version bump automation** — the release job must bump `interface/package.json`
   (canonical) and then run `scripts/devx/check_versions.py --check`; on failure it must
   fail the release rather than publish.
5. **Docs** — document the flow in `docs/interface-versioning.md` (canonical version,
   bump rules, changelog location) and link it from `CONTRIBUTING.md`.

## Done when
- A tag push produces `CHANGELOG.md` and a GitHub release.
- `check_versions.py --check` runs in CI and blocks a release when any declared version
  drifts from `interface/package.json`.
- Commit messages are linted against Conventional Commits in CI.
