# TTL and Rent Boundary Reference

Operational companion to `contracts/price-oracle/src/ttl_boundary_tests.rs`
(#522). That suite proves the contract **fails closed** at every storage
boundary; this document says what each failure means and what to do about it.

## The retention rule

An entry written at ledger `L` with a minimum TTL of `T` is readable through
ledger `L + T - 1` and evicted **at** ledger `L + T`. Verified empirically
against the Soroban test host: with `T = 100`, an entry written at ledger 1 is
present at 100 and gone at 101.

Two tiers behave differently, and the difference matters operationally:

| Tier | In the test host | In production | What you lose |
|---|---|---|---|
| **Temporary** | Expired by the host when the ledger advances | Expired by the network | History, rate-limit counters |
| **Persistent** | *Not* expired — stays readable | Archived, restored on access; gone once the rent period lapses | Sources, assets, aggregates, config |

The contract's own extension policy is `LEDGER_THRESHOLD` (10 000 ledgers) and
`LEDGER_BUMP` (40 000 ledgers) — see `storage.rs`. Hot paths call
`extend_ttl` on every read, so a regularly-polled asset is effectively
self-renewing; a dormant one is not.

## Key classes

Every key class below has a boundary test in the suite.

| Class | Tier | Keys |
|---|---|---|
| Admin / config | persistent | `Admin`, `CfgMinSources`, `CfgDecimals`, … |
| Source registry | persistent | `SrcActive`, `SrcRegistry` |
| Asset registry | persistent | `AssetRegistered`, `AssetRegistryIndex`, `AssetRegistry` |
| Submission | persistent | `Submission(asset, source)` |
| Aggregate | persistent | `Aggregate(asset)` |
| History | temporary | `PriceHistory(asset, ledger)` |

## Failure modes and the response to each

These are the distinct errors an expired key produces. They are deliberately
different codes so an operator can tell them apart from the error alone.

| What expired | Error | Meaning | Response |
|---|---|---|---|
| `Aggregate(asset)` | `NoData` (8) | The published price is gone. The asset is still registered. | Do **not** substitute a cached or default price. Treat as "no price available": sources must resubmit; the next qualifying submission republishes. Check whether the asset is still being polled — a dormant asset loses its aggregate first. |
| `AssetRegistered` / `AssetRegistryIndex` | `AssetNotRegistered` (2) | The asset's registry entry is gone, so *every* price path rejects it. | Re-register the asset (`register_asset`) and have sources resubmit. Check the TTL extension job covers asset registry keys. |
| `SrcActive(addr)` | `NotAuthorized` (0) | The source is no longer on the allow-list; its submissions are rejected. | Re-add the source (`add_source`) if it is still trusted. A source that stops submitting may also have been auto-deactivated — distinguish eviction from suspension before re-adding. |
| `CfgMinSources` and other required config | `ConfigMissing` (156) | A required configuration value has been evicted. | Re-set the configuration. Until then the affected calls fail closed; they do not fall back to a default quorum. |
| `PriceHistory(asset, ledger)` | *no error* — reads as absent | History aged out of temporary storage. This is expected, not a fault. | No action. If history must survive, it has to be exported off-chain (`export_history`) before the window closes. |

### The one that is not an error

An expired **history** entry returns "absent" rather than an error, because
history is explicitly best-effort and bounded by `max_history`. Every other
class above fails closed with a distinct error. The suite asserts both
properties, and asserts that the error codes are pairwise distinct.

## Verifying the boundaries yourself

```bash
# The boundary suite on its own
cargo test -p price-oracle --lib ttl_boundary

# The full suite
make test
```

Adding a new key class means adding it to `KeyClass` **and**
`KEY_CLASSES`/`covered_classes` in the suite — the completeness test fails
otherwise, so a class cannot skip boundary testing unnoticed.
