-- PostgreSQL schema migration for oracle events
-- Run once during setup.

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
