# DEX / AMM Manipulation Resistance (#448)

Tests: `contracts/price-oracle/src/dex_tests.rs` (adversarial section).

## Source classification

| Source | Module | Manipulability | Depth control | Manipulation cost |
|---|---|---|---|---|
| Admin-registered constant-product pool | `dex.rs` | Medium: spot = reserve ratio | Admin-only registration | Cost to move a pool by *x*: `≈ R · (√(1+x) − 1)` of quote capital plus fees |
| AMM adapter reads | `amm.rs` | Medium: spot from reserves | Admin-only | Same as above |
| Permissioned oracle sources | `prices.rs` | Low: needs a source key | N/A | Compromising a key, then bounded by the median and `source_deviation.rs` |
| Cross-reference contracts | `cross_reference.rs` | Low: admin-configured | N/A | Compromising the referenced oracle |

## Cost vs. impact

The documented economic threshold is: **manipulation cost must exceed the value
exposed downstream for one update interval**.

| Pool depth (quote) | Move | Capital needed (≈) | Net cost with a flash loan | Below threshold? | Mitigation |
|---|---|---|---|---|---|
| 1,000,000 | +10% | 48,800 | fees only (~0.3% · 48,800 ≈ 146) | Yes | Median with independent sources; swing detection (#449) |
| 1,000,000 | +50% | 224,700 | ~674 | Yes | Same |
| 100 (thin) | +1000x | ~3,100 | ~9 | Yes | Pools are admin-registered only; the canonical route is fixed |

A flash loan (sandwich) cuts net cost down to fees, so **every spot-based DEX
source is below the threshold by itself**. The mitigations below are
therefore required.

## Attacks tested

1. **Pool creation / seeding.** `test_attacker_cannot_create_and_seed_pool`:
   without admin auth, `dex_register_pool` fails and no price exists. The
   attack is infeasible because pool registration is admin-gated.
2. **Route selection.** `test_route_selection_ignores_more_favourable_thin_pool`:
   a thinner pool quoting 1000x higher, registered later, is not selected.
   Routing is deterministic (registration order) and never picks the most
   favourable quote.

## Recommended mitigations (follow-ups)

- Liquidity-weighted or TWAP DEX prices instead of spot, which removes the
  single-ledger sandwich. The residual exposure is capital held for the whole
  TWAP window.
- Minimum-reserve floor for registered pools.
- Never use a DEX price alone. It should only be one input to the median, and
  it should pass flash-swing detection (#449).
