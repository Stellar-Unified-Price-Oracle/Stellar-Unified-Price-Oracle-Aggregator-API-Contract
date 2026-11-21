# SDK Package Publishing Pipeline (#872)

This document describes the automated publishing workflow for the official client SDKs:
- **TypeScript / JavaScript:** `@stellar-oracle/sdk` on [npm](https://www.npmjs.com/)
- **Python:** `stellar-oracle-sdk` on [PyPI](https://pypi.org/)
- **Rust:** `stellar-oracle-sdk` on [crates.io](https://crates.io/)

## Trigger and Conditions

All three SDK packages publish automatically from the `.github/workflows/release.yml` GitHub Actions workflow whenever a release tag matching `v*` is pushed.

The publishing jobs depend on the upstream `release` job completing successfully (including contract builds, optimization, checksum verification, and provenance attestation).

## Authentication & Provenance

### npm (`publish-sdk-npm`)
- Uses GitHub Actions OpenID Connect (OIDC) token exchange with npm.
- Passes `--provenance` to generate cryptographic build provenance attested by GitHub Actions.
- Authenticates via secret `NPM_TOKEN` with granular publish permissions.

### PyPI (`publish-sdk-pypi`)
- Employs PyPI Trusted Publishing via OIDC (`pypa/gh-action-pypi-publish`).
- Requires `id-token: write` permission.
- Does not require a permanent API token on GitHub.

### crates.io (`publish-sdk-crates`)
- Publishes the Rust client crate `stellar-oracle-sdk` via `cargo publish`.
- Authenticates using the `CRATES_IO_TOKEN` secret.

## Consumer Usage

Consumers can install the packages immediately following release:

```bash
# npm / TypeScript
npm i @stellar-oracle/sdk

# PyPI / Python
pip install stellar-oracle-sdk

# crates.io / Rust
cargo add stellar-oracle-sdk
```
