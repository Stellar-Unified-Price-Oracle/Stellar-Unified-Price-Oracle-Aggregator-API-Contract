use soroban_sdk::{Address, BytesN, Env, String};

use crate::admin::{get_admin_address, get_decimals, get_description};
use crate::storage::read_registered_assets;
use crate::types::{
    ContractMetadata, INTERFACE_ID_ADMIN, INTERFACE_ID_COMMIT_REVEAL, INTERFACE_ID_METADATA,
    INTERFACE_ID_NATIVE_FEES, INTERFACE_ID_OPTIMISTIC, INTERFACE_ID_SEP40,
    INTERFACE_ID_SOURCE_MGMT, INTERFACE_ID_SUBSCRIPTION,
};

const CONTRACT_NAME: &str = "Stellar Unified Price Oracle";
const CONTRACT_VERSION: &str = "1.0.0";

fn supported_interfaces(env: &Env) -> Vec<BytesN<4>> {
    let mut interfaces: Vec<BytesN<4>> = Vec::new(env);
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_SEP40));
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_ADMIN));
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_SOURCE_MGMT));
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_SUBSCRIPTION));
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_OPTIMISTIC));
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_COMMIT_REVEAL));
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_NATIVE_FEES));
    interfaces.push_back(BytesN::from_slice(env, INTERFACE_ID_METADATA));
    interfaces
}

pub fn supports_interface(env: &Env, interface_id: &BytesN<4>) -> bool {
    let interfaces = supported_interfaces(env);
    for i in 0..interfaces.len() {
        if interfaces.get_unchecked(i) == *interface_id {
            return true;
        }
    }
    false
}

pub fn get_contract_metadata(env: &Env) -> ContractMetadata {
    let admin = get_admin_address(env);
    let decimals = get_decimals(env);
    let description = get_description(env);
    let interfaces = supported_interfaces(env);
    ContractMetadata {
        name: String::from_slice(env, CONTRACT_NAME),
        version: String::from_slice(env, CONTRACT_VERSION),
        description,
        admin,
        decimals,
        supported_interfaces: interfaces,
    }
}
