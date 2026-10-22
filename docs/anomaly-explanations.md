# Anomaly Explanation Reports (#496)

> Every flag carries the rule that fired, the inputs it compared and the
> threshold it applied — machine-readable and human-readable.

## Problem

"A flag with no explanation is unactionable and erodes trust with sources."
Explanations turn "your submission was rejected" into "here is exactly why and
what to change".

## The schema

`AnomalyExplanation`:

| Field | Meaning |
|---|---|
| `rule_id` | Stable numeric rule identifier; never reused. |
| `rule` | Short stable symbol (`bounds`, `nonpos`, `future`, `stale`, `chgrate`, `corr`, `quorum`, `lowconf`). |
| `subject` | Who the flag is about: the source, or the asset for an aggregate flag. |
| `asset` | The asset whose price was flagged. |
| `ledger` | Ledger in which the flag was raised. |
| `observed` | The value the rule compared. |
| `reference` | The reference it compared against. |
| `threshold` | The threshold that was breached, in the rule's own units. |

## Rejections are explained by a view, not by a stored record

A rejecting path ends in `panic_with_error!`, which reverts the whole
transaction. **A record written just before the revert is rolled back with it**,
so explaining rejections by writing state would explain nothing.

`explain_submission(source, asset, price, timestamp)` therefore evaluates the
same rules, in the same order, on the same inputs and returns the explanation
the submission *would* get — or `None` when it would be accepted. It is pure:
it reads configuration and the source's own last submission, and writes nothing.
This is what a source calls after its transaction reverts.

| Rule | Fires when | `observed` | `reference` | `threshold` |
|---|---|---|---|---|
| `NonPositivePrice` | `price <= 0` | the price | 0 | 0 |
| `PriceOutOfBounds` | outside `[min_price, max_price]` | the price | `min_price` | `max_price` |
| `FutureTimestamp` | `timestamp > ledger_time + threshold` | the timestamp | the ledger clock | the allowed horizon |
| `StaleSubmission` | older than the source's own last submission | the timestamp | the prior timestamp | the prior timestamp |
| `ChangeRateBreach` | would move the aggregate too fast | the price | the prior aggregate | the bps limit |

## Flags that do not revert are stored

A correlation-band exclusion, an aggregate served below quorum and a collapsed
confidence band all leave state behind, so their explanations are recorded in a
bounded ring and emitted as `AnomalyExplainedEvent`:

| Rule | `observed` | `reference` | `threshold` |
|---|---|---|---|
| `CorrelationBand` | the submitted price | the counterpart price | the computed ratio |
| `InsufficientSources` | contributing sources | registered sources | the quorum |
| `LowConfidenceBand` | the band width | 0 | the quorum |

The quorum explanation is raised where the below-quorum event is raised, not
inside the event-budget branch, so a suppressed *event* never suppresses the
*explanation*.

## Stability

An explanation is a pure function of its inputs, so identical inputs always
produce an identical record and a stored explanation can be re-derived and
audited later. `explanations_are_stable_across_identical_inputs` asserts that
the pure builder and the stored record agree exactly.

## Bounded retention

Explanations live in a fixed-size ring — `DEFAULT_RETENTION` (16) by default,
admin-tunable up to `MAX_RETENTION` (64) — per `(asset, source)` and per
aggregate. When full, the oldest entry is dropped. Retention is by count, not by
age, so a burst of flags cannot evict the recent record and an idle asset cannot
grow. A retention of `0` or above the ceiling is rejected, so configuration
cannot turn a bounded log into an unbounded one.

Every record is also emitted as an event, so an explanation stays reconstructible
from the event stream after the ring has rolled.

## Information-leakage review

An explanation reveals only what the flagged party could already determine:

- **The flagger's own inputs.** `observed` is the price or timestamp the source
  itself submitted, and `reference` / `threshold` are configuration values the
  source can read through the public query endpoints before submitting.
- **No other source's identity.** The record has exactly two addresses — the
  `subject` and the `asset`. No other source is named, counted or referenced, so
  one participant cannot probe the behaviour or health of its peers.
- **No secrets.** The record is a fixed set of typed fields with no free-form
  text, so it cannot carry a key, a signature, a nonce or any other material
  whose disclosure would be exploitable. `explanations_leak_no_internal_state`
  asserts the encoded record names no such field.
- **Aggregate flags are aggregate-scoped.** `InsufficientSources` reports two
  counts the operator could compute from the public source registry. It reveals
  no per-source detail.

## Out of scope

Automated remediation advice. The contract explains; it does not coach.

See `docs/degraded-mode-analytics.md` for the serving-side counterpart.
