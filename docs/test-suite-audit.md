# Test Suite Audit — Tautological and Duplicate Tests (#829)

Goal: shrink the suite (fewer tests, faster wall clock) **without lowering the
mutation score**. A test earns its place only if it kills at least one mutant or
guards a behaviour no other test guards.

## Method

1. Run the mutation gate and keep the per-module artifacts:

   ```bash
   ./scripts/mutation-gate.sh per-module   # writes mutants.out per module
   ./scripts/mutation-gate.sh critical
   ./scripts/mutation-gate.sh general
   ```

2. For every surviving mutant, record the file/line and the tests that cover it.
   A test that appears in **no** kill list for its module is a candidate.
3. Classify each candidate:
   - **tautological** — asserts on a value the test itself just wrote, or
     asserts `is_some()`/`is_ok()` without checking the payload, or re-asserts a
     constant that the contract never computes.
   - **duplicate** — same call sequence and same assertions as another test in
     the same module (or in `test.rs` vs a `*_tests.rs` file).
   - **load-bearing** — kills a mutant; keep, even if it looks redundant.
4. Merge duplicates into one test that keeps the union of the assertions;
   delete tautological tests whose behaviour is covered elsewhere. Never delete
   the last test covering a branch.
5. Re-run the gate. The mutation score per module must be **>= the score before
   the audit**; if it drops, restore the deleted test.

## Known candidates (from reading the suite)

| File | Test | Why |
|---|---|---|
| `asset_inactivity_tests.rs` | `test_submit_price_records_activity` | Only asserts `!is_asset_inactive` right after setting a 100s timeout — the same assertion as `test_is_asset_inactive_false_when_disabled`; it never observes the recorded activity. Either assert the stored last-activity value or fold into the disabled test. |
| `emergency_pause_tests.rs` | `test_emergency_pause_details` | Subset of `test_emergency_pause_activates` plus a reason check; merge the reason assertion into the activate test. |
| `test.rs` | `test_initialize_defaults` | Overlaps `test_initialize`; keep only if it is the sole killer of the `0 => default` branches in `initialize`. Verify against `mutants.out` before deleting. |

## Recording counts

Add the before/after numbers to this file when the audit lands:

```
before: <n> tests, <t> s   mutation score: <per-module table>
after:  <n> tests, <t> s   mutation score: <per-module table>
```

## Done when

- Suite test count and runtime are lower, recorded above.
- `./scripts/mutation-gate.sh all` passes with no module below its threshold and
  no module score lower than the pre-audit baseline.
- `cargo test -p price-oracle --lib`, `cargo clippy -- -D warnings`,
  `cargo fmt -- --check` are clean.
