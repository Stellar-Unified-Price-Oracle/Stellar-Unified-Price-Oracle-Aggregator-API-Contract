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
