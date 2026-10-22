# Signed Submission Binding (#468)

## Entry points that accept a signature

| Entry point | Verifies | Payload |
|---|---|---|
| `submit_price_with_proof` | `ed25519_verify` against the source's registered key | `price_proof_v2` |
| `delegate_relayer` | `ed25519_verify` against the source's registered key | `source_relayer_delegation_v2` |
| `submit_price_relayed` | no signature — `relayer.require_auth()` **and** `source.require_auth()` | — |

No batch or bulk endpoint accepts pre-signed prices; every other submission
path authenticates the source with `require_auth()`.

## `price_proof_v2`

```text
sha256("price_proof_v2" || network_id || xdr(contract) || xdr(source)
       || xdr(asset) || nonce_le(8) || price_le(16) || timestamp_le(8)
       || expiration_ledger_le(4))
```

| Field | Prevents |
|---|---|
| domain tag | cross-protocol reuse (e.g. with delegation signatures) |
| `network_id` | replay on another Stellar network |
| contract address | replay on another oracle instance |
| `source` | reuse by another source sharing the key |
| `asset` | moving a price to a different asset |
| `nonce` | replay; must be strictly greater than the last accepted nonce |
| `price`, `timestamp` | tampering with the observation |
| `expiration_ledger` | late relay; accepted up to and including this ledger |

`source_relayer_delegation_v2` binds the domain tag, `network_id`, contract
address, `nonce`, `source`, `relayer` and `expiration_ledger`.

## Fields intentionally *not* signed

| Field | Why it is safe |
|---|---|
| Relayer / transaction submitter | Anyone may relay; the signature fully determines the effect. |
| Aggregation round | Rounds are derived on-chain from ledger state; the per-source nonce orders submissions. |
| Decimals | Taken from contract config at write time, never from the caller. |
| Volume | Not accepted on this path (`None`). |

## Rotation & revocation

- `register_submission_key` overwrites the key: proofs signed by the old key
  fail immediately.
- `remove_source` deletes the key, and `check_source` rejects the source; a
  re-added source must register a key again (the old key is not revived).
- Nonces survive removal, so pre-removal proofs cannot be replayed after re-add.

## Path parity

The signed path applies the same guards as `submit_price`: pause, freeze,
source/asset registration and scoping, suspension, price floor, future-timestamp
bound, out-of-order rejection, circuit breaker.
