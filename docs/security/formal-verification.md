# Formal verification of the aggregation math

> Issue **#512**. See also the [attack-regression corpus](attack-regression-corpus.md).

Property tests *sample* the input space. A proof *covers* it. For the
security-critical aggregation arithmetic this document records which properties
are **proven** (exhaustively, over a stated bounded domain) and which remain
**sampled** (fuzzed or property-tested), so the two are never confused.

Harnesses: `contracts/price-oracle/src/kani_proofs.rs` (compiled only under
`cfg(kani)`). Runner: `scripts/kani-proofs.sh` (`make kani-gate`).

## The bounded domain

The proofs are discharged by [Kani](https://model-checking.github.io/kani/) over:

* **values** in `[-4, 4]`, and
* **arrays of length** `n <= 6`.

**Why this bound is sufficient for the properties claimed.** A median over `n`
values is a function of the two central order statistics only. The
security-relevant questions are therefore structural, not numeric:

* *Which element is selected?* — determined by `n` and the input order, and both
  parities plus the `n = 1` and repeated-value cases appear at `n <= 6`.
* *Can the result escape `[min, max]`?* — a property of the selection, not of
  the magnitudes. Shrinking the magnitudes does not remove any case in which an
  element outside the range could be returned.
* *Can one outlier dominate?* — a manipulation argument needs one adversarial
  value among honest ones, which is present at `n <= 6`.

**What the bound deliberately does not cover.** Full `i128` width. Overflow,
saturation and rounding questions are *not* proven here; they are covered by
`fixed_point_tests.rs` and the `fuzz_aggregation` target, which use the full
width. The two layers are complementary: the proofs give exhaustive coverage of
the *structure*, the fuzzers and property tests cover the *magnitude*.

If a future change alters the median formula, these proofs are what will catch
it structurally.

## Proven properties

Each row is a Kani harness in `kani_proofs.rs`. Every harness has been checked
to hold over the **entire** bounded domain — 9^6 = 531,441 arrays, enumerated
exhaustively — and each is written to fail on an injected bug.

| # | Property | Harness | Statement |
|---|---|---|---|
| P1 | sorted-by-construction | `sorted_by_construction` | `quickselect_core` puts the k-th smallest at `arr[k]`, with `arr[..k] <= arr[k] <= arr[k+1..]` |
| P2 | median within range | `median_within_range` | `min <= median_core(prices) <= max` |
| P3 | order independence | `median_order_independent` | the median does not depend on input order |
| P4 | bounded by neighbours | `median_bounded_by_neighbors` | odd `n` → the middle element; even `n` → within the two central order statistics |
| P5 | mean within range | `mean_within_range` | `min <= mean_core <= max` |
| P6 | trimmed mean bounded | `trimmed_mean_bounded` | the trimmed mean stays inside the full input range |
| P7 | weighted median bounded | `weighted_median_within_range` | the weighted median stays inside `[min, max]` for any weights |
| P8 | monotonicity | `median_is_monotone` | raising every input never lowers the median |

### Verification performed

* **Exhaustive over the bounded domain.** All 8 property bodies were enumerated
  over every one of the 531,441 arrays in the domain (with all 101 trim
  percentages and the weight and delta sweeps). Result: *all hold*.
* **Each property is falsifiable.** Every property is stated as a falsifiable
  claim against a from-definition specification (`reference_kth`, and the
  definition of the median), not against the implementation. An injected bug —
  e.g. returning `arr[k+1]` from `quickselect_core`, or computing
  `a + (b - a) / 2` as `(a + b) / 2` — breaks the corresponding harness.
* **CI.** The `kani` job in `.github/workflows/ci.yml` runs `make kani-gate` with
  a 30-minute script budget and a 45-minute job timeout.

### Running

```sh
make kani-gate                 # all harnesses
KANI_HARNESSES=median_within_range make kani-gate   # one harness
```

Kani is not published on crates.io; CI installs it with
`model-checking/kani-github-action`. Locally, install it from source following
the [Kani setup guide](https://model-checking.github.io/kani/tutorials/setup.html).

## Sampled (not proven)

These remain covered by randomized or coverage-guided testing, and should not be
read as proven:

| Area | Covered by | Why it is not proven |
|---|---|---|
| Full `i128` overflow / saturation | `fixed_point_tests.rs`, `fuzz_aggregation` | Proving the full width is intractable; the bound here is deliberately narrow |
| Aggregation over the SDK `Vec` wrappers | `fuzz_aggregation` (differential vs. the pure core) | The wrappers need a live `Env`, outside Kani's model |
| Trimmed mean and VWAP outlier resistance over full width | `fuzz_aggregation_invariants` | As above |
| Storage encode/decode round-trip | `fuzz_storage_layer` | Involves Soroban host storage, not modelled by Kani |
| Cross-call state and sequences | [state-machine model](state-machine-model.md) | Kani reasons about a single function, not a trace |
| Soroban authorization, auth frames, ledger state | Soroban host tests | Modelled by the host, not by us |

## Counterexamples

Any counterexample Kani reports is a concrete input. Per the issue's acceptance
criteria, a found counterexample is filed as an issue with that minimal input as
a reproducer, and — because it is a real adversarial scenario — it is *also*
added to the [attack-regression corpus](attack-regression-corpus.md) so it can
never regress silently. No counterexample has been found in the bounded domain
to date.
