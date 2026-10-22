# Privileged transaction review guard (#686)

A production privileged transaction — `set_admin`, `upgrade`, `pause`,
`override_price`, the timelock operations — is the highest-consequence thing an
operator does. The failure mode this guard closes is not "the reviewers were
wrong"; it is **"the reviewers approved one thing and the workflow signed
another"**: an argument edited after review, a second operation appended, a
simulation of a cheaper call swapped in, one person approving their own work.

Code: [`services/tx_review/`](../../services/tx_review/). Enforcement point:
[`scripts/blue-green-upgrade.sh`](../../scripts/blue-green-upgrade.sh) before
`invoke upgrade`.

## The three artifacts

1. **Payload** — the canonical description of the unsigned transaction:
   network passphrase, source account, fee, sequence, time bounds, and ordered
   operations with **typed** arguments.
2. **Simulation** — the result of simulating that payload. It carries the
   payload's SHA-256, the network, `success`, and the fee the simulation
   charged.
3. **Approval** — the record a reviewer signs: payload digest, the exact
   human-readable summary they read, requester, reviewers, review window, and
   the simulation.

```json
{
  "schema": "oracle.tx_review.v1",
  "network": "Public Global Stellar Network ; September 2015",
  "source_account": "G...",
  "fee": 1000,
  "sequence": 123456789,
  "max_time": 0,
  "operations": [
    {
      "contract_id": "C...",
      "function": "set_admin",
      "args": {"new_admin": {"type": "address", "value": "G..."}}
    }
  ]
}
```

## Why canonicalisation is the load-bearing part

JSON is not canonical. Key order, `"1"` versus `1`, and an extra field the
guard ignores are all ways for two payloads to look identical to a human and
differ to a machine. The normaliser therefore:

* rejects unknown top-level fields, operation fields and approval fields rather
  than ignoring them — an ignored field is an unreviewed field;
* requires every argument to be typed (`{"type": ..., "value": ...}`), because
  an untyped argument is exactly how `"1000000"` and `1000000` get confused;
* rejects functions it does not model, so the workflow can never approve a call
  whose effect it cannot describe;
* sorts argument names and keeps operation order, so the digest covers the
  exact call sequence.

The digest is SHA-256 over the canonical JSON of the normalised payload.
Changing one address, one argument, the fee, the sequence, the source account or
the operation order changes it.

## What the verifier refuses

| Condition | Code |
|---|---|
| payload digest ≠ reviewed digest | `payload_mismatch` |
| reviewed summary ≠ regenerated summary | `summary_mismatch` |
| simulation's digest or network ≠ payload | `simulation_mismatch` |
| simulation did not succeed | `simulation_failed` |
| no simulation attached | `simulation_missing` |
| simulation fee or payload fee above the cap | `fee_above_cap` |
| fewer distinct reviewers than the policy floor | `insufficient_reviewers` |
| requester listed as a reviewer | `self_review` |
| now > `expires_at` | `approval_expired` |
| now < `reviewed_at` | `approval_not_yet_valid` |
| review window longer than the policy | `review_window_too_long` |
| untyped argument, unknown function, unknown field | `untyped_argument`, `unknown_function`, `unknown_field` |

`summary_mismatch` is separate from `payload_mismatch` on purpose: copying a
valid digest while pasting a summary of a *different* call is a realistic
mistake, and the digest alone would not catch it.

`ReviewPolicy` defaults to `production` with **two** distinct reviewers and a
24-hour maximum review window. A policy that lowers the production floor is
invalid — `ReviewPolicy(environment="production", required_reviewers=1)` raises.
Non-production environments still need one independent reviewer.

## Operator workflow

```bash
# 1. What exactly will be signed?
python3 -m services.tx_review.gate render --payload payload.json

# 2. Simulate it (stellar-cli), then record the approval after two humans
#    have read the rendering.
python3 -m services.tx_review.gate approve \
  --payload payload.json --simulation simulation.json \
  --requested-by ops-bot --reviewer alice --reviewer bob \
  --reviewed-at 2026-10-20T09:00:00Z --expires-at 2026-10-20T10:00:00Z \
  --out approval.json

# 3. The release step refuses anything that is not that payload.
python3 -m services.tx_review.gate verify \
  --payload payload.json --approval approval.json --environment production
```

`approve` verifies the record it is about to write, so an approval that would be
refused at execution time is never produced.

`verify` prints `APPROVED` and exits `0`, or prints `REFUSED <code>` and exits
`1`. The upgrade script gates the admin call on it:

```bash
ORACLE_REQUIRE_TX_REVIEW=1 ./scripts/blue-green-upgrade.sh upgrade \
  --contract CDEMO... --admin oracle-admin --wasm green.wasm \
  --payload payload.json --approval approval.json
```

The review is enforced whenever `--network mainnet` is used, when
`ORACLE_REQUIRE_TX_REVIEW=1`, or when either artifact is supplied — so a
production upgrade without a reviewed simulation fails instead of quietly
signing. Testnet drills are unaffected. Set `ORACLE_REQUIRE_TX_REVIEW=1` for any
other production network (futurenet, a dedicated network).
`oracle-cli verify-review` exposes the same check for ad-hoc use.

## The tamper corpus

`services/tx_review/fixtures/cases.json` holds a valid (payload, approval) pair
and twenty-seven cases derived from it by **mutation** — edited argument,
edited sequence, appended operation, swapped payload, untyped argument, copied
digest, copied summary, wrong simulation, failed simulation, fee above cap,
single reviewer, duplicated reviewer, self-review, expired, future-dated,
over-long window, missing simulation, unknown field, bad schema. Deriving the
cases by mutation means the digests cannot go stale and each case is provably
about the thing it names.

```bash
python3 -m services.tx_review.gate replay
python3 -m services.tx_review.gate replay --report security-artifacts/tx-review.md
```

Both valid payloads must still be approved; every mutation must be refused with
the documented code. A relaxation of any check fails the gate.

## Tests

* `python3 -m pytest services/tx_review -q` — canonicalisation, digest
  coverage, rendering, every rejection class, the CLI, and the corpus.
* `make tx-review-gate` — the corpus with a report.

Related: [RPC trust boundary](rpc-trust-boundary.md) validates what the node
reports; this guard validates what the operator is about to sign.
