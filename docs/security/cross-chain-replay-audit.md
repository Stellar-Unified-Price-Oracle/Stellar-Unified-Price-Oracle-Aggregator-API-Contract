# Cross-Chain Replay & Signature Malleability Audit (#450)

Tests: `contracts/price-oracle/src/cross_chain_replay_audit_tests.rs`.

## Threat matrix

| Adapter | Identity (emitter / guardian) | Replay protection | Chain binding | Signature handling |
|---|---|---|---|---|
| **Axelar GMP** (`axelar_gmp.rs`) | Configured gateway must match and `require_auth` (`axelar_forged_gateway_fails`); `(source_chain, source_address)` must be trusted (`axelar_forged_emitter_fails`) | Global `AxelarExecutedCommand(command_id)` set (`axelar_same_command_replay_fails`) | Trusted-source key and foreign-asset mapping are both keyed by chain (`axelar_replay_on_other_chain_fails`, `axelar_chain_identity_bound_to_asset_mapping`) | Delegated to the Axelar gateway's verifier quorum; no signature bytes parsed here |
| **LayerZero** (`layerzero.rs`) | Configured endpoint must match and `require_auth` (`lz_forged_endpoint_fails`); `(src_eid, sender)` must be trusted (`lz_forged_sender_fails`) | Strictly sequential nonce per `(src_eid, sender)` (`lz_same_nonce_replay_fails`, `lz_nonce_skip_fails`) | `src_eid → chain name` must be configured, and the asset mapping is keyed by that chain (`lz_replay_on_other_eid_fails`, `lz_eid_without_chain_name_fails`) | Delegated to the LayerZero endpoint/DVNs |
| **Wormhole** (`wormhole_relay.rs`, not exposed as an endpoint) | Ed25519 signatures from the registered guardian set; forged signer, out-of-range index and duplicate index are rejected or not counted (`wormhole_forged_guardian_signature_fails`, `wormhole_out_of_range_guardian_index_fails`, `wormhole_duplicate_guardian_does_not_reach_quorum`) | Monotonic sequence per `(emitter_chain, emitter_address)` | `emitter_chain` is in the signed body, so retargeting fails (`wormhole_resigned_body_for_other_chain_fails`) | Host `ed25519_verify` rejects non-canonical `S` (`wormhole_malleated_signature_rejected`) |
| **Eth bridge / IBC** (`eth_bridge.rs`, `ibc_oracle.rs`) | — | — | — | Not compiled into the crate (`lib.rs` declares no `mod`), so they are unreachable. Out of the attack surface until they are wired in, and must be audited when they are. |
| **Cross-chain verify** (`cross_chain_verify.rs`) | Admin / bridge-only writes into the reference store | N/A (reference observations, not credited to the median) | Stored per chain id | None |

## Malleability

Ed25519 has one malleable form: `(R, S + L)`. The Soroban host verifier requires
`S < L`, so the second encoding is **rejected, not normalised**. The test builds
`S + L` byte-wise and asserts that verification fails. Axelar and LayerZero
parse no signatures on-chain.

## Cross-adapter double credit

Each adapter writes to `Submission(asset, bridge_source)`. Delivering the same
payload again through one adapter overwrites that slot (`same_adapter_redelivery_occupies_one_slot`).
If the same upstream feed is bridged over two adapters, **both must be
attributed to the same `bridge_source`**. The payload is then credited once
(`cross_adapter_same_feed_is_not_double_counted`).

## Findings requiring a design change

| ID | Severity | Finding |
|---|---|---|
| CC-1 | Medium | The `nonce` field of `CrossChainPricePayload` is decoded but never enforced. Replay protection relies only on the transport (`command_id` / LZ nonce). If the same feed is mapped to two different bridge sources, one observation is counted twice in the median. Mitigation today: the config rule above. Fix: enforce a per-`(chain, foreign_asset)` payload nonce in `apply_bridged_price`. |
| CC-2 | Medium (latent) | The Wormhole relay takes `asset` as a caller argument that is **not in the signed VAA body**, so a valid VAA can be credited to any asset. It is latent because `submit_price_via_wormhole` is not exposed. Must be fixed (bind the asset id in the payload) before exposing it. |
| CC-3 | Low (latent) | The Wormhole VAA does not carry a guardian-set index, so a VAA signed by a retired set cannot be told apart after rotation. Latent for the same reason. |
| CC-4 | Low | `apply_bridged_price` only rejects **future** timestamps. A stale-but-unreplayed payload is accepted. Consider a max-age check. |
