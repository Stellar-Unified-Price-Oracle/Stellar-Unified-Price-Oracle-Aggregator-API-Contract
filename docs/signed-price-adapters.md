# Signed CEX / Aggregator Price Adapters

Off-chain adapter connectors that feed reference prices from centralized
exchanges and price aggregators (CoinGecko, Binance, Coinbase) into the
oracle through the existing signed-submission path
([`submit_price_with_proof`](../contracts/price-oracle/src/signed_submission.rs),
#216) instead of a plain `require_auth` transaction. The registered
source's Ed25519 key signs each price observation off-chain; any address
may then relay the resulting call on-chain, so the source itself never has
to hold gas or submit a transaction directly.

## Why signed submission instead of `submit_price`

[`scripts/price-submission-bot.py`](../scripts/price-submission-bot.py)
already submits prices via the source's own Soroban-authorized
transaction. This adapter is a separate, purpose-built connector for
first-party CEX/aggregator feeds that:

* signs each observation with a long-lived Ed25519 key instead of
  requiring the source's Stellar account to co-sign every submission, and
* can be relayed by an unrelated, disposable "gas payer" identity —
  useful for running the signer offline/air-gapped from the relaying
  infrastructure.

## Provider failover

Each asset lists its providers in priority order (`providers` in the
config). On every cycle the adapter tries them in order and uses the
**first** one that returns a usable price:

```
CoinGecko (primary) → Binance (fallback) → Coinbase (fallback)
```

A provider is skipped, and the next one tried, when:

* the HTTP request fails, times out, or returns an unparseable response
  (network blip, rate limit, provider outage), or
* the provider's own reported observation time is older than
  `max_staleness_secs` (currently only CoinGecko's
  `include_last_updated_at` exposes this — see below).

If every configured provider fails or is stale, the adapter **skips the
submission cycle entirely** rather than resubmitting a cached price under
a fresh timestamp, which would misrepresent how current the data is. This
is logged and counted in `no_price_available_total`; a monitoring stack
should alert on this metric being nonzero for longer than the asset's
normal update cadence.

## Staleness handling

* **CoinGecko** exposes `last_updated_at` (Unix seconds) per asset via
  `include_last_updated_at=true`. The adapter rejects a CoinGecko price
  older than `max_staleness_secs` and fails over to the next provider,
  even though the HTTP call itself succeeded.
* **Binance** and **Coinbase**'s spot endpoints don't expose an
  observation timestamp; the adapter treats a successful response as
  current (request time = observation time), which is reasonable for a
  live ticker/spot endpoint queried on a short interval.
* Independently of any single provider, `submit_price_with_proof`'s own
  `timestamp` argument is the adapter's wall-clock time at submission, and
  the contract separately rejects timestamps too far in the future
  (`CfgTimestampThreshold`) — this adapter policy only prevents *silently
  relaying old data as fresh*, it does not replace that on-chain check.

## Replay protection & signed proof expiry

* Each source's `nonce` is tracked locally in `state.nonce_file` (JSON) so
  it survives adapter restarts and strictly increases, matching the
  contract's `nonce > last_accepted_nonce` requirement.
* `expiration_ledger` is computed each cycle as `current ledger +
  expiration_window_ledgers`, bounding how long a relayer has to land the
  transaction before the proof goes stale on-chain.

## One-time source setup

1. Register the source (admin, once): `add_source(source_address, name)`.
2. Generate a 32-byte Ed25519 seed and store it as a hex string in the
   environment variable named by `ed25519_seed_env`:
   ```bash
   export ADAPTER_ED25519_SEED=$(python3 -c "import secrets; print(secrets.token_hex(32))")
   ```
3. Register the corresponding public key on-chain (source-authorized
   transaction, run once):
   ```bash
   python3 scripts/signed_price_adapter.py --config signed_adapter_config.toml --register-key
   ```

## Running

```bash
pip install -r scripts/requirements.txt
export ADAPTER_ED25519_SEED=...
python3 scripts/signed_price_adapter.py --config signed_adapter_config.toml
```

Health and metrics are exposed at `GET /health` on `health.port`
(`{"status": "ok", "metrics": {...}}`), tracking per-provider fetch
errors, stale-skip counts, failover events, and submission counts —
see [`scripts/signed_adapter_config.example.toml`](../scripts/signed_adapter_config.example.toml)
for the full configuration schema.

## Adding another provider

Add an entry to `PROVIDER_FETCHERS` in
[`scripts/signed_price_adapter.py`](../scripts/signed_price_adapter.py)
returning `(price: float, observed_at: int | None)`, then reference it in
an asset's `providers`/`feed_ids` config. Return `None` for `observed_at`
if the provider doesn't expose an update timestamp — the adapter falls
back to treating the request time as the observation time.
