# Determinism and Interleaving in Multi-Call Ledgers (#516)

Suite: `contracts/price-oracle/src/interleaving_determinism_tests.rs`.
Run with `make interleaving` (`cargo test -p price-oracle --lib interleaving`).
CI runs it on every pull request (`.github/workflows/ci.yml`, job `conformance`).

Soroban allows several invocations per ledger, and a cross-contract callback
interleaves with the very call that triggered it. The contract must behave as if
each ledger had one well-defined outcome: **the final state may not depend on the
order in which independent operations land, and no partial state may ever be
observable between them.**

## What is covered

| Property | Test |
|---|---|
| All 24 permutations of four independent submissions in one ledger end in the same state | `permutations_of_independent_submissions_agree` |
| A mid-ledger read never observes a partial aggregate | asserted inside every permutation run |
| The median does not depend on the order the submissions landed in | `median_is_independent_of_submission_order` |
| A re-entrant callback cannot interleave with the aggregation that triggered it | `reentrant_callback_cannot_interleave_with_aggregation` |
| A rejected submission leaves nothing behind for the next call in the same ledger | `a_rejected_submission_leaves_no_partial_state` |

### The bounded operation set

Four sources submit `10, 20, 30, 40` for one asset in one ledger, with the
quorum set to four. The operations are *independent*: none of them can succeed
or fail because of another. All `4! = 24` orders are executed, and each run
compares a state fingerprint:

```
price=25 ts=1000000 last_updated=500 sources=4 version=1 agg_version=1
history_len=1 submitted=[10, 20, 30, 40]
```

`price` is the median of the four observations (`20 + (30-20)/2 = 25`),
`version` is the publication counter, `history_len` the number of history
records for the ledger. Everything that could differ between orders is in the
fingerprint, so an order-dependent median, a lost update or a double
publication fails the test.

### Mid-ledger reads

Each permutation run performs a `lastprice` read *between* submissions. With a
quorum of four, no aggregate may be served until the fourth submission lands, so
every intermediate read must return `None`: a partially aggregated median is
never observable. The last read must return the aggregate.

## Non-determinism from storage iteration

`median_is_independent_of_submission_order` replays the same ledger with the
submissions in forward and reverse order and asserts the same median. The
aggregation reads a `Submission(asset, source)` key per registered source; the
source set is a sorted `Vec` in storage, and nothing in the aggregation path
iterates a map. An implementation that derived the median from a storage
iteration order would produce a different answer for the two runs.

## Genuine ordering dependencies (modelled, not erased)

Some orderings *are* meaningful and are covered explicitly rather than hidden:

| Dependency | Where it is modelled |
|---|---|
| A submission cannot precede `add_source` / `register_asset` (rejected with `SourceNotFound` / `AssetNotRegistered`) | `a_rejected_submission_leaves_no_partial_state` |
| A submission cannot precede quorum (no aggregate is published) | every permutation run's mid-ledger read |
| A rejected submission is rolled back entirely | `a_rejected_submission_leaves_no_partial_state` |
| A rollback (`migrate_storage` resume) must follow a forward migration | `docs/version-matrix.md` |

## Re-entrancy

`reentrant_callback_cannot_interleave_with_aggregation` registers a consumer
contract as a price callback (#297). When the oracle pushes the new price, the
consumer immediately calls back into the oracle — first a read (`lastprice`),
then a write (`submit_price`). The result is the documented behaviour:

1. The re-entrant frame is aborted by the reentrancy guard, so the consumer's own
   state is rolled back (`seen() == 0`).
2. The callback failure is isolated: the oracle emits `cb_fail` and the
   aggregation result is still committed.
3. The aggregate is correct (`25`, four sources) and published exactly once
   (`version == 1`): the callback cannot interleave a second write into the same
   ledger.

## Adding an operation to the set

An operation belongs in the permutation set when it can succeed independently of
the others inside one ledger. Extend `OBSERVATIONS` and `OPS` together; the
`permutations_of_independent_submissions_agree` test asserts the expected
permutation count, so the suite fails loudly if the two drift apart or if the set
grows past a tractable size.
