# History Completeness Watermark (#712)

Module: `contracts/price-oracle/src/watermark.rs`

## Purpose

Provides a per-asset watermark indicating the ledger sequence number through
which historical price data is **complete and contiguous**. Consumers can use
this to know exactly which ledger range can be queried safely without running
into gaps.

## The problem it solves

`get_historical_price(asset, ledger)` and `get_historical_prices(asset, start, end)`
may return interpolated results (or panic with `NoData`) when ledgers in the
requested range have no recorded snapshot. Before this feature, a consumer had
no way to know — without trying every ledger — whether a range was complete or
had gaps.

The watermark makes that guarantee explicit and on-chain.

## API

| Endpoint | Description |
|---|---|
| `get_history_watermark(asset) → Option<CompletenessWatermark>` | Returns the stored watermark. `None` if no history exists yet. |
| `refresh_history_watermark(asset) → CompletenessWatermark` | Recomputes and stores the watermark from the current ledger index. |

### `CompletenessWatermark` struct

```rust
pub struct CompletenessWatermark {
    pub asset: Address,
    /// Highest ledger through which history is complete. 0 if none.
    pub watermark_ledger: u32,
    /// Unix timestamp of the watermark_ledger snapshot.
    pub watermark_timestamp: u64,
    /// Oldest retained snapshot ledger.
    pub first_ledger: u32,
    /// True when no gaps exist in [first_ledger, watermark_ledger].
    pub is_contiguous: bool,
    /// When is_contiguous = false: the first ledger of the lowest gap.
    pub gap_start_ledger: u32,
    /// Total snapshot count in the retained index.
    pub snapshot_count: u32,
}
```

## How the watermark is computed

The algorithm reads the `PriceHistoryLedgers(asset)` index — the sorted list of
ledgers for which a snapshot was written — and scans forward from the oldest
entry:

```
[100, 101, 102, 105, 106]
          ↑         ↑
    watermark=102  gap at 103
```

1. Determine the dominant snapshot stride (most common inter-snapshot distance).
2. Walk the sorted index. If the distance between two adjacent entries exceeds
   `stride × 2`, a gap is detected.
3. The watermark stays at the last entry before the gap; `gap_start_ledger` is
   set to the first missing ledger.

For dense per-ledger histories (stride = 1), any missing ledger is flagged.
For sampled histories (e.g. one snapshot per 10 ledgers), normal variation does
not create false gap alerts.

## Gap scenarios

| Cause | Effect |
|---|---|
| No submissions during a period | Watermark stops at last entry before the gap |
| History entry TTL expired | Same — the missing ledger appears as a gap |
| `max_history_length` pruned old entries | `first_ledger` advances; existing watermark may still be valid |
| New submissions resume | `refresh_history_watermark` will advance past the old gap if entries now exist |

## Consumer guidance

```
watermark_ledger > 0           → query [first_ledger, watermark_ledger] without gaps
is_contiguous = false          → gap detected at gap_start_ledger; above it is partial
watermark_ledger = 0           → no confirmed history; no safe range yet
```

Risk-sensitive consumers MUST treat any ledger above `watermark_ledger` as
potentially incomplete and either reject the query or handle the `NoData` error
explicitly.

## Limitations

- The watermark is a best-effort guarantee based on the on-chain index. If
  entries expired from Soroban temporary storage while still in the index, the
  gap is reported as missing but the root cause (TTL expiry) is indistinguishable
  from a never-submitted ledger.
- The watermark does not cover sharded bucket entries independently; it uses the
  unified `PriceHistoryLedgers` index which covers both storage paths.
