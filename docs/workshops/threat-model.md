# Threat Model — Oracle Guarantees and Gaps

> Read this before starting any workshop lab. The labs exploit the gaps described here.

**Contract revision:** see `main` branch tag at time of delivery.

---

## What the Oracle Guarantees

| Guarantee | Mechanism |
|---|---|
| Price is the **median** of all registered source submissions | `compute_median()` in `storage.rs` |
| Price is not accepted from unregistered sources | `check_source()` auth guard in `prices.rs` |
| At least `min_sources` submissions exist before aggregation | `InsufficientSources` error |
| Historical prices are bounded by `max_history_length` | enforced in `history.rs` |
| Price decimals are consistent across all assets | set once, returned by `decimals()` |
| Timestamps too far in the future are rejected | `InvalidTimestamp` error |

---

## What the Oracle Does NOT Guarantee

| Non-Guarantee | Implication for Consumers |
|---|---|
| The price is **current** at time of read | Consumer must check staleness (compare price timestamp to current block time) |
| The aggregation is **finalized** | Consumers reading during an in-progress submission round may see a partial median |
| The source set is **trustworthy** | Source registration is admin-controlled; consumers cannot assume all sources are honest |
| The price has not been **manipulated** mid-window | A consumer reading between source submissions may see a transient outlier |
| Asset address correctness | The consumer is responsible for passing the correct asset address; the oracle does not validate intent |

---

## Incident Classes Mapped to Labs

| Incident Class | Real-World Example | Lab |
|---|---|---|
| Stale price consumption | Lending protocol liquidates on hours-old price | Workshop 01, Lab A |
| Decimal mismatch | Consumer treats 18-decimal price as 6-decimal, 10^12× error | Workshop 01, Lab B |
| Asset identity confusion | Consumer passes wrong asset address, receives unrelated price | Workshop 01, Lab C |
| Single-source trust | Attacker controls one source, consumer reads before median settles | Workshop 02, Lab A |
| Unfinalized price trust | Consumer reads before `min_sources` threshold met | Workshop 02, Lab B |
| Admin path exposure | Unrestricted upgrade call allows contract replacement | Workshop 03, Lab A |

---

## Security Properties to Verify in Every Integration

Before deploying a consumer contract, verify all of the following:

1. **Staleness check** — does the consumer reject prices older than its acceptable window?
2. **Decimal awareness** — does the consumer apply `decimals()` before arithmetic?
3. **Asset identity** — does the consumer assert the returned asset matches the expected asset?
4. **Source count** — does the consumer handle `InsufficientSources` gracefully?
5. **Finalization window** — does the consumer avoid consuming mid-round?
6. **Error handling** — does the consumer handle `NoData`, `AssetNotRegistered`, and `InsufficientSources` without silently using a zero or stale value?
