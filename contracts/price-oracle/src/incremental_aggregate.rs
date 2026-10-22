//! Incremental aggregate maintenance (#473).
//!
//! `IncrementalAggregate` keeps the contributing prices in a sorted `Vec` so the
//! median can be read in O(1) and each mutation costs one binary search plus one
//! O(n) shift, instead of re-selecting over the whole source set on every call.
//!
//! Safety rules:
//! - The structure is tagged with the source-set `epoch` it was built for. Any
//!   source admission, removal, or TTL eviction bumps the epoch; a structure with a
//!   stale epoch never answers (`median_at` returns `None`) and is rebuilt from the
//!   ground truth rather than patched in place.
//! - `median_or_rebuild` falls back to a full recompute whenever the epoch differs
//!   or the sortedness invariant cannot be confirmed.
//! - The median formula is identical to [`crate::storage::compute_median`].
//!
//! See `docs/incremental-aggregate.md`.

#![allow(dead_code)]

use soroban_sdk::{Env, Vec};

use crate::storage::sort_prices;

#[derive(Clone)]
pub struct IncrementalAggregate {
    sorted: Vec<i128>,
    epoch: u32,
}

impl IncrementalAggregate {
    /// Full rebuild from the ground-truth price set for `epoch`.
    pub fn rebuild(prices: &Vec<i128>, epoch: u32) -> Self {
        let mut sorted = prices.clone();
        sort_prices(&mut sorted);
        Self { sorted, epoch }
    }

    pub fn empty(env: &Env, epoch: u32) -> Self {
        Self {
            sorted: Vec::new(env),
            epoch,
        }
    }

    pub fn len(&self) -> u32 {
        self.sorted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sorted.is_empty()
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn sorted(&self) -> &Vec<i128> {
        &self.sorted
    }

    /// First index whose value is `>= price`.
    fn lower_bound(&self, price: i128) -> u32 {
        let (mut lo, mut hi) = (0u32, self.sorted.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.sorted.get_unchecked(mid) < price {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    pub fn insert(&mut self, price: i128) {
        let pos = self.lower_bound(price);
        self.sorted.insert(pos, price);
    }

    /// Removes one occurrence of `price`; returns `false` if it was not present.
    pub fn remove(&mut self, price: i128) -> bool {
        let pos = self.lower_bound(price);
        if pos < self.sorted.len() && self.sorted.get_unchecked(pos) == price {
            self.sorted.remove(pos);
            true
        } else {
            false
        }
    }

    /// A source replaced its previous price `old` with `new`.
    pub fn replace(&mut self, old: i128, new: i128) -> bool {
        if !self.remove(old) {
            return false;
        }
        self.insert(new);
        true
    }

    /// Sortedness invariant; cheap enough to use as the fallback trigger.
    pub fn is_consistent(&self) -> bool {
        let n = self.sorted.len();
        let mut i = 1;
        while i < n {
            if self.sorted.get_unchecked(i - 1) > self.sorted.get_unchecked(i) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn median_unchecked(&self) -> i128 {
        let n = self.sorted.len();
        if n == 0 {
            return 0;
        }
        let mid = n / 2;
        if n.is_multiple_of(2) {
            let lower = self.sorted.get_unchecked(mid - 1);
            let upper = self.sorted.get_unchecked(mid);
            lower + (upper - lower) / 2
        } else {
            self.sorted.get_unchecked(mid)
        }
    }

    /// Median for `current_epoch`; `None` when the cached structure is stale.
    pub fn median_at(&self, current_epoch: u32) -> Option<i128> {
        if self.epoch != current_epoch {
            return None;
        }
        Some(self.median_unchecked())
    }

    /// Median, rebuilding from `ground_truth` when stale or inconsistent.
    /// Returns `(median, rebuilt)`.
    pub fn median_or_rebuild(
        &mut self,
        current_epoch: u32,
        ground_truth: &Vec<i128>,
    ) -> (i128, bool) {
        if self.epoch == current_epoch
            && self.sorted.len() == ground_truth.len()
            && self.is_consistent()
        {
            return (self.median_unchecked(), false);
        }
        *self = Self::rebuild(ground_truth, current_epoch);
        (self.median_unchecked(), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::compute_median;
    use proptest::prelude::*;

    fn to_vec(env: &Env, v: &[i64]) -> Vec<i128> {
        let mut out = Vec::new(env);
        for x in v {
            out.push_back(*x as i128);
        }
        out
    }

    #[derive(Debug, Clone)]
    enum Op {
        Insert(i64),
        RemoveAt(usize),
        ReplaceAt(usize, i64),
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            (-1_000_000i64..1_000_000).prop_map(Op::Insert),
            any::<usize>().prop_map(Op::RemoveAt),
            (any::<usize>(), -1_000_000i64..1_000_000).prop_map(|(i, p)| Op::ReplaceAt(i, p)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]

        /// Incremental median equals a full recompute after every mutation, for any
        /// arrival order.
        #[test]
        fn incremental_matches_full_recompute(
            initial in proptest::collection::vec(-1_000_000i64..1_000_000, 0..20usize),
            ops in proptest::collection::vec(op_strategy(), 1..60usize),
        ) {
            let env = Env::default();
            let mut truth: std::vec::Vec<i64> = initial.clone();
            let mut inc = IncrementalAggregate::rebuild(&to_vec(&env, &truth), 0);
            for op in ops {
                match op {
                    Op::Insert(p) => { truth.push(p); inc.insert(p as i128); }
                    Op::RemoveAt(i) => {
                        if truth.is_empty() { continue; }
                        let p = truth.remove(i % truth.len());
                        prop_assert!(inc.remove(p as i128));
                    }
                    Op::ReplaceAt(i, p) => {
                        if truth.is_empty() { continue; }
                        let idx = i % truth.len();
                        let old = truth[idx];
                        truth[idx] = p;
                        prop_assert!(inc.replace(old as i128, p as i128));
                    }
                }
                prop_assert!(inc.is_consistent());
                prop_assert_eq!(inc.len() as usize, truth.len());
                prop_assert_eq!(inc.median_at(0).unwrap(), compute_median(&to_vec(&env, &truth)));
            }
        }

        /// Arrival order never changes the final value.
        #[test]
        fn order_independent(mut vals in proptest::collection::vec(-1_000_000i64..1_000_000, 1..40usize)) {
            let env = Env::default();
            let mut a = IncrementalAggregate::empty(&env, 0);
            for v in &vals { a.insert(*v as i128); }
            vals.reverse();
            let mut b = IncrementalAggregate::empty(&env, 0);
            for v in &vals { b.insert(*v as i128); }
            prop_assert_eq!(a.median_at(0), b.median_at(0));
        }
    }

    #[test]
    fn removal_updates_without_rebuild() {
        let env = Env::default();
        let mut inc = IncrementalAggregate::rebuild(&to_vec(&env, &[100, 200, 300, 400, 500]), 7);
        assert_eq!(inc.median_at(7), Some(300));
        assert!(inc.remove(500));
        // Same epoch, consistent, lengths match ground truth → no rebuild.
        let truth = to_vec(&env, &[100, 200, 300, 400]);
        assert_eq!(inc.median_or_rebuild(7, &truth), (250, false));
        assert!(!inc.remove(999));
    }

    #[test]
    fn stale_aggregate_never_served_after_source_set_change() {
        let env = Env::default();
        let mut inc = IncrementalAggregate::rebuild(&to_vec(&env, &[100, 200, 300]), 1);
        // Source admitted / removed / evicted → epoch bumped to 2.
        assert_eq!(inc.median_at(2), None);
        let truth = to_vec(&env, &[100, 200, 300, 10_000]);
        assert_eq!(inc.median_or_rebuild(2, &truth), (250, true));
        assert_eq!(inc.epoch(), 2);
        assert_eq!(inc.median_at(2), Some(250));
    }

    #[test]
    fn rebuild_fallback_on_uncertain_invariant() {
        let env = Env::default();
        let mut inc = IncrementalAggregate::rebuild(&to_vec(&env, &[1, 2, 3]), 0);
        // Corrupt the structure: drift from ground truth.
        inc.sorted.set(0, 50);
        assert!(!inc.is_consistent());
        let truth = to_vec(&env, &[1, 2, 3]);
        assert_eq!(inc.median_or_rebuild(0, &truth), (2, true));
        assert!(inc.is_consistent());
        // Length drift (e.g. a failed submission that was never rolled back) also rebuilds.
        inc.insert(9);
        assert_eq!(inc.median_or_rebuild(0, &truth), (2, true));
    }

    /// Benchmark on a max-size source set: one incremental update vs a full recompute.
    #[test]
    fn incremental_update_cheaper_than_full_recompute() {
        let env = Env::default();
        // Max source-set size used by the gas benchmarks.
        let n = 64i64;
        let vals: std::vec::Vec<i64> = (0..n).map(|i| (i * 7919) % 10_007).collect();
        let truth = to_vec(&env, &vals);
        let mut inc = IncrementalAggregate::rebuild(&truth, 0);

        env.budget().reset_unlimited();
        let _ = compute_median(&truth);
        let full = env.budget().cpu_instruction_cost();

        env.budget().reset_unlimited();
        inc.replace(vals[0] as i128, 5_000);
        let _ = inc.median_at(0);
        let incremental = env.budget().cpu_instruction_cost();

        assert!(incremental < full, "incremental={incremental} full={full}");
    }
}
