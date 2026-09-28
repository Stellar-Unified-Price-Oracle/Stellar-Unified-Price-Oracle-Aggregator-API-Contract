# Hermetic Integration Harness

One command, no network:

```bash
./scripts/hermetic-integration.sh
```

It brings up the full local topology, runs the cross-contract integration
suite, replays it, and fails if the two runs disagree. Everything runs
in-process against deterministic fixtures, so it is as repeatable as the unit
tests — which is the point: integration coverage that used to require a shared
testnet (flaky, slow, not reproducible) now runs on every change.

## What it spins up

`contracts/price-oracle/src/hermetic_harness.rs` builds the topology:

| Component | Role |
|---|---|
| `PriceOracleContract` | the aggregator under test |
| `MockConsumer` | SEP-40 style consumer, reading through the standard interface |
| `IndependentOracle` | a second, unrelated contract — makes these tests cross-contract rather than self-talk |
| payment token | available via the shared test helpers |

`contracts/price-oracle/src/hermetic_integration_tests.rs` drives it: multi-source
aggregation, SEP-40 consumption, quorum behaviour, aggregation-method selection,
multi-asset independence, and the fail-closed paths.

## The three properties it guarantees

**Hermetic.** No network, no testnet, no clock, no randomness. The ledger is set
explicitly and every identity derives from a fixed per-`Env` counter, so a
scenario replayed in a fresh `Env` reproduces the same addresses.

**State is reset between tests.** Each scenario constructs its own harness, and
storage is namespaced per contract, so nothing carries over. This is asserted
rather than assumed: `fresh_harness_is_clean` checks a new harness has no
sources, assets or prices, and `harnesses_are_isolated_from_each_other` checks
that populating one harness leaves another untouched.

**Deterministic across runs.** The harness script runs the suite twice and
requires byte-identical per-test outcomes, so a flaky scenario fails the gate
instead of being averaged away. `--repeat N` deepens the check;
`scenario_replay_is_deterministic` covers repetition within a single process.
An empty outcome extraction is treated as an error, never as a pass — otherwise
the comparison would silently compare nothing.

## Artifacts on failure

`hermetic-artifacts/` (override with `ARTIFACT_DIR`):

| File | Contents |
|---|---|
| `run-N.log` | full test output per run |
| `outcomes-N.txt` | the normalised per-test outcomes that were compared |
| `determinism-N.diff` | the diff, when two runs disagree |
| `summary.md` | pass/fail table and the determinism verdict |
| `test_snapshots/` | Soroban ledger snapshots, when the SDK wrote any |

The script exits non-zero on any failure or disagreement, so it works directly
as a CI gate.

## Known divergences from the real network

This is a mock, and a mock is more forgiving than a real network. These are the
places where a green harness run does **not** prove production behaviour. This
harness complements the testnet lifecycle job; it does not replace it.

| Area | Harness | Real network |
|---|---|---|
| **Auth** | `mock_all_auths` satisfies every `require_auth`, and `mock_all_auths_allowing_non_root_auth` covers bundled cross-contract calls. Nothing is actually signed. | Real signatures, real nonces, real failure modes — a forged or replayed authorization is rejected by cryptography, not by a mock. |
| **Gas / fees** | No fee metering, no resource limits. A call that is far too expensive still passes. | Fees and the ledger footprint limits apply; an expensive call can fail outright. `docs/gas-budget.md` covers the gates for this. |
| **Ledger closure** | Ledgers advance only when a test says so. | Ledgers close on a schedule, with real finality and ordering. |
| **TTL / rent** | Temporary entries expire on schedule, but **persistent entries are not archived by the test host** — advancing the ledger leaves them readable. #522 models the post-expiry state explicitly. | Persistent entries are archived and restored on access, then genuinely gone once the rent period lapses. |
| **Concurrency** | Single-threaded; no interleaving between transactions. | Concurrent submissions race. Ordering, duplicate suppression and the replay nonce are only meaningfully exercised on a real network. |
| **WASM deployment** | The contract is registered natively, not uploaded as WASM. | Deployment goes through `installContract` with upload costs, size limits and a WASM VM. |
| **Protocol version** | Fixed at 26 via `LedgerInfo`. | The network upgrades its protocol version; a contract that works on 26 may need changes for the next. |
| **Events** | Recorded and inspectable, but not delivered to any subscriber. | Event delivery, ordering and durability are real, and indexers depend on them. |

The practical rule: use the harness for **logic** — aggregation, quorum,
fail-closed paths, interface conformance — and keep the testnet lifecycle job for
everything that depends on real signatures, real fees, real ordering, or real
archival.

## Running the pieces individually

```bash
# Just the cross-contract scenarios
cargo test -p price-oracle --lib hermetic_integration_tests

# Single pass, skip the determinism replay
./scripts/hermetic-integration.sh --once

# Deeper determinism check
./scripts/hermetic-integration.sh --repeat 3
```
