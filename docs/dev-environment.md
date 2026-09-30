# Local Dev Environment

From a fresh clone, one command provisions everything and runs the full suite:

```bash
make dev        # or ./scripts/dev-setup.sh [--no-test]
```

It installs the Rust toolchain pinned in `rust-toolchain.toml` (plus the
`wasm32v1-none` target, rustfmt and clippy) via rustup, creates a repo-local
Python `.venv` with `scripts/requirements.txt` and pytest, installs the Node dev
dependencies (git hooks), builds the contract with `--locked`, and runs the
contract and service tests. Nothing is installed globally and nothing needs
`sudo`: rustup lives in `~/.rustup`/`~/.cargo`, everything else inside the repo.

Prefer a container? Open the repo in a devcontainer (`.devcontainer/`), which
runs the same script on creation.

## Prerequisites

`bash`, `curl`, `git`, a C linker (`cc`, for build scripts), `python3` ≥ 3.10
with `venv`. `npm` is optional (only for the pre-push hook).

## Determinism

* Toolchain comes only from `rust-toolchain.toml`; the script aborts if the
  active `rustc` differs (e.g. a stray `RUSTUP_TOOLCHAIN`).
* Rust dependencies are resolved from `Cargo.lock` (`--locked`), Node from
  `package-lock.json` (`npm ci`), Python from pinned `requirements.txt`.

## CI

`.github/workflows/dev-setup.yml` runs `./scripts/dev-setup.sh` on a clean
`ubuntu-latest` runner with no pre-installed Rust setup, on changes to the setup
inputs and every Monday, so rot is caught within a week.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `active rustc X != pinned Y` | `unset RUSTUP_TOOLCHAIN`, remove any `rustup override` for the directory |
| `linker 'cc' not found` | install your OS build tools (`build-essential`, Xcode CLT) |
| `No module named venv` | install `python3-venv` (Debian/Ubuntu) |
| `cargo build --locked` says the lockfile needs updating | you edited a `Cargo.toml`; run `cargo update -p <crate>` and commit `Cargo.lock` |
| `rustup: command not found` after setup | `source "$HOME/.cargo/env"` or open a new shell |
| tests fail with stale `test_snapshots/` | `git clean -fdX contracts/price-oracle/test_snapshots` |
