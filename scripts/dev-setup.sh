#!/usr/bin/env bash
# One-command contributor environment (#541).
#
#   ./scripts/dev-setup.sh            # provision + build + full test run
#   ./scripts/dev-setup.sh --no-test  # provision + build only
#
# Everything installs under $HOME (rustup) or the repo (.venv, node_modules);
# nothing needs sudo. The Rust toolchain is read from rust-toolchain.toml, so
# this script can never drift from it. Troubleshooting: docs/dev-environment.md
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
RUN_TESTS=1
[[ "${1:-}" == "--no-test" ]] && RUN_TESTS=0

step() { printf '\n==> %s\n' "$*"; }
die() { printf 'dev-setup: %s\n(see docs/dev-environment.md#troubleshooting)\n' "$*" >&2; exit 1; }

CHANNEL="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)"
TARGETS="$(sed -n 's/^targets *= *\[\(.*\)\]/\1/p' rust-toolchain.toml | tr -d '" ' | tr ',' ' ')"
[[ -n "$CHANNEL" ]] || die "could not read channel from rust-toolchain.toml"

step "Rust toolchain $CHANNEL (from rust-toolchain.toml)"
if ! command -v rustup >/dev/null 2>&1; then
  command -v curl >/dev/null || die "curl is required to install rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi
rustup toolchain install "$CHANNEL" --profile minimal --component rustfmt,clippy
for t in $TARGETS; do rustup target add --toolchain "$CHANNEL" "$t"; done
ACTIVE="$(rustc --version | awk '{print $2}')"
[[ "$ACTIVE" == "$CHANNEL" ]] || die "active rustc $ACTIVE != pinned $CHANNEL (a RUSTUP_TOOLCHAIN override?)"

step "Python venv (.venv) for services/ and scripts/"
command -v python3 >/dev/null || die "python3 (>=3.10) is required"
[[ -d .venv ]] || python3 -m venv .venv
.venv/bin/pip install -q --upgrade pip
.venv/bin/pip install -q -r scripts/requirements.txt pytest

if command -v npm >/dev/null 2>&1; then
  step "Node dev dependencies (husky hooks)"
  npm ci --no-audit --no-fund
else
  echo "npm not found: skipping git hooks (optional)"
fi

step "Build contract (wasm32v1-none, release)"
cargo build --locked -p price-oracle --target wasm32v1-none --release

if [[ "$RUN_TESTS" == 1 ]]; then
  step "Contract tests"
  cargo test --locked -p price-oracle --lib
  step "Service tests"
  .venv/bin/python -m pytest services scripts/test_docs_freshness.py -q
fi

step "Done. Activate the venv with: source .venv/bin/activate"
