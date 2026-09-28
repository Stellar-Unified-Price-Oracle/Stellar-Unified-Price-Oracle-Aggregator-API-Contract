# Data Provenance for Published Aggregates — #493

A published price was an assertion: nothing on chain said *why* it was that value. This feature
records, for every published aggregate, the exact submissions, sources, weights and reference
value that produced it, so a consumer can prove a price rather than trust it — the basis for
dispute resolution and audit.

Code: `contracts/price-oracle/src/provenance.rs`. Type: `types::ProvenanceRecord`. Queries:
`get_provenance(asset, ledger)`, `verify_provenance(asset, ledger)`.

## The record

One record per published aggregate, keyed by `(asset, ledger)`:

| Field | Meaning |
|---|---|
| `id` | the record's SHA-256 commitment; the stable identifier emitted in `ProvenanceRecordedEvent` |
| `asset`, `ledger`, `timestamp` | what was published, where and when |
| `price` | the published aggregate price |
| `reference` | the unweighted median of the counted prices — the value a weighted aggregate was checked against |
| `num_sources`, `method` | contributing count and the aggregation method in force (0..=4) |
| `contributors` | one `ProvenanceEntry` per contributing submission: source, price, `weight_bps`, `submission_ledger` |
| `deferral_ledgers` | the round's publication deferral per contributor, aligned with `contributors` |
| `previous_hash` | hash of the previous record of the same asset, or 32 zero bytes for the first |
| `hash` | SHA-256 over all of the above and `previous_hash` |

`reference` and `method` are what make a weighted aggregate explainable: a method-4 aggregate is a
weighted median, and without the unweighted median it could not be checked against the value its
sources actually reported.

`contributors` describes exactly the submissions that survived the robust pre-filter (#491) and
the deviation filter, so provenance and the published price can never disagree about who
contributed.

## Integrity: a per-asset hash chain

`hash` is `SHA-256(domain_separator ‖ previous_hash ‖ XDR(asset fields) ‖ contributors…)`, and
`previous_hash` is the `hash` of the previous record **of the same asset**. Records therefore form
a per-asset chain, and each successor commits to its predecessor.

`verify_provenance(asset, ledger)` returns `true` only when both hold:

1. **Internal consistency** — recomputing the commitment over the record's stored fields
   reproduces its `hash`, and `id == hash`. Any edited field fails here.
2. **Chain linkage** — the record commits to the hash its predecessor actually carries. A record
   that was replaced or back-dated fails here, because the successor (or the chain head for the
   latest record) no longer matches.

The two checks are separate on purpose: a forged record can be internally consistent, so
consistency alone is not proof of authenticity. Chain linkage is what makes retroactive forgery
detectable, and it is why provenance cannot be quietly edited to explain away a bad price.

The domain separator (`SUPRO1`) is mixed into every hash so a provenance commitment can never
collide with a hash produced by another module over the same field bytes.

## Corrections and source removal

Provenance is written **per publication**, not per price. A correction — a new round, a
`set_override`, a re-aggregation after new submissions — produces a *new* record at a *new*
ledger, with its own contributors, chained to its predecessor. Earlier records are never rewritten,
so "what did we publish, and why" stays answerable across a correction, and both the old and the
new explanation remain retrievable.

Removing a source does not rewrite history either. Past records still name it, with the weight it
held at the time, which is the point: a dispute is about what was known when, not about the
current source set.

## Pruning and storage cost

Records live at `DataKey::Provenance(asset, ledger)` and are removed by the **same** history-pruning
loop that drops the asset's `PriceHistory` entry for that ledger. Consequences:

- one record per stored history entry, so provenance storage is bounded by the configured
  `MaxHistoryLength` (default 10 per asset) and cannot grow independently;
- a record can never outlive the price it explains — no orphaned provenance, no unbounded growth;
- after pruning, `get_provenance` for the pruned ledger reverts to its documented `NoData` error,
  which is the same answer a consumer already gets for a pruned price.

Per-record cost is the record itself: one XDR blob holding `num_sources` entries of
(source, price, weight, submission ledger, deferral) plus the fixed header. Storage is measured by
the test `provenance_storage_is_bounded_by_history`, which asserts one record exists per retained
history entry after a run of publications and pruning rounds.

## Consumer guidance

1. Read the aggregate, then call `get_provenance(asset, ledger)` for the ledger it names.
2. Call `verify_provenance` before relying on the record. An unverified record is a claim; a
   verified one is a commitment the oracle cannot walk back.
3. Recompute the aggregate from `contributors` if you need certainty: sort
   `contributors[].price` and take the median for methods 0–2, or apply the weights for method 4
   and compare with `price`. `reference` gives you the unweighted median directly.
4. If the ledger is older than the retained history, the record is gone along with the price. That
   is the retention policy working as designed, not a missing-data bug.

## Storage cost

A published round writes two entries per asset: the record itself, keyed by `(asset, ledger)`,
and a single chain-head entry holding the head's hash and ledger. The head deliberately keeps both
facts in one key — a separate key for the ledger alone is a third entry per asset, and per-round
entry count is what decides whether a wide batch stays inside the network's 100-entry footprint
cap. Records are pruned by exactly the same policy as the `PriceHistory` entry they explain, under
both the global and the per-asset cap, so the retained set is bounded by the same window that
bounds price history.

`provenance_storage_is_bounded_by_the_history_window` measures this: after 19 rounds with a
3-entry history window, exactly 3 provenance records remain.
