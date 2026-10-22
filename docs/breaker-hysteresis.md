# Hysteresis circuit breaker with automatic re-arming (#481)

## The problem

The deviation breaker had **one** threshold. A price oscillating either side of
it therefore tripped and cleared on alternate ledgers. That is
indistinguishable from a broken feed: an operator cannot tell a flapping
indicator from a genuinely unstable market, and the manual clearing it demands
does not scale across a portfolio of assets.

## The deadband

Two thresholds, not one:

| Threshold | Meaning |
|-----------|---------|
| `trip_bps` | Deviation **at or above** which the breaker opens. |
| `clear_bps` | Deviation **below** which the market counts as settled. Must be strictly below `trip_bps`. |

A deviation between the two is neither: the breaker holds whatever state it is
in. That band is the deadband, and it is what stops chatter.

`clear_bps == trip_bps` is rejected. Equal thresholds leave no deadband and
reproduce exactly the chattering breaker this replaces.

Defaults: trip at 20 %, clear at 10 %, settle for 10 ledgers, escalate after
1 000 ledgers, auto re-arm on. The deadband is half the trip threshold.

## Re-arm requires a settle condition

A single below-`clear_bps` observation is **not** enough. The deviation must
stay below `clear_bps` for `settle_ledgers` **consecutive** ledgers. Any ledger
at or above the clear threshold resets the streak to zero.

This is what stops the flapping attack in the other direction: an adversary who
dips the price into calmness for one ledger, lets the breaker re-arm, and pushes
it back up has not satisfied a five-ledger settle window.

## Manual override is never blocked

`clear_breaker(asset)` is admin-authorised and consults **neither** the settle
streak **nor** the escalation flag. The automatic path is a convenience for the
common case; it is never a gate on the operator.

- Automatic re-arm disabled (`auto_rearm = false`) → manual clear still works.
- Breaker escalated → manual clear still works.

## Bounded open time

A breaker that can never re-arm is a permanent pause with extra steps; one that
can always re-arm is not a control. `max_open_ledgers` bounds the automatic
path: past it the contract emits `BreakerEscalatedEvent`, stops attempting
automatic re-arm, and `evaluate_breaker_rearm` fails with
`BreakerEscalationRequired`.

Escalation is a *narrowing* of automation, never a widening — it can only ever
reduce what the contract does on its own, which is why the operator is still
free to clear the breaker afterwards.

## Reconstructibility

Every transition is evented with the deviation that drove it:

| Event | When |
|-------|------|
| `BreakerTrippedEvent` | The breaker opened. Carries the deviation, and both thresholds. |
| `BreakerRearmAttemptEvent` | Every re-arm evaluation, **including** the ones that do not re-arm. |
| `BreakerRearmedEvent` | The settle condition was satisfied. |
| `BreakerEscalatedEvent` | The open-time bound was passed. |

The attempt event fires on both outcomes. That is what makes the state machine
reconstructible: a consumer replaying the stream can distinguish "evaluated and
held" from "never evaluated". Silence would be ambiguous.

## API

| Endpoint | Purpose |
|----------|---------|
| `set_breaker_policy(asset, policy)` | Configure thresholds and windows. Admin only. |
| `get_breaker_policy(asset)` | Read the policy. |
| `get_breaker_status(asset)` | Open/closed, settle streak, open duration, escalated. |
| `evaluate_breaker_rearm(asset, deviation_bps)` | Evaluate the automatic path. |
| `clear_breaker(asset)` | Manual clear. Admin only, never blocked. |

## State machine

```
                 deviation >= trip_bps
      ┌────────┐ ─────────────────────▶ ┌────────┐
      │ ARMED  │                        │  OPEN  │
      └────────┘ ◀───────────────────── └────────┘
                   settle_ledgers consecutive
                   ledgers below clear_bps
                   (and not escalated)
```

Inside `OPEN`, a deviation in `[clear_bps, trip_bps)` holds the state. A
deviation `>= clear_bps` additionally resets the settle streak.
