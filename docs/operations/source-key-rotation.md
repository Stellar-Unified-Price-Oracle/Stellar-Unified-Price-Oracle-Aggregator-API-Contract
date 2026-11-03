# Source key rotation runbook (#641)

Rotating a source's signing key without a gap in that source's submissions.
The procedure is rehearsed on testnet by `scripts/testnet-key-rotation.sh`
(run by the Testnet Lifecycle workflow) and natively by
`key_rotation_rehearsal_tests.rs`.

## Procedure

1. **Pre-check.** Confirm the outgoing key is a source
   (`is_source --source <old>`) and its latest submission was accepted.
2. **Stage.** Add the incoming key: `add_source --source <new> --name <name>`.
   Leave the outgoing key in place.
3. **Verify the overlap.** Both keys are sources, and a submission from each is
   accepted. Switch the feeder to the incoming key.
4. **Retire.** Remove the outgoing key: `remove_source --source <old>`.
   Confirm a submission signed with it is rejected. Removal recomputes the
   aggregates the old key fed; if more than two assets are affected, finish the
   ones listed in `RecomputeDeferredEvent` with `recompute_asset_price`.

## Rollback

Before step 4, remove the incoming key (`remove_source --source <new>`). The
outgoing key keeps working throughout, so an aborted rotation does not
interrupt the feed. A rolled-back key can be added again later.

## Notes

- `rotate_source_key` moves a source's registration to a new address in one
  call. Prefer the staged procedure above when the feeder cannot switch keys
  atomically.
- Keep `min_sources_required` satisfiable during the overlap: the outgoing key
  still counts until it is removed.
