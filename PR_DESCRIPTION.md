# PR: Add indexer archives, explorer deployment, testnet bootstrap, and tracing

## Summary

- Stream canonical HTTPS JSONL archives into the persistent SQLite indexer; replayed events remain idempotent.
- Deploy the explorer to GitHub Pages with live per-asset price, source, freshness, and pause-state details.
- Add a configurable testnet bootstrap that checks for resets, redeploys, registers the configured assets and sources, and updates the explorer contract ID.
- Add Node and Python OpenTelemetry spans, W3C trace-context propagation, a local Jaeger Compose setup, and CI coverage for the Node indexer.

## Configuration and follow-up

The testnet workflow requires the documented Actions secrets and `ORACLE_TESTNET_ASSETS` repository variable. Price-submission workers must also be operated for continuous feeds; the 30-day feed acceptance must be observed after deployment. The archive input must be canonical event JSONL; other archive formats need conversion before ingestion.

## Validation

- `services/indexer`: 41 tests passed.
- `examples/indexer`: 2 tests passed.
- JavaScript and shell syntax, YAML parsing, and whitespace checks passed.
- Branch-specific GitHub Actions were not run before publishing this branch. Existing main-branch CI was failing at the time of review.

Closes #870
Closes #869
Closes #868
Closes #867
