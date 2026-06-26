use soroban_sdk::{contractevent, Address, String};

// ContractInitializedEvent uses manual publishing due to String field
// limitations with the macro in soroban-sdk 26.

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

#[allow(dead_code)]
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

#[contractevent]
#[derive(Clone)]
pub struct SourcesInsufficientEvent {
    #[topic]
    pub asset: Address,
    pub current_source_count: u32,
    pub min_sources_required: u32,
}

#[allow(deprecated)]
pub fn emit_initialized(
    env: &soroban_sdk::Env,
    admin: Address,
    min_sources: u32,
    max_history: u32,
    decimals: u32,
    description: String,
) {
    let sym = soroban_sdk::symbol_short!("init");
    env.events().publish(
        (sym, admin),
        (min_sources, max_history, decimals, description),
    );
}

#[contractevent]
#[derive(Clone)]
pub struct PriceAggregatedEvent {
    #[topic]
    pub asset: Address,
    pub price: i128,
    pub num_sources: u32,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct HistoryPrunedEvent {
    #[topic]
    pub asset: Address,
    pub pruned_ledger: u32,
    pub remaining: u32,
}

// TimestampThresholdChangedEvent uses manual publishing (u64 value in
// contractevent triggers a soroban-sdk 26 macro limitation).
#[allow(deprecated)]
pub fn emit_timestamp_threshold_changed(env: &soroban_sdk::Env, admin: Address, value: u64) {
    let sym = soroban_sdk::symbol_short!("tthr");
    env.events().publish((sym, admin), (value,));
}

#[contractevent]
#[derive(Clone)]
pub struct PriceDeviationFlaggedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub price: i128,
    pub median_price: i128,
    pub deviation_percent: u32,
}

#[allow(deprecated)]
pub fn emit_max_price_deviation_changed(env: &soroban_sdk::Env, admin: Address, value: u32) {
    let sym = soroban_sdk::symbol_short!("devn");
    env.events().publish((sym, admin), (value,));
}

#[contractevent]
#[derive(Clone)]
pub struct SourceHeartbeatEvent {
    #[topic]
    pub source: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceInactiveEvent {
    #[topic]
    pub source: Address,
    pub last_heartbeat: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct HeartbeatIntervalChangedEvent {
    pub value: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceActiveAgainEvent {
    #[topic]
    pub source: Address,
    pub timestamp: u64,
}
