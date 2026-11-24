# Consumer quickstart templates

Five-minute consumer quickstarts per language (#874):

| Quickstart | Language / Target | Description | CI |
|---|---|---|---|
| [rust-consumer](rust-consumer/) | Rust (Soroban contract) | Cross-contract SEP-40 `lastprice` read | `cargo test` |
| [typescript-dapp](typescript-dapp/) | TypeScript (Frontend / Node) | dApp price reading via `@stellar/stellar-sdk` | `npm test` |
| [python-script](python-script/) | Python (Script / Backend) | Python price reading via `stellar-sdk` | `pytest` |

Additional integration templates:

| Template | Consumer | Uses | CI |
|---|---|---|---|
| [web](web/) | dApp / frontend | SEP-40 `lastprice` + `decimals` (read-only) | `npm test` |
| [python-bot](python-bot/) | Price submitter | `submit_price` (signed) | `pytest` |
| [indexer](indexer/) | Data pipeline | RPC `getEvents` | `npm test` |

All templates are built and smoke-tested by the `templates` job in `.github/workflows/devx.yml`,
dependency versions are pinned exactly and bumped by Dependabot (PRs must pass the same job),
and `scripts/devx/lint_deprecated.py` flags any use of deprecated contract functions.
