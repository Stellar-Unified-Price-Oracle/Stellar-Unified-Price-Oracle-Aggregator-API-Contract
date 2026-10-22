# Oracle contract build & verification helpers
#
# Contract verification checklist (issue #612)
# --------------------------------------------
# Operators use these targets to reproduce, from a clean checkout and without
# privileged local state or production credentials, that the source published
# to explorers matches the WASM deployed on mainnet.
#
# Prerequisites (all public):
#   - git, curl, sha256sum
#   - Rust toolchain pinned in rust-toolchain.toml (rustup will honor it)
#   - soroban-cli (cargo install --locked soroban-cli)
#   - stellar-cli / network RPC endpoint for the target network
#
# Checklist (run in order, record outputs in the release ticket):
#   1. make verify-toolchain   -> rustc/cargo/soroban versions match the release
#   2. make verify-lockfile    -> Cargo.lock is committed and unmodified
#   3. make verify-build       -> deterministic WASM build + sha256 of artifacts
#   4. make verify-onchain     -> fetch deployed WASM hash per contract id
#   5. Compare step 3 and step 4 hashes; they MUST be identical.
#   6. Confirm the explorer shows the same source commit + compiler inputs.
#
# A second operator can repeat steps 1-6 verbatim; no secrets are required.

SHELL := /bin/bash

# Network + contract ids are supplied by the operator, never committed.
NETWORK ?= mainnet
RPC_URL ?= https://soroban-rpc.mainnet.stellar.gateway.fm
CONTRACTS ?=

WASM_DIR := target/wasm32-unknown-unknown/release
BUILD_FLAGS := --locked --release --target wasm32-unknown-unknown

.PHONY: build verify-toolchain verify-lockfile verify-build verify-onchain verify

build:
	cargo build $(BUILD_FLAGS)

# Step 1: record the exact toolchain used for the release build.
verify-toolchain:
	@echo "== toolchain =="
	@rustc --version
	@cargo --version
	@soroban version 2>/dev/null || stellar version 2>/dev/null || echo "soroban/stellar cli not found"
	@echo "Compare the above against the versions recorded in the release notes."

# Step 2: the lockfile must be committed and unchanged for reproducibility.
verify-lockfile:
	@echo "== lockfile =="
	@test -f Cargo.lock || (echo "Cargo.lock missing" && exit 1)
	@git diff --exit-code -- Cargo.lock || (echo "Cargo.lock has uncommitted changes" && exit 1)
	@sha256sum Cargo.lock

# Step 3: deterministic build and hash of every produced WASM artifact.
verify-build: verify-lockfile
	cargo build $(BUILD_FLAGS)
	@echo "== wasm sha256 =="
	@for f in $(WASM_DIR)/*.wasm; do sha256sum "$$f"; done

# Step 4: fetch the deployed WASM hash for each contract id from the network.
# CONTRACTS is a space-separated list of contract ids supplied by the operator.
verify-onchain:
	@test -n "$(CONTRACTS)" || (echo "set CONTRACTS=\"<id> [<id>...]\"" && exit 1)
	@echo "== on-chain wasm hash (network=$(NETWORK)) =="
	@for id in $(CONTRACTS); do \
		echo "contract $$id"; \
		soroban contract fetch --id "$$id" --network "$(NETWORK)" --rpc-url "$(RPC_URL)" --out /tmp/$$id.wasm 2>/dev/null \
			|| stellar contract fetch --id "$$id" --network "$(NETWORK)" --rpc-url "$(RPC_URL)" --out /tmp/$$id.wasm; \
		sha256sum /tmp/$$id.wasm; \
	done

# Convenience: run the reproducible portion of the checklist.
verify: verify-toolchain verify-build
