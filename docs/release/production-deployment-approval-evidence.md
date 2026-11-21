# Production Deployment Approval Evidence Capture (#729)

Every privileged production deployment or upgrade (`scripts/blue-green-upgrade.sh`)
is gated by the privileged transaction review guard
([`docs/security/privileged-transaction-review.md`](../security/privileged-transaction-review.md)).
To preserve an auditable record of *what was approved*, *who approved it*, and
*which exact WASM hash was authorized*, [`services/tx_review/evidence.py`](../../services/tx_review/evidence.py)
captures and verifies a self-contained deployment approval evidence bundle
(`oracle.deployment_approval_evidence.v1`).

## What the evidence bundle binds

| Field | Bound value |
|---|---|
| `schema` | `"oracle.deployment_approval_evidence.v1"` |
| `release_tag` | Git release tag being deployed (e.g. `v0.2.0`) |
| `environment` | Guarded environment (`production` or `mainnet`) |
| `contract_id` | Target Soroban contract address (`C...`) |
| `green_wasm_sha256` | 64-character SHA-256 of the green contract WASM being installed |
| `blue_wasm_sha256` | Optional 64-character SHA-256 of the active blue contract WASM recorded for rollback |
| `payload_sha256` | Canonical SHA-256 digest of the normalised `oracle.tx_review.v1` payload |
| `requested_by` / `reviewers` | Requester and at least two distinct, non-self reviewers |
| `reviewed_at` / `expires_at` | Bounded review window within which the deployment was authorized |
| `rendered_summary` | Exact human-readable operation summary reviewed by the approvers |
| `simulation` | Simulation result bound to `payload_sha256`, network passphrase, and fee cap |
| `evidence_sha256` | Canonical SHA-256 over the entire evidence object |

## Fail-closed capture invariants

`capture_deployment_evidence` refuses to emit an artifact if:

1. `verify_approval` rejects the payload or approval (expired window, self-review,
   fewer than two reviewers in `production`/`mainnet`, digest mismatch, summary
   mismatch, or failed simulation).
2. Any operation in `payload.operations` targets a contract other than
   `contract_id`.
3. Any `upgrade` operation's `new_wasm_hash` argument differs from
   `green_wasm_sha256`.
4. Any Stellar secret seed (`S[A-Z2-7]{55}`) or PEM private key appears anywhere
   in the bundle.

## CLI usage

```bash
# Capture evidence during a production deployment
python3 -m services.tx_review.evidence capture \
  --payload payload.json \
  --approval approval.json \
  --release-tag v0.2.0 \
  --contract-id CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM \
  --green-wasm-sha256 <64-char-sha256> \
  --blue-wasm-sha256 <64-char-sha256> \
  --environment production \
  --out security-artifacts/deployment-approval-evidence.json

# Verify a captured evidence bundle offline
python3 -m services.tx_review.evidence verify \
  --evidence security-artifacts/deployment-approval-evidence.json
```
