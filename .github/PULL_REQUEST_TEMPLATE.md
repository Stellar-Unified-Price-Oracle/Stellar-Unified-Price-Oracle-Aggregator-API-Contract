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

## Interface changes
- [ ] Ran `scripts/devx/gen_interface.py` and `scripts/devx/gen_error_registry.py`
- [ ] Breaking change? Bumped `interface/package.json` and added a guide in `docs/migrations/`

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
