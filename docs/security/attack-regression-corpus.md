# Attack-regression corpus

> Issue **#511**. See also [formal verification](formal-verification.md) and
> the [state-machine model](state-machine-model.md).

A security fix that is later reverted by an unrelated refactor is a *silent*
regression: nothing fails, CI is green, and the vulnerability is live again.
The only durable defence is a corpus that pins each historical attack as a test
and runs on every change.

This document describes that corpus, how to run it, and — most importantly —
**how to add a new attack to it**.

## What is in the corpus

`contracts/price-oracle/src/attack_regression_tests.rs` holds every pin. Each one
declares the **attack class** it pins, and the class is enforced: an entry whose
class is not in `ATTACK_CLASSES` fails its own test, and
`corpus_entries_are_complete` fails if a registered class has no pin.

| Class | Meaning | Example pin |
|---|---|---|
| `reentrancy` | Re-entering a state-mutating path mid-execution | `reentrancy_guard_rejects_nested_entry` |
| `replay` | Re-submitting an already-accepted payload | `replay_nonce_must_be_strictly_increasing` |
| `malformed-payload` | Structurally invalid input that must be rejected | `malformed_non_positive_price_is_rejected` |
| `quota-evasion` | Circumventing a limit, cap, or per-window bound | `quota_evasion_open_challenges_are_bounded` |
| `manipulation` | Influencing the published aggregate | `manipulation_outlier_cannot_move_median` |
| `authorization` | Acting without the required authority | `authorization_removed_source_loses_authority` |
| `fail-open` | Degraded storage restoring a permissive default | `fail_open_evicted_min_sources_fails_closed` |

## Running it

```sh
make attack-gate
```

This runs only the corpus module, under an explicit wall-clock budget
(`ATTACK_BUDGET_SECONDS`, default 300). A budget breach **fails** the gate: a
corpus that cannot run on every PR is a corpus that stops being run. The script
also refuses to pass if the module is unregistered or contains no pins, so the
gate cannot become vacuous.

In CI it is its own job (`attack-regression` in `.github/workflows/ci.yml`) with a
15-minute job timeout, so the corpus is budgeted separately from the rest of the
suite. Current cost: **22 tests in ~2.8 s**.

## Does the corpus actually have teeth?

A regression corpus that passes on vulnerable code is worse than no corpus,
because it is trusted. `scripts/attack-regression-mutation-check.sh` proves the
opposite: it mechanically reverts historical fixes, one at a time, and requires
the corpus to fail for each.

```sh
./scripts/attack-regression-mutation-check.sh
```

Each mutation is a single exact-string replacement that undoes one fix. The
working tree is restored on exit, including on failure or interrupt.

Three fixes are currently pinned this way:

| # | Reverted fix | Caught by |
|---|---|---|
| 1 | `reentrancy::enter` no longer refuses a nested entry | `reentrancy_guard_rejects_nested_entry` |
| 2 | `compute_vwap` no longer skips non-positive volume | `manipulation_negative_volume_carries_no_weight` |
| 3 | `get_min_sources_required` falls back to `1` instead of failing closed | `fail_open_evicted_min_sources_fails_closed` |

All three are caught. If you add a fix and want it pinned this way, add a
mutation to the script — the acceptance bar is that reverting the fix fails the
corpus.

## Adding a new attack to the corpus

This is the intake path. It is deliberately short.

1. **Reproduce the attack as a failing test.** Write the test so that it *fails*
   against the vulnerable behaviour and passes against the fix. If you cannot
   make it fail first, it is not a regression pin — it is a feature test, and it
   belongs in the module that owns the feature.

2. **Choose the attack class.** Add it to `ATTACK_CLASSES` if it is genuinely
   new, and add it to the `pinned` list in `corpus_entries_are_complete`.

3. **Write the pin with the `attack!` macro**, naming the class:

   ```rust
   attack!("manipulation", manipulation_attacker_cannot_inflate_vwap, {
       // Arrange the attack, assert the *security-relevant outcome*:
       // an error code, a bound, or an unchanged aggregate.
   });
   ```

4. **Assert the outcome, not the call.** A pin that only checks "the call
   succeeded" or "the call failed" without naming *why* is not a regression pin.
   Prefer `assert_eq!(result, Err(Ok(ErrorCode::X.into())))` or an explicit
   bound.

5. **Prove it has teeth.** Temporarily revert the fix and confirm your new pin
   fails. If it still passes, the pin is not testing what you think.

6. **Wire it into the mutation check** if the fix is a durable one worth
   protecting (add a `revert` block to
   `scripts/attack-regression-mutation-check.sh`).

7. **Document it** — a short section in the relevant doc under `docs/security/`,
   and a link from the table above if it introduces a new class.

### Review checklist for a new pin

- [ ] Declares an attack class that is in `ATTACK_CLASSES`
- [ ] Fails when the fix is reverted (demonstrated, not assumed)
- [ ] Asserts a specific error code, bound, or unchanged value
- [ ] Does not depend on wall-clock time or entropy (the suite must be
      reproducible)
- [ ] Runs fast — the whole corpus is budgeted, so a slow pin taxes every PR

## Relationship to the other suites

The corpus is not the only defence; each layer catches what the others cannot.

| Suite | Catches | Cannot catch |
|---|---|---|
| Attack-regression corpus (#511) | A *specific* historical attack, returning | Novel attacks nobody has thought of |
| Bounded proofs (#512) | Arithmetic violations across the whole bounded domain | State/sequence bugs, anything outside the bound |
| State-machine traces (#513) | Interaction bugs across *sequences* of calls | Anything outside the modelled operations |
| Coverage-guided fuzzing (#514) | Deep, structurally-unexpected inputs | Rare sequences; it samples one call at a time |
| cargo-mutants (#412) | Fixes whose logic is redundant with the tests | Behavioural changes that are not mutants |

When a new vulnerability is found, it should enter the corpus *and* whichever
other layer fits. A manipulation bug in the VWAP belongs in the corpus, in a
`fuzz_aggregation_invariants` seed, and in the proof for the VWAP bound.
