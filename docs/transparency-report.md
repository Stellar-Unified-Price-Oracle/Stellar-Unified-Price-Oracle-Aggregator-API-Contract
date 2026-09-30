# Public Transparency Report (#543)

Module: `contracts/price-oracle/src/transparency_report.rs`

The report is computed by `build_report(period, events)`, a **pure function** over
public event records, so any third party can decode contract events and reproduce it
bit-for-bit. No private data is used.

## Cadence
One report per `CADENCE_SECS` (7 days). `period = timestamp / CADENCE_SECS`.
`publish_report` only accepts closed (past) periods and is idempotent: republishing a
period with a different result is rejected, so the stored report is the canonical one.
Each publish emits a `tr_report` event.

## Metric definitions
| Metric | Definition |
|---|---|
| `event_count` | Events whose timestamp falls in the period |
| `participating_sources` | Distinct `source` addresses with ≥1 `Submission` |
| `submissions` | Count of `Submission` events |
| `rounds_total` | `RoundOk + RoundDegraded` |
| `uptime_bps` | `RoundOk * 10000 / rounds_total` (0 if no rounds) |
| `degradation_bps` | `RoundDegraded * 10000 / rounds_total` (0 if no rounds) |
| `corrections` | Count of `Correction` events |
| `total_cost` | Sum of `Fee.amount` (stroops) |
| `empty` | `true` when the period had no events |

## Reproduction
1. Fetch contract events for `[period*CADENCE, (period+1)*CADENCE)`.
2. Map them to `EventRecord { kind, source, amount, timestamp }`.
3. Run `build_report` and compare with `get_report(period)`.

## Limitations
- Integer (floor) division for bps metrics.
- Uptime counts rounds, not wall-clock time.
- Empty periods produce a zeroed report with `empty = true`.
