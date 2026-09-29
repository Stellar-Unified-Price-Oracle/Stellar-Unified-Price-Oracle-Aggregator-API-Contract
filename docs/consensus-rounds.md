# Multi-Round Price Confirmation (#397)

> A consensus protocol specified as a consensus protocol: distinct-round
> quorum, equivocation handling, timeout/liveness, and an explicit statement of
> the maximum adversary fraction tolerated.

Single-round aggregation can be gamed by short-term manipulation. Multiple
rounds raise the cost — **but only if the protocol says what happens when
participants equivocate, withhold, or split across rounds.** This document is
that statement.

- Config: `set_round_config(asset, config)` (admin).
- Query: `get_round_config(asset) -> RoundConfig`.
- Lifecycle: `start_round`, `submit_round_vote`, `finalize_confirmation`,
  `get_confirmed_price`, `get_round_status`, `abandon_stalled_round`,
  `get_round_tally`.
- Equivocation: `report_equivocation`, `get_equivocation_count`,
  `is_barred_from_round`.

```text
RoundConfig {
  required_rounds,  // consecutive agreeing rounds required; 1 = off (default)
  quorum,           // distinct sources that must observe within a round
  round_ledgers,    // ledgers a round may stay open before it can be abandoned
  agreement_bps,    // max spread between consecutive round medians
}
```

**Single-round is the default and is unchanged.** `required_rounds == 1` is the
pre-#397 behaviour: `consensus_rounds` is a no-op and the aggregate published by
`submit_price` remains the final price. Multi-round mode is strictly opt-in per
asset.

## Tolerated adversary fraction

**The protocol tolerates an adversary fraction strictly below `1/3` of the
voting set.**

With a quorum `q` drawn from `n` registered sources and a median tally, a round
is Byzantine-robust when `2q - 1` honest sources participate, i.e. when
`n >= 3f + 1` for `f` adversarial sources. Under that condition an adversary
holding `f < n/3` sources cannot move the round's median: it can cast at most
`f` of the `q` votes, and the median of the remaining honest majority is
unaffected.

The multi-round extension inherits this. The confirming rounds are independent
draws of the same quorum, so an adversary within that fraction cannot produce
`required_rounds` consecutive adversarial-majority rounds undetected — and even
if it could, the run additionally requires every pair of adjacent medians to
agree within `agreement_bps`, which a manipulation that moved a median would
violate.

### Assumptions this rests on

The `1/3` bound is only meaningful under these assumptions, and each is an
explicit precondition rather than an aspiration:

1. **Sources are registered, bonded and identified.** Voting requires
   `check_source`, so an adversary must spend a bond per identity. This is what
   makes `f` a bound on *distinct* parties rather than on sybil identities.
2. **Sources do not equivocate undetected.** A participant submitting two
   different values in one round is detected and barred. Without this, one
   identity could count twice toward a quorum and `f` would be meaningless.
3. **The network advances ledgers.** The liveness bound below is denominated in
   ledgers; a network that stops producing blocks stalls everything, and no
   on-chain protocol can defend against that.
4. **`q <= 64` and `required_rounds <= 16`.** These bound the per-round cost and
   the maximum time a price can be withheld, so the protocol's own resource use
   is bounded regardless of configuration.
5. **Admin is honest for configuration only.** A malicious admin can set
   `required_rounds = 1` and disable the protection; they cannot forge a tally
   or a confirmation.

## The four attacks, and what defeats each

A naive "N consecutive matching values" rule is defeated by four distinct
attacks. Each has a dedicated mechanism, and each has a dedicated test.

### 1. Replay across rounds

**Attack.** Replay the same market observation across `required_rounds` rounds
to satisfy the count cheaply — the values "match" because they are literally
the same observation.

**Mechanism.** Every observation is committed to a content-addressed identity
derived by the *contract* (never caller-supplied, so it cannot be chosen to
collide):

```text
observation_id = sha256(asset_xdr || round_le4 || source_xdr || price_le16 || timestamp_le8)
```

When a round tallies, the first round to consume each counted id records it
under `ConsensusRoundEvidenceOwner(asset, id)`. A later round that sees the same
id bound to a different round is rejected with `RoundEvidenceReplay`.

Because the id commits to the round, a *genuine* second observation of the same
market value in a later round has a different id and is unaffected — the replay
check rejects reusing an observation, never re-observing a market.

**Test.** `test_a_round_binds_its_evidence_so_it_cannot_be_replayed`.

### 2. Equivocation

**Attack.** A participant submits different values to different observers inside
the same round, so no single view of the round shows a contradiction.

**Mechanism.** The vote is keyed by `(asset, round, source)`, so there is only
ever *one* stored observation per source per round — a split submission cannot
exist on-chain. A second, different value is detected by comparison against the
stored vote and rejected with `RoundEquivocation`.

The durable penalty is applied by a **separate** call, `report_equivocation`.
This is a necessity, not a stylistic choice: the conflicting submission panics,
and a panicking call rolls back every write it made, so a penalty recorded
inside it would leave no trace at all. Reporting separately makes the penalty
durable and the event observable. The report is permissionless because the
*evidence* is on-chain — a source must already have a vote in that round — so a
reporter can only surface a contradiction, never invent one.

Effects, applied atomically in `report_equivocation`:

* the source is barred for the remainder of the round;
* its lifetime equivocation counter increments (attributable across rounds);
* a `RoundEquivocationEvent` is emitted.

The source's original vote is left in place: it is the honest observation the
tally uses, and rewriting it would let the equivocator choose which value
survives after the fact.

**Tests.** `test_equivocation_is_detected_and_the_original_vote_stands`,
`test_equivocating_source_is_barred_for_the_rest_of_the_round`,
`test_identical_resubmission_is_idempotent_not_equivocation`,
`test_equivocation_penalty_is_durable_cumulative_and_permissionless`.

### 3. Withhold / stall

**Attack.** An adversary stalls one round to hold the last-known-good value
indefinitely.

**Mechanism.** Every round is bounded by `round_ledgers`. After that deadline:

* `abandon_stalled_round` can be called by **anyone**, opening a fresh round;
* a vote arriving after the deadline is rejected with `RoundExpired`, so a
  stalled round cannot be quietly back-filled;
* before the deadline the round is *not* abandonable, so a healthy round cannot
  be griefed into a restart.

The bound is therefore: **finalization is delayed by at most `round_ledgers`
ledgers per stalled round, and never blocked outright.** `get_round_status`
reports `stalled` so a keeper can act without polling internals.

**Tests.** `test_a_stalled_round_can_be_abandoned_within_the_documented_bound`,
`test_a_vote_after_the_deadline_is_rejected`.

### 4. Round-boundary gaming

**Attack.** A manipulation lands exactly on the final confirming round, so the
"last" round is the manipulated one.

**Mechanism.** Two independent defences:

* every round requires its **own** quorum of distinct sources, so a round cannot
  inherit the previous round's evidence (see attack 1);
* the confirming run must be **mutually consistent**: every pair of adjacent
  medians must agree within `agreement_bps`. A single rogue median outside the
  band breaks the run and `finalize_confirmation` refuses with `NoData`.

Only the first `quorum` distinct sources in a round count, and the tally is
written once, so a later flood of votes cannot retroactively move a tallied
round's median.

**Tests.** `test_a_manipulation_on_one_round_breaks_the_confirming_run`,
`test_a_run_within_the_band_finalizes_and_reports_its_worst_spread`.

## Confirmation

`finalize_confirmation` collects the most recent run of consecutive tallied
rounds, newest first. It requires:

1. `required_rounds` consecutive rounds, each with a stored tally;
2. every adjacent pair of medians within `agreement_bps`.

It then stores and returns:

```text
ConfirmedPrice {
  price,            // median of the confirming rounds' medians
  decimals,
  rounds,           // the consecutive round identities, newest first
  spread_bps,       // worst adjacent spread observed across the run
  finalized_ledger,
  timestamp,        // newest counted observation across the run
}
```

`spread_bps` is reported rather than merely checked, so a consumer can see how
tight the agreement actually was.

## Configuration bounds

| Field | Bound | Rejected because |
|---|---|---|
| `required_rounds` | `1..=16` | `0` would demand an empty run; a large value lets a price be withheld for a long time |
| `quorum` | `1..=64` | `0` can never be met; a large value makes the per-round scan arbitrarily expensive |
| `round_ledgers` | `>= 1` | `0` would make every round immediately abandonable — a liveness bug, not a policy |
| `agreement_bps` | `<= 10 000` | above 100% is not a spread |

All violations raise `InvalidConfiguration`.

## Errors

| Code | Meaning |
|---|---|
| `RoundNotFound` (164) | the round is not the asset's current round, or has no vote to report against |
| `RoundExpired` (165) | the round's deadline has passed; no further observations, and it is abandonable |
| `RoundEvidenceReplay` (166) | the observation was already consumed by an earlier round |
| `RoundEquivocation` (167) | a second, different observation inside one round |
