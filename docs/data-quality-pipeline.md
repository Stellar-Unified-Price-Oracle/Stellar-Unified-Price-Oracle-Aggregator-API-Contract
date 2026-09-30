# Pre-aggregation Data-Quality Pipeline (#398)

Module: `contracts/price-oracle/src/dq_pipeline.rs`. Opt-in per asset via
`set_dq_config(asset, DqConfig)`; `clear_dq_config` removes it. Assets without
a config pay one storage read per aggregation and are otherwise unaffected.

## Checks

Each submission reaching the aggregation loop is screened in order. The first
failing check excludes it and emits `DqInputRejectedEvent`.

| Reason | Check | `value` | `threshold` | `reference` |
|---|---|---|---|---|
| 1 `STALE` | age > `max_staleness_secs` | age (s) | `max_staleness_secs` | submission ledger time |
| 2 `BOUNDS` | price < `min_price` or > `max_price` | price | bound breached | bound breached |
| 3 `STEP` | distance from last aggregate > `max_step_bps` | price | `max_step_bps` | last published aggregate |
| 4 `DRIFT` | distance from drift anchor > `max_drift_bps` | price | `max_drift_bps` | drift anchor |

Distances are compared by cross-multiplication
(`|p - ref| * 10_000 > ref * bps`), so integer truncation never admits a value
past a threshold.

The event carries every input to the decision (asset, source, reason, value,
threshold, reference), so the reason for any rejection can be reconstructed
from events alone. `get_dq_config` and `get_dq_anchor` expose the thresholds in
force.

### Freshness is per asset

`max_staleness_secs` is part of the per-asset config and applies on top of
the existing policy-layer `freshness_secs`.

### Sanity bounds: justification and update path

`min_price` / `max_price` are set by the admin per asset and should be derived
from the asset's historical range with a wide margin. They exist to catch
decimal-scaling and fat-finger errors, not manipulation. They change only
through `set_dq_config` (admin auth, `DqConfigUpdatedEvent`). Changing the
config also resets the drift anchor.

### Consistency against a manipulated majority

The step and drift checks compare each input with **published history**
instead of with the other inputs in the round. A colluding majority therefore
cannot redefine "normal":

* **Per-round bound:** every accepted value lies within `max_step_bps` of the
  last aggregate, so their median does too. One round moves the aggregate by
  at most `max_step_bps`.
* **Per-window bound:** every accepted value lies within `max_drift_bps` of
  the anchor, which is the aggregate in force when the window opened. Within
  one `drift_window_secs`, cumulative movement is at most `max_drift_bps`.

Staged drift (many increments, each inside the step bound) is stopped by the
drift bound. This is covered by `dq_detects_slow_staged_drift`.

## Limitations

Each check has an evasion it does not stop:

| Check | Evasion it does **not** stop |
|---|---|
| Staleness | A value submitted at exactly `max_staleness_secs` old is accepted. Its influence is still limited by the step and drift checks. |
| Sanity bounds | Any value inside `[min_price, max_price]` passes. The bounds are wide by design and provide no manipulation resistance. |
| Step | An adversary controlling a majority can move the aggregate by up to `max_step_bps` per round, in either direction. This is **bounded, not detected**. |
| Drift | The same majority can move the aggregate by `max_drift_bps` per window, then continue after the anchor re-bases. Sustained drift across windows is bounded to `max_drift_bps` per `drift_window_secs`. The long-horizon detector for that case is the benchmark drift monitor in `docs/drift-detection.md`. |

**A majority-of-sources adversary is not detected; it is bounded.** No
on-chain check can tell a colluding majority apart from a real market move
using only the submissions, because by assumption the majority *is* the
consensus. The pipeline instead limits how fast the published aggregate can
move. Collusion detection belongs to the source comparison dashboard
(`docs/source-comparison.md`).

**Liveness trade-off:** a real move larger than `max_step_bps` is rejected
along with manipulation. The aggregate then goes stale until either the market
returns inside the bound or the admin updates the config. `set_dq_config`
resets the anchor. The last aggregate keeps its original timestamp, so
consumers see it ageing.
