# AI Agent Guide — Stellar Unified Price Oracle Aggregator

> **Note for AI agents:** This file is your primary onboarding document. Read it fully before making any changes.

## Project Overview

A Soroban (Stellar smart contract) **price oracle aggregator** that collects price data from multiple permissioned oracle sources, aggregates via **median**, and exposes price data (current + historical) for consumer contracts via SEP-40.

- **Language:** Rust, compiled to WASM (`wasm32v1-none`)
- **SDK:** soroban-sdk v26
- **Testing:** `#[cfg(test)]` with `soroban-sdk/testutils`
- **Entrypoint:** `contracts/price-oracle/src/lib.rs`

## Repository Structure

```
.
├── .github/workflows/ci.yml    # CI: contract build, fmt, clippy, tests
├── .husky/pre-push              # Pre-push hook: fmt + clippy + nextest fast tier
├── AGENTS.md                    # This file
├── Cargo.toml                   # Workspace root
├── Cargo.lock
├── Makefile                     # build / test / lint / fmt / check / clean
├── package.json                 # npm scripts + husky devDep
├── package-lock.json
├── rust-toolchain.toml          # Rust channel + wasm32v1-none target
├── contracts/
│   └── price-oracle/
│       ├── .cargo/config.toml   # WASM link flags
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs           # Contract struct + all 27 endpoints
│           ├── types.rs         # DataKey, ErrorCode, all structs
│           ├── storage.rs       # Storage helpers + median computation
│           ├── admin.rs         # Admin management functions
│           ├── assets.rs        # Asset registration and management
│           ├── errors.rs        # Error types and handling
│           ├── events.rs        # 13 event types + 2 emit helpers
│           ├── history.rs       # Price history management
│           ├── prices.rs        # Price submission and aggregation
│           ├── sources.rs       # Oracle source management
│           ├── test.rs          # 71 unit tests
│           ├── test_helpers.rs  # Shared test utilities
│           └── prop_tests.rs    # 5 property-based tests
```

### Domains

| Domain | Where | What lives there |
|---|---|---|
| **Contract — price oracle** | `contracts/price-oracle/src/` | The production aggregator: entrypoints (`lib.rs`), storage keys and types (`types.rs`), storage/median helpers (`storage.rs`), admin, assets, sources, price submission, history, events, errors, plus unit/property tests. |
| **Contract — minimal oracle** | `contracts/minimal-oracle/src/` | The stripped-down reference contract used for onboarding and conformance checks. |
| **Fuzzing** | `fuzz/src/` | `cargo-fuzz` targets for the aggregation and decoding paths. |
| **Tooling / DevX** | `scripts/` | Build, gate, interface-generation and doc-freshness scripts (`scripts/devx/`). |
| **Docs** | `docs/` | Design notes, ADRs (`docs/adr/`), runbooks, security reviews, error registry (`docs/errors/`), migrations, monitoring config. |
| **CI & hooks** | `.github/`, `.husky/`, `Makefile` | Workflows, issue/PR templates, pre-commit/pre-push hooks, and the `make` targets that wrap the gates. |
| **Config** | `config/` | Security allowlists, baselines, governance policy registry, audit findings. |

### Inside the price-oracle domain

Read the modules in this order; each one only depends on the ones above it:

1. `types.rs` — `DataKey` (storage keys), `ErrorCode`, and the data structs.
2. `storage.rs` — `read_*`/`write_*` helpers, `get_admin`, `check_source`,
   `check_registered_asset`, and median computation.
3. `events.rs` — event types and the manual publish helpers.
4. `admin.rs`, `assets.rs`, `sources.rs`, `prices.rs`, `history.rs` — one module
   per domain operation; each owns its auth checks and emits its events.
5. `lib.rs` — the `#[contractimpl]` block that exposes the public endpoints and
   the SEP-40 interface, delegating to the modules above.
6. `test.rs`, `test_helpers.rs`, `prop_tests.rs` — tests, shared fixtures, and
   property-based tests.

Do not hard-code endpoint, event, or test counts in this file; the generator
reports them from the source.

## What NOT to Push

- **Never commit** files under `target/`, `node_modules/`, `test_snapshots/`, or `.kiro/` (gitignored)
- **Never commit** API keys, secrets, or identity files
- **Never commit** `.env` files or wallet keypairs
- **Never commit** WASM binaries (`*.wasm` is gitignored)

## What to Push

- Rust source code (`.rs` files under `contracts/`)
- `Cargo.toml` / `Cargo.lock` (lockfile for deterministic builds)
- `package.json` / `package-lock.json`
- `AGENTS.md`, `CONTRIBUTING.md`, `README.md`, `LICENSE`
- `.github/` workflows, issue templates, PR templates
- `.husky/` hooks and config
- `.gitignore`

## How to Verify Task Completion

Before marking any task complete, run the full verification suite:

```bash
# 1. Build the contract
cargo build -p price-oracle --target wasm32v1-none --release

# 2. Run all tests
cargo test -p price-oracle --lib

# 3. Check for lint/style issues
cargo clippy -p price-oracle -- -D warnings
cargo fmt --manifest-path contracts/price-oracle/Cargo.toml -- --check

# 4. Run the security gates (each has its own time budget)
make attack-gate   # #511 attack-regression corpus
make fuzz-gate     # #514 corpus size + replay
make kani-gate     # #512 bounded proofs (needs the Kani verifier)
```

All tests must pass with zero compiler warnings and zero clippy warnings.

### Security gates

Each gate is budgeted separately, because a security suite that cannot run on
every PR is a suite that stops being run:

| Gate | Issue | What it proves |
|---|---|---|
| `make attack-gate` | #511 | every historical attack is pinned as a test, by attack class, within a wall-clock budget |
| `./scripts/attack-regression-mutation-check.sh` | #511 | the corpus actually **fails** when a historical fix is reverted |
| `make kani-gate` | #512 | the aggregation properties hold over the whole bounded domain |
| `cargo test -p price-oracle --lib model_state_machine` | #513 | an independent model matches the contract after every step of a random trace |
| `make fuzz-gate` | #514 | the committed corpora are within budget and replay without crashing |

See `docs/security/attack-regression-corpus.md` for how to add a new attack to
the corpus.

## Key Constraints

- `#![no_std]` — no standard library available in contract code
- `wasm32v1-none` target — only `soroban-sdk` and `core` dependencies
- Storage uses `env.storage().persistent()` and `env.storage().temporary()`
- Admin functions require `admin.require_auth()`
- Source submit functions require `source.require_auth()`
