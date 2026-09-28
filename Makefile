# Makefile for Stellar Unified Price Oracle Aggregator
#
# Targets:
#   all     - build and test
#   build   - compile the contract to WASM
#   test    - run all tests
#   lint    - run clippy
#   fmt     - format code
#   check   - check formatting without modifying files
#   clean   - remove build artifacts
#   gas-gate  - adversarial gas-budget regression gates (#419)
#   load-test - adversarial load test v2 (#413)
#   soak      - sustained soak rig, latency/memory/state ceilings (#523)
#   services  - off-chain service + ops harness tests (python)
#   ci-audit  - OIDC trust-policy + no-long-lived-secret audit (#524)
#   failover-drill - multi-region ingest failure drill (#526)

.PHONY: all build test lint fmt check clean watch gas-gate load-test soak leak-drill \
        services ci-audit failover-drill provenance-verify

# Sustained soak rig: realistic + adversarial mix over many rounds, asserting
# latency, memory and state-growth ceilings. Prints a report; non-zero exit on
# any breach. See docs/soak-rig.md.
soak:
	python -m services.soak.rig --rounds $${ROUNDS:-2000}

# The leak drill: the deliberately leaky state model MUST breach a threshold.
# A zero exit here means the rig is not actually detecting unbounded growth.
leak-drill:
	python -m services.soak.rig --rounds 1500 --model leaky --expect-breach

# Off-chain services and ops harnesses (soak, SLA monitor, failover, CI policy,
# provenance). These are pure-Python and independent of the Rust toolchain.
services:
	python -m pytest services -q

# OIDC trust-policy scoping + proof that no long-lived deploy secret remains in
# the repository or its CI configuration. See docs/secretless-ci.md.
ci-audit:
	python -m services.ci_policy.policy

# Multi-region ingest failover drill: kills a region mid-run and asserts the
# router recovers with zero double submissions. See docs/multi-region-ingest.md.
failover-drill:
	python -m services.ingest_failover.router --victim $${VICTIM:-us-east}

# Verify a release's provenance against a freshly built artifact.
#   make provenance-verify BUNDLE=provenance.json
provenance-verify:
	@test -n "$(BUNDLE)" || { echo "usage: make provenance-verify BUNDLE=provenance.json"; exit 2; }
	python -m services.provenance.attest verify \
	  --bundle "$(BUNDLE)" \
	  --artifact target/wasm32v1-none/release/price_oracle.wasm

all: build test

# Compile the contract to wasm32v1-none release
build:
	cargo build -p price-oracle --target wasm32v1-none --release

# Run all unit tests
test:
	cargo test -p price-oracle --lib

# Adversarial gas-budget regression gates (docs/gas-budget.md)
gas-gate:
	cargo test -p price-oracle --lib gas_budget_tests -- --nocapture

# Adversarial load test v2 (docs/gas-usage.md)
load-test:
	cargo test -p price-oracle --lib load_v2 -- --nocapture --test-threads=1

# Run clippy linter
lint:
	cargo clippy -p price-oracle -- -D warnings

# Format source code
fmt:
	cargo fmt --manifest-path contracts/price-oracle/Cargo.toml

# Check formatting without modifying files
check:
	cargo fmt --manifest-path contracts/price-oracle/Cargo.toml -- --check

# Watch for changes and re-run cargo check + tests
# Requires: cargo install cargo-watch
watch:
	cargo watch -x "check -p price-oracle" -x "test -p price-oracle --lib" -x "clippy -p price-oracle -- -D warnings"

# Remove build artifacts
clean:
	cargo clean
