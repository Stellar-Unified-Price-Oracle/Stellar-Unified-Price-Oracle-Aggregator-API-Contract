-- ClickHouse schema migration for oracle events
-- Run once during setup.

CREATE TABLE IF NOT EXISTS oracle_events (
    ledger UInt32,
    timestamp UInt64,
    contract_id String,
    topic String,
    data String,
    created_at DateTime DEFAULT now()
) ENGINE = MergeTree()
ORDER BY (ledger, topic)
