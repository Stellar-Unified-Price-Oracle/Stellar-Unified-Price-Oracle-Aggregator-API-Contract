# Market Impact / Slippage Analytics

Estimates the market impact (slippage) of trading a given size of one asset for
another, using the depth of the Soroswap-style AMM pools already registered via
`register_soroswap_pool` (`amm.rs`, issue #281). Lending protocols, DEXes, and
other consumers combine this with the oracle's spot price to judge whether a
trade size is safe to execute against on-chain liquidity, without reimplementing
constant-product math themselves.

## API

```rust
estimate_market_impact(asset_in: Address, asset_out: Address, amount_in: i128) -> MarketImpactEstimate

get_market_impact_curve(asset_in: Address, asset_out: Address, sizes: Vec<i128>) -> Vec<ImpactCurvePoint>
```

`estimate_market_impact` returns:

```rust
pub struct MarketImpactEstimate {
    pub asset_in: Address,
    pub asset_out: Address,
    pub amount_in: i128,
    pub amount_out: i128,       // after pool fee + slippage
    pub spot_price: i128,       // reserve_out / reserve_in, scaled 1e18
    pub execution_price: i128,  // amount_out / amount_in, scaled 1e18
    pub price_impact_bps: u32,  // deviation of execution vs. spot price
}
```

`get_market_impact_curve` computes impact at several trade sizes in one call —
pass explicit `sizes` (units of `asset_in`), or pass an empty vector to use the
default curve: 1%, 5%, 10%, 25%, and 50% of the pool's `asset_in` reserve. Sizes
that are non-positive or would drain the pool are skipped rather than failing
the whole call, so callers get a best-effort curve.

Both functions look up the registered Soroswap pool in either registration
order (`(asset_in, asset_out)` or `(asset_out, asset_in)`), so callers don't need
to know which order the pool was registered in.

## Model

For a pool with reserves `(reserve_in, reserve_out)` and fee `fee_bps` (read from
the pool config):

```text
amount_in_after_fee = amount_in * (10_000 - fee_bps) / 10_000
amount_out          = reserve_out - k / (reserve_in + amount_in_after_fee)
spot_price          = reserve_out * 1e18 / reserve_in
execution_price     = amount_out * 1e18 / amount_in
price_impact_bps    = |execution_price - spot_price| * 10_000 / spot_price
```

This mirrors the swap pricing in `amm::swap` but is purely read-only: no
reserves are mutated and no tokens change hands.

## Errors

| Error | Cause |
|---|---|
| `PoolNotFound` | No enabled Soroswap pool registered for the pair. |
| `InvalidTradeSize` | `amount_in <= 0`, or `amount_in >= reserve_in` (would drain the pool). |
