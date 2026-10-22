# Chainlink-Style External Adapter Interface

This lets operators running existing Chainlink External Adapter (EA)
tooling submit prices to this oracle without changing their job specs,
by fronting the contract with an adapter that speaks the standard
Chainlink EA request/response format.

## Specification

**Endpoint:** `POST /submit`

**Request body** (standard Chainlink EA job-run payload):

```json
{
  "id": "job-run-id",
  "data": {
    "asset": "CA...ASSET_CONTRACT_ID",
    "price": "100000000",
    "timestamp": 1735000000
  }
}
```

* `data.asset` — Stellar contract/account address of the asset, as
  registered via `register_asset`.
* `data.price` — raw price scaled by `10^decimals`, as a decimal string
  (fits Chainlink's `i128`-unsafe JSON number handling).
* `data.timestamp` — Unix timestamp (seconds) of the observation.

**Response body** (standard Chainlink EA success/error envelope):

```json
{
  "jobRunID": "job-run-id",
  "status": "success",
  "data": { "result": "<submitted-price>", "txHash": "<stellar-tx-hash>" }
}
```

On failure, `status` is `"errored"` and `data.error` describes the
failure (invalid signature, contract revert, etc).

## Verification middleware

Requests must carry an `X-EA-Signature` header: `hex(HMAC-SHA256(body,
ADAPTER_SHARED_SECRET))`. The adapter rejects any request whose
signature doesn't match before touching the contract, so an operator's
existing EA node only needs the shared secret configured, not a Stellar
key on the request path itself. The adapter holds the Stellar submission
key server-side (see `--source-identity` below).

## Reference implementation

See [`scripts/chainlink_adapter.py`](../scripts/chainlink_adapter.py) —
a minimal stdlib-only HTTP server implementing the endpoint and
signature middleware, then shelling out to `stellar contract invoke ...
submit_price` for the actual on-chain submission.

```bash
export ADAPTER_SHARED_SECRET="..."
python3 scripts/chainlink_adapter.py \
  --contract CXXXX... \
  --source-identity my-source-key \
  --network testnet \
  --port 8080
```
