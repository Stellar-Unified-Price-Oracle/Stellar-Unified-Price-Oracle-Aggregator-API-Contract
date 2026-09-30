# Parameter Registry (#544)

Module: `contracts/price-oracle/src/param_registry.rs`

Every configurable parameter is listed in `PARAMS` with type (`i128`), unit, bounds,
default and owner. `registry_is_complete()` checks `PARAMS` against `REQUIRED_PARAMS`
and that each default lies within its bounds.

| Param | Unit | Min | Max | Default | Owner | Bound derivation |
|---|---|---|---|---|---|---|
| decimals | digits | 0 | 18 | 7 | Admin | 10^18 fits i128 products with headroom (see derived-feeds) |
| max_age | seconds | 1 | 86400 | 3600 | Admin | >0; staler than 1 day is unsafe for consumers |
| min_sources | count | 1 | 64 | 1 | Governance | median needs ≥1; 64 bounds per-round gas |
| max_deviation | bps | 1 | 10000 | 1000 | Governance | 0 blocks all updates; >100% is meaningless |
| ts_threshold | seconds | 1 | 3600 | 300 | Admin | must be < max_age default |
| history_len | entries | 1 | 1000 | 100 | Admin | storage/TTL cost bound |
| quorum | bps | 5001 | 10000 | 6667 | Governance | strict majority; ≤100% |
| fee | stroops | 0 | 1e8 | 100 | Governance | ≤10 XLM per query caps consumer cost |

## Writes
- `set_param` / `emergency_set_param` require the actor's auth and validate bounds.
- Out-of-bounds → `ErrorCode::ParamOutOfBounds` (166); unknown → `ParamNotRegistered` (167).
- Every accepted write (emergency included, flagged `emergency = true`) appends
  `ParamChange { actor, old, new, ledger, timestamp, emergency }`.
- History is a ring of `HISTORY_CAP = 32` entries per parameter; `get_history` queries it.
