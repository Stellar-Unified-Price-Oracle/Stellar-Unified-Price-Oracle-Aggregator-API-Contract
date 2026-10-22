# Contract Interface Package

The contract interface is published to npm as **`@stellar-unified-price-oracle/interface`**, containing:

| File | Contents |
|---|---|
| `interface.json` | Every `#[contractimpl]` function with params/return types (generated from `lib.rs`) |
| `errors.json` | The [error registry](errors/REGISTRY.md) |
| `versions.json` | Interface version → contract version, WASM hash, per-network contract IDs |

```bash
npm install --save-exact @stellar-unified-price-oracle/interface@0.1.0
```
```js
const { interface: iface, errorByCode, versions } = require("@stellar-unified-price-oracle/interface");
```

## Generation

`python3 scripts/devx/gen_interface.py` regenerates `interface/interface.json`. CI runs it with `--check`.

## Semver rules (enforced in CI)

| Change | Required bump (≥1.0) | Required bump (0.x) |
|---|---|---|
| Function removed / signature changed / error code removed or renumbered | major | minor |
| Function added | minor | patch |
| Docs only | none | none |

A breaking bump also requires a migration guide ([docs/migrations](migrations/README.md)).

## Traceability

Releases are cut by pushing a tag `interface-vX.Y.Z`. The `publish-interface` workflow builds the WASM,
records its SHA-256 as `wasm_hash`, checks that `versions.json` has an entry for the version, and
publishes with **npm provenance** (`--provenance`), so each package links to the exact commit and build.
After deploying, add the network contract IDs to `versions.json` in the next release.

## Deprecation policy

Functions are deprecated for at least 90 days before removal; see [migrations/README.md](migrations/README.md).
Deprecated functions carry a `deprecated` field in `interface.json`.
