# Migrating to vX.Y.Z

- **Interface version:** X.Y.Z (previous: A.B.C)
- **Contract WASM hash:** see `interface/versions.json`
- **Deprecation window:** <since> → <removal date>

## Summary of breaking changes

| Change | Kind (removed / signature / behaviour / error code) | Replacement |
|---|---|---|

## Before / after

### Before (A.B.C)
```js
```

### After (X.Y.Z)
```js
```

## Behavioural changes (no signature change)

## Checklist for consumers
- [ ] Bump `@stellar-unified-price-oracle/interface` to `X.Y.Z`
- [ ] Run `python3 scripts/devx/lint_deprecated.py <your-src>`
