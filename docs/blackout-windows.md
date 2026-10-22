# Blackout / Quiet-Period Windows (#400)

Module: `contracts/price-oracle/src/blackout.rs`.

A blackout suspends **aggregation** for one asset. During a window:

* Submissions are still accepted and stored.
* No new aggregate is published. This includes a corrective aggregate
  following an earlier value.
* The last aggregate stays readable with its original timestamp.

## Authority

| Action | Who | Constraint |
|---|---|---|
| `schedule_blackout(asset, start, end)` | admin | `start >= now + MIN_NOTICE_SECS` (1 h); `end - start <= MAX_BLACKOUT_SECS` (6 h) |
| `extend_blackout(asset, new_end)` | admin | window pending/active; total `<= MAX_BLACKOUT_SECS` |
| `cancel_blackout(asset)` | admin | any time |
| `signal_volatility(source, asset)` | registered, non-probation source | window opens only when `get_volatility_quorum()` (default 3, minimum 2) **distinct** sources signal within `SIGNAL_WINDOW_SECS` (5 min) |
| `set_volatility_quorum(q)` | admin | `q >= 2` |

A single source cannot open a window. Repeated signals from one source count
once. No source can extend a window: extension is admin-only, and signals sent
during an active window do not change its end. Covered by
`single_source_cannot_trigger_or_extend_blackout`.

## Griefing bound and escalation

* A window lasts at most `MAX_BLACKOUT_SECS` (6 h), extensions included.
* A new window may start only `COOLDOWN_SECS` (6 h) after the previous one
  ended.

This cooldown is the hysteresis. The asset therefore aggregates at least 50 %
of the time in the worst case. Volatility windows last 30 min, so a
volatility-only griefer achieves at most 30 min of blackout per 6.5 h.

The escalation path for a persistent griefer is to offboard it (#402) or to
use the pause and freeze machinery.

## Boundaries

A window is the half-open interval `[start, end)` of ledger time:

| Aggregation at | Result |
|---|---|
| `start - 1` | published |
| `start` … `end - 1` | withheld (`BlackoutWithheldEvent`) |
| `end` | published, using any submissions stored during the window (subject to freshness/DQ); `BlackoutExitedEvent` |

In-flight submissions are handled by ledger time. A transaction included at
ledger time `t` is judged against `t`, not against the time it was signed.
Covered by `blackout_boundaries_are_half_open`.

## Precedence

`pause > freeze > blackout`:

| Overlap | Behaviour |
|---|---|
| pause + blackout | submissions rejected with `ContractPaused` |
| freeze + blackout | frozen snapshot served; submissions rejected |
| pause + freeze | existing behaviour, pause checked first |
| blackout only | submissions stored, aggregation withheld |

Covered by `blackout_precedence_with_pause_and_freeze`.

## Event reconstruction

| Event | Emitted when |
|---|---|
| `BlackoutScheduledEvent{start,end,origin}` | entry (the start time is explicit) |
| `BlackoutExtendedEvent{old_end,new_end}` | extension |
| `BlackoutExitedEvent{start,end,cancelled}` | exit on expiry or cancellation |
| `BlackoutWithheldEvent{at,window_end}` | each withheld aggregation, including a withheld correction |
| `VolatilitySignalEvent{source,signals,quorum}` | each signal |

Together these fully reconstruct every window, its trigger and every withheld
publication.

## Manipulate-to-freeze analysis

**Attack:** push the price to trigger a blackout, then trade against the
now-unpriced asset. Four properties defeat or bound it:

1. **Triggering needs quorum.** Price movement alone never opens a window. The
   attacker must control `quorum` distinct, graduated sources. Probation
   sources cannot signal. At that point the attacker already controls a
   majority, and the DQ step/drift bounds (#398) cap what that majority can
   publish anyway.
2. **The manipulated value is never published by the blackout.** A window
   freezes the *last published* aggregate. If the manipulation had not been
   published before the window, it never is. If it had been, the DQ step bound
   limits it to `max_step_bps` from the prior value.
3. **Bounded duration.** The free option lasts at most 30 min for volatility
   windows and 6 h in total, followed by a 6 h cooldown. Consumers see the
   aggregate timestamp age and can apply their own staleness policy (#408).
4. **Admin misuse is visible.** Scheduled windows need 1 h notice, so the
   admin cannot open one reactively to hide a manipulation. Every withheld
   correction is evented.

The expected gain is at most a `max_step_bps` mispricing for up to 30 min. The
cost is controlling a quorum of bonded sources, whose bonds are exposed to
evidence-based slashing (#402). For any sensible bond size the attack costs
more than it gains.
