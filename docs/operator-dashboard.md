# Operator dashboard (#533)

`services/operator_dashboard/dashboard.py` shows one read-only view of what
needs operator action.

## Data pipeline

```
contract events ──> indexer (oracle_events table / JSONL export)
                ──> services.common.events.iter_envelopes
                ──> dashboard.fold  (timelock, multisig, expirations, health)
                ──> dashboard.build (freshness + alerts) ──> HTTP GET / text
```

Handled topics: `timelock_queued`/`op_proposed`, `timelock_executed`/`_cancelled`,
`multisig_proposed`/`_approved`/`_executed`/`_expired`, `ttl_extended`,
`key_registered`, `health_changed`.

## Guarantees

- **On-chain truth:** state is rebuilt from events on every request; there is
  no cache. `lag_s = now - last_event_timestamp`.
- **Stale marking:** `stale = true` (and a `STALE` banner) when `lag_s > max_lag_s`
  (default 120 s) or no events were seen.
- **Read-only:** the server answers `GET` only and returns 405 for all other
  verbs. It loads no keys and has no signing code.
- **Alerts:** any deadline within `alert_window_s` (default 1 h) and any
  non-healthy state is passed to registered hooks (webhook, pager, stdout).

Run: `python -m services.operator_dashboard.dashboard --events events.jsonl [--port 8088]`.
