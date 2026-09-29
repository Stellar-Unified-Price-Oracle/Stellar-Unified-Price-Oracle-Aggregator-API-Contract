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
#   mutation-gate       - mutation gate self-test, no cargo-mutants needed (#520)
#   mutation-per-module - per-module mutation scores and thresholds (#520)
#   hermetic           - hermetic integration harness, verifies determinism (#519)

.PHONY: all build test lint fmt check clean watch gas-gate load-test \
        mutation-gate mutation-per-module hermetic

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

# Verify the mutation gate's own logic (scoring, thresholds, waivers).
# Needs only bash + jq, so it runs in seconds and needs no cargo-mutants.
mutation-gate:
	./scripts/mutation-gate.sh self-test

# Per-module mutation scores and thresholds (requires cargo-mutants; slow).
mutation-per-module:
	./scripts/mutation-gate.sh per-module

# Hermetic integration harness (#519). Runs the cross-contract suite twice and
# fails if the runs disagree. No network or testnet required.
hermetic:
	./scripts/hermetic-integration.sh

# Run clippy linter
lint:
	cargo clippy -p price-oracle -- -D warnings

# Ops documentation gates: every paging alert has a runbook entry (#527),
# every postmortem is structured, indexed and owned (#528), and the capacity
# and backup docs match their code (#529, #530).
ops-check:
	python3 -m services.runbook.check_runbook
	python3 -m services.incident_review.check_postmortems
	python3 -m pytest services/runbook services/incident_review services/backup services/capacity -q

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
