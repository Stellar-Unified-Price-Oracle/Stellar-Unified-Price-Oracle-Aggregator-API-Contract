#!/usr/bin/env bash
# devcontainer-tools.sh — pre-install the CI tooling a fresh Codespace needs (#846).
#
# Runs after ./scripts/dev-setup.sh --no-test in the devcontainer. Every version
# is pinned to the one CI uses, so `make check` and the gates behave the same
# locally as on a runner:
#   cargo-nextest, cargo-mutants, cargo-deny  (cargo install, --locked)
#   stellar-cli 26.1.0                        (prebuilt binary, as in
#                                              .github/workflows/testnet-lifecycle.yml)
#   Kani                                      (prebuilt release bundle)
#   jq                                        (used by the gate scripts)
#
# Idempotent: an already-installed tool at the pinned version is left alone.
# Set SKIP_KANI=1 to skip the (large) Kani download.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CARGO_NEXTEST_VERSION="0.9.85"
CARGO_MUTANTS_VERSION="25.3.1"
CARGO_DENY_VERSION="0.18.3"
STELLAR_CLI_VERSION="26.1.0"
KANI_VERSION="0.56.0"

step() { printf '\n==> %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

step "System packages (jq, build essentials)"
if ! have jq; then
  if have sudo; then
    sudo apt-get update -qq
    sudo apt-get install -y -qq jq build-essential pkg-config libssl-dev
  else
    echo "devcontainer-tools: jq missing and no sudo; install it manually" >&2
  fi
fi

step "cargo-nextest ${CARGO_NEXTEST_VERSION}"
if ! have cargo-nextest; then
  cargo install --locked --version "${CARGO_NEXTEST_VERSION}" cargo-nextest
fi

step "cargo-mutants ${CARGO_MUTANTS_VERSION}"
if ! have cargo-mutants; then
  cargo install --locked --version "${CARGO_MUTANTS_VERSION}" cargo-mutants
fi

step "cargo-deny ${CARGO_DENY_VERSION}"
if ! have cargo-deny; then
  cargo install --locked --version "${CARGO_DENY_VERSION}" cargo-deny
fi

step "stellar-cli ${STELLAR_CLI_VERSION}"
if ! have stellar; then
  tmp="$(mktemp -d)"
  curl -fsSL \
    "https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_CLI_VERSION}/stellar-cli-${STELLAR_CLI_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
    | tar -xz -C "$tmp"
  if [[ -w /usr/local/bin ]]; then
    install -m 0755 "$tmp/stellar" /usr/local/bin/stellar
  else
    sudo install -m 0755 "$tmp/stellar" /usr/local/bin/stellar
  fi
  rm -rf "$tmp"
fi
stellar --version

if [[ "${SKIP_KANI:-0}" == "1" ]]; then
  step "Kani skipped (SKIP_KANI=1)"
else
  step "Kani ${KANI_VERSION}"
  if ! have cargo-kani; then
    tmp="$(mktemp -d)"
    curl -fsSL \
      "https://github.com/model-checking/kani/releases/download/kani-${KANI_VERSION}/kani-${KANI_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
      | tar -xz -C "$tmp"
    ( cd "$tmp/kani-${KANI_VERSION}-x86_64-unknown-linux-gnu" && ./install.sh )
    rm -rf "$tmp"
  fi
  cargo kani --version
fi

step "Done. Tooling matches CI; run 'make check' and the gates directly."
