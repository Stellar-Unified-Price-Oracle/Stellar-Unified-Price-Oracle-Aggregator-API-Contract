# Indexer quickstart

Streams oracle contract events via RPC `getEvents` into JSON rows (pipe into your DB).

```bash
npm ci && npm test
CONTRACT_ID=C... npm start > events.jsonl
```

Key handling: indexing is read-only and needs no secret key. Do not give indexer hosts signing credentials.
