# Consumer quickstart templates

| Template | Consumer | Uses | CI |
|---|---|---|---|
| [web](web/) | dApp / frontend | SEP-40 `lastprice` + `decimals` (read-only) | `npm test` |
| [python-bot](python-bot/) | Price submitter | `submit_price` (signed) | `pytest` |
| [indexer](indexer/) | Data pipeline | RPC `getEvents` | `npm test` |

All templates are built and smoke-tested by the `templates` job in `.github/workflows/devx.yml`,
dependency versions are pinned exactly and bumped by Dependabot (PRs must pass the same job),
and `scripts/devx/lint_deprecated.py` flags any use of deprecated contract functions.
