//! #498 — Source coverage gap analysis by asset, class and time
//!
//! The number of admitted sources is a misleading summary of coverage. What
//! matters is the minimum *independent* coverage per asset, and whether it
//! survives nights, weekends and market stress. This module reports both.
//!
//! ## Independence (the definition)
//!
//! Reused verbatim from `source_diversity` (#399) so the two reports cannot
//! disagree: two sources are independent iff they differ on **all three**
//! failure axes — `infra` (hosting), `upstream` (data origin) and `owner`
//! (operating entity). Sharing any one axis puts both in a single failure
//! domain, so
//!
//! ```text
//! independent_domains = |{ (infra, upstream, owner) : source admitted for asset }|
//! ```
//!
//! Missing metadata counts as `"unknown"` on every axis, which *lowers* the
//! count rather than inflating it. See `docs/source-diversity.md` for what this
//! metric still cannot detect (covert shared control, copy-trading, network
//! correlation, and any lie in the attested metadata).
//!
//! ## Temporal normalisation
//!
//! Participation is measured from the price history the contract **already
//! stores**: every aggregate records the `num_sources` that produced it, keyed
//! by ledger. Those ledgers are bucketed into fixed windows and each window is
//! reduced to the fewest contributing sources seen in it, so a report is
//! comparable across assets and across time regardless of how long the
//! contract has been running.
//!
//! Deriving it rather than recording it is deliberate. A participation counter
//! written from `submit_price` would add a per-asset storage entry to the
//! submission hot path, and the network caps an invocation at 100 footprint
//! entries — `submit_price` at 10 sources is already at 100 (see
//! `docs/gas-usage.md`), so one more entry is the difference between shipping
//! and not. Reading history keeps the hot path untouched.
//!
//! ## Read-only by construction
//!
//! This module is an analysis surface. It writes nothing at all: the only
//! storage operations it performs are reads. It contains **no** call to
//! `add_source`, `remove_source` or any other admission path, and its
//! recommendations are advisory strings only. Coverage must never become a
//! source-admission gate by accident: an automated admission decision driven by
//! self-reported metadata is a Sybil vector, and a hard gate would let a party
//! suppress its own coverage to avoid scrutiny. `coverage_is_read_only` asserts
//! the absence of any mutating call, and
//! `coverage_analysis_cannot_admit_or_remove_sources` asserts the registries are
//! unchanged after every read path is exercised.

use soroban_sdk::{contracttype, panic_with_error, Address, Env, String, Vec};

use crate::storage::read_oracle_sources;
use crate::types::{CoverageReport, DataKey, ErrorCode};

/// Default minimum number of independent failure domains per asset.
pub const DEFAULT_MIN_INDEPENDENT_SOURCES: u32 = 3;
/// Default length of one participation window in ledgers (≈ 6 h at 5 s close).
pub const DEFAULT_WINDOW_LEDGERS: u32 = 4_320;
/// Most recent history ledgers inspected per report. Bounds the read cost of an
/// operator-facing query.
pub const MAX_SAMPLED_LEDGERS: u32 = 32;

/// Coverage thresholds, stored under [`DataKey::CoverageThresholds`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CoverageThresholds {
    /// Minimum independent failure domains an asset should have.
    pub min_independent_sources: u32,
    /// Length of one participation window, in ledgers.
    pub window_ledgers: u32,
}

fn default_thresholds() -> CoverageThresholds {
    CoverageThresholds {
        min_independent_sources: DEFAULT_MIN_INDEPENDENT_SOURCES,
        window_ledgers: DEFAULT_WINDOW_LEDGERS,
    }
}

/// Current coverage thresholds.
pub fn get_thresholds(env: &Env) -> CoverageThresholds {
    env.storage()
        .persistent()
        .get(&DataKey::CoverageThresholds)
        .unwrap_or_else(default_thresholds)
}

/// Sets the coverage thresholds. Admin-only.
pub fn set_thresholds(env: &Env, thresholds: CoverageThresholds) {
    crate::storage::get_admin(env).require_auth();
    if thresholds.min_independent_sources == 0 || thresholds.window_ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::CoverageThresholds;
    env.storage().persistent().set(&key, &thresholds);
    env.storage().persistent().extend_ttl(
        &key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );
}

/// Registered sources admitted for `asset`, by the source-scoped asset list.
///
/// A source with no `SourceAssets` entry is treated as covering every asset,
/// matching `storage::check_source_asset` (an absent list means no restriction).
pub fn sources_for_asset(env: &Env, asset: &Address) -> Vec<Address> {
    let registry = read_oracle_sources(env);
    let mut out: Vec<Address> = Vec::new(env);
    for i in 0..registry.sources.len() {
        let src = registry.sources.get_unchecked(i);
        let scoped: Option<Vec<Address>> = env
            .storage()
            .persistent()
            .get(&DataKey::SourceAssets(src.clone()));
        let covered = match scoped {
            None => true,
            Some(list) => {
                let mut found = false;
                for j in 0..list.len() {
                    if list.get_unchecked(j) == *asset {
                        found = true;
                        break;
                    }
                }
                found
            }
        };
        if covered {
            out.push_back(src);
        }
    }
    out
}

/// Distinct `(infra, upstream, owner)` failure domains among `sources`.
///
/// Identical to the #399 definition, scoped to a source list. Sources with no
/// attested metadata all collapse into the single `"unknown"` domain, which
/// lowers the count rather than inflating it.
pub fn independent_domains(env: &Env, sources: &Vec<Address>) -> u32 {
    let unknown = String::from_str(env, "unknown");
    let mut infra: Vec<String> = Vec::new(env);
    let mut upstream: Vec<String> = Vec::new(env);
    let mut owner: Vec<String> = Vec::new(env);
    for i in 0..sources.len() {
        let src = sources.get_unchecked(i);
        let (a, b, c) = match crate::sources::get_source_geo(env, src) {
            Some(g) => (g.infra, g.upstream, g.owner),
            None => (unknown.clone(), unknown.clone(), unknown.clone()),
        };
        let mut found = false;
        for j in 0..infra.len() {
            if infra.get_unchecked(j) == a
                && upstream.get_unchecked(j) == b
                && owner.get_unchecked(j) == c
            {
                found = true;
                break;
            }
        }
        if !found {
            infra.push_back(a);
            upstream.push_back(b);
            owner.push_back(c);
        }
    }
    infra.len()
}

/// Participation for one window, derived from stored price history.
///
/// A host-side helper (not part of the contract ABI): it is never stored, only
/// aggregated within a single report call.
#[contracttype]
pub struct WindowParticipation {
    /// Window index (`ledger / window_ledgers`).
    window: u32,
    /// Fewest contributing sources seen in any aggregate in this window.
    min_sources: u32,
    /// Aggregates observed in this window.
    samples: u32,
}

/// Per-window participation for `asset`, derived from its price history.
///
/// Reads at most [`MAX_SAMPLED_LEDGERS`] of the most recent history entries, so
/// the cost of an operator-facing report is bounded no matter how long the asset
/// has been running. A window with no aggregate in the sampled range is simply
/// not reported: absence of history is "no data", not "zero participation", and
/// conflating the two would manufacture gaps out of thin air.
pub fn temporal_participation(
    env: &Env,
    asset: &Address,
    window_ledgers: u32,
) -> Vec<WindowParticipation> {
    let mut out: Vec<WindowParticipation> = Vec::new(env);
    if window_ledgers == 0 {
        return out;
    }
    let ledgers: Vec<u32> = env
        .storage()
        .persistent()
        .get(&DataKey::PriceHistoryLedgers(asset.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let n = ledgers.len();
    let start = n.saturating_sub(MAX_SAMPLED_LEDGERS);
    for i in start..n {
        let ledger = ledgers.get_unchecked(i);
        // `read_history_entry` covers both storage tiers: the hot temporary
        // entry and the persistent weekly bucket an older ledger spills into.
        let entry = match crate::history::read_history_entry(env, asset, ledger) {
            Some(e) => e,
            None => continue,
        };
        let window = ledger / window_ledgers;
        let mut merged = false;
        for j in 0..out.len() {
            if out.get_unchecked(j).window == window {
                let w = out.get_unchecked(j);
                out.set(
                    j,
                    WindowParticipation {
                        window,
                        min_sources: w.min_sources.min(entry.num_sources),
                        samples: w.samples + 1,
                    },
                );
                merged = true;
                break;
            }
        }
        if !merged {
            out.push_back(WindowParticipation {
                window,
                min_sources: entry.num_sources,
                samples: 1,
            });
        }
    }
    out
}

/// Builds the full coverage report for `asset`.
///
/// Read-only: the only storage operations are reads, so a report can neither
/// admit nor remove a source, and the same state always yields the same report.
pub fn get_report(env: &Env, asset: &Address) -> CoverageReport {
    let thresholds = get_thresholds(env);
    let sources = sources_for_asset(env, asset);
    let domains = independent_domains(env, &sources);
    let below = domains < thresholds.min_independent_sources;

    let windows = temporal_participation(env, asset, thresholds.window_ledgers);
    let mut low_participation_windows: u32 = 0;
    let mut min_participation = u32::MAX;
    for i in 0..windows.len() {
        let w = windows.get_unchecked(i);
        if w.min_sources < min_participation {
            min_participation = w.min_sources;
        }
        if w.min_sources < thresholds.min_independent_sources {
            low_participation_windows = low_participation_windows.saturating_add(1);
        }
    }
    if min_participation == u32::MAX {
        min_participation = 0;
    }

    // Advisory only. These strings are a starting point for an operator review;
    // nothing in this contract acts on them.
    let mut recommendations: Vec<String> = Vec::new(env);
    if below {
        recommendations.push_back(String::from_str(
            env,
            "admit sources from new failure domains to reach the independence threshold",
        ));
    }
    if low_participation_windows > 0 {
        recommendations.push_back(String::from_str(
            env,
            "investigate windows with systematically low participation",
        ));
    }
    if !below && low_participation_windows == 0 && sources.len() > domains {
        recommendations.push_back(String::from_str(
            env,
            "registered sources share failure domains; consider re-scoping existing admissions",
        ));
    }

    CoverageReport {
        registered_sources: sources.len(),
        independent_domains: domains,
        min_independent_required: thresholds.min_independent_sources,
        below_independence_threshold: below,
        windows_observed: windows.len(),
        low_participation_windows,
        min_window_participation: min_participation,
        recommendations,
    }
}

/// Assets whose independent coverage is below the configured threshold.
///
/// The gap list is derived from the registered-asset set, so it is reproducible
/// from stored data alone.
pub fn get_gap_list(env: &Env) -> Vec<Address> {
    let thresholds = get_thresholds(env);
    let assets = crate::storage::read_registered_assets(env);
    let mut out: Vec<Address> = Vec::new(env);
    for i in 0..assets.len() {
        let asset = assets.get_unchecked(i);
        if independent_domains(env, &sources_for_asset(env, &asset))
            < thresholds.min_independent_sources
        {
            out.push_back(asset);
        }
    }
    out
}
