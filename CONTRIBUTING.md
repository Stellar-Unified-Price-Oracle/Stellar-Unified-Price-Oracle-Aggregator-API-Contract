# Contributing

Thanks for your interest in contributing to the Stellar Unified Price Oracle Aggregator.

## Getting Started

1. Fork and clone the repo.
2. Install Rust (stable) with the `wasm32v1-none` target:
   ```bash
   rustup target add wasm32v1-none
   ```
3. Build the contract:
   ```bash
   cargo build -p price-oracle --target wasm32v1-none --release
   ```
4. Run tests:
   ```bash
   cargo test -p price-oracle --lib
   ```

All 56 tests should pass with zero warnings.

## Code Style

- Run `cargo fmt` before committing (formatting is enforced in CI).
- Run `cargo clippy -- -D warnings` and fix any warnings (also enforced in CI).
- Follow the existing patterns in the codebase — see `types.rs`, `storage.rs`, `events.rs` for reference.
- Keep functions focused and modular.
- Use meaningful names for types, fields, and variables.

## Making Changes

1. Create a branch off `main`.
2. Make your changes, keeping commits small and focused.
3. Add or update tests in `test.rs` to cover your changes.
4. Ensure all tests pass and clippy is clean.
5. Open a pull request.

## Pull Request Guidelines

- Link the PR to the issue it resolves.
- Describe what the change does and why.
- Mention any breaking changes or migration steps.
- Keep PRs focused on a single concern — split large changes into multiple PRs.

## Reporting Issues

- Check existing issues before opening a new one.
- Include the error code, the function called, and a minimal reproduction.
- For feature requests, describe the use case and how it fits the oracle aggregator model.

## Governance Proposals

For proposals related to contract upgrades, parameter changes, source additions/removals, or asset management, use the standardized template:

👉 [Governance Proposal Template](docs/governance-proposal-template.md)

Submit completed proposals as a GitHub issue or pull request for community review before any on-chain action is taken.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
