//! #510 — Dead-Man Switch (liveness watchdog)
//!
//! If the operating team disappears — key loss, compromise, an outage nobody
//! is watching — the oracle should **fail toward safety**, not keep serving
//! values that no one is accountable for. This module is that brake: an
//! operator-controlled heartbeat, and a switch that trips automatically once
//! the heartbeat has been missing for longer than a configured interval.
//!
//! ## The safe degraded state
//!
//! When tripped, the contract enters a **degraded** state that is deliberately
//! boring:
//!
//! * **price submissions are rejected** — no new aggregate can be produced
//!   while nobody is watching the sources;
//! * **price reads return nothing** — a stale value served from storage must
//!   not be mistaken for a live one, so [`prices`](crate::prices::prices)
//!   returns `None` rather than the last value;
//! * everything else keeps working: reads of configuration, the health report,
//!   governance, and the recovery path itself all remain available, so the
//!   degraded state can be observed and reversed.
//!
//! "Serves no unsafely-fresh value" is the property that matters: in the
//! degraded state a consumer receives *nothing* rather than something whose
//! freshness nobody is attesting to.
//!
//! ## Why it cannot be used to force downtime
//!
//! The switch is operator-controlled, so an admin alone must not be able to
//! weaponise it against consumers. Two properties prevent that:
//!
//! * **A heartbeat cannot be spoofed.** Only an address in the registered
//!   operator set may heartbeat, and it must `require_auth` — an unauthorized
//!   caller cannot keep the contract out of the degraded state, and equally
//!   cannot *push* it there by forging someone else's heartbeat.
//! * **Recovery does not need the admin.** The degraded state is cleared by the
//!   **recovery guardians** of [`crate::recovery`], an authority that is
//!   independent of the admin key precisely because the admin key is the thing
//!   that may be lost. (The admin may also clear it while still holding the
//!   key, but recovery never *depends* on that.)
//!
//! ## Warnings before the trigger
//!
//! Tripping on the first missed heartbeat would punish a brief network gap, so
//! the switch warns first: a warning is emitted once the heartbeat is overdue by
//! `warn_after`, and the trigger only fires at `trigger_after`
//! (`trigger_after > warn_after`). Off-chain monitoring therefore always sees
//! [`DeadManWarningEvent`] before [`DeadManTriggeredEvent`], giving operators a
//! window in which to heartbeat and avert the trip entirely.
//!
//! ## Accepting the false-trigger risk
//!
//! A false trigger during a benign outage is possible by construction: that is
//! the trade this issue asks for, and the degraded state is chosen so the cost
//! of a false trip (no prices served) is strictly smaller than the cost of the
//! alternative (serving values while the operator team is unreachable). The
//! interval is admin-configurable so it can be set above the worst realistic
//! outage.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{DeadManRecoveredEvent, DeadManTriggeredEvent, DeadManWarningEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, DeadManConfig, DeadManState, ErrorCode};

/// Default trigger interval: 7 days. Long enough that a weekend-sized outage
/// does not trip it, short enough that a silently dead oracle fails safe
/// before consumers act on week-old prices.
pub const DEFAULT_TRIGGER_AFTER: u64 = 604_800;

/// Default warning lead time: 24 h before the trigger fires, so the warning is
/// actionable rather than merely a formality.
pub const DEFAULT_WARN_AFTER: u64 = 86_400;

fn read_config(env: &Env) -> DeadManConfig {
    env.storage()
        .persistent()
        .get(&DataKey::DeadManConfig)
        .unwrap_or(DeadManConfig {
            trigger_after: 0,
            warn_after: 0,
            operators: Vec::new(env),
        })
}

fn write_config(env: &Env, config: &DeadManConfig) {
    env.storage()
        .persistent()
        .set(&DataKey::DeadManConfig, config);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::DeadManConfig, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Whether the dead-man switch is armed.
pub fn is_enabled(env: &Env) -> bool {
    read_config(env).trigger_after > 0
}

/// Returns the current dead-man configuration.
pub fn get_config(env: &Env) -> DeadManConfig {
    read_config(env)
}

/// Configures the dead-man switch. Admin-only.
///
/// # Arguments
///
/// * `trigger_after` - seconds of missed heartbeat after which the contract
///   enters the degraded state. `0` disables the switch entirely.
/// * `warn_after` - seconds of missed heartbeat after which a warning is
///   emitted. Must be strictly less than `trigger_after` so a warning always
///   precedes the trigger.
/// * `operators` - the addresses permitted to send heartbeats. Each one must
///   authorize its own heartbeat call, so membership alone is not enough.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] - caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] - `warn_after >= trigger_after` while
///   armed, or the operator set is empty while armed.
pub fn configure(env: &Env, trigger_after: u64, warn_after: u64, operators: Vec<Address>) {
    let admin = get_admin(env);
    admin.require_auth();

    if trigger_after > 0 {
        // A warning that fires at or after the trigger would never be seen
        // before the trip, defeating the purpose of the warning.
        if warn_after >= trigger_after {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
        if operators.is_empty() {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
    }

    write_config(
        env,
        &DeadManConfig {
            trigger_after,
            warn_after,
            operators,
        },
    );
}

/// Records a liveness heartbeat.
///
/// `operator` must authorize the call **and** be in the registered operator
/// set. Requiring both is what makes the heartbeat unspoofable: an attacker
/// cannot keep the contract alive by submitting on someone else's behalf, and
/// cannot trip it by forging a heartbeat either.
///
/// A heartbeat that arrives while degraded does **not** clear the state — only
/// the recovery path does. Otherwise the switch would be trivially reversible by
/// whoever still holds an operator key, which is the very case it exists to
/// catch.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] - `operator` did not authorize, or is not a
///   registered operator.
/// * [`ErrorCode::DeadManDisabled`] - the switch is not configured.
pub fn heartbeat(env: &Env, operator: Address) {
    let config = read_config(env);
    if config.trigger_after == 0 {
        panic_with_error!(env, ErrorCode::DeadManDisabled);
    }

    operator.require_auth();
    if !config.operators.contains(&operator) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    env.storage()
        .persistent()
        .set(&DataKey::DeadManLastHeartbeat, &env.ledger().timestamp());
    env.storage().persistent().extend_ttl(
        &DataKey::DeadManLastHeartbeat,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

/// Evaluates the heartbeat deadline and, if it has passed, enters the degraded
/// state. Permissionless: anyone may call it, and it is idempotent.
///
/// # Returns
///
/// The state after evaluation.
pub fn evaluate(env: &Env) -> DeadManState {
    if !is_enabled(env) {
        return DeadManState::Operational;
    }
    // Already degraded: nothing to re-evaluate, and re-tripping must not
    // re-emit the trigger event on every call.
    if is_degraded(env) {
        return DeadManState::Degraded;
    }

    let config = read_config(env);
    let now = env.ledger().timestamp();
    let last = last_heartbeat(env);
    let elapsed = now.saturating_sub(last);

    // Warning first: emitted once, so a well-monitored deployment does not
    // produce an event every ledger for the whole warning window.
    if config.warn_after > 0 && elapsed >= config.warn_after && !warning_emitted(env) {
        env.storage()
            .persistent()
            .set(&DataKey::DeadManWarned, &true);
        DeadManWarningEvent {
            elapsed_secs: elapsed,
            trigger_after: config.trigger_after,
            last_heartbeat: last,
        }
        .publish(env);
    }

    if elapsed >= config.trigger_after {
        enter_degraded(env, elapsed, last);
        return DeadManState::Degraded;
    }

    DeadManState::Operational
}

/// Enters the degraded state. Internal: called by [`evaluate`] once the
/// deadline has actually passed, so the trip cannot be forced by a caller.
fn enter_degraded(env: &Env, elapsed: u64, last: u64) {
    env.storage()
        .persistent()
        .set(&DataKey::DeadManDegraded, &true);
    env.storage()
        .persistent()
        .set(&DataKey::DeadManTriggeredAt, &env.ledger().timestamp());
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::DeadManDegraded, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Serving stops too: submissions are already gated on the pause flag, and
    // the read path is gated on `DeadManDegraded` so a stored value is never
    // handed out while nobody is attesting to its freshness.
    env.storage()
        .persistent()
        .set(&DataKey::CfgPauseFlag, &true);

    let config = read_config(env);
    DeadManTriggeredEvent {
        elapsed_secs: elapsed,
        trigger_after: config.trigger_after,
        last_heartbeat: last,
    }
    .publish(env);
}

/// Clears the degraded state and returns the contract to service.
///
/// Reachable with **independent authority**: any registered recovery guardian
/// may call this, so recovery survives the loss of the admin key. The admin may
/// also call it while it still holds the key, but nothing here depends on that.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] - caller is neither the admin nor a
///   registered recovery guardian.
/// * [`ErrorCode::NotDegraded`] - the contract is not in the degraded state.
pub fn recover(env: &Env, guardian: Address) {
    guardian.require_auth();

    if !is_degraded(env) {
        panic_with_error!(env, ErrorCode::NotDegraded);
    }

    // The guardian path is the one that must work when the admin key is lost,
    // so authorization is: guardian in the recovery set, OR the admin itself.
    let admin = get_admin(env);
    let guardians = crate::recovery::get_guardians(env);
    if guardian != admin && !guardians.contains(&guardian) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    env.storage().persistent().remove(&DataKey::DeadManDegraded);
    env.storage()
        .persistent()
        .remove(&DataKey::DeadManTriggeredAt);
    // Clear the warning latch so a future lapse warns again.
    env.storage().persistent().remove(&DataKey::DeadManWarned);
    // Resume serving. The pause flag is only ever set by this module when it
    // trips, so clearing it here restores the pre-trip service level.
    env.storage()
        .persistent()
        .set(&DataKey::CfgPauseFlag, &false);

    DeadManRecoveredEvent { guardian }.publish(env);
}

/// Whether the contract is currently in the degraded state.
pub fn is_degraded(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::DeadManDegraded)
        .unwrap_or(false)
}

/// Returns the degraded/operational state.
pub fn get_state(env: &Env) -> DeadManState {
    if is_degraded(env) {
        DeadManState::Degraded
    } else {
        DeadManState::Operational
    }
}

/// The timestamp of the last accepted heartbeat, or `0` when none was ever
/// sent. A never-configured or never-beaten switch reports `0`, so arming the
/// switch is a deliberate act that should be accompanied by a heartbeat.
pub fn last_heartbeat(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::DeadManLastHeartbeat)
        .unwrap_or(0)
}

/// Whether the pre-trigger warning has already been emitted for this lapse.
pub fn warning_emitted(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::DeadManWarned)
        .unwrap_or(false)
}

/// Panics if the contract is in the degraded state. Called from the submission
/// path so the degraded state is actually fail-safe rather than merely
/// recorded.
pub fn check_not_degraded(env: &Env) {
    if is_degraded(env) {
        panic_with_error!(env, ErrorCode::OracleDegraded);
    }
}
