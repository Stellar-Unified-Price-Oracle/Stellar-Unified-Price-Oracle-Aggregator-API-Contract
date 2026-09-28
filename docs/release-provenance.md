# Signed Release Artifacts and SLSA Provenance

**Issue:** #525 — sign every contract build and publish an in-toto/SLSA
provenance attestation tying the WASM artifact to its source commit and build
environment.

Independent verification that the deployed contract matches the audited source
is a core assurance for an oracle. Provenance is how that verification becomes
automatable: instead of trusting a maintainer's word, a consumer can check three
facts about a `price_oracle.wasm` they were handed — *which commit produced it*,
*what built it*, and *whether the bytes still match a fresh build of that
commit*.

Every release ships three files:

| File | What it is |
|---|---|
| `price_oracle.wasm` | The artifact |
| `price_oracle.wasm.sig` | A keyless `cosign` signature over its digest |
| `provenance.json` | An in-toto Statement v1 with a SLSA v1.0 provenance predicate |

```bash
# Generate (release job)
python -m services.provenance.attest generate \
  --artifact target/wasm32v1-none/release/price_oracle.wasm \
  --commit "$(git rev-parse HEAD)" \
  --workflow-ref "$GITHUB_WORKFLOW_REF" \
  --run-id "$GITHUB_RUN_ID" --tag v1.2.3 --out provenance.json

# Verify (anyone, offline, no keys)
python -m services.provenance.attest verify \
  --bundle provenance.json \
  --artifact target/wasm32v1-none/release/price_oracle.wasm \
  --commit "$(git rev-parse HEAD)" --signer "$EXPECTED_SIGNER"
```

---

## 1. What the attestation binds

```json
{
  "_type": "https://in-toto.io/Statement/v1",
  "subject": [{"name": "price_oracle.wasm", "digest": {"sha256": "…"}}],
  "predicateType": "https://slsa.dev/provenance/v1",
  "predicate": {
    "buildDefinition": {
      "buildType": "https://slsa.dev/container-based-build/v1",
      "externalParameters": {
        "repository": "…", "workflowRef": "…", "commit": "<40-char sha>"
      },
      "resolvedDependencies": [
        {"uri": "git+…@<sha>", "digest": {"gitCommit": "<sha>"}}
      ]
    },
    "runDetails": {
      "builder": {"id": "https://github.com/actions/runner/github-hosted"},
      "metadata": {"environment": {
        "runner_image": "ubuntu-latest", "toolchain": "rust 1.91.0",
        "target": "wasm32v1-none", "build_command": "make build"
      }}
    }
  }
}
```

`verify()` checks, offline: the statement and predicate types; that there is
exactly one subject with a 64-char sha256 named `price_oracle.wasm`; that the
build definition pins a 40-char commit; that `resolvedDependencies` names the
**same** commit (a statement that claims one commit but builds another is
rejected); that a builder, repository and full environment are recorded; and —
given an artifact path — that the file's fresh hash equals the attested digest.

Covering the *build steps* and not just the artifact hash is the point: a bare
hash says the bytes are stable, not that they came from the audited source.

## 2. Keyless signing

Signing uses `cosign` with **no key**: the identity is the workflow's OIDC token,
exchanged for a short-lived Fulcio certificate whose Rekor transparency-log entry
is public. The verifiable identity is

```
https://github.com/stellar-unified-price-oracle/stellar-unified-price-oracle-aggregator-api-contract/.github/workflows/release.yml@refs/tags/v1.2.3
```

This satisfies the issue's "no long-lived signing key is stored statically"
requirement without reintroducing a secret: there is no key to store, rotate,
or leak, and the certificate is bound to a specific workflow *and tag*, so a
signature from an untagged branch build will not verify as a release signature.

`services/ci_policy` scans the workflows and would fail the audit if a
`*_PRIVATE_KEY`-style secret were reintroduced for signing.

## 3. CI verifies the published artifact against a fresh build

`.github/workflows/release.yml` runs in three jobs:

1. **`build`** — builds, generates the attestation, signs, and **verifies its own
   signature against the pinned keyless identity before publishing**. A release
   whose signature does not verify is never uploaded.
2. **`publish`** — emits a GitHub build-provenance attestation and attaches the
   WASM, signature and `provenance.json` to the release.
3. **`verify-published`** — checks out the **tagged** source, rebuilds from
   scratch, and runs `provenance.attest verify` against the published
   `provenance.json` with the freshly built WASM. A mismatch fails the run.

Job 3 is what makes "the published artifact matches the build" a checked fact
rather than a claim, and it also catches toolchain drift, since the toolchain and
target are part of the attested environment.

## 4. Third-party verification

No keys, no repository access, and no trust in the maintainers are required.

**With cosign** (verifies the signature and the transparency-log entry):

```bash
# 1. Check the signature and its keyless identity
cosign verify-blob \
  --certificate-identity-regexp \
    '^https://github\.com/stellar-unified-price-oracle/stellar-unified-price-oracle-aggregator-api-contract/\.github/workflows/release\.yml@refs/tags/v[0-9].*$' \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  price_oracle.wasm.sig price_oracle.wasm

# 2. Confirm the bytes you hold are the attested bytes
sha256sum price_oracle.wasm

# 3. Rebuild from the attested commit and compare
git checkout <commit-from-provenance.json>
make build
sha256sum target/wasm32v1-none/release/price_oracle.wasm
```

**With the offline checker** (no network; validates the statement's internal
consistency and the digest match):

```bash
python -m services.provenance.attest verify \
  --bundle provenance.json --artifact price_oracle.wasm \
  --commit <expected-sha> --signer <expected-identity>
```

**With `gh`** (for the GitHub-issued attestation):

```bash
gh attestation verify price_oracle.wasm \
  --repo stellar-unified-price-oracle/stellar-unified-price-oracle-aggregator-api-contract
```

The tooling is exercised by `services/provenance/test/test_attest.py`, including
the failure paths: a tampered artifact, a missing artifact, provenance for a
different commit, a different workflow ref, a different signer, a missing
environment, and a `resolvedDependencies` commit that disagrees with
`externalParameters`.

## 5. Out of scope

Smart-contract wallet key custody — a different key class from build signing,
covered by `docs/secretless-ci.md` and `docs/disaster-recovery.md`.
