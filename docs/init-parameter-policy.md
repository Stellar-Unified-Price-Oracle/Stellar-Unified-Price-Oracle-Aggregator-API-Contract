# Initialization Parameter Policy

Before initializing a production deployment, review the four initialization
parameters below against the policy for that deployment target.

### Parameters under review

| Parameter | Meaning | Mainnet policy | Testnet / local policy |
|-----------|---------|----------------|------------------------|
| `min_sources` | Minimum distinct sources required before an aggregate price is published | `>= 3` (median needs an odd quorum to resist a single outlier) | `>= 1` |
| `decimals` | Price scaling exponent exposed via SEP-40 `decimals()` | `<= 18` (must match consumer expectations; no silent rescale) | `<= 18` |
| `max_history` | Maximum historical records retained per asset | `>= 1` and `<= 10_000` (bounds storage growth) | `>= 1` and `<= 10_000` |
| `description` | Human-readable contract description emitted in `ContractInitializedEvent` | non-empty, `<= 256` bytes | non-empty, `<= 256` bytes |

### Boundary conditions

- `min_sources = 0` is always rejected — aggregation would be undefined.
- `min_sources` greater than the number of registered sources means no price
  is published until enough sources are added; this is a valid but degraded
  state and is surfaced by `oracle-cli health-check`.
- `decimals = 0` is allowed (integer prices); values above 18 are rejected.
- `max_history = 0` disables history retention and is rejected for production.
- An empty or whitespace-only `description` is rejected.

### Failure and recovery

If `initialize` is called with out-of-policy values the contract still stores
them (the contract does not hard-code network policy), so the review is the
gate. To recover, the admin uses `set_min_sources_required`,
`set_max_history_length`, `set_decimals`, and `set_description` to bring the
contract back within policy, then re-runs the validation command below.

### Example config

A candidate config is a simple `KEY=VALUE` env file:

```env
MIN_SOURCES=3
DECIMALS=18
MAX_HISTORY=1000
DESCRIPTION="Stellar Unified Price Oracle (mainnet)"
```

> An automated validator (`scripts/check-init-params.sh`) is planned; until it
> lands, review these values manually before every deployment.
