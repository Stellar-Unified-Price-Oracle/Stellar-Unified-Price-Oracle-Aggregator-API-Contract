//! # Deterministic median tie-breaking and canonical ordering (#480)
//!
//! See `docs/median-determinism.md` for the consumer-facing statement of the
//! rule. This module is the single normative definition; `storage::compute_median`
//! is the on-chain implementation and the two are kept in step by
//! `median_determinism_tests::sdk_and_documented_rule_agree`.
//!
//! ## Why a rule is needed at all
//!
//! A median of an **even**-sized set has no single middle element: the two
//! central order statistics `lo` and `hi` bracket the answer, and any function
//! of the pair is a legitimate median. If the choice between them is left to
//! implementation detail — iteration order, storage layout, the order sources
//! happened to submit in — then the published value is not a function of the
//! *set* of submissions, only of their *arrival*. An oracle whose value can
//! change without any input changing is not reproducible, and a consumer that
//! replays the published aggregate to verify it will disagree with the chain.
//!
//! Duplicate values make the same point sharper: when `lo == hi` the two
//! central statistics coincide, so any interpolation between them returns
//! `lo`. That is the correct answer, and it is reached *by the rule*, not by
//! luck.
//!
//! ## The rule
//!
//! Let the eligible submissions be sorted ascending. With `n` values:
//!
//! * **`n` odd** — the value at index `n / 2` (0-based), exactly.
//! * **`n` even** — `lo + (hi - lo) / 2`, where `lo` is the value at index
//!   `n / 2 - 1` and `hi` the value at index `n / 2`. Integer division floors,
//!   so the result is the lower of the two central values whenever they differ
//!   by an odd number: the **lower-of-two** tie-break.
//!
//! Both branches read only the *multiset* of values. Nothing in the rule
//! consults a source address, a submission timestamp, a ledger number or a
//! storage key, so no permutation of the same set can reach a different
//! answer. In particular the tie-break is **not** "whichever source the host
//! happened to iterate first" and **not** "whichever submission is newest" —
//! both of those would make the value depend on arrival order.
//!
//! ## Why `lo + (hi - lo) / 2` and not `(lo + hi) / 2`
//!
//! `(lo + hi) / 2` overflows `i128` for two large prices of the same sign, and
//! it rounds toward zero, which for a negative pair would round *up* — the
//! opposite of the documented lower-of-two tie-break. `lo + (hi - lo) / 2`
//! has neither defect: the subtraction cannot overflow because `hi >= lo`
//! makes the gap non-negative and bounded by the `i128` range the two values
//! already span, and the floor is the rule itself.
//!
//! ## Stability across source removal
//!
//! The rule depends only on the multiset, so removing a source changes the
//! answer only if its value was one of the two central statistics — the same
//! as for any other median. There is no per-source term to drop, so a removal
//! can never shift the value for a reason *other* than the multiset changing.

use soroban_sdk::{Env, Vec};

/// Lower-of-two median of an unordered set of eligible submissions.
///
/// This is the normative reference for [`crate::storage::compute_median`]:
/// given the same multiset, the two must return the same value. The
/// implementation sorts a copy and reads off the central order statistics,
/// which is deliberately the *slowest* correct way to do it — it exists to
/// state the rule unambiguously, not to be the hot path.
///
/// An empty set has no median and yields `0`, matching the on-chain function's
/// documented empty-input behaviour.
pub fn canonical_median(values: &Vec<i128>) -> i128 {
    let n = values.len();
    if n == 0 {
        return 0;
    }
    let mut sorted = values.clone();
    sort_ascending(&mut sorted);
    if n % 2 == 1 {
        return sorted.get_unchecked(n / 2);
    }
    let lo = sorted.get_unchecked(n / 2 - 1);
    let hi = sorted.get_unchecked(n / 2);
    lo + (hi - lo) / 2
}

/// Sorts `values` ascending, in place, by insertion sort.
///
/// Insertion sort is chosen over the heapsort used on the aggregation hot path
/// because it is simple enough to audit line-by-line against the written rule,
/// and this function is only ever called from tests and documentation examples.
fn sort_ascending(values: &mut Vec<i128>) {
    let n = values.len();
    if n <= 1 {
        return;
    }
    let mut i = 1;
    while i < n {
        let key = values.get_unchecked(i);
        let mut j = i;
        while j > 0 && values.get_unchecked(j - 1) > key {
            values.set(j, values.get_unchecked(j - 1));
            j -= 1;
        }
        values.set(j, key);
        i += 1;
    }
}

/// Builds an SDK `Vec` from a slice, for tests and documentation examples.
pub fn to_env_vec(env: &Env, values: &[i128]) -> Vec<i128> {
    let mut out: Vec<i128> = Vec::new(env);
    for v in values {
        out.push_back(*v);
    }
    out
}

/// A fixed, deterministic family of orderings of `values`.
///
/// Used by the order-independence tests. A full permutation group is
/// exponential, so this returns the identity, the reversal, a rotation and a
/// stable partition that hoists one value to the front — enough to catch any
/// dependence on position. The exhaustive check over *all* orderings is done
/// separately for the small inputs where it is tractable.
pub fn orderings(env: &Env, values: &[i128]) -> Vec<Vec<i128>> {
    let mut out: Vec<Vec<i128>> = Vec::new(env);
    out.push_back(to_env_vec(env, values));

    let mut reversed = values.to_vec();
    reversed.reverse();
    out.push_back(to_env_vec(env, &reversed));

    if values.len() > 1 {
        let mut rotated = values.to_vec();
        rotated.rotate_left(1);
        out.push_back(to_env_vec(env, &rotated));

        // Hoist the last value to the front. This is the shape a host produces
        // when it returns storage keys in a different order than insertion.
        let mut hoisted = values.to_vec();
        let last = hoisted.pop().unwrap();
        hoisted.insert(0, last);
        out.push_back(to_env_vec(env, &hoisted));
    }

    out
}
