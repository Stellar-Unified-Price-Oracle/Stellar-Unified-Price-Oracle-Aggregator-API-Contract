# Testnet Track Record: 30-Day Uptime and Accuracy Criteria

**Status:** Active | Issue: #882

Before promotional release or mainnet migration, the Stellar Unified Price Oracle must demonstrate an uninterrupted 30-day track record of operational stability on Stellar Testnet.

## Acceptance Thresholds

| Metric | Target | Measurement Window | Action on Violation |
|--------|--------|--------------------|---------------------|
| **Oracle Uptime** | ≥ 99.9% | Rolling 30 calendar days | Counter resets to 0 days |
| **Feed Staleness** | < 60 seconds | 99th percentile across all feeds | Investigation drill triggered |
| **Price Accuracy** | < 0.25% deviation | vs. multi-exchange reference median | Feed marked degraded |
| **Quorum Health** | 100% valid signatures | Continuous audit of round consensus | Relayer failover test |

## Verification Procedures

1. **Synthetic Telemetry**: Automated continuous probes submit heartbeat queries every 10 seconds.
2. **Consensus Audits**: Daily aggregation of submitted round signatures against on-chain transaction logs.
3. **Dispute Replays**: Mock dispute simulations executed weekly to verify on-chain resolution pathways.
