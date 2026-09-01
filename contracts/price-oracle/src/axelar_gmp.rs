//! # Axelar GMP Integration
//!
//! Lets a foreign-chain price attestation reach this oracle over Axelar's
//! General Message Passing (GMP), as an alternative to the in-house relay
//! (`cross_chain_relay`, #182) that leans on Axelar's own validator network
//! for trust minimization instead of a bespoke light client.
//!
//! ## Trust model
//!
//! Axelar's Gateway contract is the trust root: its verifier set
//! independently confirms that a source-chain contract genuinely sent a
//! given message before ever calling into a destination contract. Mirroring
//! the "AxelarExecutable" pattern used on EVM chains (`execute()` only
//! trusts `msg.sender == gateway`), this module only accepts calls whose
//! `gateway` argument matches the admin-configured trusted Gateway address
//! *and* authorizes the call (`gateway.require_auth()`) — which a direct
//! contract-to-contract invocation from the real Axelar Gateway satisfies
//! automatically. All GMP signature/quorum verification is Axelar's
//! responsibility and happens inside the Gateway contract, not here.
//!
//! ## Message format
//!
//! The GMP `payload` bytes are the canonical
//! [`crate::types::CrossChainPricePayload`] wire format defined in
//! [`crate::bridge_common`] — shared with the LayerZero integration so a
//! single format is used regardless of transport. The payload's
//! `foreign_asset` id is resolved to a Stellar asset via the canonical
//! [`crate::asset_registry`], keyed by `source_chain`.
//!
//! ## Replay protection
//!
//! Each GMP message carries a unique Axelar-assigned `command_id`. This
//! module records every `command_id` it has executed and rejects repeats.
//!
//! ## Relayer / keeper
//!
//! Once the Gateway has approved a command, delivering it here is
//! permissionless — any keeper may relay the call, exactly as with a real
//! `AxelarExecutable`, since authorization comes from the Gateway's own
//! approval rather than from the caller's identity.

use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, String};

use crate::asset_registry::resolve_enabled_mapping;
use crate::bridge_common::{apply_bridged_price, decode_price_payload};
use crate::events::{AxelarGatewaySetEvent, AxelarMessageExecutedEvent, AxelarTrustedSourceSetEvent};
use crate::pause::check_not_paused;
use crate::storage::{check_source, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode};

/// Configures the trusted Axelar Gateway contract address. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
pub fn set_axelar_gateway(env: &Env, gateway: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .set(&DataKey::AxelarGateway, &gateway);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::AxelarGateway, LEDGER_THRESHOLD, LEDGER_BUMP);

    AxelarGatewaySetEvent { gateway }.publish(env);
}

/// Returns the currently configured Axelar Gateway address, if any.
pub fn get_axelar_gateway(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&DataKey::AxelarGateway)
}

/// Registers `bridge_source` — an already-registered oracle source (see
/// [`crate::sources::add_source`]) — as the attribution target for GMP
/// messages arriving from `(source_chain, source_address)`. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::NotAuthorized`] (via [`check_source`]) — `bridge_source` is
///   not a registered oracle source.
pub fn set_axelar_trusted_source(
    env: &Env,
    source_chain: String,
    source_address: String,
    bridge_source: Address,
) {
    let admin = get_admin(env);
    admin.require_auth();
    check_source(env, &bridge_source);

    let key = DataKey::AxelarTrustedSource(source_chain.clone(), source_address.clone());
    env.storage().persistent().set(&key, &bridge_source);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    AxelarTrustedSourceSetEvent {
        bridge_source,
        source_chain,
        source_address,
    }
    .publish(env);
}

/// Revokes a trusted Axelar GMP source. Admin-only.
pub fn remove_axelar_trusted_source(env: &Env, source_chain: String, source_address: String) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .remove(&DataKey::AxelarTrustedSource(source_chain, source_address));
}

fn read_trusted_source(env: &Env, source_chain: &String, source_address: &String) -> Address {
    env.storage()
        .persistent()
        .get(&DataKey::AxelarTrustedSource(
            source_chain.clone(),
            source_address.clone(),
        ))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::AxelarSourceNotTrusted))
}

/// Delivers a price update relayed over Axelar GMP.
///
/// `gateway` must equal the configured trusted Gateway address and must
/// authorize this call — satisfied automatically when the real Axelar
/// Gateway contract invokes this function directly after its own
/// verifier-set quorum check has approved the message.
///
/// # Errors
///
/// * [`ErrorCode::AxelarGatewayNotConfigured`] — no Gateway has been configured.
/// * [`ErrorCode::NotAuthorized`] — `gateway` does not match the configured Gateway.
/// * [`ErrorCode::AxelarCommandAlreadyExecuted`] — `command_id` was already processed.
/// * [`ErrorCode::AxelarSourceNotTrusted`] — no bridge source is registered for
///   `(source_chain, source_address)`.
/// * [`ErrorCode::ForeignAssetNotMapped`] / [`ErrorCode::ForeignAssetMappingDisabled`] —
///   the payload's foreign asset id has no active registry mapping on `source_chain`.
pub fn execute_axelar_message(
    env: &Env,
    gateway: Address,
    command_id: BytesN<32>,
    source_chain: String,
    source_address: String,
    payload: Bytes,
) {
    check_not_paused(env);

    let configured = get_axelar_gateway(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::AxelarGatewayNotConfigured));
    if gateway != configured {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
    gateway.require_auth();

    let executed_key = DataKey::AxelarExecutedCommand(command_id.clone());
    if env.storage().persistent().has(&executed_key) {
        panic_with_error!(env, ErrorCode::AxelarCommandAlreadyExecuted);
    }
    env.storage().persistent().set(&executed_key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&executed_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let bridge_source = read_trusted_source(env, &source_chain, &source_address);
    let message = decode_price_payload(env, &payload);
    let mapping = resolve_enabled_mapping(env, &source_chain, &message.foreign_asset);

    apply_bridged_price(
        env,
        bridge_source.clone(),
        mapping.stellar_asset.clone(),
        message.price,
        message.timestamp,
        message.decimals,
        source_chain.clone(),
    );

    AxelarMessageExecutedEvent {
        asset: mapping.stellar_asset,
        bridge_source,
        command_id,
        source_chain,
        source_address,
        price: message.price,
    }
    .publish(env);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_common::encode_price_payload;
    use crate::types::CrossChainPricePayload;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn setup(env: &Env) -> (Address, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let asset = Address::generate(env);
        crate::admin::initialize(env, admin.clone(), 1, 100, 18, String::from_str(env, "Oracle"));
        crate::assets::register_asset(env, asset.clone());
        env.ledger().with_mut(|l| l.timestamp = 1_000_000);
        (admin, asset, Address::generate(env))
    }

    fn wire_axelar(
        env: &Env,
        asset: &Address,
        gateway: &Address,
        bridge_source: &Address,
        chain: &String,
        foreign_address: &BytesN<32>,
    ) {
        crate::sources::add_source(env, bridge_source.clone(), String::from_str(env, "Axelar"));
        crate::sources::add_source_asset(env, bridge_source.clone(), asset.clone());
        set_axelar_gateway(env, gateway.clone());
        set_axelar_trusted_source(
            env,
            chain.clone(),
            String::from_str(env, "0xSourceContract"),
            bridge_source.clone(),
        );
        crate::asset_registry::register_foreign_asset_mapping(
            env,
            asset.clone(),
            chain.clone(),
            foreign_address.clone(),
            18,
        );
    }

    #[test]
    fn test_execute_axelar_message_updates_price() {
        let env = Env::default();
        let (_, asset, gateway) = setup(&env);
        let bridge_source = Address::generate(&env);
        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[11u8; 32]);
        wire_axelar(&env, &asset, &gateway, &bridge_source, &chain, &foreign_address);

        let payload = CrossChainPricePayload {
            foreign_asset: foreign_address,
            price: 1_000_000_000_000_000_000i128,
            decimals: 18,
            timestamp: env.ledger().timestamp(),
            nonce: 1,
        };
        let encoded = encode_price_payload(&env, &payload);

        execute_axelar_message(
            &env,
            gateway,
            BytesN::from_array(&env, &[1u8; 32]),
            chain,
            String::from_str(&env, "0xSourceContract"),
            encoded,
        );

        let entry: crate::types::PriceEntry = env
            .storage()
            .persistent()
            .get(&DataKey::Submission(asset, bridge_source))
            .unwrap();
        assert_eq!(entry.price, 1_000_000_000_000_000_000i128);
    }

    #[test]
    #[should_panic]
    fn test_execute_axelar_message_rejects_replay() {
        let env = Env::default();
        let (_, asset, gateway) = setup(&env);
        let bridge_source = Address::generate(&env);
        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[12u8; 32]);
        wire_axelar(&env, &asset, &gateway, &bridge_source, &chain, &foreign_address);

        let payload = CrossChainPricePayload {
            foreign_asset: foreign_address,
            price: 1_000_000_000_000_000_000i128,
            decimals: 18,
            timestamp: env.ledger().timestamp(),
            nonce: 1,
        };
        let encoded = encode_price_payload(&env, &payload);
        let command_id = BytesN::from_array(&env, &[2u8; 32]);

        execute_axelar_message(
            &env,
            gateway.clone(),
            command_id.clone(),
            chain.clone(),
            String::from_str(&env, "0xSourceContract"),
            encoded.clone(),
        );
        // Second delivery of the same command_id must be rejected.
        execute_axelar_message(
            &env,
            gateway,
            command_id,
            chain,
            String::from_str(&env, "0xSourceContract"),
            encoded,
        );
    }

    #[test]
    #[should_panic]
    fn test_execute_axelar_message_rejects_untrusted_gateway() {
        let env = Env::default();
        let (_, asset, gateway) = setup(&env);
        let bridge_source = Address::generate(&env);
        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[13u8; 32]);
        wire_axelar(&env, &asset, &gateway, &bridge_source, &chain, &foreign_address);

        let payload = CrossChainPricePayload {
            foreign_asset: foreign_address,
            price: 1_000_000_000_000_000_000i128,
            decimals: 18,
            timestamp: env.ledger().timestamp(),
            nonce: 1,
        };
        let encoded = encode_price_payload(&env, &payload);

        let impostor_gateway = Address::generate(&env);
        execute_axelar_message(
            &env,
            impostor_gateway,
            BytesN::from_array(&env, &[3u8; 32]),
            chain,
            String::from_str(&env, "0xSourceContract"),
            encoded,
        );
    }
}
