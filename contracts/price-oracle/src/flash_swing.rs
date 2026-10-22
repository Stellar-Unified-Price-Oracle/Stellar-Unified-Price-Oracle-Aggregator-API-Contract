//! # Single-ledger price swing / flash-liquidity detection (#449)
//!
//! A *swing* is a move of at least `threshold_bps` away from a pre-move level
//! that reverts to within `threshold_bps / 4` of that level inside
//! `reversal_window` ledgers of the pre-move observation. Such a move can be
//! produced by capital that exists only for a single ledger (flash loans,
//! atomic sandwiches) and is therefore not a sustainable fair value.
//!
//! Detection works on the sequence of aggregated per-ledger prices, so it
//! catches swings built out of several steps that each stay inside their own
//! per-source deviation bound. See `docs/flash-swing-detection.md` for the
//! policy, configuration bounds, and residual-exposure analysis.

use soroban_sdk::{symbol_short, Address, Env};

/// Minimum / maximum allowed reversal window, in ledgers.
pub const MIN_REVERSAL_WINDOW: u32 = 2;
pub const MAX_REVERSAL_WINDOW: u32 = 20;
/// Minimum / maximum allowed swing threshold, in basis points.
pub const MIN_THRESHOLD_BPS: u32 = 100;
pub const MAX_THRESHOLD_BPS: u32 = 5_000;
/// Confidence served alongside a price under [`SwingPolicy::ServeDegraded`].
pub const DEGRADED_CONFIDENCE_BPS: u32 = 2_500;

/// Detector configuration. Owned by the contract admin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwingConfig {
    pub threshold_bps: u32,
    pub reversal_window: u32,
}

impl SwingConfig {
    /// Returns `true` when both parameters are within documented bounds.
    pub fn is_valid(&self) -> bool {
        (MIN_THRESHOLD_BPS..=MAX_THRESHOLD_BPS).contains(&self.threshold_bps)
            && (MIN_REVERSAL_WINDOW..=MAX_REVERSAL_WINDOW).contains(&self.reversal_window)
    }
}

impl Default for SwingConfig {
    fn default() -> Self {
        Self {
            threshold_bps: 1_000,
            reversal_window: 3,
        }
    }
}

/// Evidence of a detected swing (indices into the observed series).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swing {
    pub magnitude_bps: u32,
    pub base_index: u32,
    pub peak_index: u32,
    pub reversal_index: u32,
}

/// What to do with a price when a swing is detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwingPolicy {
    Reject,
    /// Hand off to the multi-round confirmation path (#397).
    DeferForConfirmation,
    ServeDegraded,
}

/// Result of applying a [`SwingPolicy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwingOutcome {
    Accept,
    Reject,
    Defer,
    Degraded { confidence_bps: u32 },
}

fn deviation_bps(base: i128, price: i128) -> u32 {
    if base <= 0 {
        return 0;
    }
    let diff = (price - base).unsigned_abs();
    let bps = diff.saturating_mul(10_000) / base.unsigned_abs();
    bps.min(u32::MAX as u128) as u32
}

/// Scans `series` (oldest first, one aggregated price per ledger) for a swing.
/// Returns the largest-magnitude swing found, or `None`.
pub fn detect(series: &[i128], cfg: &SwingConfig) -> Option<Swing> {
    let n = series.len();
    let w = cfg.reversal_window as usize;
    let revert_bps = cfg.threshold_bps / 4;
    let mut best: Option<Swing> = None;
    for a in 0..n {
        let end = core::cmp::min(n - 1, a + w);
        for p in (a + 1)..=end {
            let mag = deviation_bps(series[a], series[p]);
            if mag < cfg.threshold_bps {
                continue;
            }
            for r in (p + 1)..=end {
                if deviation_bps(series[a], series[r]) <= revert_bps {
                    if best.is_none_or(|b| mag > b.magnitude_bps) {
                        best = Some(Swing {
                            magnitude_bps: mag,
                            base_index: a as u32,
                            peak_index: p as u32,
                            reversal_index: r as u32,
                        });
                    }
                    break;
                }
            }
        }
    }
    best
}

/// Maps a detection result to an outcome under `policy`.
pub fn apply_policy(swing: Option<Swing>, policy: SwingPolicy) -> SwingOutcome {
    match (swing, policy) {
        (None, _) => SwingOutcome::Accept,
        (Some(_), SwingPolicy::Reject) => SwingOutcome::Reject,
        (Some(_), SwingPolicy::DeferForConfirmation) => SwingOutcome::Defer,
        (Some(_), SwingPolicy::ServeDegraded) => SwingOutcome::Degraded {
            confidence_bps: DEGRADED_CONFIDENCE_BPS,
        },
    }
}

/// Emits `("flash_sw", asset)` with `(magnitude_bps, base, peak, reversal)`.
pub fn emit_swing_detected(env: &Env, asset: Address, swing: &Swing) {
    env.events().publish(
        (symbol_short!("flash_sw"), asset),
        (
            swing.magnitude_bps,
            swing.base_index,
            swing.peak_index,
            swing.reversal_index,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, testutils::Address as _, testutils::Events as _};

    #[contract]
    struct Host;

    const P: i128 = 1_000_000;

    #[test]
    fn detects_multi_step_swing_within_per_step_bounds() {
        // Each step is <= 5% (typical per-source bound) but the round trip is 15%.
        let s = [P, P * 105 / 100, P * 110 / 100, P * 115 / 100, P];
        let cfg = SwingConfig {
            threshold_bps: 1_000,
            reversal_window: 4,
        };
        let swing = detect(&s, &cfg).expect("swing");
        assert_eq!(swing.magnitude_bps, 1_500);
        assert_eq!(
            (swing.base_index, swing.peak_index, swing.reversal_index),
            (0, 3, 4)
        );
    }

    #[test]
    fn detects_downward_flash_dip() {
        let s = [P, P * 80 / 100, P];
        assert!(detect(&s, &SwingConfig::default()).is_some());
    }

    #[test]
    fn no_false_positive_on_sustained_trend() {
        let s = [
            P,
            P * 110 / 100,
            P * 120 / 100,
            P * 125 / 100,
            P * 130 / 100,
        ];
        assert_eq!(detect(&s, &SwingConfig::default()), None);
    }

    #[test]
    fn no_false_positive_on_ordinary_volatility() {
        // Synthetic benign series: +/- 3% chop around a slowly drifting mean.
        let mut s = [0i128; 64];
        for (i, v) in s.iter_mut().enumerate() {
            let drift = P + (i as i128) * P / 1_000;
            let chop = if i % 2 == 0 { 3 } else { -3 };
            *v = drift + drift * chop / 100;
        }
        assert_eq!(detect(&s, &SwingConfig::default()), None);
    }

    #[test]
    fn reversal_outside_window_is_not_flagged() {
        let s = [
            P,
            P * 120 / 100,
            P * 120 / 100,
            P * 120 / 100,
            P * 120 / 100,
            P,
        ];
        let cfg = SwingConfig {
            threshold_bps: 1_000,
            reversal_window: 3,
        };
        assert_eq!(detect(&s, &cfg), None);
    }

    #[test]
    fn policy_outcomes() {
        let swing = detect(&[P, 2 * P, P], &SwingConfig::default());
        assert_eq!(
            apply_policy(None, SwingPolicy::Reject),
            SwingOutcome::Accept
        );
        assert_eq!(
            apply_policy(swing, SwingPolicy::Reject),
            SwingOutcome::Reject
        );
        assert_eq!(
            apply_policy(swing, SwingPolicy::DeferForConfirmation),
            SwingOutcome::Defer
        );
        assert_eq!(
            apply_policy(swing, SwingPolicy::ServeDegraded),
            SwingOutcome::Degraded {
                confidence_bps: DEGRADED_CONFIDENCE_BPS
            }
        );
    }

    #[test]
    fn config_bounds() {
        assert!(SwingConfig::default().is_valid());
        let bad = [(99, 3), (5_001, 3), (1_000, 1), (1_000, 21)];
        for (t, w) in bad {
            let cfg = SwingConfig {
                threshold_bps: t,
                reversal_window: w,
            };
            assert!(!cfg.is_valid());
        }
    }

    #[test]
    fn emits_event_with_evidence() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let swing = detect(&[P, 2 * P, P], &SwingConfig::default()).unwrap();
        let id = env.register(Host, ());
        env.as_contract(&id, || emit_swing_detected(&env, asset, &swing));
        assert_eq!(env.events().all().events().len(), 1);
    }
}
