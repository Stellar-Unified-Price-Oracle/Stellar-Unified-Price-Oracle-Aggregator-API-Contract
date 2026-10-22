# Oracle contract build & operational tooling
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
#   source-count-bench - worst-case configured source-count benchmark (#693)
#   endpoint-budgets   - per-endpoint compute and storage budget gate (#692)
#   retention-bench    - worst-case history retention benchmark (#694)
#   burst-capacity     - ledger burst submission capacity test (#695)
#   mutation-gate       - mutation gate self-test, no cargo-mutants needed (#520)
#   mutation-per-module - per-module mutation scores and thresholds (#520)
#   hermetic           - hermetic integration harness, verifies determinism (#519)
#   rpc-gate           - RPC response trust boundary fixture corpus (#684)
#   indexer-trust      - indexer authenticity + gap-safe cursor suite (#685)
#   tx-review-gate     - privileged transaction tamper corpus (#686)
#   redaction-gate     - incident evidence redaction, fail-closed (#687)
#   trust-boundary     - all four off-chain trust boundaries above
#   sast               - SAST + advisory + unsafe-baseline gate (#500)
#   secret-scan        - secret scan of the tree, with a history report (#502)
#   check-pins         - every dependency pinned exactly, lockfile in sync (#501)
#   reproducible-build - two clean builds must produce identical WASM (#501)
#   dr-drill           - disaster recovery restoration evidence drill (#620)

# Determinism & Interleaving Suite (#516)
#
# SEP-40 conformance (#515), event-schema golden snapshots (#518) and the
# N-2..N upgrade matrix (#517) also have their own targets. All of them are
# plain `cargo test` filters over the contract's test binary.
.PHONY: all build test lint fmt check clean watch gas-gate load-test \
        mutation-gate mutation-per-module hermetic sast secret-scan \
        check-pins reproducible-build security rpc-gate indexer-trust \
        tx-review-gate redaction-gate trust-boundary source-count-bench \
        endpoint-budgets retention-bench burst-capacity

all: build test

# Compile the contract to wasm32v1-none release
build:
	cargo build --target wasm32-unknown-unknown --release

test:
	cargo test -p price-oracle --lib

# Adversarial gas-budget regression gates (docs/gas-budget.md)
gas-gate:
	cargo test -p price-oracle --lib gas_budget_tests -- --nocapture

# Adversarial load test v2 (docs/gas-usage.md)
load-test:
	cargo test -p price-oracle --lib load_v2 -- --nocapture --test-threads=1

# Worst-case configured source-count benchmark (docs/source-count-benchmark.md)
source-count-bench:
	cargo test -p price-oracle --lib source_count_bench -- --nocapture --test-threads=1

# Per-endpoint compute and storage budget gate (docs/endpoint-budgets.md)
endpoint-budgets:
	cargo test -p price-oracle --lib endpoint_budget -- --nocapture

# Worst-case history retention benchmark (docs/history-retention-benchmark.md)
retention-bench:
	cargo test -p price-oracle --lib retention_bench -- --nocapture --test-threads=1

# Ledger burst submission capacity test (docs/ledger-burst-capacity.md)
burst-capacity:
	cargo test -p price-oracle --lib burst_capacity -- --nocapture --test-threads=1

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

# Static analysis gate: source-level SAST, the unsafe/finding baseline, and
# dependency advisories (cargo-audit). Writes a report for the run artifacts.
sast:
	python3 -m services.static_analysis.gate --report security-artifacts/static-analysis.md

# Secret scanning: working tree + full history, redacted report.
secret-scan:
	python3 -m services.secret_scan.scanner . --history --report secret-artifacts/secret-scan.md

# Dependency pinning: exact requirements, committed + hashed lockfile, no drift.
check-pins:
	python3 -m services.pinned_deps.pin_check .

# Byte-reproducibility: two clean builds, compared digests.
reproducible-build:
	./scripts/reproducible-build.sh

# Everything the three hardening issues gate on, plus their test suites.
security: sast secret-scan check-pins
	python3 -m pytest services/static_analysis services/secret_scan services/pinned_deps -q

# #684: replay the RPC response fixture corpus. Fails if a documented
# rejection class stops being rejected, or a valid response stops being accepted.
rpc-gate:
	python3 -m services.rpc_guard.gate --report security-artifacts/rpc-boundary.md

# #685: spoofed events, ledger continuity and cursor safety.
indexer-trust:
	python3 -m pytest services/indexer -q

# #686: privileged transaction review; every tampered payload must be refused.
tx-review-gate:
	python3 -m services.tx_review.gate replay --report security-artifacts/tx-review.md

# #687: incident evidence redaction, including the fail-closed paths.
redaction-gate:
	python3 -m services.evidence_redaction.gate --report security-artifacts/redaction.md

# All four off-chain trust boundaries, plus their unit tests.
trust-boundary: rpc-gate indexer-trust tx-review-gate redaction-gate
	python3 -m pytest services/rpc_guard services/indexer services/tx_review \
		services/evidence_redaction -q

# #620: disaster recovery restoration evidence drill. Restores a
# production-shaped off-chain state backup into an isolated environment and
# records completeness checks (checksums, schema version, reconciled ledger
# height) before declaring the restore successful.
dr-drill:
	python3 -m services.dr_drill.gate --report dr-artifacts/restore-drill.md

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

clean:
	cargo clean
	rm -f $(BENCH_CURRENT)
