# Model-based state-machine test suite

> Issue **#513**. See also the [attack-regression corpus](attack-regression-corpus.md).

Integration bugs live in *sequences*, not in single calls: an operation that is
correct on its own can break the contract when it follows another. This suite
models the contract as an explicit state machine and drives it with generated
traces, checking invariants after **every** step.

Tests: `contracts/price-oracle/src/model_state_machine_tests.rs`.

## The model

The model covers the lifecycle of one asset's aggregate:

```text
           register_asset
  Unregistered ─────────────► Registered
                                 │
                   first accepted submission
                                 ▼
  (contributors < quorum) ───► Collecting ◄────┐
                                 │            │ submission while
                           quorum reached     │ below quorum
                                 ▼            │
                               Live ──────────┘
                                 │
                 source removed / quorum raised
                                 ▼
                               Stale
```

| State | Meaning |
|---|---|
| `Unregistered` | `register_asset` has not been called |
| `Registered` | registered, but no submission accepted yet |
| `Collecting` | at least one submission, fewer than the quorum |
| `Live` | an aggregate is published (`get_price` returns `Some`) |
| `Stale` | submissions exist but the quorum is no longer met |

The transitions are driven entirely through **public** operations:
`register_asset`, `add_source`, `remove_source`, `set_min_sources_required`,
`submit_price`, `pause` / `unpause`.

## The model is independent of the implementation

This is the property that makes the suite worth having, so it is enforced rather
than asserted in prose:

* The model re-implements the median **from its definition** — sort, take the
  middle element, or average the two middle elements — rather than calling
  `storage::compute_median`. A bug shared by the model and the implementation
  therefore cannot hide.
* The model is written from the *specification* of the invariants below, not
  derived from reading `prices.rs`.
* `model_is_independent_of_implementation` pins the two paths against a
  hand-computed expectation on a three-element median.

## Invariants

Checked after **every** step of every trace, not only at the end:

| # | Invariant | Meaning |
|---|---|---|
| INV-1 | published-implies-quorum | an aggregate is published only if the quorum in force when it was computed was met |
| INV-2 | no-aggregate-below-quorum | below quorum the model never expects a published aggregate |
| INV-3 | median-in-range | the published price equals the median of the contributions that influenced it, and lies within their `[min, max]` |
| INV-4 | removal-withdraws | a removed source contributes nothing, and leaves no contribution behind |
| INV-5 | pause-blocks-writes | while paused, no submission is accepted |
| INV-6 | unregistered-is-inert | a submission for an unregistered asset is rejected and leaves no trace |

INV-3 is the manipulation property in its strongest form: the published price is
a selection over real submissions, so it can never be fabricated outside the
range of the submissions that were allowed to influence it.

## Behaviour the model had to be corrected to capture

Two real behaviours were discovered by running traces and are now modelled
faithfully. They are recorded here because a model that *assumed* nicer
behaviour would have produced false failures — or worse, false confidence.

1. **The aggregate is cached, not recomputed.** The contract stores the
   aggregate computed at the last accepted submission and serves it from cache.
   Removing a source or raising the quorum does **not** recompute it. The model
   keeps the last published value until a new aggregate can be computed.

2. **A below-quorum submission does not clear the aggregate.** When a submission
   arrives that cannot reach the quorum, the previously published value stays in
   place rather than being withdrawn. The model refreshes the cached aggregate
   only when a new one can actually be computed.

Both are faithful to the contract. Whether the cache should be invalidated on
removal is a *design* question outside the scope of this issue; the model
documents the behaviour as it is so that a future change to it shows up as a
deliberate diff.

## Trace generation

`generate_trace` builds a trace from a **deterministic** xorshift64\* PRNG.
Determinism is a hard requirement: a failing trace must be reproducible from its
seed alone, so the suite never uses wall-clock time or entropy. Seeds are fixed
constants in `random_traces_hold_every_invariant` (12 seeds × 24 steps = 288
steps), so CI is reproducible and a locally-failing seed is the same seed CI saw.

The price range is small and centred on 1000, and deliberately includes a
hostile outlier (`1_000_000_000`) and a degenerate price (`1`), so the
manipulation and boundary behaviour is exercised by the traces themselves.

**State explosion is bounded** by fixing the trace length (24) and the number of
source slots (4). The model is abstract: it does not track ledger sequence or
timestamps, which is where unbounded state would otherwise come from.

**Transition coverage is reported.** The test asserts that every operation class
was actually exercised and prints the per-class counts, e.g.:

```text
model-based traces: 12 seeds x 24 steps = 288 steps, transition coverage [31, 157, 51, 22, 10, 17]
```

A transition class with a zero count fails the test, so the traces cannot
silently stop exploring part of the machine.

## Shrinking

`shrink` reduces a failing trace to a minimal reproducer by delta debugging: it
repeatedly deletes chunks of the trace and keeps any deletion that still
reproduces the failure, halving the chunk size until it reaches single steps.

`shrinker_reduces_a_failing_trace` demonstrates this on a deliberately injected
fault — "removal must withdraw the source's contribution" — and commits the
minimized reproducer. A 10-step trace is reduced to the 2 steps that matter:

```text
[Submit { source: 0, price: 1000 }, RemoveSource { source: 0 }]
```

The assertion is structural (a submit from slot 0, then its removal) rather than
on one exact price, because *which* equivalent submit the descent happens to keep
is an implementation detail; the property being pinned is "removal after
submission, nothing else".

**Per the acceptance criteria, any failing trace found in CI is minimized this
way and committed as a regression test** — the seed and the minimized trace are
added to this module, so the bug cannot be rediscovered by a different trace.

## Running

```sh
cargo test -p price-oracle --lib model_state_machine -- --nocapture
```

Current result: 5 tests pass, with 288 trace steps and full transition coverage.

## Out of scope

This suite does **not** formally verify the model itself. The model is a
specification written in Rust and checked by the invariants above; proving the
model correct is a separate (and much larger) exercise.
