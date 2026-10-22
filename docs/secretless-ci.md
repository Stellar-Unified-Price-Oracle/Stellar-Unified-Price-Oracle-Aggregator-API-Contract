# Secretless CI/CD — OIDC Federation

**Issue:** #524 — replace long-lived CI credentials with short-lived,
OIDC-federated tokens scoped to the deployment environment.

Static deploy credentials are the highest-value secret in this repository and
the one most often leaked. This document is the migration record; the parts of
it that matter operationally are **enforced by code**, not by convention:

| Claim | Enforced by |
|---|---|
| No long-lived deploy secret remains in the repo or CI | `services/ci_policy/policy.py::find_long_lived_secrets`, asserted against the real tree in `test_policy.py` |
| Only the intended workflows can deploy | `TrustPolicy` / `evaluate`, asserted for forks, unrelated workflows, PR refs, and wrong environments |
| Every deployment is attributable to a commit and run | `DeploymentAttestation`, written by `services/ci_policy/attest.py` in the deploy job |

```bash
make ci-audit     # or: python -m services.ci_policy.policy
```

---

## 1. Secret inventory and classification

`secret_inventory()` enumerates every `secrets.*` reference in
`.github/workflows/`, `scripts/` and the `Makefile`, and classifies each one.

| Secret | Before | Classification | After |
|---|---|---|---|
| `CANARY_SECRET_KEY` | Long-lived Stellar deploy key, injected as a workflow secret | **Unnecessary** — replaced by federation | Removed. The signer is fetched at deploy time with a short-lived, role-scoped session. |
| `TESTNET_DEPLOY_ROLE_ARN` | — | **Not a secret** | Repository variable (`vars.*`). A role ARN is an identifier; the role and its trust policy live in the cloud account. |
| `TESTNET_DEPLOY_KEY_NAME` | — | **Not a secret** | Repository variable. The name of the secret-store entry, not its value. |
| `SOAK_PROMETHEUS_PUSH_URL` | — | **Not a credential** | Pushgateway URL; managed outside the repository, listed in `ALLOWED_SECRET_NAMES`. |

The pattern list (`RETIRED_SECRET_PATTERNS`) is intentionally broad —
`*_PRIVATE_KEY`, `*_SECRET_KEY`, `*_API_KEY`, `*_ACCESS_KEY*`, `*_TOKEN`,
`DEPLOY_*` — so a newly introduced long-lived credential fails the audit even
if nobody remembers to extend the list.

**Result: zero retired entries.** `test_no_long_lived_secret_remains_in_the_repository`
asserts this against the working tree, so the claim cannot rot.

## 2. Trust policy

`services/ci_policy/policy.py::deploy_policies` encodes the federation trust
relationship as data. Two roles:

**`price-oracle-testnet-deploy`**

| Claim | Required value |
|---|---|
| `repository` | `Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract` |
| `workflow_ref` | `.../testnet-lifecycle.yml@refs/heads/main` or `.../canary.yml@refs/heads/main` |
| `environment` | `testnet` |

**`price-oracle-mainnet-deploy`** — the same `repository` pinning, `workflow_ref`
restricted to `canary.yml@refs/heads/main`, and `environment` = `production`.

### Why these three claims

Each closes a distinct hole, and `REQUIRED_CLAIM_MATCHES` makes omitting any of
them a hard error (`TrustPolicy.validate`):

- **`repository`** — a fork must not be able to mint a token the role accepts.
  This is the most important pin; a wildcard here is catastrophic.
- **`workflow_ref`** — closes the same-repository case: an unrelated workflow
  (or a *modified* one) cannot assume the role. Pinning the file **and the ref**
  also blocks `@refs/pull/N/merge`, so a pull request cannot deploy from its own
  branch.
- **`environment`** — the deploy job must run inside the GitHub Environment.
  Because `testnet` / `production` are configured with required reviewers, this
  pin also carries the human approval gate into the cloud role itself.

`validate()` additionally rejects a bare `*` wildcard matcher, because a
wildcard reads as if it were scoped while pinning nothing.

### What is asserted

`services/ci_policy/test/test_policy.py`:

- the intended workflow **can** assume the role;
- a **fork** cannot;
- an **unrelated workflow in the same repo** cannot;
- a **pull-request ref** cannot;
- a job **outside the environment** (or with the claim absent) cannot;
- the **mainnet role is not reachable from the testnet environment**, and *is*
  reachable from `production` via `canary.yml`;
- an **incomplete policy** is rejected at evaluation time, not merely warned
  about.

## 3. How a deploy obtains credentials

`scripts/federated-deploy-credentials.sh`:

1. Requests a GitHub Actions OIDC token (requires `permissions: id-token: write`)
   for the provider's audience. Single-use, minutes-long, bound to the claims above.
2. Exchanges it (`sts assume-role-with-web-identity`, or GCP workload identity)
   for a session of one hour or less.
3. Fetches the deploy signer with that session and writes it to
   `$FEDERATED_KEY_FILE` with mode `0600`.

There is **no static-credential fallback**, on purpose: a fallback would
reintroduce exactly the secret being removed. The script prints the run's
identity (`run=$GITHUB_RUN_ID commit=$GITHUB_SHA`) to stderr as it goes.

In CI (`.github/workflows/canary.yml`) the role ARN comes from `vars.*`, and
the resulting key file is read by the deploy step — no `secrets.*` reference
remains anywhere in the workflow.

## 4. Attributable deployments

The federation token is ephemeral, so the durable audit record is written by the
job itself. `services/ci_policy/attest.py` emits a `DeploymentAttestation`:

```json
{
  "repository": "…/Stellar-Unified-Price-Oracle-Aggregator-API-Contract",
  "commit_sha": "<40-char sha>",
  "workflow_ref": "…/canary.yml@refs/heads/main",
  "run_id": "…", "run_attempt": "1",
  "environment": "testnet",
  "role": "…",
  "artifact_sha256": "<64-char digest>",
  "actor": "…"
}
```

Validation is strict: a malformed commit SHA, a malformed artifact digest, or a
missing run id **fails the job** rather than emitting a weak record. The
attestation is written to the run summary and uploaded as an artifact, so each
deployment maps to exactly one commit and one workflow run.

## 5. Break-glass

Federation can fail — IdP outage, trust-policy typo, clock skew. The documented
manual path, in order of preference:

1. **Fix the trust policy first.** It is code in this repository; a correct fix
   is a reviewed PR, and re-running the workflow then succeeds. This covers the
   most common cause by far.
2. **Environment-scoped manual deployment.** A maintainer deploys from a local
   checkout using a key held in their own password manager, against the
   `testnet` environment only. Mainnet still requires the `production`
   environment's reviewer approval, so the human gate is preserved. The
   deployment is recorded in `docs/deployment.md` with its commit and actor, as
   the non-federated path always was.
3. **Time-box it.** Break-glass credentials are never added to the repository
   or to workflow secrets. If a *short-lived* federated credential is minted
   manually by an operator with role-assumption rights, it is valid for that
   session only and is not persisted.

`docs/disaster-recovery.md` remains the entry point for contract-level
incidents; this section covers the CI/credential path only.

## 6. Out of scope

Application-level authentication (the issue excludes it explicitly).
