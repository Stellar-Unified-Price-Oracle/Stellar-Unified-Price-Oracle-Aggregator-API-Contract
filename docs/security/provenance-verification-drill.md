# Release Artifact Provenance Verification Drill (#682)

## Overview

This drill establishes the operational procedure for independently verifying release artifact signatures, in-toto statements, and SLSA v1.0 build provenance from a clean, untrusted environment using published instructions.

Consumers, downstream oracles, and security auditors must never trust published binary hashes implicitly; they must verify:
1. **Source Authenticity**: The artifact was built from the official upstream repository (`Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract`) and pinned commit hash.
2. **Workflow & Builder Integrity**: The build was produced by the signed release workflow (`release.yml`) under the trusted GitHub Actions builder identity.
3. **Bytecode Integrity**: The artifact digest matches the cryptographic subject digest in the signed provenance statement.
4. **Rejection Guarantees**: Any modified artifact (even a 1-byte patch) or mismatched provenance statement (wrong commit, unreviewed workflow, forged signer) is **unconditionally rejected**.

---

## Clean Environment Verification Instructions

To verify a released contract artifact (`price_oracle.wasm`) from a fresh environment:

### Step 1: Download Release Artifacts
Download the release assets into a clean working directory:
- `price_oracle.wasm`
- `price_oracle.wasm.provenance.json`

### Step 2: Run Verification CLI
Using the repo verifier:

```bash
python -m services.provenance.attest verify \
  --bundle price_oracle.wasm.provenance.json \
  --artifact price_oracle.wasm \
  --commit <RELEASE_COMMIT_SHA> \
  --workflow-ref .github/workflows/release.yml@refs/tags/<RELEASE_TAG> \
  --signer Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/release.yml@refs/tags/<RELEASE_TAG>
```

### Verification Checks Executed

1. `_type` equals in-toto Statement v1 (`https://in-toto.io/Statement/v1`).
2. `predicateType` equals SLSA Provenance v1.0 (`https://slsa.dev/provenance/v1`).
3. Single subject name equals `price_oracle.wasm` with valid 64-char SHA-256 digest.
4. Subject digest strictly matches a fresh `sha256(price_oracle.wasm)` of the binary.
5. External parameters bind the exact 40-char release commit SHA and repository.
6. Resolved dependencies include the pinned commit.
7. Builder ID and runner environment (compiler toolchain `rust 1.91.0`, target `wasm32v1-none`) match published specifications.
8. Signer identity matches the trusted OIDC release workflow.

---

## Automated Verification Drill

The provenance drill exercises both positive verification and adversarial rejection:

- **Scenario 1 (Valid Release)**: Verifies valid release artifact against its genuine provenance bundle. Result: **PASS**.
- **Scenario 2 (Altered Artifact)**: Tampered binary (simulating compromised CDN or malicious proxy). Result: **STRICTLY REJECTED** (`fresh build digest != attested`).
- **Scenario 3 (Mismatched Provenance)**: Valid binary paired with a statement bound to a different commit. Result: **STRICTLY REJECTED** (`attested commit != release commit`).
- **Scenario 4 (Forged Signer)**: Provenance signed by unauthorized repository or workflow. Result: **STRICTLY REJECTED** (`signer identity != expected`).

Run drill locally:

```bash
python -m services.provenance.drill --out provenance-drill-report.json
```

Run test suite:

```bash
python -m pytest services/provenance/test/test_drill.py -q
```
