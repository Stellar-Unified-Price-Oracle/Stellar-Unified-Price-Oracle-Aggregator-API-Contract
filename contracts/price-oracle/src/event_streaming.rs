//! # Event Streaming to External Databases (#284)
//!
//! Provides off-chain reference implementations for streaming contract events
//! to PostgreSQL and ClickHouse for analytics and historical queries.
//!
//! This module contains no on-chain logic; it documents the expected event
//! schema and provides a reference listener implementation.

use serde::{Deserialize, Serialize};

/// External database sink configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EventSinkConfig {
    /// PostgreSQL connection string (e.g. `postgres://user:pass@host:5432/db`).
    pub postgres_url: String,
    /// ClickHouse connection string (optional).
    pub clickhouse_url: Option<String>,
    /// Number of events to buffer before flushing.
    pub batch_size: u32,
    /// Flush interval in milliseconds.
    pub flush_interval_ms: u64,
}

/// Canonical event envelope emitted by the oracle contract.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OracleEventEnvelope {
    /// Ledger sequence number when the event was emitted.
    pub ledger: u32,
    /// Unix timestamp of the ledger close.
    pub timestamp: u64,
    /// Contract address that emitted the event.
    pub contract_id: String,
    /// Event topic (first indexed field).
    pub topic: String,
    /// Serialized event data.
    pub data: serde_json::Value,
}

impl OracleEventEnvelope {
    /// Creates a new event envelope.
    pub fn new(
        ledger: u32,
        timestamp: u64,
        contract_id: String,
        topic: String,
        data: serde_json::Value,
    ) -> Self {
        Self {
            ledger,
            timestamp,
            contract_id,
            topic,
            data,
        }
    }
}

/// PostgreSQL schema migration (run once during setup).
pub const POSTGRES_MIGRATION_SQL: &str = r#"
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
"#;

/// ClickHouse schema migration (run once during setup).
pub const CLICKHOUSE_MIGRATION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS oracle_events (
    ledger UInt32,
    timestamp UInt64,
    contract_id String,
    topic String,
    data String,
    created_at DateTime DEFAULT now()
) ENGINE = MergeTree()
ORDER BY (ledger, topic)
"#;
