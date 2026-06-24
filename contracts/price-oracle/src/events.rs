use soroban_sdk::{contractevent, Address, String};

#[contractevent]
#[derive(Clone)]
pub struct PriceSubmittedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub price: i128,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct PriceUpdatedEvent {
    #[topic]
    pub asset: Address,
    pub new_price: i128,
    pub old_price: i128,
    pub timestamp: u64,
    pub prev_timestamp: u64,
    pub decimals: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceAddedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub admin: Address,
    pub name: String,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceRemovedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct AssetRegisteredEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct AssetUnregisteredEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct AdminChangedEvent {
    #[topic]
    pub old_admin: Address,
    #[topic]
    pub new_admin: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct ContractUpgradedEvent {
    #[topic]
    pub new_wasm_hash: soroban_sdk::BytesN<32>,
}

#[contractevent]
#[derive(Clone)]
pub struct MinSourcesChangedEvent {
    pub value: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct MaxHistoryChangedEvent {
    pub value: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct ResolutionChangedEvent {
    pub value: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct DecimalsChangedEvent {
    pub value: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct DescriptionChangedEvent {
    pub description: String,
}
