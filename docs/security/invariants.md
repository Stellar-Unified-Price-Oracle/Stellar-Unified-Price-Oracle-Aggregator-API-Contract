# Invariant Catalogue (#471)

Harness: `contracts/price-oracle/src/invariant_harness_tests.rs` (runs in CI as part of
`cargo test -p price-oracle --lib`).

## Related verification layers

This catalogue checks invariants during hostile sequences. It sits alongside
three other layers, each of which catches what the others cannot:

| Layer | Doc | Catches |
|---|---|---|
| Attack-regression corpus (#511) | [attack-regression-corpus.md](attack-regression-corpus.md) | a *specific* historical attack silently returning |
| Bounded formal proofs (#512) | [formal-verification.md](formal-verification.md) | arithmetic violations across the whole bounded domain |
| Model-based state machine (#513) | [state-machine-model.md](state-machine-model.md) | interactions across *sequences* of public calls |
| Coverage-guided fuzzing (#514) | [fuzzing.md](fuzzing.md) | deep, structurally-unexpected inputs |

A new invariant should normally be entered into this catalogue **and** the
attack-regression corpus.

A seeded hostile generator (xorshift64*) drives the public API with source
admission/removal, asset registration, time advance, and submissions with extreme,
zero, and negative prices. Safety invariants are asserted after **every** step;
liveness invariants are checked once the sequence settles. Every run prints how
often each invariant was *exercised* (non-vacuously checked); an invariant with zero
exercises fails the test as a coverage gap. Failing sequences are minimized by
one-at-a-time delta debugging.

## Catalogue

| Id | Kind | Invariant | Rationale | Owner | Assertion | Status |
|---|---|---|---|---|---|---|
| S1 | Safety | On-chain source registry equals admitted-minus-removed set, no duplicates | No removed source can influence aggregates | sources.rs | `INV_REGISTRY` | Checked |
| S2 | Safety | Asset registry equals the set of successful registrations | Prices exist only for known assets | assets.rs | `INV_ASSETS` | Checked |
| S3 | Safety | An unregistered or removed source cannot submit | Permissioned feed | prices.rs | `INV_REJECT_UNREGISTERED` | Checked |
| S4 | Safety | A non-override aggregate lies within [min, max] of accepted submissions | Median derivable from source set | storage.rs | `INV_AGG_BOUNDED` | Checked |
| S5 | Safety | Aggregate timestamp never decreases | A price never becomes less final | prices.rs | `INV_AGG_MONOTONIC` | Checked |
| S6 | Safety | A non-override aggregate has `num_sources >= min_sources` | Quorum never satisfied without quorum | prices.rs | `INV_QUORUM` | Checked |
| L1 | Liveness | If `min_sources` currently-registered sources submitted accepted prices, a price is eventually readable | Oracle must not stall | prices.rs | `INV_LIVE` | Checked |
| A1 | Safety | Aggregate equals the exact median of the *eligible* (current, fresh) set | Stronger form of S4 | prices.rs | — | **Assumed** (S4 is the checked relaxation) |
| A2 | Safety | Staked/treasury balances are conserved across slash/unstake | Funds safety | reputation.rs | covered by existing staking tests, not by this harness | **Assumed** here |
| A3 | Safety | Only admin can mutate configuration | Access control | admin.rs | auth mocked in harness | **Assumed** here (covered by auth tests) |

## Harness self-test

`injected_violation_is_detected_and_minimized` corrupts the model's handling of
`RemoveSource`; the harness detects an S1 violation and minimizes a 47-step sequence
to a ≤ 4-step reproducer ending in `RemoveSource`.

## Violations found

None in the contract. During development the harness reported an L1 counterexample
(`[RegisterAsset, AddSource a, AddSource b, Submit b, RemoveSource b, Submit a]`) that
turned out to be a model error — a removed source's submission correctly no longer
counts towards quorum; the model was fixed accordingly.

## Coverage (12 seeds × 45 steps)

S1 540 · S2 540 · S3 91 · S4 651 · S5 651 · S6 651 · L1 22. Gaps tracked: A1–A3 above.
