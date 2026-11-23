# Indexer quickstart

Streams oracle contract events via RPC `getEvents` into JSON rows (pipe into your DB).

```bash
npm ci && npm test
CONTRACT_ID=C... npm start > events.jsonl
```

Key handling: indexing is read-only and needs no secret key. Do not give indexer hosts signing credentials.

To export OpenTelemetry traces to the local Jaeger UI, start the shared
observability stack and set the OTLP/HTTP endpoint:

```bash
docker compose -f ../../compose.observability.yml up -d
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318/v1/traces \
  CONTRACT_ID=C... npm start
```

Event spans include contract and ledger attributes and attach a W3C
`traceparent` to each output row for downstream indexers.
