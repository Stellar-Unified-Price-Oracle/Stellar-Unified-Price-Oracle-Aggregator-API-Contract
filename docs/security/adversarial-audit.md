# Adversarial Audit: Fees, Governance, Relayer Bonds, TTL Eviction

Covers issues #458, #459, #460 and #463. Tests live in
`contracts/price-oracle/src/adversarial_security_tests.rs`.

> These scenarios are also mirrored into the
> [attack-regression corpus](attack-regression-corpus.md) (#511), which pins
> every historical attack by class and runs as its own budgeted CI job. New
> adversarial findings should be added there — see its intake process.

## #458 — Fee market (`fee_market.rs`)

### Rounding audit

| Computation | Formula | Rounding | Favours |
|---|---|---|---|
| Floor check | `priority_fee < min_fee` → reject | exact (integer compare) | protocol |
| Source share | `fee * ratio / 100` | floor | protocol |
| Treasury share | `fee - source_share` | receives the remainder | protocol |
| Pool accrual | `pool.saturating_add(fee)` | exact | — |

No computation rounds against the protocol. The source share is always `≤ fee * ratio / 100`
and `source_share + treasury_share == fee` (test `fee_split_rounds_in_protocol_favour`).

### Floor
`enqueue_submission` rejects any `priority_fee` below `FmMinPriorityFee` with
`FeeMarketBelowMinimum`. There is no discount path, so no admissible submission pays less
than the floor (test `fee_below_floor_is_always_rejected`).

### Oscillation and cornering
Fees are bid, not algorithmically priced: there is no base fee that reacts to demand, so an
attacker cannot drive the market between extremes. The only lever is outbidding, which
costs the attacker `(100 - ratio)%` of every bid (20% by default) as a permanent treasury
loss. An honest source's cost is bounded by `min_priority_fee` per submission and is
unaffected by the attacker's bids. Accepted cost: honest submissions can be delayed by at
most `ceil(queue_ahead / MAX_PROCESS_PER_LEDGER)` processing rounds (20 per round).

### Targeted griefing
Outbidding one source does not reduce its earnings (fees are credited per submission) and
the attacker forfeits the treasury share of everything it spends, so removing a source by
griefing is never profitable (test `targeted_griefing_is_paid_by_attacker_only`).

### Parameter changes under occupancy
`fm_set_min_priority_fee` and `fm_set_fee_distribution_ratio` write config directly and do
not touch the queue, so a full queue cannot block them; the new floor applies immediately
(test `parameter_change_applies_under_adversarial_occupancy`).

## #459 — Governance capture

**Finding (informational):** the contract has no token-weighted vote-delegation or
quadratic-voting path. `vote_delegation_tests.rs` and `quadratic_voting_tests.rs` reference
endpoints (`delegate_voting_power`, `create_proposal`, …) that do not exist and are not
compiled. Governance weight comes only from the admin-set multisig governor list
(`multisig.rs`), one identity = one approval.

- **Capital-to-influence curve:** flat. Capital buys no weight; capture requires control of
  `required_approvals` of the governor set, which only the admin can modify. Sybil
  identities are worthless because the governor list is permissioned (the identity-cost
  assumption is "admin does not add attacker identities"; if the admin key is compromised,
  capture is total — severity: critical, mitigated by `recovery.rs` guardians).
- **Duplicate votes:** `approve_operation` rejects repeat approvals (`AlreadyApproved`) —
  test `governor_cannot_approve_twice`.
- **Direct + delegated voting:** impossible; there is no delegation path for governance
  votes (`delegate_role` in `rbac.rs` grants roles, not approvals).
- **Snapshot-boundary double voting:** removing and re-adding a governor does not reset its
  recorded approval — test `snapshot_boundary_rotation_cannot_double_vote`. Retract +
  re-approve counts once — test `retract_then_reapprove_counts_once`.
- **Note:** an approval from a governor later removed from the set still counts towards
  quorum. Admins rotating out a compromised governor should also have it retract, or cancel
  pending operations.

## #460 — Relayer bonds (`relayer_bonds.rs`)

### Lifecycle and windows
`deposit → (record_relayer_failure)* → slash_relayer → withdraw`. The window between an
offence and its punishment is the time until the admin records a failure and then slashes.

**Fixed:** `withdraw_relayer_bond` previously had no lock, so a relayer could withdraw its
full bond as soon as a failure was reported. Withdrawal now fails with `RelayerBondLocked`
while `failure_count > 0` (pending dispute / executable slash) — test
`withdrawal_blocked_while_dispute_pending`. Operators must call `record_relayer_failure`
before (or in the same transaction as) a forced slash, since a relayer with no recorded
failure can still withdraw.

### Deterrent margin
One slash removes `slash_percent` (default 20%) of the bond `B`. Deterrence requires
`profit_per_offence < 0.2 · B`. A relayer can commit up to `failure_threshold` (default 3)
offences before the slash is executable, and slashing resets the counter, so the
worst-case margin is `3 · profit_per_offence < 0.2 · B`, i.e. `B > 15 × profit_per_offence`.
**Finding (medium):** with defaults, repeated offences can out-earn a single slash unless
`B` is sized to at least 15× the maximum per-offence profit; admins should set
`slash_percent` to 100 for unauthorized-price offences via `set_relayer_slash_percent`.

### Valuation risk
The bond is held in the configured stake token and valued at slash time. If the token
falls by `d`% between deposit and slash, the effective deterrent falls by the same `d`%.
Worst case gap is the token's max drawdown over the dispute window; size `B` accordingly.

### Identity rotation
The bond and failure count are keyed by relayer address; de-listing the relayer
(`remove_relayer`) does not release either — test `identity_rotation_cannot_escape_slash`.
A fresh identity starts with no bond and must deposit before relaying.

## #463 — TTL eviction (`storage.rs`, `admin.rs`)

| Key | Eviction consequence | Behaviour |
|---|---|---|
| `Admin` | admin ops unusable | fail closed (`unwrap` panics) — `evicted_admin_fails_closed` |
| `CfgMinSources` | quorum drops to default | **Fixed:** was fail-open (default 1); now `ConfigMissing` — `evicted_min_sources_fails_closed` |
| `SrcActive(addr)` | source rejected | fail closed (`NotAuthorized`) — `evicted_source_entry_fails_closed` |
| Asset registration | asset rejected | fail closed (`AssetNotRegistered`) |
| `FmMinPriorityFee` | floor falls to 0 | **Open:** falls back to `DEFAULT_MIN_PRIORITY_FEE`; admin must re-set after eviction |
| Price history entries | gap in history | **Open:** not distinguished from "no data"; monitoring should compare `get_price_history` against emitted price events |

- **Re-submission after eviction:** not yet covered by a test; tracked as open follow-up.
- **TTL starvation:** every read of a hot key (`CfgMinSources`, fee queue, bonds) extends
  its TTL by `LEDGER_BUMP` (40 000 ledgers) once below `LEDGER_THRESHOLD`; extensions are
  per-key, so state growth in other keys cannot starve them. Rent cost grows linearly with
  entries written by sources, bounded by `max_history` per asset.
