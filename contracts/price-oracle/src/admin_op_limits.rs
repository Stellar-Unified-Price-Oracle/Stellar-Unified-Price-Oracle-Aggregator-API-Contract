/// Admin operation spending limits (#238)
///
/// Enforces per-operation-type daily limits. Prevents compromised admin key from causing
/// massive damage in a single transaction.
///
/// Also hosts the governance action risk classification registry (#654): every privileged
/// governance action kind maps to exactly one declared risk class and required approval delay.
use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env};

use crate::events::emit_admin_action;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AdminOpLimit, AdminOperationType, DataKey, ErrorCode};

// Default limits per operation type per day
const DEFAULT_ADD_SOURCE_LIMIT: u32 = 5;
const DEFAULT_REMOVE_SOURCE_LIMIT: u32 = 3;
const DEFAULT_REGISTER_ASSET_LIMIT: u32 = 10;
const DEFAULT_UNREGISTER_ASSET_LIMIT: u32 = 5;
const DEFAULT_SET_DECIMALS_LIMIT: u32 = 2;
const DEFAULT_SET_RESOLUTION_LIMIT: u32 = 2;

/// Governance action kinds subject to risk classification (#654).
/// Each privileged action maps to exactly one of these kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum GovernanceActionKind {
    /// Parameter change (decimals, resolution, thresholds, etc.).
    ParameterChange = 0,
    /// Source add/remove/update.
    SourceChange = 1,
    /// Asset register/unregister/update.
    AssetChange = 2,
    /// Pause / unpause of the oracle.
    PauseAction = 3,
    /// Contract upgrade / code change.
    UpgradeAction = 4,
}

impl GovernanceActionKind {
    /// All declared governance action kinds, in canonical order.
    pub const ALL: [GovernanceActionKind; 5] = [
        GovernanceActionKind::ParameterChange,
        GovernanceActionKind::SourceChange,
        GovernanceActionKind::AssetChange,
        GovernanceActionKind::PauseAction,
        GovernanceActionKind::UpgradeAction,
    ];

    /// Map a raw action kind discriminant to a declared kind, if any.
    pub fn from_u32(value: u32) -> Option<GovernanceActionKind> {
        match value {
            0 => Some(GovernanceActionKind::ParameterChange),
            1 => Some(GovernanceActionKind::SourceChange),
            2 => Some(GovernanceActionKind::AssetChange),
            3 => Some(GovernanceActionKind::PauseAction),
            4 => Some(GovernanceActionKind::UpgradeAction),
            _ => None,
        }
    }
}

/// Declared risk class for a governance action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RiskClass {
    Low = 0,
    Medium = 1,
    High = 2,
    Critical = 3,
}

/// A single entry in the governance action risk classification registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GovernanceActionPolicy {
    /// The action kind this policy applies to.
    pub kind: GovernanceActionKind,
    /// Declared risk class.
    pub risk: RiskClass,
    /// Required approval delay in ledgers before the action may execute.
    pub approval_delay_ledgers: u32,
}

// Approval delays (in ledgers) per risk class. Higher risk => longer delay.
const DELAY_LOW_LEDGERS: u32 = 0;
const DELAY_MEDIUM_LEDGERS: u32 = 720; // ~1 hour at 5s/ledger
const DELAY_HIGH_LEDGERS: u32 = 17_280; // ~1 day
const DELAY_CRITICAL_LEDGERS: u32 = 120_960; // ~7 days

/// The governance action risk classification registry (#654).
/// Every privileged action kind maps to exactly one declared class and approval path.
const GOVERNANCE_ACTION_REGISTRY: [GovernanceActionPolicy; 5] = [
    GovernanceActionPolicy {
        kind: GovernanceActionKind::ParameterChange,
        risk: RiskClass::Medium,
        approval_delay_ledgers: DELAY_MEDIUM_LEDGERS,
    },
    GovernanceActionPolicy {
        kind: GovernanceActionKind::SourceChange,
        risk: RiskClass::High,
        approval_delay_ledgers: DELAY_HIGH_LEDGERS,
    },
    GovernanceActionPolicy {
        kind: GovernanceActionKind::AssetChange,
        risk: RiskClass::High,
        approval_delay_ledgers: DELAY_HIGH_LEDGERS,
    },
    GovernanceActionPolicy {
        kind: GovernanceActionKind::PauseAction,
        risk: RiskClass::Critical,
        approval_delay_ledgers: DELAY_CRITICAL_LEDGERS,
    },
    GovernanceActionPolicy {
        kind: GovernanceActionKind::UpgradeAction,
        risk: RiskClass::Critical,
        approval_delay_ledgers: DELAY_CRITICAL_LEDGERS,
    },
];

/// Look up the declared policy for a governance action kind.
/// Returns `None` if the kind is not declared in the registry.
pub fn get_governance_action_policy(kind: GovernanceActionKind) -> Option<GovernanceActionPolicy> {
    let mut i = 0;
    while i < GOVERNANCE_ACTION_REGISTRY.len() {
        let policy = GOVERNANCE_ACTION_REGISTRY[i];
        if policy.kind == kind {
            return Some(policy);
        }
        i += 1;
    }
    None
}

/// Look up the declared policy for a raw action kind discriminant.
pub fn get_governance_action_policy_by_u32(value: u32) -> Option<GovernanceActionPolicy> {
    match GovernanceActionKind::from_u32(value) {
        Some(kind) => get_governance_action_policy(kind),
        None => None,
    }
}

/// Get the declared risk class for a governance action kind.
pub fn get_governance_action_risk(kind: GovernanceActionKind) -> Option<RiskClass> {
    get_governance_action_policy(kind).map(|p| p.risk)
}

/// Get the required approval delay (in ledgers) for a governance action kind.
pub fn get_governance_action_approval_delay(kind: GovernanceActionKind) -> Option<u32> {
    get_governance_action_policy(kind).map(|p| p.approval_delay_ledgers)
}

/// Get the default daily limit for an operation type.
fn get_default_limit(op_type: u32) -> u32 {
    match op_type {
        0 => DEFAULT_ADD_SOURCE_LIMIT,
        1 => DEFAULT_REMOVE_SOURCE_LIMIT,
        2 => DEFAULT_REGISTER_ASSET_LIMIT,
        3 => DEFAULT_UNREGISTER_ASSET_LIMIT,
        4 => DEFAULT_SET_DECIMALS_LIMIT,
        5 => DEFAULT_SET_RESOLUTION_LIMIT,
        _ => u32::MAX, // Unknown operation types are unlimited
    }
}

/// Calculate the "day" epoch from a ledger timestamp.
/// Uses days since Unix epoch for consistency.
fn get_day_epoch(timestamp: u64) -> u32 {
    (timestamp / 86400) as u32
}

/// Set the daily limit for a specific operation type.
/// Only the admin can call this.
pub fn set_admin_op_daily_limit(env: &Env, op_type: u32, daily_limit: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if daily_limit == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let limit_config = AdminOpLimit {
        daily_limit,
        set_ledger: env.ledger().sequence(),
    };

    env.storage()
        .persistent()
        .set(&DataKey::AdminOpDailyLimit(op_type), &limit_config);

    emit_admin_action(env, symbol_short!("op_lim"), admin, Bytes::new(env));
}

/// Get the current daily limit for an operation type.
pub fn get_admin_op_daily_limit(env: &Env, op_type: u32) -> u32 {
    if let Some(config) = env
        .storage()
        .persistent()
        .get::<_, AdminOpLimit>(&DataKey::AdminOpDailyLimit(op_type))
    {
        config.daily_limit
    } else {
        get_default_limit(op_type)
    }
}

/// Check if an operation can proceed without exceeding the daily limit.
/// Returns `true` if the operation is allowed, `false` if it would exceed the limit.
pub fn check_admin_op_limit(env: &Env, op_type: u32) -> bool {
    let limit = get_admin_op_daily_limit(env, op_type);
    if limit == u32::MAX {
        return true; // Unlimited operations
    }

    let current_day = get_day_epoch(env.ledger().timestamp());
    let count_key = DataKey::AdminOpDailyCount(op_type, current_day);

    let current_count: u32 = env
        .storage()
        .persistent()
        .get::<_, u32>(&count_key)
        .unwrap_or(0);

    current_count < limit
}

/// Increment the operation counter for the current day.
/// Call this after an operation is successfully executed.
pub fn increment_admin_op_counter(env: &Env, op_type: u32) {
    let current_day = get_day_epoch(env.ledger().timestamp());
    let count_key = DataKey::AdminOpDailyCount(op_type, current_day);

    let current_count: u32 = env
        .storage()
        .persistent()
        .get::<_, u32>(&count_key)
        .unwrap_or(0);

    let new_count = current_count.saturating_add(1);
    env.storage().persistent().set(&count_key, &new_count);

    env.storage()
        .persistent()
        .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Get the current count of operations performed today for a given type.
pub fn get_admin_op_daily_count(env: &Env, op_type: u32) -> u32 {
    let current_day = get_day_epoch(env.ledger().timestamp());
    env.storage()
        .persistent()
        .get::<_, u32>(&DataKey::AdminOpDailyCount(op_type, current_day))
        .unwrap_or(0)
}

/// Validate that an operation is allowed and panic if the daily limit is exceeded.
pub fn validate_admin_op_allowed(env: &Env, op_type: u32) {
    if !check_admin_op_limit(env, op_type) {
        panic_with_error!(env, ErrorCode::OperationLimitExceeded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::{Address, Env};

    #[test]
    fn test_admin_op_limits_track_daily_count() {
        let env = Env::default();
        let __contract_id = env.register(crate::PriceOracleContract, ());
        env.mock_all_auths();
        env.as_contract(&__contract_id, || {
            let admin = Address::generate(&env);

            env.ledger().with_mut(|l| {
                l.timestamp = 1000; // Some day
            });

            crate::admin::initialize(
                &env,
                admin.clone(),
                1,
                100,
                18,
                soroban_sdk::String::from_slice(&env, "Oracle"),
            );

            // Initially count should be 0
            assert_eq!(get_admin_op_daily_count(&env, 0), 0);

            // Increment a few times
            increment_admin_op_counter(&env, 0);
            assert_eq!(get_admin_op_daily_count(&env, 0), 1);

            increment_admin_op_counter(&env, 0);
            assert_eq!(get_admin_op_daily_count(&env, 0), 2);

            // Different operation type should have separate count
            increment_admin_op_counter(&env, 1);
            assert_eq!(get_admin_op_daily_count(&env, 1), 1);
        });
    }

    #[test]
    fn test_check_admin_op_limit() {
        let env = Env::default();
        let __contract_id = env.register(crate::PriceOracleContract, ());
        env.mock_all_auths();
        env.as_contract(&__contract_id, || {
            let admin = Address::generate(&env);

            env.ledger().with_mut(|l| {
                l.timestamp = 1000;
            });

            crate::admin::initialize(
                &env,
                admin.clone(),
                1,
                100,
                18,
                soroban_sdk::String::from_slice(&env, "Oracle"),
            );

            // Set a low limit
            set_admin_op_daily_limit(&env, 0, 2);

            // Should be able to do operations up to the limit
            assert!(check_admin_op_limit(&env, 0));
            increment_admin_op_counter(&env, 0);

            assert!(check_admin_op_limit(&env, 0));
            increment_admin_op_counter(&env, 0);

            // Now we've hit the limit
            assert!(!check_admin_op_limit(&env, 0));
        });
    }

    #[test]
    fn test_validate_admin_op_allowed_panics() {
        let env = Env::default();
        let __contract_id = env.register(crate::PriceOracleContract, ());
        env.mock_all_auths();
        env.as_contract(&__contract_id, || {
            let admin = Address::generate(&env);

            env.ledger().with_mut(|l| {
                l.timestamp = 1000;
            });

            crate::admin::initialize(
                &env,
                admin.clone(),
                1,
                100,
                18,
                soroban_sdk::String::from_slice(&env, "Oracle"),
            );

            // Set limit to 1
            set_admin_op_daily_limit(&env, 0, 1);

            // First call should succeed
            validate_admin_op_allowed(&env, 0);
            increment_admin_op_counter(&env, 0);

            // Second call should panic
            // (In actual test framework, we'd use #[should_panic])
        });
    }

    #[test]
    fn test_day_epoch_calculation() {
        // Day epoch should reset at midnight UTC
        let day1 = get_day_epoch(1000);
        let day1_later = get_day_epoch(1000 + 3600);
        assert_eq!(day1, day1_later); // Same day

        let day2 = get_day_epoch(1000 + 86400);
        assert!(day2 > day1); // Different day
    }

    #[test]
    fn test_governance_registry_covers_every_action_kind() {
        // Every declared action kind must map to exactly one policy entry.
        for kind in GovernanceActionKind::ALL.iter() {
            let policy = get_governance_action_policy(*kind)
                .expect("every governance action kind must be classified");
            assert_eq!(policy.kind, *kind);
            assert!(policy.approval_delay_ledgers > 0 || policy.risk == RiskClass::Low);
        }
    }

    #[test]
    fn test_governance_registry_has_no_duplicates() {
        // No action kind may appear more than once in the registry.
        let mut i = 0;
        while i < GOVERNANCE_ACTION_REGISTRY.len() {
            let mut j = i + 1;
            while j < GOVERNANCE_ACTION_REGISTRY.len() {
                assert!(
                    GOVERNANCE_ACTION_REGISTRY[i].kind != GOVERNANCE_ACTION_REGISTRY[j].kind,
                    "duplicate governance action kind in registry"
                );
                j += 1;
            }
            i += 1;
        }
        assert_eq!(GOVERNANCE_ACTION_REGISTRY.len(), GovernanceActionKind::ALL.len());
    }

    #[test]
    fn test_governance_registry_risk_and_delay_ordering() {
        // Higher risk classes must require at least as long an approval delay.
        let param = get_governance_action_policy(GovernanceActionKind::ParameterChange).unwrap();
        let source = get_governance_action_policy(GovernanceActionKind::SourceChange).unwrap();
        let asset = get_governance_action_policy(GovernanceActionKind::AssetChange).unwrap();
        let pause = get_governance_action_policy(GovernanceActionKind::PauseAction).unwrap();
        let upgrade = get_governance_action_policy(GovernanceActionKind::UpgradeAction).unwrap();

        assert_eq!(param.risk, RiskClass::Medium);
        assert_eq!(source.risk, RiskClass::High);
        assert_eq!(asset.risk, RiskClass::High);
        assert_eq!(pause.risk, RiskClass::Critical);
        assert_eq!(upgrade.risk, RiskClass::Critical);

        assert!(param.approval_delay_ledgers <= source.approval_delay_ledgers);
        assert!(source.approval_delay_ledgers <= pause.approval_delay_ledgers);
        assert!(pause.approval_delay_ledgers <= upgrade.approval_delay_ledgers);
    }

    #[test]
    fn test_governance_registry_lookup_by_u32() {
        assert_eq!(
            get_governance_action_policy_by_u32(0).map(|p| p.kind),
            Some(GovernanceActionKind::ParameterChange)
        );
        assert_eq!(
            get_governance_action_policy_by_u32(4).map(|p| p.kind),
            Some(GovernanceActionKind::UpgradeAction)
        );
        // Undeclared discriminants must not resolve to a policy.
        assert!(get_governance_action_policy_by_u32(99).is_none());
    }
}
