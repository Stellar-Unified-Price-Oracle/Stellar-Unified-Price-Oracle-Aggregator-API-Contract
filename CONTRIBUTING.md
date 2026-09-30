# Contributing

Thanks for your interest in contributing to the Stellar Unified Price Oracle Aggregator.

## Getting Started

New here? Start with the [onboarding guide](docs/ONBOARDING.md) to pick a first task and a mentor.
Fastest setup: `make dev` provisions everything and runs the suite ([details](docs/dev-environment.md)).


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

## Contributor Ladder

The contributor ladder is a security control as much as a recognition scheme. Access escalates
only when a contributor demonstrates the judgment required to exercise that access safely.
Contribution volume alone does not justify promotion.

### Levels

| Level | Permissions | Entry Criteria |
|---|---|---|
| **Contributor** | Fork, open PRs, file issues | First merged PR |
| **Triager** | Label issues, request reviews, close stale issues | ≥3 merged PRs *and* at least one documented security-review action (see below) |
| **Maintainer** | Merge PRs (non-upgrade paths), manage releases | ≥2 months as Triager, second Maintainer approval, documented security-judgment evidence |
| **Core Maintainer** | Merge upgrade/admin-path PRs, rotate signing keys | Separate nomination; requires unanimous current Core Maintainer approval |

### Least-Privilege Permission Map

Each rung is limited to the minimum permissions required:

- **Contributor** — no repository write access; no CI secrets access.
- **Triager** — GitHub Triage role only; no merge rights; no secrets.
- **Maintainer** — GitHub Write role; can merge to non-protected branches; no key/deploy access.
- **Core Maintainer** — GitHub Admin role (scoped); upgrade-path merge rights; signing-key rotation.

### Security Judgment Requirement

Promotion to **Triager or above** requires at least one documented instance of security judgment. Examples:

- Caught a validation gap or missing auth check in a PR review.
- Rejected a plausible-but-unsafe change with a written rationale.
- Identified a contract invariant violation in a proposed change.

Document the instance in the promotion PR (reference the PR/issue number where the judgment was exercised).

### Inactivity Expiry

Privileged access (Triager and above) expires automatically after **90 days of inactivity**
(no merged PR, no substantive review, no triaging action). On expiry:

1. Access is downgraded one level automatically.
2. The contributor is notified and may request reinstatement by demonstrating current activity.

A dry run of the expiry process is performed quarterly by a Core Maintainer, with results documented as an issue.

### Removal and Demotion

Any maintainer may propose removal or demotion by opening an issue with:

- The rung being revoked.
- The specific trigger (inactivity, conduct, security incident, role change).
- Evidence supporting the action.

Removal is effective after 48 hours unless disputed. Disputed removals require a majority vote
of Core Maintainers. Removed access is logged in the project's access-change history.

### Security-Relevant Promotions

Promotions to Maintainer or Core Maintainer require a **second Maintainer's approval** documented
in the promotion PR. The approving Maintainer must not be the nominee's primary collaborator and
must independently verify the security-judgment evidence.

### Automated Recognition

- GitHub Achievements and profile badges are assigned automatically on promotion.
- The README contributor table is regenerated on each merge to `main`.
- All access changes are logged as GitHub issues under the `access-change` label.

---

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
