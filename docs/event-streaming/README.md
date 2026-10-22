# Event Streaming to External Databases

This document describes how to stream oracle contract events to PostgreSQL and ClickHouse for analytics and historical queries.

## Overview

The oracle contract emits structured events for price updates, source management, and asset registration. An off-chain listener service can subscribe to these events and persist them to external databases.

## Schema Migrations

### PostgreSQL

Run the following SQL to create the `oracle_events` table:

```sql
CREATE TABLE IF NOT EXISTS oracle_events (
    id BIGSERIAL PRIMARY KEY,
    ledger INT NOT NULL,
    timestamp BIGINT NOT NULL,
    contract_id TEXT NOT NULL,
    topic TEXT NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_oracle_events_ledger ON oracle_events(ledger);
CREATE INDEX IF NOT EXISTS idx_oracle_events_topic ON oracle_events(topic);
CREATE INDEX IF NOT EXISTS idx_oracle_events_timestamp ON oracle_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_oracle_events_data ON oracle_events USING GIN(data);
```

### ClickHouse

Run the following SQL to create the `oracle_events` table:

```sql
CREATE TABLE IF NOT EXISTS oracle_events (
    ledger UInt32,
    timestamp UInt64,
    contract_id String,
    topic String,
    data String,
    created_at DateTime DEFAULT now()
) ENGINE = MergeTree()
ORDER BY (ledger, topic)
```

## Event Envelope

Each event is wrapped in a canonical envelope:

```json
{
  "ledger": 12345,
  "timestamp": 67890,
  "contract_id": "ContractId...",
  "topic": "price_updated",
  "data": {
    "asset": "G...",
    "new_price": 1000000,
    "old_price": 990000,
    "decimals": 18
  }
}
```

## Setup

1. Configure the event sink connection strings in your listener service.
2. Run the schema migrations against PostgreSQL and/or ClickHouse.
3. Start the listener service to begin streaming events.

## References

- `contracts/price-oracle/src/event_streaming.rs` — off-chain reference implementation
- `contracts/price-oracle/src/events.rs` — on-chain event definitions
