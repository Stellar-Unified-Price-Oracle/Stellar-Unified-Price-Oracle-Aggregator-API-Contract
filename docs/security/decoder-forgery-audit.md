# Decoder Forgery Resistance (#451)

Tests: `contracts/price-oracle/src/decoder_forgery_tests.rs`.

## Decoder inventory and grammar

| Decoder | Grammar | Length rule | Error on malformed input |
|---|---|---|---|
| `bridge_common::decode_price_payload` (Axelar, LayerZero) | `foreign_asset[32] ‖ price u128 LE[16] ‖ decimals u32 LE[4] ‖ timestamp u64 LE[8] ‖ nonce u64 LE[8]` | exactly 68 bytes | `InvalidProof` (#112) |
| `wormhole_relay::decode_price_payload` | `price i128 LE[16] ‖ decimals u32 LE[4] ‖ timestamp u64 LE[8]` | exactly 28 bytes | `InvalidVaaPayload` (#147) |
| Timelock batch `execute_single_op` | Upgrade: `hash[32]`. SetMinSources / MaxHistory / Resolution / Decimals: `u32 BE[4]`. SetTimestampThreshold: `u64 BE[8]`. SetAdmin / SetDescription: ignored. | exact (**fixed in this change**) | `InvalidConfiguration` (#10) |
| `price_proof::validate_proof` | opaque `payload_hash` / `signature` blobs with a minimum length | minimum only | `InvalidProof` (#112) |
| `signed_submission` digest | `sha256("price_proof_v2" ‖ network_id ‖ contract ‖ source ‖ asset ‖ nonce ‖ price ‖ timestamp ‖ expiration)` | N/A (fields are typed) | `NotAuthorized` |

## Properties

- **Canonical form only.** All wire formats are fixed-width with no optional or
  length-prefixed fields, so every value has exactly one encoding
  (`bridge_payload_round_trip_is_canonical`). There are no duplicate fields, so
  first-wins and last-wins ordering cannot differ.
- **Truncation and trailing bytes fail closed.** Every truncation length and one
  extra byte are rejected (`bridge_payload_every_truncation_rejected`,
  `bridge_payload_trailing_byte_rejected`,
  `wormhole_payload_truncation_and_extension_rejected`).
- **Length-field overflow is unreachable.** No decoder reads a length field; all
  offsets are compile-time constants below 68.
- **Sign confusion.** The bridge decoder casts `u128 → i128`. A top-bit price
  becomes negative and is rejected with `InvalidPrice` on apply
  (`bridge_payload_high_bit_price_rejected_on_apply`).
- **Contextual binding.** Chain: see #450 (the mapping is keyed by chain). Asset:
  the foreign asset id must map on that chain (`ForeignAssetNotMapped`). Round:
  Axelar `command_id` / LayerZero nonce.

## Fixed in this change

- **DEC-1 (Medium, fixed).** Timelock batch payloads silently **skipped** a
  truncated operation (`data.len() < 4` did nothing) and **ignored** trailing
  bytes. A short Upgrade hash panicked in `slice`. All three cases now fail with
  `InvalidConfiguration` (`batch_payload_*`, `batch_upgrade_short_hash_rejected`).

## Filed findings (design change)

| ID | Severity | Finding |
|---|---|---|
| DEC-2 | High | The `signed_submission` digest binds neither `asset` nor the contract address or network. A relayer holding an unsubmitted signed price can credit it to **any asset the source is authorised for**, or replay it on another deployment. Fix: a `price_proof_v2` digest that includes the contract address and asset (breaks `scripts/signed_price_adapter.py`, so it needs a coordinated rollout). |
| DEC-3 | Low | `price_proof` checks only minimum lengths, so trailing data in `payload_hash` / `signature` is accepted. The proofs are opaque, stored for audit and not verified on-chain, so impact is limited. Tighten to exact lengths once the formats are fixed. |
| DEC-4 | Low | Wormhole `price` is decoded as `i128` directly; negative values rely on the downstream `InvalidPrice` check. |
