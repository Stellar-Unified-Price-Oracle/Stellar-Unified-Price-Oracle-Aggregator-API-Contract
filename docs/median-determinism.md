# Deterministic median tie-breaking (#480)

This document is the **consumer reference** for the tie-break rule the
aggregator applies. If you need to reproduce a published price off-chain — to
verify it, to settle a dispute, or to reason about what a value *would* have
been — this page tells you exactly how to compute it.

## The rule in one paragraph

Sort the eligible submissions ascending. With `n` values:

- **`n` odd** — the value at index `n / 2` (0-based), exactly.
- **`n` even** — `lo + (hi - lo) / 2`, where `lo` is the value at index
  `n / 2 - 1` and `hi` the value at index `n / 2`. Integer division floors, so
  when the two central values differ by an odd number the result is the
  **lower** of the two.

An empty set yields `0`.

## Why it is specified at all

A median of an even-sized set has no single middle element. The two central
order statistics bracket the answer, and infinitely many functions of the pair
are equally valid medians. If the choice is left to implementation detail, the
published value stops being a function of the *set* of submissions and becomes a
function of their *arrival* — which would make it unreproducible and would let
the value move with no input having changed.

**The rule reads only the multiset of values.** It never consults:

- which source submitted which value,
- submission timestamps or ledger numbers,
- the order the host happened to iterate storage keys in,
- which values are duplicates of which.

So any permutation of the same set produces an identical aggregate. This is
asserted three ways in the test suite: a shuffled-input property test, an
exhaustive all-permutations test over a small set, and a test that submits the
same values in forward and reverse order through the real contract and compares
the published aggregates.

## Worked examples

| Eligible values (sorted)      | `n` | Central values | Result | Why |
|-------------------------------|-----|----------------|--------|-----|
| `1 2 3`                       | 3   | `2`            | `2`    | odd: exact middle |
| `1 2 3 4`                     | 4   | `2, 3`         | `2`    | `2 + 1/2` floors |
| `10 20 30 40`                 | 4   | `20, 30`       | `25`   | gap is even |
| `100 101`                     | 2   | `100, 101`     | `100`  | **lower-of-two** |
| `5 5`                         | 2   | `5, 5`         | `5`    | central values coincide |
| `10 20 20 100`                | 4   | `20, 20`       | `20`   | duplicates counted with multiplicity |
| `1 1 1 1 1 9`                 | 6   | `1, 1`         | `1`    | duplicates dominate the centre |
| `2 3`                         | 2   | `2, 3`         | `2`    | never rounds up |
| `-3 -1`                       | 2   | `-3, -1`       | `-2`   | `-3 + 2/2`, floor is toward `-inf` |

## Two details worth knowing

**It is not `(lo + hi) / 2`.** That form overflows `i128` for two large prices
of the same sign, and it rounds toward zero, which for a negative pair would
round *up* — the opposite of the documented tie-break. The contract computes
`lo + (hi - lo) / 2`, where `hi >= lo` guarantees the subtraction cannot
overflow.

**Duplicates are not deduplicated.** Two sources reporting the same price are
two observations, and they occupy two slots in the ordering. This is what makes
`10 20 20 100` a median of `20` rather than of `10` or `100`.

## Effect of removing a source

Because the rule depends only on the multiset, removing a source changes the
answer **only if** its value was one of the two central statistics. There is no
per-source term to drop, so a removal can never shift the value for some other
reason.

## Where this lives in the code

| Concern | Location |
|---------|----------|
| Normative statement of the rule | `contracts/price-oracle/src/median_determinism.rs` |
| On-chain implementation (hot path) | `crate::storage::compute_median` |
| Named tests for the cases above | `contracts/price-oracle/src/median_determinism_tests.rs` |
| Shuffled-input property test | `contracts/price-oracle/src/prop_tests.rs` |

`median_determinism::canonical_median` is a deliberately slow, obviously-correct
reference implementation. `storage::compute_median` is the one the contract
actually runs, using selection rather than sorting. A test asserts the two agree
on every documented case, and the property test asserts both match the rule
above on random input.
