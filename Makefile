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
#   attack-gate - attack-regression corpus (#511), fails on a time-budget breach
#   fuzz-gate  - committed fuzz corpus replay + crash gate (#514)

.PHONY: all build test lint fmt check clean watch gas-gate load-test attack-gate fuzz-gate kani-gate

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

# Attack-regression corpus (#511). Runs only the corpus module so it can be
# given an explicit wall-clock budget; the CI job fails if the budget is
# exceeded, which keeps the corpus fast enough to run on every PR.
attack-gate:
	./scripts/attack-regression-gate.sh

# Replay the committed, minimized fuzz corpora and fail on any crash (#514).
fuzz-gate:
	./scripts/fuzz-corpus-gate.sh

# Bounded formal proofs for the aggregation math (#512). Requires the Kani
# verifier; skipped with a clear message when it is not installed.
kani-gate:
	./scripts/kani-proofs.sh

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
