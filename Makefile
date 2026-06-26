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

.PHONY: all build test lint fmt check clean watch

all: build test

# Compile the contract to wasm32v1-none release
build:
	cargo build -p price-oracle --target wasm32v1-none --release

# Run all unit tests
test:
	cargo test -p price-oracle --lib

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
