# Auditable price corrections (#486)

Errors happen: a typo in a source adapter, a bad value discovered hours after
it was published. Today there is no legitimate correction path, so operators
either leave wrong data live or mutate history opaquely. This module adds an
auditable one: a correction carries a **mandatory reason** and appends to an
**immutable revision chain** that links the corrected entry back to the
original and to every intervening change.

## Guarantees

* **Evidence is never destroyed.** Revision `0` is the original publication
  and is copied to its own storage slot; corrections only ever *append*. Both
  `get_original_price` and `get_price_revisions` stay queryable forever.
* **Authority is scoped, not general.** `correct_price` requires the admin
  (or a `PriceUpdater` delegate) and is bounded on three axes — see below.
* **Every correction is evented** with actor, reason, old value and new value
  in `PriceCorrectedEvent`, and appended to the admin audit trail under the
  `corr_px` action symbol.
* **Hard bounds still bind.** A correction value outside the asset's
  `[hard_min, hard_max]` tier is rejected with
  `ErrorCode::AggregateRejectedByBounds` (160) — a correction is not a way
  around [#484](price-bounds-tiers.md)'s hard limits.

## Using it

```rust
// Correct the published aggregate for `asset`.
let index = client.correct_price(&asset, &150i128, &String::from_str(&e, "typo in feed"));

// The full chain, oldest first.
let chain = client.get_price_revisions(&asset);
// chain[0]  -> the original publication, `corrected == false`
// chain[1]  -> the correction, `corrected == true`, with actor + reason

// The original value is still there, untouched.
assert_eq!(client.get_original_price(&asset), Some(100));
assert_eq!(client.get_price_revision(&asset, &1).unwrap().price, 150);
assert_eq!(client.get_correction_count(&asset), 1);
```

`PriceRevision` fields: `index`, `price`, `timestamp`, `ledger`, `actor`,
`reason`, `corrected`.

A correction is a *republication*, not a history rewrite: already-published
history entries are untouched, and the aggregate is republished with
`is_override = true` and a bumped `version`, so a consumer that cached the old
value can detect the change.

## Scope limits

`set_correction_scope` configures three independent limits; the values are
clamped to hard maxima so the scope can never become unbounded.

Enabling the scope is also what turns on revision-chain recording: while no
scope has ever been configured, the publication path skips the chain write
entirely. If a correction is filed before a scope was configured, revision 0
is seeded from the aggregate that is live at that moment — i.e. the erroneous
value itself — and is preserved from then on.

| Field | Meaning | Max |
|---|---|---|
| `assets` | Which assets a correction may target (`None` = any) | — |
| `window_ledgers` | The published aggregate must be younger than this | 17 280 (≈ 1 day) |
| `max_corrections` | Corrections an asset may accumulate | 32 |

| Violation | Error |
|---|---|
| Caller is not the admin / lacks `PriceUpdater` | `ErrorCode::NotAuthorized` |
| Asset not in the correction scope | `ErrorCode::NotAuthorized` |
| `reason` empty or longer than 256 characters | `ErrorCode::InvalidCorrectionReason` (158) |
| `new_price <= 0` | `ErrorCode::InvalidPrice` |
| `new_price` outside the hard bounds | `ErrorCode::AggregateRejectedByBounds` (160) |
| Aggregate older than `window_ledgers` | `ErrorCode::CorrectionWindowExpired` (161) |
| Correction cap reached | `ErrorCode::CorrectionLimitReached` (159) |
| No published aggregate to correct | `ErrorCode::NoData` |
| Requested revision index does not exist | `ErrorCode::RevisionNotFound` (162) |

## Event

`PriceCorrectedEvent` — topics `asset`, `actor`; fields `reason`, `old_price`,
`new_price`, `revision_index`, `ledger`, `affects_downstream`.

`affects_downstream` is `true` when the corrected value was live for more than
a single ledger before the correction, i.e. when downstream contracts may
already have consumed it. A monitor should alert on any correction, and
particularly on one with `affects_downstream == true`.

## Out of scope

Automated corrections driven by anomaly detection. Every correction here is an
explicit, human-attributed, on-chain act.
