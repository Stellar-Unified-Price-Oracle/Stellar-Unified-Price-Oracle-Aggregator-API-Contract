# BLS12-381 aggregate signatures for batched multi-source submission: message format

Design input for #805. This document fixes **what is signed**. Verification code,
key registration endpoints and gas numbers are out of scope and follow once the
format is agreed.

## Why a new format

Today each source's Ed25519 proof signs its own digest
(`price_proof_v2`, see `contracts/price-oracle/src/signed_submission.rs` and
`docs/security/signed-submission-binding.md`) and a relayer submits N proofs as
N `ed25519_verify` host calls. With BLS12-381 (CAP-0059 host functions, exposed
through `env.crypto().bls12_381()`) N signatures collapse to **one** aggregate
signature and one pairing check, so a batch costs one verification regardless of N.

## Signed message

Each source `i` signs its **own** message (the source is bound into the
message, as in `price_proof_v2`), so the batch is a *distinct-message*
aggregate. The signed preimage is the concatenation, in this order:

| # | Field | Encoding | Bytes | Binds against |
|--:|---|---|--:|---|
| 1 | domain tag | ASCII `price_bls_v1` | 12 | cross-protocol and cross-version reuse; `price_proof_v2` Ed25519 signatures can never be valid here |
| 2 | network id | `sha256(network passphrase)` (32 raw bytes, `env.ledger().network_id()`) | 32 | replay across networks (testnet / mainnet / standalone) |
| 3 | contract id | `xdr(contract_address)` (`env.current_contract_address().to_xdr`) | 40 | replay across contract instances |
| 4 | source | `xdr(source_address)` | 40 (`C…`) or 48 (`G…`) | moving a signature to another source |
| 5 | asset | `xdr(asset_address)` | 40 (`C…`) or 48 (`G…`) | moving a signature to another asset |
| 6 | round | `u64` little-endian | 8 | replay into another consensus round (`consensus_rounds`) |
| 7 | price | `u128` little-endian of the non-negative `i128` price | 16 | price tampering |
| 8 | timestamp | `u64` little-endian, Unix seconds of the observation | 8 | stale or back-dated observations |
| 9 | expiration ledger | `u32` little-endian | 4 | late relay of an old signed price |

**Network passphrase.** The passphrase string is never signed directly: it is
variable-length, and the host already exposes its SHA-256 as the network id, which
is exactly what `price_proof_v2` binds. Signing `network_id` is equivalent to
signing the passphrase (SHA-256 is collision-resistant) and keeps the preimage
fixed-width and cheap to build on-chain.

**Unambiguous layout.** Fields 1–2 and 6–9 are fixed-width; fields 3–5 are the
canonical XDR `ScVal` address encoding already used by `price_proof_v2`, which is
type-tagged and fixed-length per address kind (40 bytes for a contract, 48 for an
account). Every field is therefore self-delimiting, so no two distinct field tuples
produce the same bytes.

```text
msg_i = "price_bls_v1" || network_id(32) || xdr(contract) || xdr(source_i) || xdr(asset)
        || round_le(8) || price_le(16) || timestamp_le(8) || expiration_ledger_le(4)
```

`nonce` from `price_proof_v2` is intentionally replaced by `round`: a round is
single-use per source and asset by construction (see replay below), which removes
the per-source nonce counter from the batch hot path.

## Curve, ciphersuite and hashing

| Item | Choice | Reason |
|---|---|---|
| Signature group | **G1** (48-byte compressed signatures) | smaller calldata and cheaper aggregation than G2 for the many-sources case |
| Public-key group | **G2** (96-byte compressed keys) | keys are stored once at registration, signatures are sent every batch ("minimal-signature-size" variant) |
| Hash-to-curve | `hash_to_g1(msg_i, DST)` with `DST = "BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_"` | the `_POP_` ciphersuite from the IETF BLS draft; uses the standard SSWU map, not try-and-increment |
| Signing | `sig_i = sk_i · H(msg_i)` | standard BLS |
| Aggregation | `sig = Σ sig_i` (G1 point addition) | done off-chain by the relayer |

## Verification equation (distinct messages)

For a batch of N `(source_i, asset_i, …)` entries with registered keys `pk_i`:

```text
e(sig, g2) == Π_i e(H(msg_i), pk_i)
```

evaluated as a single `pairing_check` of N+1 pairs
(`[(-sig, g2), (H(msg_1), pk_1), …, (H(msg_N), pk_N)]`). Cost is N hash-to-curve
operations plus one multi-pairing — still one verification call, but not O(1) in
hashing, which is why `round` and `source` must be in every message and why a
shared-message "everyone signs the same price" scheme is **not** used (it would
need all sources to agree on one price first, defeating independent submission).

## Security requirements the format relies on

1. **Rogue-key resistance.** Distinct-message aggregation in the `_POP_` ciphersuite
   is only safe if every public key has been proven to be well-formed. Registration
   of a BLS key must therefore verify a **proof of possession**
   (`pop = sk · H_pop(pk)`, with the separate DST `BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_`)
   before the key can appear in any batch. A key without a verified PoP is rejected.
2. **Subgroup checks.** Every `pk_i` is checked to be in the prime-order G2 subgroup
   at registration, and `sig` is checked to be in the G1 subgroup and not the
   identity at verification time.
3. **Distinct sources per batch.** A batch may contain a given `(source, asset)` at
   most once. Duplicate entries are rejected before pairing so one source's
   signature cannot be counted twice.
4. **Replay.** The contract stores the last accepted `round` per `(source, asset)`;
   an entry is accepted only if its `round` is strictly greater. Together with
   `expiration_ledger` this bounds both replay and late relay, and the domain tag,
   `network_id` and `contract` fields make a signature valid for exactly one
   deployment.
5. **No domain overlap.** `price_bls_v1` is not a prefix of any other signed
   domain in the contract (`price_proof_v2`, state-channel digests), and the BLS
   hash-to-curve DST differs from every SHA-256 digest domain, so an existing
   Ed25519 or SHA-256 signature can never validate as a BLS message.

## Open questions

- **Aggregate over mixed rounds?** The format allows entries with different
  `round` values in one batch; the first implementation should require a single
  `round` per batch to keep storage writes uniform.
- **Key rotation.** Rotating a source's BLS key needs the same PoP check plus a
  delay, mirroring the existing `register_submission_key` flow.
- **Gas.** N hash-to-curve + N+1 pairings must be measured with
  `simulateTransaction` against the ed25519 baseline before choosing a maximum
  batch size.
