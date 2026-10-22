# RPC response trust boundary (#684)

Soroban RPC responses are untrusted input. The contract validates its own state,
but the operator tooling around it makes decisions from RPC replies: which
network answered, which ledger head was observed, whether the transaction that
was just signed is actually in a block, whether an operation should be retried.
A compromised, misconfigured or merely lagging endpoint can push that tooling
into a wrong operational decision long before the contract sees anything.

`services/rpc_guard/` is the boundary between "bytes a node sent" and "facts an
operator acts on". It is pure: no network I/O, no clock, no third-party
dependency, so every rule is exercised by a committed fixture corpus.

```bash
python3 -m services.rpc_guard.gate
python3 -m services.rpc_guard.gate --report security-artifacts/rpc-boundary.md
```

## The three checks, in order

A guard is sequential on purpose. `accept_network` must succeed before
`accept_ledger`, and `accept_ledger` before `accept_transaction`, so a caller
cannot hold a transaction from a context that was never validated.

| Step | Validated | Rejected when |
|---|---|---|
| `accept_network` | `passphrase`, `protocolVersion` | another network's passphrase; protocol outside the supported window; JSON-RPC error object; missing fields |
| `accept_ledger` | `sequence`, `hash`, `protocolVersion`, `closedAt` | non-positive sequence; hash that is not 64 hex characters; protocol disagreeing with the negotiated network; close time going backwards; sequence behind the validated head; same height with a different hash (reorg) |
| `accept_transaction` | `hash`, `status`, `ledger`, `successful`, `envelopeXdr`/`resultXdr`/`resultMetaXdr`, `createdAt`, `feeCharged` | `NOT_FOUND` (a distinct outcome, not a result); hash of another submission; ledger beyond the validated head or older than the age window; result flag disagreeing with status; successful transaction without envelope or meta; creation time far from the ledger close; negative fee |

`NOT_FOUND` deserves its own line: a caller that treats it as "done" aborts a
retry loop and reports success for a transaction that may still land. The guard
refuses it, so the retry path stays the only way forward.

## Bounded, observable errors

Every rejection is a `RpcValidationError` with:

* a `code` from a **closed enum** — safe as a metric label, with cardinality
  bounded by construction (`network_mismatch`, `ledger_reorg`, `tx_ledger`, …);
* the offending `field`;
* a **bounded, sanitised** `detail` (control characters stripped, 160 chars
  max) that describes the failure and **never echoes the received value**.

That last point is not cosmetic. A hostile endpoint can put a newline and a fake
log line into a `passphrase`. Echoing it would turn a validation failure into
log injection — the same trust boundary as
[evidence redaction](evidence-redaction.md). The `passphrase` is compared and
reported only as "not testnet"; the expected value is a trust anchor, the
received one is untrusted input.

`RpcGuard.rejections` and `RpcGuard.summary()` give an operator a bounded
snapshot for a log line or a metric:

```python
from services.rpc_guard import NetworkPolicy, RpcGuard, RpcValidationError

guard = RpcGuard(NetworkPolicy.testnet())
try:
    guard.accept_network(response)
    ledger = guard.accept_ledger(header)
    tx = guard.accept_transaction(result, expected_hash=submitted_hash)
except RpcValidationError as error:
    print(error.code.value, error.field, error.detail)
```

`summary()` → `{"network": "testnet", "protocol_version": 23, "ledger": 1000, "rejections": {...}}`.

## Policy, not assumptions

`NetworkPolicy` carries what the operator *believes*:

| Field | Default | Meaning |
|---|---|---|
| `passphrase` | required | the exact network passphrase this process belongs to |
| `network_id` | `testnet` | operator-facing name used in messages |
| `min_protocol_version` / `max_protocol_version` | 22 / 27 | the range this build understands |
| `max_tx_age_ledgers` | 128 | how far behind the head a result is still attributable |
| `max_clock_skew_secs` | 300 | tolerance between `createdAt` and the ledger close |

Use `NetworkPolicy.mainnet()` to get the public-network passphrase. A policy is
data, so a new protocol upgrade is a reviewed constant change rather than a
loosened check in the call site.

## The fixture corpus

`services/rpc_guard/fixtures/cases.json` lists nineteen cases: the accepted
shapes and one malformed example per rejection class, each with the code it must
produce. The gate replays them and fails if any case stops being accepted or
stops being rejected with its documented code, which is what makes "we validate
RPC responses" a check rather than a claim.

Adding a case is the way to add a rule: add the fixture, add the expectation,
and the gate tells you the day the rule stops firing.

## How this is verified

* `python3 -m pytest services/rpc_guard -q` — unit tests for every field,
  ordering rule, sanitisation and the gate itself.
* `python3 -m services.rpc_guard.gate` — the fixture corpus, as one command.
* `make rpc-gate` — the same, with a report for the run artifacts.
* `make trust-boundary` — this gate plus the three sibling boundaries.

Related: [indexer authenticity](indexer-authenticity.md) applies the same
"validate before you act" rule to the event stream.
