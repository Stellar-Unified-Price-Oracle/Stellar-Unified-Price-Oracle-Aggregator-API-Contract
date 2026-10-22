# Wormhole Price Relay

Lets prices observed on other Wormhole-connected chains feed this oracle, by
verifying a Wormhole VAA (Verified Action Approval) on-chain and mapping its
payload into the existing cross-chain price table (`submit_cross_chain_price`,
issue #226) — the same storage the admin-driven cross-reference checks use.

## Setup (admin, once per guardian rotation / new source chain)

```rust
// 1. Register the current Wormhole guardian set and the quorum required to
//    accept a VAA (Wormhole mainnet currently runs 19 guardians, quorum 13).
client.set_wormhole_guardian_set(&guardian_pubkeys, &quorum);

// 2. Map each Wormhole chain id you want to accept prices from to the
//    `Address` key this oracle already uses under CrossChainPrice(asset, oracle_chain).
client.set_wormhole_chain_mapping(&2u32, &ethereum_oracle_chain_marker); // 2 = Ethereum
```

## Relaying a price (permissionless — anyone may call this)

```rust
client.submit_price_via_wormhole(&asset, &vaa);
```

`vaa` (`WormholeVaa`) carries:

- `emitter_chain` / `emitter_address` / `sequence` — standard VAA identity fields
  (`sequence` is used for replay protection, per emitter).
- `payload` — 28 bytes: `price(16 LE) || decimals(4 LE) || timestamp(8 LE)`, built
  with `encode_price_payload`.
- `signatures` / `guardian_indices` — Ed25519 signatures from a quorum of
  registered guardians over the VAA body's digest, and the index of each signer
  within the registered guardian set.

On success the price lands under `DataKey::CrossChainPrice(asset, oracle_chain)`
exactly as if the admin had called `submit_cross_chain_price` directly, and a
`WormholePriceRelayedEvent` is emitted. Read it back with the existing
`get_cross_chain_price(asset, oracle_chain)`.

## Guardian signature scheme

Real-world Wormhole guardians sign a `keccak256(keccak256(body))` digest with
secp256k1 and are identified by 20-byte Ethereum-style addresses. This contract
verifies Ed25519 signatures directly against registered guardian public keys over
a `sha256(sha256(body))` digest instead — the same simplification the existing
Stellar SCP light-client verifier (`cross_chain_relay::verify_validator_set`)
already makes, since Soroban's host crypto surface exposes `sha256` /
`ed25519_verify` directly. The quorum rule — accept once `>= quorum` distinct
guardians have validly signed — mirrors Wormhole's actual 2/3-plus-one guardian
consensus.

A production deployment bridging real Wormhole VAAs needs an off-chain relayer
that: (1) subscribes to Wormhole guardian attestations for the configured
emitter, (2) re-signs (or otherwise re-attests) the price with an Ed25519 key
registered as a "guardian" in this contract's simplified scheme, since this
contract cannot verify the real secp256k1/keccak256 guardian signatures
directly. `contracts/price-oracle/src/wormhole_relay.rs`'s test module builds
such attestations end-to-end with real Ed25519 signatures (via `ed25519-dalek`,
a dev-only dependency) against a mock guardian set, exercising the full
verify → decode → store → replay-protect path without any real testnet
connectivity required.

## Errors

| Error | Cause |
|---|---|
| `GuardianSetNotConfigured` | No guardian set registered yet. |
| `InvalidGuardianSignatureSet` | Empty/mismatched signature arrays, or an out-of-range guardian index. |
| `GuardianQuorumNotMet` | Fewer than the configured quorum of guardians validly signed. |
| `UnmappedWormholeChain` | No oracle-chain mapping registered for `vaa.emitter_chain`. |
| `VaaAlreadyProcessed` | Replay of an already-processed (or zero) sequence number for this emitter. |
| `InvalidVaaPayload` | Payload is not exactly 28 bytes. |
