# OHLCV Aggregation

The oracle exposes hourly/daily-style OHLCV (open/high/low/close) candles derived
from its indexed price-history snapshots, so charting and backtesting tools don't
need to replay raw per-ledger entries themselves.

## Endpoint

```
get_ohlcv(asset: Address, bucket_seconds: u64, from_ts: u64, to_ts: u64) -> Vec<OhlcvBar>
```

- `asset` — a registered asset address.
- `bucket_seconds` — bucket width in seconds. Common values: `3600` (hourly),
  `86400` (daily). Must be `> 0`.
- `from_ts` / `to_ts` — inclusive Unix-timestamp range to cover. `to_ts` must be
  `>= from_ts`.

Buckets are aligned with `bucket_start = (timestamp / bucket_seconds) *
bucket_seconds`, so a 3600-second bucket always starts on the hour.

Returns up to `MAX_OHLCV_BARS` (500) bars, ordered chronologically:

```rust
pub struct OhlcvBar {
    pub bucket_start: u64, // Unix timestamp, start of this bucket
    pub open: i128,
    pub high: i128,
    pub low: i128,
    pub close: i128,
    pub sample_count: u32, // number of price-history snapshots in this bar
}
```

`sample_count` reflects how many aggregated price-history snapshots fell in the
bucket — the oracle does not track trade volume, so this stands in for a
liquidity/activity signal rather than a token volume figure.

### Errors

- `AssetNotRegistered` — `asset` is not registered.
- `InvalidOhlcvRange` — `bucket_seconds == 0`, or `to_ts < from_ts`.

## Caching

A bucket is only immutable once it has fully elapsed — the bucket containing the
current ledger time may still receive new price-history snapshots. `get_ohlcv`
therefore:

1. Scans the asset's price-history once, grouping snapshots into bars.
2. Write-through caches every bar whose bucket has **already closed**
   (`bucket_start + bucket_seconds <= now`) under a dedicated cache key.
3. Always recomputes the in-progress bucket rather than caching it.

Once a closed bar has been cached, it can be read directly via:

```
get_cached_ohlcv_bar(asset: Address, bucket_seconds: u64, bucket_start: u64) -> Option<OhlcvBar>
```

which is an O(1) storage read — no history rescan — making repeated reads of
historical (already-closed) candles cheap after the first `get_ohlcv` call that
covered them. `get_ohlcv` is the entry point that populates the cache; callers
that only need one specific historical bar and know it has already been computed
can call `get_cached_ohlcv_bar` directly.

## Example: hourly candles for the last day

```rust
let day_ago = now - 86_400;
let bars = client.get_ohlcv(&asset, &3_600u64, &day_ago, &now);
```
