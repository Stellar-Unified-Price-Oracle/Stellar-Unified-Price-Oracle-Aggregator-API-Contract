//! # Source Submission Validation by External Proof (#296)
//!
//! Allows oracle sources to attach an external proof when submitting a price.
//! The contract verifies the proof format before accepting the submission.
//!
//! ## Supported Proof Types
//!
//! | Type | Description |
//! |------|-------------|
//! | `CexSignedResponse` | A CEX API signed response (signature + payload hash). |
//! | `DexTrade` | On-chain DEX trade evidence (pool address + trade hash). |
//! | `MultiSigAttestation` | Multi-signature attestation from M-of-N signers. |
//!
//! ## Proof Requirements
//!
//! Per-asset proof requirements can be configured by the admin:
//! - `AnyOrNone` — any proof type (or no proof) is accepted.
//! - `RequireCex` — only CEX signed response proofs pass.
//! - `RequireDex` — only DEX trade proofs pass.
//! - `RequireMultiSig` — only multi-sig attestation proofs pass.
//!
//! Proof **format** validation is always performed (byte-length checks).
//! Cryptographic verification of the proof contents is intentionally deferred
//! to off-chain consumers, as the Soroban VM cannot reach external services.

use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env};

use crate::events::emit_admin_action;
use crate::prices::submit_price;
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AssetProofRequirement, DataKey, ErrorCode, PriceProof, ProofType};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum byte length of a CEX signed response payload hash (SHA-256 = 32 bytes).
const CEX_MIN_PAYLOAD_LEN: u32 = 32;
/// Minimum byte length of a CEX signature field.
const CEX_MIN_SIG_LEN: u32 = 16;
/// Minimum byte length of a DEX trade hash (SHA-256 = 32 bytes).
const DEX_MIN_TRADE_HASH_LEN: u32 = 32;
/// Minimum number of signers in a multi-sig attestation.
const MULTISIG_MIN_SIGNERS: u32 = 2;
/// Maximum number of signers to avoid DoS on attestation verification.
const MULTISIG_MAX_SIGNERS: u32 = 20;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Submit a price with an attached external proof.
///
/// The source must be registered and authorized.  The proof format is validated
/// according to the type; if the asset has a proof requirement configured, the
/// proof type must match that requirement.
///
/// On success, the price is forwarded to the standard submission pipeline
/// (validation, aggregation, history, events).
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not a registered source.
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
/// * [`ErrorCode::InvalidProof`] — proof format validation failed.
/// * [`ErrorCode::ProofTypeMismatch`] — proof type does not satisfy asset requirement.
/// * [`ErrorCode::InvalidPrice`] — price is zero or negative.
/// * [`ErrorCode::InvalidTimestamp`] — timestamp too far in the future.
pub fn submit_price_with_external_proof(
    env: &Env,
    source: Address,
    asset: Address,
    price: i128,
    timestamp: u64,
    proof: PriceProof,
) {
    // Validate proof format before any auth or storage checks so we fail fast.
    validate_proof(env, &proof);

    // Check per-asset proof requirement.
    check_asset_proof_requirement(env, &asset, &proof.proof_type);

    // Store the proof so it can be audited later.
    let proof_key = DataKey::SubmissionProof(asset.clone(), source.clone());
    env.storage().persistent().set(&proof_key, &proof);
    env.storage()
        .persistent()
        .extend_ttl(&proof_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Emit proof-submitted event.
    env.events().publish(
        (symbol_short!("proof_sub"), asset.clone(), source.clone()),
        (price, proof.proof_type.clone()),
    );

    // Derive an auto-incrementing nonce for the proof submission path using the
    // current ledger sequence.  We use a separate nonce key for proof submissions
    // to avoid conflicts with regular submit_price nonces.
    let proof_nonce_key = DataKey::SubmissionProofNonce(source.clone(), asset.clone());
    let last_proof_nonce: u64 = env
        .storage()
        .persistent()
        .get::<DataKey, u64>(&proof_nonce_key)
        .unwrap_or(0);
    let new_proof_nonce = last_proof_nonce
        .saturating_add(1)
        .max(env.ledger().sequence() as u64);
    env.storage()
        .persistent()
        .set(&proof_nonce_key, &new_proof_nonce);
    env.storage()
        .persistent()
        .extend_ttl(&proof_nonce_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Delegate to the standard submission pipeline.
    // `submit_price` re-checks auth, source registration, asset registration,
    // price validity, timestamp, circuit breaker, and triggers aggregation.
    submit_price(env, source, asset, price, timestamp, new_proof_nonce);
}

/// Configure a per-asset proof requirement.
///
/// Admin only.  Set `requirement` to [`AssetProofRequirement::AnyOrNone`] to
/// remove any existing requirement.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
pub fn set_asset_proof_requirement(env: &Env, asset: Address, requirement: AssetProofRequirement) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let key = DataKey::AssetProofRequirement(asset.clone());
    env.storage().persistent().set(&key, &requirement);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    emit_admin_action(env, symbol_short!("set_prf"), admin, Bytes::new(env));
}

/// Return the proof requirement configured for an asset.
///
/// Returns [`AssetProofRequirement::AnyOrNone`] when no requirement is set.
pub fn get_asset_proof_requirement(env: &Env, asset: Address) -> AssetProofRequirement {
    let key = DataKey::AssetProofRequirement(asset);
    env.storage()
        .persistent()
        .get::<DataKey, AssetProofRequirement>(&key)
        .unwrap_or(AssetProofRequirement::AnyOrNone)
}

/// Retrieve the most recently stored proof for a (asset, source) pair.
///
/// Returns `None` if no proof has been submitted yet.
pub fn get_submission_proof(env: &Env, asset: Address, source: Address) -> Option<PriceProof> {
    let key = DataKey::SubmissionProof(asset, source);
    env.storage().persistent().get::<DataKey, PriceProof>(&key)
}

// ---------------------------------------------------------------------------
// Proof validation
// ---------------------------------------------------------------------------

/// Validates the format of a [`PriceProof`] without doing cryptographic verification.
///
/// Panics with [`ErrorCode::InvalidProof`] if format requirements are not met.
fn validate_proof(env: &Env, proof: &PriceProof) {
    match &proof.proof_type {
        ProofType::CexSignedResponse => validate_cex_proof(env, proof),
        ProofType::DexTrade => validate_dex_proof(env, proof),
        ProofType::MultiSigAttestation => validate_multisig_proof(env, proof),
    }
}

/// CEX proof requires: `payload_hash` ≥ 32 bytes AND `signature` ≥ 16 bytes.
fn validate_cex_proof(env: &Env, proof: &PriceProof) {
    if proof.payload_hash.len() < CEX_MIN_PAYLOAD_LEN {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }
    if proof.signature.len() < CEX_MIN_SIG_LEN {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }
}

/// DEX proof requires: `payload_hash` ≥ 32 bytes (trade hash) AND
/// `signature` non-empty (used as pool-address identification bytes).
fn validate_dex_proof(env: &Env, proof: &PriceProof) {
    if proof.payload_hash.len() < DEX_MIN_TRADE_HASH_LEN {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }
    if proof.signature.is_empty() {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }
}

/// Multi-sig attestation requires: `signer_count` in `[2, 20]` AND
/// `payload_hash` ≥ 32 bytes.
fn validate_multisig_proof(env: &Env, proof: &PriceProof) {
    if proof.signer_count < MULTISIG_MIN_SIGNERS || proof.signer_count > MULTISIG_MAX_SIGNERS {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }
    if proof.payload_hash.len() < CEX_MIN_PAYLOAD_LEN {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }
}

/// Checks that the proof type satisfies the configured asset requirement.
fn check_asset_proof_requirement(env: &Env, asset: &Address, proof_type: &ProofType) {
    let key = DataKey::AssetProofRequirement(asset.clone());
    let requirement: AssetProofRequirement = env
        .storage()
        .persistent()
        .get::<DataKey, AssetProofRequirement>(&key)
        .unwrap_or(AssetProofRequirement::AnyOrNone);

    match requirement {
        AssetProofRequirement::AnyOrNone => {}
        AssetProofRequirement::RequireCex => {
            if *proof_type != ProofType::CexSignedResponse {
                panic_with_error!(env, ErrorCode::ProofTypeMismatch);
            }
        }
        AssetProofRequirement::RequireDex => {
            if *proof_type != ProofType::DexTrade {
                panic_with_error!(env, ErrorCode::ProofTypeMismatch);
            }
        }
        AssetProofRequirement::RequireMultiSig => {
            if *proof_type != ProofType::MultiSigAttestation {
                panic_with_error!(env, ErrorCode::ProofTypeMismatch);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, LedgerInfo},
        Bytes, Env,
    };

    use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};
    use crate::types::{AssetProofRequirement, PriceProof, ProofType};

    fn ledger_at(e: &Env, seq: u32, ts: u64) {
        e.ledger().set(LedgerInfo {
            timestamp: ts,
            protocol_version: 26,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 4096,
        });
    }

    fn make_cex_proof(e: &Env) -> PriceProof {
        PriceProof {
            proof_type: ProofType::CexSignedResponse,
            payload_hash: Bytes::from_array(e, &[0u8; 32]),
            signature: Bytes::from_array(e, &[1u8; 64]),
            signer_count: 0,
        }
    }

    fn make_dex_proof(e: &Env) -> PriceProof {
        PriceProof {
            proof_type: ProofType::DexTrade,
            payload_hash: Bytes::from_array(e, &[2u8; 32]),
            signature: Bytes::from_array(e, &[3u8; 20]),
            signer_count: 0,
        }
    }

    fn make_multisig_proof(e: &Env) -> PriceProof {
        PriceProof {
            proof_type: ProofType::MultiSigAttestation,
            payload_hash: Bytes::from_array(e, &[4u8; 32]),
            signature: Bytes::from_array(e, &[5u8; 64]),
            signer_count: 3,
        }
    }

    // ── #296 Test 1: valid CEX proof accepted ─────────────────────────────────
    #[test]
    fn test_cex_proof_accepted() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "CEX-Source");
        let asset = register_test_asset(&e, &client);

        let proof = make_cex_proof(&e);
        client.submit_price_with_external_proof(&source, &asset, &1_000i128, &1_000_000u64, &proof);

        let price = client.get_price(&asset, &0u64).unwrap();
        assert_eq!(price.price, 1_000i128);
    }

    // ── #296 Test 2: valid DEX proof accepted ─────────────────────────────────
    #[test]
    fn test_dex_proof_accepted() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "DEX-Source");
        let asset = register_test_asset(&e, &client);

        let proof = make_dex_proof(&e);
        client.submit_price_with_external_proof(&source, &asset, &2_000i128, &1_000_000u64, &proof);

        let price = client.get_price(&asset, &0u64).unwrap();
        assert_eq!(price.price, 2_000i128);
    }

    // ── #296 Test 3: valid multi-sig proof accepted ────────────────────────────
    #[test]
    fn test_multisig_proof_accepted() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "MS-Source");
        let asset = register_test_asset(&e, &client);

        let proof = make_multisig_proof(&e);
        client.submit_price_with_external_proof(&source, &asset, &3_000i128, &1_000_000u64, &proof);

        let price = client.get_price(&asset, &0u64).unwrap();
        assert_eq!(price.price, 3_000i128);
    }

    // ── #296 Test 4: CEX proof with short payload_hash rejected ───────────────
    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_cex_proof_short_payload_rejected() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);

        let bad_proof = PriceProof {
            proof_type: ProofType::CexSignedResponse,
            payload_hash: Bytes::from_array(&e, &[0u8; 10]), // too short
            signature: Bytes::from_array(&e, &[1u8; 64]),
            signer_count: 0,
        };
        client.submit_price_with_external_proof(
            &source,
            &asset,
            &1_000i128,
            &1_000_000u64,
            &bad_proof,
        );
    }

    // ── #296 Test 5: DEX proof with empty pool address rejected ───────────────
    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_dex_proof_empty_pool_rejected() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);

        let bad_proof = PriceProof {
            proof_type: ProofType::DexTrade,
            payload_hash: Bytes::from_array(&e, &[2u8; 32]),
            signature: Bytes::new(&e), // empty — no pool address
            signer_count: 0,
        };
        client.submit_price_with_external_proof(
            &source,
            &asset,
            &1_000i128,
            &1_000_000u64,
            &bad_proof,
        );
    }

    // ── #296 Test 6: multi-sig with signer_count < 2 rejected ────────────────
    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_multisig_too_few_signers_rejected() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);

        let bad_proof = PriceProof {
            proof_type: ProofType::MultiSigAttestation,
            payload_hash: Bytes::from_array(&e, &[4u8; 32]),
            signature: Bytes::from_array(&e, &[5u8; 64]),
            signer_count: 1, // too few
        };
        client.submit_price_with_external_proof(
            &source,
            &asset,
            &1_000i128,
            &1_000_000u64,
            &bad_proof,
        );
    }

    // ── #296 Test 7: per-asset CEX requirement blocks DEX proof ───────────────
    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_asset_requires_cex_rejects_dex() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);

        client.set_asset_proof_requirement(&asset, &AssetProofRequirement::RequireCex);

        let dex_proof = make_dex_proof(&e);
        client.submit_price_with_external_proof(
            &source,
            &asset,
            &1_000i128,
            &1_000_000u64,
            &dex_proof,
        );
    }

    // ── #296 Test 8: correct proof type satisfies asset requirement ───────────
    #[test]
    fn test_correct_proof_type_satisfies_requirement() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);

        client.set_asset_proof_requirement(&asset, &AssetProofRequirement::RequireDex);

        let dex_proof = make_dex_proof(&e);
        client.submit_price_with_external_proof(
            &source,
            &asset,
            &5_000i128,
            &1_000_000u64,
            &dex_proof,
        );

        let price = client.get_price(&asset, &0u64).unwrap();
        assert_eq!(price.price, 5_000i128);
    }

    // ── #296 Test 9: get_submission_proof returns stored proof ────────────────
    #[test]
    fn test_get_submission_proof() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);

        let proof = make_cex_proof(&e);
        client.submit_price_with_external_proof(&source, &asset, &1_000i128, &1_000_000u64, &proof);

        let stored = client.get_submission_proof(&asset, &source);
        assert!(stored.is_some());
        let p = stored.unwrap();
        assert_eq!(p.proof_type, ProofType::CexSignedResponse);
    }

    // ── #296 Test 10: get_asset_proof_requirement default is AnyOrNone ────────
    #[test]
    fn test_default_proof_requirement_is_any_or_none() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);

        let req = client.get_asset_proof_requirement(&asset);
        assert_eq!(req, AssetProofRequirement::AnyOrNone);
    }
}
