# Subscription Auto-Renewal (#289)

Lets a consumer grant the contract a **bounded, revocable, standing right** to
pay for its own subscription out of a pre-approved SAC allowance, so a keeper
can renew on their behalf without a fresh signature per period.

| Item | Value |
|---|---|
| Config | `enable_auto_renewal(consumer, token, plan_duration, max_amount_per_period, periods_authorized)` |
| Per-period sign-off | `authorize_renewal(consumer, period_id, nonce)` — consumer-signed |
| Revoke | `disable_auto_renewal(consumer)` (consumer-signed), or `cancel_subscription` |
| Execute | `try_auto_renew(consumer)` — permissionless keeper, **never panics** |
| Read | `get_auto_renewal_record(consumer)`, `get_renewal_authorization(consumer, period_id)` |
| Bounds | `1 <= plan_duration <= 31_536_000`; `max_amount_per_period > 0`; `1 <= periods_authorized <= 1_000` |
| Max tokens ever moved | `periods_authorized * max_amount_per_period`, and never more than the live SAC allowance |
| Events | `AutoRenewalAuthorizationEvent { consumer, action, max_amount_per_period, plan_duration }`, `AutoRenewalAttemptEvent { consumer, success, amount, reason, period_id, expiry }` |
| `action` | `0` granted · `1` revoked · `2` cancelled with the subscription |

## The period model

A **period** is identified by its due timestamp:

```text
period_id == record.next_renewal_timestamp
renewal is possible only once  env.ledger().timestamp() >= period_id
a successful renewal sets  next_renewal_timestamp = period_id + plan_duration
```

`enable_auto_renewal` sets `next_renewal_timestamp = now`, so the first period
is immediately renewable. Because a period id is a strictly increasing
timestamp and each success advances it by exactly `plan_duration`, a given
period id can be renewed **at most once, ever**.

## Maximum tokens the contract can ever move — the bound, and its proof

```text
max_tokens_moved(consumer) = periods_authorized * max_amount_per_period
```

and, independently,

```text
max_tokens_moved(consumer) <= the consumer's live SAC allowance to this contract
```

**Claim.** For any consumer, over any sequence of ledger history, keeper
invocations, cancellations, re-grants and token callbacks, the total tokens the
contract transfers *out of that consumer's account* is at most
`periods_authorized * max_amount_per_period`.

**Proof.** Let `T` be the total moved. The argument is a chain of four
independent bounds, each of which can only reduce `T`.

1. *At most one success per period id.* `try_auto_renew` targets exactly
   `period_id = record.next_renewal_timestamp`. On success it advances
   `next_renewal_timestamp` to `period_id + plan_duration`, strictly greater
   than `period_id` (because `plan_duration > 0` is validated on grant). Every
   later attempt therefore targets a different, larger period id. So a period id
   is the target of at most one successful renewal in the entire history.

2. *At most `periods_authorized` successes.* `periods_used` is checked against
   `periods_authorized` **before** any effect is written, and incremented exactly
   once per success. After `periods_authorized` successes the check fails with
   `AutoRenewalAllowanceExceeded` forever (only a fresh `enable_auto_renewal`
   resets `periods_used`, and that is consumer-signed).
   Hence `n_successes <= periods_authorized`.

3. *At most `max_amount_per_period` per success.* The transferred amount is
   read from the stored `RenewalAuthorization.amount`, which
   `authorize_renewal` set to `min(plan_amount, max_amount_per_period)` and which
   `try_auto_renew` never recomputes from ambient state. So
   `amount_i <= max_amount_per_period` for every `i`.
   Combining (1) and (2) with (3):
   `T = Σ amount_i <= n_successes * max_amount_per_period <= periods_authorized * max_amount_per_period`. ∎

4. *The SAC allowance is an independent ceiling.* The live allowance is read on
   **every** attempt via `token::Client::allowance(consumer, this_contract)`
   and the attempt is refused when it is below the authorized amount. A
   consumer who never approved, or who revoked (even mid-period), can never be
   renewed against; the ceiling is the smaller of the two bounds.

**Cancellation only lowers the bound.** `revoke_on_cancel` sets
`cancelled = true` *and* `active = false` in the same invocation, so a cancelled
subscription can never renew again, and it consumes the outstanding
authorization. The bound is monotone non-increasing in every adversarial
action; there is no path in the contract that raises either factor.

**Amount is not recomputed.** Re-pricing the plan after an authorization was
issued does not enlarge what that single-use authorization permits
(`the_authorization_amount_is_frozen_at_issue_time`). A consumer who wants a
higher per-period amount must issue a *new* authorization, which is
consumer-signed.

## Checks-effects-interactions ordering

`try_auto_renew` is strictly CEI:

1. **Checks** — record present, not cancelled, active, periods remaining, period
   not spent, period due, authorization present, authorization unconsumed and
   nonce-matched, live SAC allowance sufficient, subscription active, subscription
   not lapsed. Each returns `RenewalAttempt { renewed: false, amount: 0, reason }`
   and emits a failure event. **No check panics.**
2. **Effects** — mark the authorization `consumed = true`; set
   `DataKey::SubscriptionRenewalSpent(consumer, period_id)`; `periods_used += 1`;
   `next_renewal_timestamp = period_id + plan_duration`; bump
   `authorization_nonce`; extend the subscription expiry by `plan_duration`; write
   the record. **All of this happens before the external call**, so there is no
   window in which a re-entrant caller observes a half-applied renewal.
3. **Interaction** — `reentrancy::enter(env)`, the single token pull,
   `reentrancy::exit(env)`.

**Why the pull is `transfer_from`.** The keeper's call is permissionless: there
is no `consumer.require_auth()` in this frame. The only authority the contract
can spend is the pre-approval the consumer granted at the token, which is
exactly what the allowance check verified is still live. `transfer` would move
the consumer's own balance and would require their auth in this frame, which a
keeper can never supply — the allowance model would be unreachable.

**Failed transfer ⇒ nothing is committed.** If the token pull itself fails, the
whole invocation reverts, so the effects in step 2 are rolled back with it. A
transfer can therefore never half-apply a renewal: either the tokens moved and
the effects stuck, or neither.

## Reentrancy guard placement

The `reentrancy` guard is entered **immediately before the token pull** and
exited immediately after — it is not held across the checks, so a keeper's
failing attempts never contend for it. All cheap validation that returns a
`RenewalAttempt` runs *before* `enter`, so the "missing or invalid record" path
is genuinely non-panicking; only the path that is about to move tokens touches
the guard. A callback that re-enters `try_auto_renew` therefore hits
`ErrorCode::Reentrant` (25) and fails, which is asserted by
`reentrant_token_transfer_cannot_re_enter_try_auto_renew` — that test uses a
hostile mock token whose `transfer` calls back into `try_auto_renew`, and fails
on both `reentered() == true` and a doubled `moved()` if `enter` were removed.

## Replay model

Two independent, layered defences; either alone blocks a captured authorization.

**Monotonic nonce.** `authorize_renewal` rejects any `nonce <=
record.authorization_nonce` with `RenewalAuthorizationReplay` (175). A
successful renewal *bumps* `record.authorization_nonce` past the nonce it just
consumed, so the spent authorization can never satisfy
`auth.nonce == record.authorization_nonce` again. Re-granting resets the record
to `authorization_nonce = 0`, so a fresh authorization must be issued with nonce
`>= 1`; an authorization captured under the *old* grant carries a stale nonce
relative to the new record and is rejected.

**Spent marker.** `DataKey::SubscriptionRenewalSpent(consumer, period_id)` is
written on every success (and on cancellation). Because period ids strictly
increase, a spent period id can never legitimately reappear, so the marker is a
permanent, per-period tombstone. There is no unbounded scan: the only period that
can ever be authorized is `record.next_renewal_timestamp`, so
`consume_current_authorization` needs a single read.

**No pre-authorization.** `authorize_renewal` requires
`period_id == record.next_renewal_timestamp` (`RenewalAuthorizationMissing`,
176), so a consumer cannot pre-sign a future period. A captured *future*
authorization is structurally useless.

## Reason codes

`RenewalAttempt.reason` and `AutoRenewalAttemptEvent.reason` are the
`ErrorCode` discriminant, `0` on success.

| `reason` | Code | Meaning |
|---|---|---|
| success | `0` | tokens moved, expiry advanced |
| `NoData` | 8 | the period is identified by its due timestamp and has not arrived yet (nothing to renew *yet*) |
| `SubscriptionExpired` | 18 | the current subscription expiry has already lapsed |
| `NoActiveSubscription` | 138 | the consumer has no subscription expiry at all |
| `RenewalAuthorizationReplay` | 175 | the authorization was already consumed, its nonce is stale, or the period is spent |
| `RenewalAuthorizationMissing` | 176 | no single-use authorization exists for the period currently due |
| `AutoRenewalNotEnabled` | 177 | no standing record, or it has been revoked with `disable_auto_renewal` |
| `AutoRenewalCancelled` | 178 | the subscription was cancelled; the right is void permanently |
| `AutoRenewalAllowanceExceeded` | 179 | `periods_authorized` is exhausted, or the live SAC allowance is below the authorized amount |

A not-yet-due period reports `NoData` (8) — distinct from every denial — unless
the *preceding* period is already spent, in which case it reports
`RenewalAuthorizationReplay` (175) because the attempt really is a second drain
attempt in an already-renewed cycle, not a merely premature one.

`authorize_renewal`, `enable_auto_renewal` and `disable_auto_renewal` are all
consumer-signed and *do* panic with these codes, which is why they are not part
of the keeper's non-panicking surface.

## Failure is a value, never a panic

`try_auto_renew` is permissionless and returns a `RenewalAttempt` on **every**
path. A failed renewal therefore:

- cannot lock a consumer out of their query path, and
- cannot burn unbounded gas — the work is a fixed number of storage reads and
  **at most one** token pull, with no loop and no unbounded scan.

A failure consumes nothing: the authorization stays unconsumed, the period stays
unspent and `periods_used` is unchanged, so granting the missing approval and
retrying succeeds on the very next call
(`a_renewal_denied_for_allowance_succeeds_immediately_after_the_approval`).

## Atomicity with cancellation

`subscription::cancel_subscription` calls `revoke_on_cancel(env, consumer)` in
the same invocation, which sets `cancelled = true` **and** `active = false`,
consumes the outstanding authorization, marks its period id spent, and emits
`AutoRenewalAuthorizationEvent { action: 2 }`. There is therefore **no window**
in which a cancelled subscription can still be renewed by a keeper — proven by
`renewal_and_cancellation_race_leaves_at_most_one_winner` and
`cancellation_before_any_attempt_wins_the_race_outright`, in both orderings of
the same-ledger race, and both assert zero tokens move after the cancellation.

`revoke_on_cancel` is a **total no-op** (no panic, no event) when the consumer
has no record, because `cancel_subscription` calls it unconditionally for every
consumer. `consume_current_authorization` is exposed separately as the
bookkeeping half, and `revoke_on_cancel` is expressed in terms of it so the two
cannot drift.

## Storage

| Key | Value | Lifetime |
|---|---|---|
| `DataKey::SubscriptionAutoRenew(consumer)` | `AutoRenewRecord` | persistent, TTL-extended |
| `DataKey::SubscriptionRenewalAuthorization(consumer, period_id)` | `RenewalAuthorization` | persistent, TTL-extended |
| `DataKey::SubscriptionRenewalSpent(consumer, period_id)` | `true` | persistent, TTL-extended |
| `DataKey::ReentrancyGuard` | `true` | temporary, held only across the token pull |

## Test map

| Criterion | Tests |
|---|---|
| 1. Token-approval integration | `auto_renewal_moves_exactly_the_authorized_amount_on_the_approved_allowance`, `the_authorization_amount_is_frozen_at_issue_time`, `renewal_moves_min_of_plan_price_and_the_per_period_cap` |
| 2. Per-period drain bound | `repeated_renewal_attempts_in_one_period_move_at_most_one_period_amount`, `exhausting_every_authorized_period_blocks_further_renewals` |
| 3. Reentrancy | `reentrant_token_transfer_cannot_re_enter_try_auto_renew` |
| 4. Cancellation / supersession | `cancelled_subscription_cannot_renew_even_with_a_live_authorization`, `superseded_authorization_cannot_be_replayed_after_a_regrant`, `renewal_and_cancellation_race_leaves_at_most_one_winner`, `cancellation_before_any_attempt_wins_the_race_outright`, `revoke_on_cancel_is_a_total_no_op_without_a_record`, `revoke_on_cancel_voids_the_outstanding_authorization_and_its_period` |
| 5. Replay | `captured_authorization_cannot_be_reissued_after_it_is_spent`, `authorize_renewal_rejects_a_nonce_that_is_not_strictly_increasing`, `authorize_renewal_rejects_a_future_period_id`, `authorize_renewal_is_refused_after_cancellation`, `authorize_renewal_is_refused_without_a_record` |
| 6. Failures never lock out | `a_renewal_denied_for_allowance_succeeds_immediately_after_the_approval`, `every_failure_path_returns_a_value_rather_than_panicking`, `a_consumer_who_never_approved_gets_allowance_exceeded_and_moves_nothing`, `a_revoked_allowance_stops_renewals`, `a_renewal_without_an_active_subscription_is_refused`, `a_renewal_of_a_lapsed_subscription_is_refused` |
| 7. Events | `attempt_events_carry_success_and_the_documented_error_discriminants`, `grant_revoke_and_cancel_authorization_events_carry_their_action` |
| 8. `disable_auto_renewal` | `disable_auto_renewal_stops_renewals_immediately` |
| Config validation | `enable_auto_renewal_rejects_a_zero_period_cap`, `enable_auto_renewal_rejects_a_zero_period_count`, `enable_auto_renewal_rejects_a_zero_duration`, `enable_auto_renewal_rejects_more_periods_than_the_documented_bound` |
