# Automatic Price Submission Bot — Design Document

This document describes the architecture and operational considerations for an off-chain bot
that submits prices to the Stellar Unified Price Oracle on a recurring schedule.

## Architecture Overview

```
┌──────────────┐     fetch      ┌─────────────────┐
│ Price Sources │ ──────────── ▶ │  Aggregator Bot  │
│ (CEX / DEX / │                │  (off-chain)     │
│  APIs)        │                │                  │
└──────────────┘                │  1. Fetch prices │
                                 │  2. Compute mid  │
                                 │  3. Sign & submit│
                                 └────────┬─────────┘
                                          │ submit_price()
                                          ▼
                                 ┌─────────────────┐
                                 │  Soroban Oracle  │
                                 │  Contract        │
                                 └─────────────────┘
```

The bot runs as a long-lived daemon (or via cron/scheduled task) and performs the following
steps on each cycle:

1. Fetch raw prices from one or more external sources.
2. Optionally compute a local mid-price to detect outliers before submitting.
3. Sign and submit the price via `submit_price(source, asset, price, timestamp)`.

## Source Configuration

Each bot instance acts as a **single registered oracle source**. Configuration is provided
through environment variables or a config file:

```toml
[oracle]
contract_id   = "CXXXX..."      # Oracle contract address
network       = "testnet"       # "testnet" | "mainnet"
rpc_url       = "https://soroban-testnet.stellar.org"

[source]
name          = "MyOracleBot"
secret_key    = "${SOURCE_SECRET_KEY}"   # Never hard-code; use env var

[assets]
# Map Stellar asset contract addresses to price feed identifiers
CBTC = "bitcoin"
CETH = "ethereum"

[schedule]
interval_secs = 60              # Submit every 60 seconds
```

## Price Data Sources

| Provider | Type | Notes |
|----------|------|-------|
| CoinGecko API | REST | Free tier; rate-limited |
| Binance REST API | REST | High reliability |
| Stellar DEX (Horizon) | REST | On-chain prices |
| Pyth Network | WebSocket | Low-latency push feed |

The bot should query at least two independent sources and take a median before submitting,
to reduce the impact of any single provider outage.

## Submission Scheduling

- Use an interval timer (e.g., `tokio::time::interval` in Rust or `setInterval` in Node.js).
- Compare the new price against the last submitted price; skip submission if the change is
  below a configurable **minimum deviation** (e.g., 0.1 %) to avoid unnecessary gas spend.
- Always submit if the elapsed time since the last submission exceeds the oracle's
  **heartbeat interval**, regardless of price movement.

## Error Handling and Retry Logic

```
submit_price()
  ├─ Success          → log, update last-submitted state
  ├─ Transient error  → exponential back-off, max 3 retries
  │    (network timeout, RPC unavailable)
  └─ Permanent error  → alert operator, skip this cycle
       (contract paused, source suspended, invalid price)
```

Retry delays: 5 s → 15 s → 45 s.  After three failures, skip the cycle and send an alert.

## Security Considerations

- **Key management**: Store the source secret key in a secrets manager (AWS Secrets Manager,
  HashiCorp Vault, or an encrypted `.env` file). Never commit keys to version control.
- **Key rotation**: Support rotating the source keypair by registering the new address with
  `add_source` before decommissioning the old one.
- **Firewall**: Restrict outbound connections to the Soroban RPC endpoint and the price feed
  APIs only.
- **Replay protection**: The oracle contract validates timestamps; ensure the bot's system
  clock is NTP-synchronised.
- **Rate limiting**: Respect exchange API rate limits to avoid IP bans.

## Monitoring and Alerting

- Export Prometheus metrics: `oracle_submissions_total`, `oracle_submission_errors_total`,
  `oracle_last_price{asset}`, `oracle_last_submission_timestamp{asset}`.
- Alert when:
  - No successful submission in the last `2 × interval_secs` seconds.
  - Price deviation between sources exceeds 5 %.
  - Bot process restarts unexpectedly.
- See `docs/monitoring/README.md` for the Grafana dashboard JSON.

## Deployment

```bash
# Build (Rust example)
cargo build --release --bin oracle-bot

# Run with environment variables
SOURCE_SECRET_KEY=S... ./target/release/oracle-bot --config config.toml
```

For production, run the bot as a systemd service or inside a Docker container with a
restart policy of `always`.
