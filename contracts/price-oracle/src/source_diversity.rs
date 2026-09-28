//! #399 — Source diversity: effective independence of the active source set.
//!
//! Hardened revision: diversity counts are easy to fake, so this module measures
//! *independence*, not jurisdiction labels.
//!
//! ## Effective independent source count (definition)
//!
//! Two sources are **independent** iff they differ on ALL three failure axes:
//! `infra` (hosting/infrastructure), `upstream` (upstream data origin), and
//! `owner` (operating entity / funding). Sharing ANY one axis means one cloud
//! outage, one upstream feed compromise, or one operator failure can take both
//! down at once — they form ONE failure domain, not two.
//!
//! ```text
//! effective_independent_count = |{ (infra, upstream, owner) : active source }|
//! ```
//!
//! Always `<= raw_count`. Diverges exactly in the Sybil / nominal-diversity
//! trap: ten legal names on one cloud with one upstream and one owner score
//! `raw = 10, effective = 1`.
//!
//! ## Assumptions (stated explicitly per acceptance criteria)
//!
//! 1. Metadata is honestly reported. The metric trusts `set_source_geo`
//!    admin-attested values; it cannot detect lies (see `docs/source-diversity.md`).
//! 2. Active set = registered sources NOT flagged `SrcInactive` (pure read via
//!    `storage::is_source_inactive`, no heartbeat side effects so
//!    `get_source_diversity` stays read-only).
//! 3. Missing geo (never set, or pre-#399 triple without re-registration)
//!    counts as `"unknown"` on every axis — conservative: unknowns cluster
//!    into one domain and LOWER the effective count rather than inflating it.
//! 4. HHI per axis is the standard Herfindahl index scaled 0–10000.
//! 5. Thresholds default to `min_effective_sources = 3`, `max_hhi_per_axis = 5000`.
//!
//! ## What this does NOT prove
//!
//! See `docs/source-diversity.md` § "What this metric cannot detect". In short:
//! covert shared control, off-chain collusion, copy-trading / shared code bugs,
//! network-level correlation, stale/compromised-but-diverse feeds, and any lie
//! in the underlying metadata.

use soroban_sdk::{Address, Env, Map, String, Vec};

use crate::events::{DivThreshBreachedEvent, DivThreshChangedEvent, SourceDiversityUpdatedEvent};
use crate::sources::get_source_geo;
use crate::storage::{get_admin, is_source_inactive as check_inactive, read_oracle_sources};
use crate::types::{DataKey, DiversityThresholds, ErrorCode, SourceDiversityReport};
use soroban_sdk::panic_with_error;

pub const DEFAULT_MIN_EFFECTIVE_SOURCES: u32 = 3;
pub const DEFAULT_MAX_HHI_PER_AXIS: u32 = 5000;
pub const MAX_HHI: u32 = 10000;

fn default_thresholds() -> DiversityThresholds {
    DiversityThresholds {
        min_effective_sources: DEFAULT_MIN_EFFECTIVE_SOURCES,
        max_hhi_per_axis: DEFAULT_MAX_HHI_PER_AXIS,
    }
}

pub fn get_diversity_thresholds(env: &Env) -> DiversityThresholds {
    env.storage()
        .persistent()
        .get(&DataKey::DiversityThresholds)
        .unwrap_or_else(default_thresholds)
}

pub fn set_diversity_thresholds(env: &Env, min_effective_sources: u32, max_hhi_per_axis: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if min_effective_sources == 0 || max_hhi_per_axis == 0 || max_hhi_per_axis > MAX_HHI {
        panic_with_error!(env, ErrorCode::InvalidDiversityThresholds);
    }
    let cfg = DiversityThresholds {
        min_effective_sources,
        max_hhi_per_axis,
    };
    env.storage()
        .persistent()
        .set(&DataKey::DiversityThresholds, &cfg);
    DivThreshChangedEvent {
        admin,
        min_effective_sources,
        max_hhi_per_axis,
    }
    .publish(env);
}

/// Admin convenience: update only the three #399 independence axes for a
/// source, preserving the #208 (`region`, `provider`, `jurisdiction`) tags.
/// Useful for migrating pre-#399 sources without re-entering geo data.
pub fn set_source_diversity(
    env: &Env,
    source: Address,
    infra: String,
    upstream: String,
    owner: String,
) {
    let admin = get_admin(env);
    admin.require_auth();
    if !env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    let unknown = String::from_str(env, "unknown");
    let (region, provider, jurisdiction) = match get_source_geo(env, source.clone()) {
        Some(g) => (g.region, g.provider, g.jurisdiction),
        None => (unknown.clone(), unknown.clone(), unknown.clone()),
    };
    let meta = crate::types::SourceGeoMetadata {
        region,
        provider,
        jurisdiction,
        infra: infra.clone(),
        upstream: upstream.clone(),
        owner: owner.clone(),
    };
    let key = DataKey::SourceGeo(source.clone());
    env.storage().persistent().set(&key, &meta);
    env.storage().persistent().extend_ttl(
        &key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );
    SourceDiversityUpdatedEvent {
        source,
        infra,
        upstream,
        owner,
    }
    .publish(env);
}

fn hhi(counts: &Map<String, u32>, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    let mut sum: u64 = 0;
    let keys = counts.keys();
    for i in 0..keys.len() {
        let k = keys.get_unchecked(i);
        let c = counts.get(k).unwrap_or(0) as u64;
        sum = sum.saturating_add(c.saturating_mul(c));
    }
    (sum.saturating_mul(MAX_HHI as u64) / ((total as u64).saturating_mul(total as u64))) as u32
}

fn bump(counts: &mut Map<String, u32>, label: String) {
    let c = counts.get(label.clone()).unwrap_or(0);
    counts.set(label, c + 1);
}

/// Compute the full diversity report over the active source set (read-only).
pub fn get_source_diversity(env: &Env) -> SourceDiversityReport {
    let registry = read_oracle_sources(env);
    let unknown = String::from_str(env, "unknown");

    // Collect active-source metadata triples.
    let mut region_counts: Map<String, u32> = Map::new(env);
    let mut provider_counts: Map<String, u32> = Map::new(env);
    let mut jurisdiction_counts: Map<String, u32> = Map::new(env);
    let mut infra_counts: Map<String, u32> = Map::new(env);
    let mut upstream_counts: Map<String, u32> = Map::new(env);
    let mut owner_counts: Map<String, u32> = Map::new(env);

    // Distinct failure domains via parallel vecs (avoids String concat).
    let mut d_infra: Vec<String> = Vec::new(env);
    let mut d_upstream: Vec<String> = Vec::new(env);
    let mut d_owner: Vec<String> = Vec::new(env);
    let mut d_sizes: Vec<u32> = Vec::new(env);

    let mut raw_count: u32 = 0;

    for i in 0..registry.sources.len() {
        let src = registry.sources.get_unchecked(i);
        if check_inactive(env, &src) {
            continue;
        }
        raw_count += 1;
        let (region, provider, jurisdiction, infra, upstream, owner) =
            match get_source_geo(env, src) {
                Some(g) => (
                    g.region,
                    g.provider,
                    g.jurisdiction,
                    g.infra,
                    g.upstream,
                    g.owner,
                ),
                None => (
                    unknown.clone(),
                    unknown.clone(),
                    unknown.clone(),
                    unknown.clone(),
                    unknown.clone(),
                    unknown.clone(),
                ),
            };
        bump(&mut region_counts, region);
        bump(&mut provider_counts, provider);
        bump(&mut jurisdiction_counts, jurisdiction);
        bump(&mut infra_counts, infra.clone());
        bump(&mut upstream_counts, upstream.clone());
        bump(&mut owner_counts, owner.clone());

        // Merge into failure-domain groups: share ANY axis value alone is not
        // enough — a domain is the full triple, and two sources share a domain
        // iff all three match (i.e. a single infra AND upstream AND owner
        // failure fells both). Distinct-triple count == effective count.
        let mut found: Option<u32> = None;
        for j in 0..d_infra.len() {
            if d_infra.get_unchecked(j) == infra
                && d_upstream.get_unchecked(j) == upstream
                && d_owner.get_unchecked(j) == owner
            {
                found = Some(j);
                break;
            }
        }
        match found {
            Some(j) => {
                let c = d_sizes.get_unchecked(j);
                d_sizes.set(j, c + 1);
            }
            None => {
                d_infra.push_back(infra);
                d_upstream.push_back(upstream);
                d_owner.push_back(owner);
                d_sizes.push_back(1);
            }
        }
    }

    if raw_count == 0 {
        let thresholds = get_diversity_thresholds(env);
        return SourceDiversityReport {
            raw_count: 0,
            effective_independent_count: 0,
            region_hhi: 0,
            provider_hhi: 0,
            jurisdiction_hhi: 0,
            infra_hhi: 0,
            upstream_hhi: 0,
            owner_hhi: 0,
            overall_score: 0,
            largest_domain_size: 0,
            is_low_diversity: thresholds.min_effective_sources > 0,
        };
    }

    let region_hhi = hhi(&region_counts, raw_count);
    let provider_hhi = hhi(&provider_counts, raw_count);
    let jurisdiction_hhi = hhi(&jurisdiction_counts, raw_count);
    let infra_hhi = hhi(&infra_counts, raw_count);
    let upstream_hhi = hhi(&upstream_counts, raw_count);
    let owner_hhi = hhi(&owner_counts, raw_count);

    let effective_independent_count = d_infra.len();
    let mut largest_domain_size: u32 = 0;
    for j in 0..d_sizes.len() {
        let c = d_sizes.get_unchecked(j);
        if c > largest_domain_size {
            largest_domain_size = c;
        }
    }

    let avg = (region_hhi as u64
        + provider_hhi as u64
        + jurisdiction_hhi as u64
        + infra_hhi as u64
        + upstream_hhi as u64
        + owner_hhi as u64)
        / 6;
    let overall_score = MAX_HHI.saturating_sub(avg as u32);

    let thresholds = get_diversity_thresholds(env);
    let max_hhi = region_hhi
        .max(provider_hhi)
        .max(jurisdiction_hhi)
        .max(infra_hhi)
        .max(upstream_hhi)
        .max(owner_hhi);
    let is_low_diversity = effective_independent_count < thresholds.min_effective_sources
        || max_hhi > thresholds.max_hhi_per_axis;

    SourceDiversityReport {
        raw_count,
        effective_independent_count,
        region_hhi,
        provider_hhi,
        jurisdiction_hhi,
        infra_hhi,
        upstream_hhi,
        owner_hhi,
        overall_score,
        largest_domain_size,
        is_low_diversity,
    }
}

/// Evaluate thresholds against the live set; emit a breach event and return
/// `true` when effective diversity is low EVEN IF the raw count looks healthy.
/// Read-compute + single write (breach ledger marker); never removes sources
/// (automated removal is out of scope — see #402).
pub fn check_diversity_alert(env: &Env) -> bool {
    let report = get_source_diversity(env);
    if !report.is_low_diversity {
        return false;
    }
    let thresholds = get_diversity_thresholds(env);
    let max_hhi = report
        .region_hhi
        .max(report.provider_hhi)
        .max(report.jurisdiction_hhi)
        .max(report.infra_hhi)
        .max(report.upstream_hhi)
        .max(report.owner_hhi);
    DivThreshBreachedEvent {
        raw_count: report.raw_count,
        effective_independent_count: report.effective_independent_count,
        largest_domain_size: report.largest_domain_size,
        max_hhi,
        min_effective_required: thresholds.min_effective_sources,
    }
    .publish(env);
    env.storage().persistent().set(
        &DataKey::DiversityLastBreachLedger,
        &env.ledger().sequence(),
    );
    true
}

/// Last ledger at which a diversity breach was recorded (`None` if never).
pub fn get_last_diversity_breach_ledger(env: &Env) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::DiversityLastBreachLedger)
}
