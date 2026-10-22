# Canary Deployment Pipeline (#414)

The canary pipeline promotes or rolls back a candidate ("green") oracle
contract against the live ("blue") one based on **correctness divergence** —
whether the two versions publish different numbers — not error or latency
rates, which cannot catch an oracle that is wrong the same way on both sides.

| Piece | Location |
|---|---|
| Decision engine (routing, diffing, record) | `services/canary/pipeline.py` |
| Tests, incl. deliberately divergent green | `services/canary/test/test_pipeline.py` |
| Testnet snapshot + decision driver | `scripts/canary.sh` |
| Workflow | `.github/workflows/canary.yml` |
| Assets compared | `config/canary-assets.txt` |

## Routing rule

Consumers resolve which contract answers a query for `asset` with:

```
bucket  = u64_be(sha256(salt ":" asset)[0..8]) mod 10000
version = green if bucket < canary_bps else blue
```

`salt` is fixed for a canary epoch (default: the green contract id). The rule
takes no caller, query or ledger input, so **every asset is served by exactly
one version for the whole epoch, and therefore in every ledger**. Split-brain
(two aggregates for one asset at the same time) is impossible by construction,
not merely monitored for. `test_each_asset_served_by_exactly_one_version_every_ledger`
asserts it.

## Divergence tolerance

Default tolerance is **0 bps**. Both versions read the same on-chain source
submissions and aggregate with deterministic integer arithmetic, so there is
no legitimate source of numeric drift; any difference in `price`, `decimals`,
`num_sources`, or presence of an aggregate is a behaviour change. Every asset
is compared — not just the canary slice — so a regression that only affects a
minority of assets or a rare input shape is still caught.

A non-zero `--tolerance-bps` exists only for an intentional, documented
aggregation change, and must be justified in the PR that raises it.

## Decisions

* Any divergence → `rollback`: `scripts/canary.sh` exits non-zero, the job
  fails, and the green contract is never routed to.
* No divergence → `await_approval`: the `promote` job waits on the
  `production` environment, which must have required reviewers. The approving
  reviewer's login is written into `promotion-record.json` as `approved_by`.

## Evidence

`deployment-record.json` contains both contract ids, the routing table, the
tolerance, every divergence, the decision, and `evidence_digest`
(sha256 over the canonical JSON of the rest of the record). The raw
`blue.json` / `green.json` snapshots are uploaded alongside it as a workflow
artifact retained for 400 days; the deployment record is linked from the run.

## Running

```bash
# CI self-test (divergent green caught and rolled back)
python -m pytest -q services/canary

# End-to-end on testnet (requires CANARY_SECRET_KEY + environments `testnet`
# and `production` configured in repo settings):
gh workflow run canary.yml -f blue_contract=<C...> -f canary_bps=1000
```
