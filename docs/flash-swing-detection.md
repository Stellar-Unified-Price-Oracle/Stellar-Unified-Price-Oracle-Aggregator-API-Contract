# Single-Ledger Swing / Flash-Liquidity Detection (#449)

Implementation: `contracts/price-oracle/src/flash_swing.rs`.

## Definition

A **swing** is a move of at least `threshold_bps` away from a base price that
reverts to within `threshold_bps / 4` of that base inside `reversal_window`
ledgers. Detection runs over the per-ledger *aggregated* series, so a swing
assembled from several steps that each respect the per-source deviation bound
(`source_deviation.rs`) is still caught.

## Configuration

Owner: contract admin.

| Parameter | Default | Min | Max |
|---|---|---|---|
| `threshold_bps` | 1,000 (10%) | 100 | 5,000 |
| `reversal_window` (ledgers) | 3 | 2 | 20 |

`SwingConfig::is_valid` rejects values outside these bounds.

## Policy

| Policy | Outcome |
|---|---|
| `Reject` | The price is not aggregated or finalized. |
| `DeferForConfirmation` | Handed to the multi-round confirmation path (#397). No parallel mechanism exists. |
| `ServeDegraded` | Served with confidence `DEGRADED_CONFIDENCE_BPS` (25%). |

Each detection emits `("flash_sw", asset)` with
`(magnitude_bps, base_index, peak_index, reversal_index)` as reversal evidence.

## False positives

Tests cover a 64-ledger benign series (±3% chop on a drifting mean) and a
sustained 30% trend. Neither triggers detection, because the detector needs the
price to leave the threshold *and* come back within the window.

## Residual exposure

A swing that **never reverses inside the window** looks the same as a real
repricing and is not flagged. An attacker who can hold capital for more than
`reversal_window` ledgers pays for that time, so the cost of the attack grows
with the window. Manipulation that lasts across many ledgers is out of scope
here and is covered by #398. A larger window lowers this exposure, but real
V-shaped moves then get flagged more often. The 20-ledger cap (~100s) limits
that trade-off.
