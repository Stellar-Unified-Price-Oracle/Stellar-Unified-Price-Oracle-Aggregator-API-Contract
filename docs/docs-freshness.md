# Docs Freshness Gate

`python3 scripts/docs_freshness.py` (CI job `docs-freshness`, runs on doc and
contract changes; `--external` weekly) fails on the following. How to fix each:

| Failure | Meaning | Fix |
|---|---|---|
| `missing file X` | relative link target doesn't exist | fix the path, or add the file |
| `dead link URL` | external link returned an error (`--external` only) | update it; if it is legitimately unreachable from CI, add it to [`link-allowlist.txt`](link-allowlist.txt) with a reason |
| `<lang> snippet does not compile` | a `json`, `python`, `bash` or `rust,compile` block is invalid | fix the snippet; for intentionally partial code tag the fence `ignore` (e.g. `json ignore`) |
| `example calls fn, which is not a contract endpoint` | a `client.fn(...)` or `invoke ... -- fn` example uses a function not in `lib.rs` | rename to the current endpoint or delete the example |
| `examples pinned to contract A, current is B` | a `<!-- contract-version: A -->` marker is stale | re-verify the examples, then bump the marker |
| `baseline entry fixed` | a known problem no longer reproduces | delete its line from [`docs-freshness-baseline.txt`](docs-freshness-baseline.txt) |

Snippet conventions: `<PLACEHOLDER>` values in shell blocks are accepted. Rust
blocks are only compiled when tagged `rust,compile` and must be dependency-free.

## Allowlist and baseline ownership

Both files are owned by the maintainers via `.github/CODEOWNERS`, so any
addition needs their review. The baseline lists problems that predate the gate;
it may only shrink.

The checker's own behaviour is demonstrated by `scripts/test_docs_freshness.py`.
