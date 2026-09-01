//! Role-Based Access Control (RBAC) module (#241)
//!
//! Implements admin delegation with specific roles. The primary admin can delegate
//! capabilities to other addresses without transferring full admin authority.
//!
//! Roles:
//! - SourceManager: add/remove oracle sources
//! - AssetManager: register/unregister assets
//! - PriceUpdater: submit prices, override prices
//! - ConfigManager: modify configuration settings
//! - UpgradeManager: perform contract upgrades and admin transfers

use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, Vec};

use crate::events::{emit_admin_action, RoleDelegatedEvent, RoleRevokedEvent};
use crate::storage::get_admin;
use crate::types::{DataKey, ErrorCode, Role};

/// Check if an address has a specific role.
///
/// The primary admin implicitly has all roles. Delegated addresses only have
/// the roles explicitly granted to them.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `caller` - The address to check.
/// * `role` - The role to verify.
///
/// # Returns
///
/// `true` if the caller has the role, `false` otherwise.
pub fn has_role(env: &Env, caller: &Address, role: Role) -> bool {
    let admin = get_admin(env);

    // Admin implicitly has all roles
    if caller == &admin {
        return true;
    }

    let role_discriminant = role as u32;
    let role_mask: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::DelegatedRole(caller.clone(), role_discriminant))
        .unwrap_or(0);

    role_mask != 0
}

/// Check if a caller has a specific role, panicking if not.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `caller` - The address to check.
/// * `role` - The role to verify.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller does not have the role.
pub fn check_role(env: &Env, caller: &Address, role: Role) {
    if !has_role(env, caller, role) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
}

/// Delegate a role to an address.
///
/// Only the primary admin can delegate roles. The delegated address gains the
/// specified capability without becoming the primary admin.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `delegatee` - The address to grant the role to.
/// * `role` - The role to delegate.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the primary admin.
pub fn delegate_role(env: &Env, delegatee: Address, role: Role) {
    let admin = get_admin(env);
    admin.require_auth();

    let role_discriminant = role as u32;

    // Mark the role as granted (simple boolean flag approach)
    env.storage().persistent().set(
        &DataKey::DelegatedRole(delegatee.clone(), role_discriminant),
        &1u32,
    );

    // Add to role holders list if not already present
    let mut holders: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::RoleHolders(role_discriminant))
        .unwrap_or_else(|| Vec::new(env));

    // Check if already in list
    let mut already_present = false;
    for i in 0..holders.len() {
        if holders.get_unchecked(i) == delegatee {
            already_present = true;
            break;
        }
    }

    if !already_present {
        holders.push_back(delegatee.clone());
        env.storage()
            .persistent()
            .set(&DataKey::RoleHolders(role_discriminant), &holders);
    }

    // Emit event
    RoleDelegatedEvent {
        delegatee: delegatee.clone(),
        role: role_discriminant,
        delegator: admin.clone(),
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("delrole"), admin, Bytes::new(env));
}

/// Revoke a role from an address.
///
/// Only the primary admin can revoke roles. The revoked address loses the
/// specified capability.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `delegatee` - The address to revoke the role from.
/// * `role` - The role to revoke.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the primary admin.
pub fn revoke_role(env: &Env, delegatee: Address, role: Role) {
    let admin = get_admin(env);
    admin.require_auth();

    let role_discriminant = role as u32;

    // Remove the role grant
    env.storage().persistent().remove(&DataKey::DelegatedRole(
        delegatee.clone(),
        role_discriminant,
    ));

    // Remove from role holders list
    let mut holders: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::RoleHolders(role_discriminant))
        .unwrap_or_else(|| Vec::new(env));

    // Find and remove the delegatee
    let mut new_holders = Vec::new(env);
    for i in 0..holders.len() {
        let holder = holders.get_unchecked(i);
        if holder != delegatee {
            new_holders.push_back(holder);
        }
    }

    env.storage()
        .persistent()
        .set(&DataKey::RoleHolders(role_discriminant), &new_holders);

    // Emit event
    RoleRevokedEvent {
        delegatee: delegatee.clone(),
        role: role_discriminant,
        delegator: admin.clone(),
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("revrole"), admin, Bytes::new(env));
}

/// Get all holders of a specific role.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `role` - The role to query.
///
/// # Returns
///
/// Ordered list of addresses that have been granted this role.
pub fn get_role_holders(env: &Env, role: Role) -> Vec<Address> {
    let role_discriminant = role as u32;

    env.storage()
        .persistent()
        .get(&DataKey::RoleHolders(role_discriminant))
        .unwrap_or_else(|| Vec::new(env))
}

/// Get all roles held by a specific address.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `holder` - The address to query.
///
/// # Returns
///
/// Vector of role discriminants (as u32) that the address holds.
pub fn get_address_roles(env: &Env, holder: &Address) -> Vec<u32> {
    let admin = get_admin(env);

    let mut roles = Vec::new(env);

    // Admin has all roles
    if holder == &admin {
        roles.push_back(Role::SourceManager as u32);
        roles.push_back(Role::AssetManager as u32);
        roles.push_back(Role::PriceUpdater as u32);
        roles.push_back(Role::ConfigManager as u32);
        roles.push_back(Role::UpgradeManager as u32);
        return roles;
    }

    // Check each role
    for role_discriminant in 0..5u32 {
        if env
            .storage()
            .persistent()
            .get::<_, u32>(&DataKey::DelegatedRole(holder.clone(), role_discriminant))
            .is_some()
        {
            roles.push_back(role_discriminant);
        }
    }

    roles
}
