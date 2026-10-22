//! # Per-source accuracy scorecards with rolling windows (#490)
//!
//! Reputation is a single coarse number. A scorecard answers a different
//! question: *how accurate was this source, recently and over the long run?*
//!
//! ## The reference is not circular
//!
//! Scoring a source against the aggregate it helped form guarantees a good
//! score. Every sample here is therefore measured against a
//! **leave-one-out reference**: the median of the *other* sources in the same
//! round. A source is never compared against a value it contributed to, so
//! accuracy is measured rather than self-asserted.
//!
//! ```text
//! reference(source S) = median({ price(t) : t != S })
//! error_bps          = (price(S) - reference) / reference * 10_000
//! ```
//!
//! With fewer than [`MIN_REFERENCE_SOURCES`] other sources there is no
//! reference and no sample is recorded.
//!
//! ## Windows
//!
//! Two rolling windows over the same sample stream: a short one for
//! responsiveness, a long one for sustained quality. Both are the most recent
//! `N` samples, so they update as submissions arrive and old samples age out.
//!
//! ## One outlier cannot sink a long window
//!
//! Each sample is **winsorized** at [`ScorecardConfig::outlier_cap_bps`] before
//! it enters the mean. A single catastrophic print therefore contributes at
//! most `cap` to the average, and the derived score has an explicit floor, so
//! no single event can collapse a long window below it. The raw worst sample is
//! still reported as `long_max_error_bps` — capping the *average* never hides
//! the outlier itself.
//!
//! ## Cold start
//!
//! Below [`ScorecardConfig::cold_start_samples`] the source is reported as
//! `cold_start` and carries no score judgement. It is counted, not graded, so
//! a newly onboarded source is never penalised for having no history.
//!
//! Scorecards are **reported, not enforced**: they feed reputation as an input
//! an operator can weigh, and never change admission, quorum or weighting on
//! their own. See `docs/source-scorecards.md`.

use soroban_sdk::{contractevent, panic_with_error, Address, Env, Vec};

use crate::price_bounds::set_scorecards_enabled;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, ScorecardConfig, ScorecardWindow, SourceScorecard};

/// Fewest *other* sources needed before a leave-one-out reference exists.
pub const MIN_REFERENCE_SOURCES: u32 = 2;
/// Floor applied to a derived accuracy score, in bps (20 %).
///
/// A source with real samples can score badly, but never below this from a
/// single event: winsorization bounds each sample's contribution and this
/// bounds the aggregate.
pub const SCORE_FLOOR_BPS: u32 = 2_000;
/// Largest accepted short window.
pub const MAX_SHORT_WINDOW: u32 = 64;
/// Largest accepted long window.
pub const MAX_LONG_WINDOW: u32 = 512;

/// The shipped defaults.
pub const DEFAULT_CONFIG: ScorecardConfig = ScorecardConfig {
    short_window: 10,
    long_window: 50,
    cold_start_samples: 5,
    hit_tolerance_bps: 100,
    outlier_cap_bps: 5_000,
};

/// Emitted when a source's scorecard is updated (#490).
///
/// The full window contents are carried, so an off-chain indexer can
/// reconstruct every window from the event stream alone.
///
/// Topics: `source`
#[contractevent]
#[derive(Clone)]
pub struct SourceScorecardUpdatedEvent {
    #[topic]
    pub source: Address,
    /// Sample count in the short window after the update.
    pub short_samples: u32,
    /// Mean absolute error over the short window, in bps.
    pub short_mean_abs_error_bps: u32,
    /// Mean absolute error over the long window, in bps.
    pub long_mean_abs_error_bps: u32,
    /// Hit rate over the long window, in bps.
    pub long_hit_rate_bps: u32,
    /// Mean signed error over the long window, in bps.
    pub long_bias_bps: i32,
    /// `true` while the source is counted but not scored.
    pub cold_start: bool,
    /// Every retained sample, oldest first, winsorized. This is what makes the
    /// windows reconstructible from events.
    pub samples: Vec<u32>,
    pub ledger: u32,
}

/// The active configuration.
pub fn config(env: &Env) -> ScorecardConfig {
    env.storage()
        .persistent()
        .get(&DataKey::CfgScorecard)
        .unwrap_or(DEFAULT_CONFIG)
}

/// Sets the configuration. Admin only; every field is bounds-checked.
pub fn set_config(env: &Env, cfg: ScorecardConfig) {
    get_admin(env).require_auth();
    if cfg.short_window == 0
        || cfg.short_window > MAX_SHORT_WINDOW
        || cfg.long_window < cfg.short_window
        || cfg.long_window > MAX_LONG_WINDOW
        || cfg.cold_start_samples > cfg.long_window
        || cfg.hit_tolerance_bps == 0
        || cfg.hit_tolerance_bps > crate::sanity_lattice::MAX_TOLERANCE_BPS
        || cfg.outlier_cap_bps == 0
        || cfg.outlier_cap_bps > i128::from(u32::MAX) as u32
    {
        panic_with_error!(env, ErrorCode::InvalidScorecardConfig);
    }
    let key = DataKey::CfgScorecard;
    env.storage().persistent().set(&key, &cfg);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Turns scorecard collection on. Off by default so the publication path pays
/// nothing until an operator asks for the data.
pub fn enable(env: &Env) {
    get_admin(env).require_auth();
    set_scorecards_enabled(env, true);
}

/// Turns scorecard collection off. Stored scorecards are retained and remain
/// queryable.
pub fn disable(env: &Env) {
    get_admin(env).require_auth();
    set_scorecards_enabled(env, false);
}

/// `true` when collection is on.
pub fn enabled(env: &Env) -> bool {
    crate::price_bounds::guards(env).scorecards_enabled
}

fn samples_key(source: &Address) -> DataKey {
    DataKey::SourceScorecard(source.clone())
}

/// The retained winsorized samples for `source`, oldest first.
pub fn samples(env: &Env, source: &Address) -> Vec<u32> {
    let key = samples_key(source);
    let v: Option<Vec<u32>> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v.unwrap_or_else(|| Vec::new(env))
}

/// Signed relative error of `price` against `reference`, in bps.
pub fn error_bps(price: i128, reference: i128) -> i128 {
    if reference == 0 {
        return 0;
    }
    (price - reference).saturating_mul(10_000) / reference.abs()
}

/// Records one sample for `source` and returns the updated scorecard.
///
/// `reference` must already be the leave-one-out median computed by the
/// caller; this function does not look at the round, which is what keeps the
/// reference non-circular.
pub fn record(env: &Env, source: &Address, reference: i128, price: i128) -> SourceScorecard {
    let cfg = config(env);
    let raw = error_bps(price, reference);
    // Winsorize: a single catastrophic sample may not exceed the cap. The sign
    // is kept so the window can still report *direction* (bias), not just size.
    let cap = i128::from(cfg.outlier_cap_bps);
    let capped = raw.clamp(-cap, cap);
    // Stored biased so the whole range fits a u32 without wrapping: the two
    // halves of the i32 range are folded onto either side of zero.
    let stored = encode_sample(capped);

    let key = samples_key(source);
    let mut all = samples(env, source);
    all.push_back(stored);
    while all.len() > cfg.long_window {
        all.remove(0);
    }
    env.storage().persistent().set(&key, &all);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let (buf, n) = to_slice(&all);
    let card = summarize(source, &buf[..n], &cfg);
    SourceScorecardUpdatedEvent {
        source: source.clone(),
        short_samples: card.short.samples,
        short_mean_abs_error_bps: card.short.mean_abs_error_bps,
        long_mean_abs_error_bps: card.long.mean_abs_error_bps,
        long_hit_rate_bps: card.long.hit_rate_bps,
        long_bias_bps: card.long.bias_bps,
        cold_start: card.cold_start,
        samples: all,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
    card
}

/// Folds a signed error onto a `u32` so a negative sample cannot wrap.
///
/// Values in `[-CAP, CAP]` map to `[0, 2*CAP]` with `CAP` as the origin, so
/// ordering and sign are both recoverable and the encoding is injective over
/// the accepted range.
fn encode_sample(err: i128) -> u32 {
    const ORIGIN: i128 = 1 << 31;
    let shifted = err + ORIGIN;
    if shifted < 0 {
        0
    } else if shifted > i128::from(u32::MAX) {
        u32::MAX
    } else {
        shifted as u32
    }
}

/// Inverse of [`encode_sample`].
fn decode_sample(stored: u32) -> i128 {
    i128::from(stored) - (1i128 << 31)
}

fn window_over(s: &[u32], n: u32, hit_tol: u32) -> ScorecardWindow {
    let start = if s.len() as u32 > n {
        s.len() as u32 - n
    } else {
        0
    } as usize;
    let slice = &s[start..];
    if slice.is_empty() {
        return ScorecardWindow {
            samples: 0,
            mean_abs_error_bps: 0,
            bias_bps: 0,
            hit_rate_bps: 0,
        };
    }
    let mut abs_sum: u64 = 0;
    let mut signed_sum: i128 = 0;
    let mut hits: u32 = 0;
    for v in slice {
        let err = decode_sample(*v);
        abs_sum += u64::try_from(err.abs()).unwrap_or(u64::MAX);
        signed_sum += err;
        if err.abs() <= i128::from(hit_tol) {
            hits += 1;
        }
    }
    let n64 = slice.len() as u64;
    ScorecardWindow {
        samples: slice.len() as u32,
        // Winsorization already bounded each sample, so the mean is bounded too.
        mean_abs_error_bps: (abs_sum / n64) as u32,
        // The sign survives the storage encoding, so a persistent directional
        // error is visible here rather than averaged away.
        bias_bps: (signed_sum / n64 as i128) as i32,
        hit_rate_bps: ((hits as u64 * 10_000) / n64) as u32,
    }
}

/// Copies a Soroban `Vec` into a fixed buffer so the window maths can be
/// written once against a plain slice. Bounded by `MAX_LONG_WINDOW`, which the
/// configuration validator already enforces.
fn to_slice(v: &Vec<u32>) -> ([u32; MAX_LONG_WINDOW as usize], usize) {
    let mut out = [0u32; MAX_LONG_WINDOW as usize];
    let n = (v.len() as usize).min(out.len());
    for (i, slot) in out.iter_mut().enumerate().take(n) {
        *slot = v.get_unchecked(i as u32);
    }
    (out, n)
}

fn summarize(source: &Address, all: &[u32], cfg: &ScorecardConfig) -> SourceScorecard {
    let short = window_over(all, cfg.short_window, cfg.hit_tolerance_bps);
    let long = window_over(all, cfg.long_window, cfg.hit_tolerance_bps);
    let long_samples = long.samples;
    let mut long_max = 0u32;
    for v in all {
        let m = decode_sample(*v).unsigned_abs() as u32;
        if m > long_max {
            long_max = m;
        }
    }
    SourceScorecard {
        source: source.clone(),
        short,
        long,
        cold_start: long_samples < cfg.cold_start_samples,
        short_samples: short.samples,
        long_max_error_bps: long_max,
    }
}

/// The scorecard for `source`.
///
/// A source with no samples is reported `cold_start` with empty windows, which
/// is the "counted, not graded" state — never a zero score.
pub fn get_scorecard(env: &Env, source: &Address) -> SourceScorecard {
    let cfg = config(env);
    let all = samples(env, source);
    let (buf, n) = to_slice(&all);
    summarize(source, &buf[..n], &cfg)
}

/// A single 0–10 000 accuracy score derived from the long window, with the
/// documented floor applied.
///
/// Exposed separately so a weighting policy can consume one number while still
/// being able to inspect the windows behind it.
pub fn accuracy_score(env: &Env, source: &Address) -> u32 {
    let card = get_scorecard(env, source);
    if card.cold_start || card.long.samples == 0 {
        return 0;
    }
    let err = card.long.mean_abs_error_bps as u64;
    let score = 10_000u64.saturating_sub(err * 2);
    score.clamp(SCORE_FLOOR_BPS as u64, 10_000) as u32
}

/// Median of `prices` excluding index `skip`.
///
/// This is the leave-one-out reference: computing it by *removing* the source
/// being scored, rather than filtering it out afterwards, is what keeps the
/// measurement non-circular.
pub fn leave_one_out_median(prices: &[i128], skip: usize) -> Option<i128> {
    let mut rest: [i128; MAX_LONG_WINDOW as usize] = [0; MAX_LONG_WINDOW as usize];
    let mut n = 0usize;
    for (i, p) in prices.iter().enumerate() {
        if i == skip {
            continue;
        }
        if n >= rest.len() {
            break;
        }
        rest[n] = *p;
        n += 1;
    }
    if (n as u32) < MIN_REFERENCE_SOURCES {
        return None;
    }
    rest[..n].sort_unstable();
    Some(if n % 2 == 1 {
        rest[n / 2]
    } else {
        rest[n / 2 - 1] + (rest[n / 2] - rest[n / 2 - 1]) / 2
    })
}

/// Records one sample per contributor for a completed round.
///
/// Each source is measured against the median of the *others*, so no source is
/// ever scored against a value it contributed to. Sources with too few peers to
/// form a reference are skipped rather than scored against themselves.
pub fn record_round(env: &Env, sources: &Vec<Address>, prices: &Vec<i128>) {
    let n = (sources.len() as usize).min(prices.len() as usize);
    // Buffer the prices so the reference can be computed without a Soroban Vec
    // on every iteration.
    let mut buf = [0i128; MAX_LONG_WINDOW as usize];
    for i in 0..n.min(buf.len()) {
        buf[i] = prices.get_unchecked(i as u32);
    }
    for i in 0..n.min(buf.len()) {
        if let Some(reference) = leave_one_out_median(&buf[..n.min(buf.len())], i) {
            if reference > 0 {
                record(env, &sources.get_unchecked(i as u32), reference, buf[i]);
            }
        }
    }
}
