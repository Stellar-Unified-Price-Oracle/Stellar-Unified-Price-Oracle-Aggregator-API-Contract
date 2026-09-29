# Cross-Module Seam Invariant Catalogue (#411)

Single-module tests stay green while the composition is broken. This catalogue
lists the high-risk module pairs, the invariant that must hold across each
boundary, and the regression test that enforces it
([`cross_module_seam_tests.rs`](../contracts/price-oracle/src/cross_module_seam_tests.rs)).
Test names follow `seam_<module_a>_<module_b>__<invariant>`.

| Pair | Invariant | Test | Ordering |
|------|-----------|------|----------|
| pause ↔ submission | A pause between the two halves of a quorum rejects the second half with no partial effect; the rejected value is never counted | `seam_pause_submission__rejected_half_leaves_no_partial_state` | interleaved (submit → pause → submit → unpause → submit) |
| pause ↔ submission | Pausing before a round, then unpausing, counts only post-unpause submissions | `seam_pause_submission__reversed_order_only_counts_accepted` | reversed |
| global pause ↔ asset pause | Lifting the global pause never lifts an asset-level pause | `seam_global_pause_asset_pause__unpause_does_not_clear_asset_pause` | interleaved |
| pause ↔ freeze | Lifting the global pause never unfreezes a frozen price; frozen aggregate cannot move | `seam_pause_freeze__unpause_does_not_unfreeze` | interleaved |
| sources ↔ aggregation | A source removed between rounds has no influence on later aggregates and cannot submit | `seam_sources_aggregation__removed_source_has_no_influence` | interleaved (submit → remove → round) |
| validation ↔ quorum | A rejected submission leaves the quorum intact; a single source never meets `min_sources_required = 2` | `seam_config_aggregation__quorum_holds_after_rejected_submission` | sequential |

The first and fifth rows are interleavings a single-module suite cannot express:
they require the pause (or source-registry) module to change state *between*
two calls into the pricing module.

## Mutant demonstration

Disabling the pause guard in `pause::check_not_paused`
(`if is_paused(env)` → `if false && is_paused(env)`) makes both
`seam_pause_submission__*` tests fail:

```
test cross_module_seam_tests::seam_pause_submission__rejected_half_leaves_no_partial_state ... FAILED
test cross_module_seam_tests::seam_pause_submission__reversed_order_only_counts_accepted ... FAILED
test result: FAILED. 4 passed; 2 failed
```

## Not yet covered

Timelock ↔ multisig, circuit breaker ↔ aggregation, RBAC ↔ admin rotation and
finality ↔ upgrade are listed in the issue but not yet exercised here; they
need their own seam tests.
