# Release Artifact Retention and Retrieval Verification (#725)

Every tagged release of `price-oracle` produces three mandatory artifacts (see
[`release-provenance.md`](../release-provenance.md)):

| Artifact | Purpose | Minimum retention window |
|---|---|---|
| `price_oracle.wasm` | Reproducible contract binary deployed or queued as a rollback target | Permanent for all tagged releases (`v*`, `interface-v*`) and at least `N-2..N` active schema versions |
| `price_oracle.wasm.sig` | Keyless Sigstore (`cosign`) signature bound to `.github/workflows/release.yml@refs/tags/<tag>` | Same as `price_oracle.wasm` |
| `provenance.json` | in-toto Statement v1 carrying the SLSA v1.0 build provenance predicate | Same as `price_oracle.wasm` |

Because rollback (`scripts/blue-green-upgrade.sh rollback`) and third-party audit
both depend on retrieving the exact prior release binary and verifying its provenance
offline, retention is verified by an automated retrieval drill in
[`services/provenance/retention.py`](../../services/provenance/retention.py).

## What the retrieval verification checks

For every retained release tag `<tag>` in the store:

1. **Completeness** — `price_oracle.wasm`, `price_oracle.wasm.sig`, and
   `provenance.json` are all present and non-empty.
2. **Digest stability** — fresh SHA-256 digests of the retrieved WASM, signature,
   and provenance bundle match the `retention-manifest.json` recorded when the
   release was archived (detecting bit-rot or silent replacement).
3. **SLSA v1.0 provenance integrity** — `services.provenance.attest.verify` passes
   against the retrieved `price_oracle.wasm`, confirming the 40-char commit SHA,
   the `.github/workflows/release.yml@refs/tags/<tag>` workflow reference, and the
   keyless OIDC signer identity.
4. **Negative controls** — a release directory missing any of the three files, or a
   retained WASM whose bytes were modified after archival, fails retrieval
   verification with a non-zero exit code.

## Running the verification

```bash
python3 -m services.provenance.retention
```
