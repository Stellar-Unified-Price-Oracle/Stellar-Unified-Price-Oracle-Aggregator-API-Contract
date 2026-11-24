## Description

<!-- What does this PR do? Why is it needed? -->

## Related issue

<!-- Link to the GitHub issue this resolves, e.g., Closes #123 -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / performance
- [ ] Documentation
- [ ] CI / tooling

## Testing

- [ ] All existing tests pass (`cargo test -p price-oracle --lib`)
- [ ] New tests added for the change
- [ ] Clippy is clean (`cargo clippy -- -D warnings`)
- [ ] Formatting is clean (`cargo fmt -- --check`)

## Checklist

- [ ] My code follows the existing code style and patterns
- [ ] I have updated the README if needed
- [ ] Events are emitted for state-changing operations
- [ ] Authorization checks are in place for admin-only functions
- [ ] WASM size delta, endpoint delta, `DataKey` variants and migration notes sections are filled in

## Interface changes
- [ ] Ran `scripts/devx/gen_interface.py` and `scripts/devx/gen_error_registry.py`
- [ ] Ran `scripts/devx/gen_api_reference.py` (regenerates `docs/api-reference.md`)
- [ ] Breaking change? Bumped `interface/package.json` and added a guide in `docs/migrations/`

## WASM size delta

<!--
Pre-filled by the WASM size CI gate. Do not edit the numbers by hand: if the
gate has not run yet, leave the placeholders and re-run CI. The gate compares
the `price_oracle.wasm` built from this PR against the same artifact built from
the merge base on `main` and writes the table below into this section.
-->

| Artifact | Size (bytes) | Delta vs `main` |
|----------|--------------|-----------------|
| `price_oracle.wasm` | `<pre-filled by CI>` | `<pre-filled by CI>` |

- [ ] Size delta reviewed (a positive delta is justified below, or the PR is size-neutral)
- [ ] Still within the Soroban contract size limit

Justification for a size increase (required if delta > 0):

<!-- e.g. new endpoint, extra validation, larger error enum -->

## Endpoint delta

<!-- List every exported contract function added or removed. Write "none" if the ABI is unchanged. -->

- Added endpoints:
- Removed endpoints:
- Changed signatures (name / params / return type):

- [ ] `docs/abi.md` and `docs/abi.json` regenerated (`./scripts/generate-abi.sh`) if the ABI changed

## Storage layout impact

<!-- New or removed `DataKey` variants, and any change to the value type stored under an existing key. -->

- New `DataKey` variants:
- Removed `DataKey` variants:
- Changed value types / storage tier (persistent vs temporary) for existing keys:

- [ ] `docs/ARCHITECTURE.md` storage-key table updated if keys changed
- [ ] No existing on-chain entries are orphaned or misread by this change

## Migration notes

<!-- Required for any storage-layout or interface change that affects deployed state. Write "none" if not applicable. -->

- Affected deployments (testnet / mainnet):
- Migration steps (script, governance proposal, or manual call sequence):
- Rollback plan:
- Migration guide added under `docs/migrations/` (link):

## Mainnet release freeze

<!-- Required for any PR that targets a mainnet release. See the freeze and exception procedure in the README. -->

- [ ] This PR is not part of a mainnet release, or the freeze window does not apply
- [ ] Freeze window respected: no code/config changes merged after the freeze start (T-72h before the release tag) unless an exception is recorded below
- [ ] Release record updated with the freeze decision (see template below)

### Exception record (only if a change lands during the freeze)

- Reason:
- Scope of change:
- Risk assessment / rollback plan:
- Approver (role): <!-- Release Manager + one of: Protocol Lead, Security Lead -->
- Approval link (issue/PR comment):

### Release record template

```
Release:            <version / tag>
Freeze start:       <UTC timestamp, T-72h>
Freeze end:         <UTC timestamp, release tag published>
Frozen surface:     contract code, interface package, deploy config, CI release workflow
Freeze decision:    [ ] held  [ ] exception granted
Exception reason:   <required if exception granted>
Approved by:        <Release Manager> + <Protocol Lead | Security Lead>
Approval link:      <issue/PR comment URL>
```
