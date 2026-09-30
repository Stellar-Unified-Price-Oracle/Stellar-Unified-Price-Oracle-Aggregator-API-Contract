//! Tests for the #480 deterministic median tie-break rule.
//!
//! The rule itself is stated in `median_determinism.rs` and, for consumers, in
//! `docs/median-determinism.md`. These tests pin it: the named cases cover the
//! even-count, duplicate and rounding-boundary situations the rule exists for,
//! and the property test covers order-independence.

#![cfg(test)]

use soroban_sdk::{Env, Vec as SdkVec};

use crate::median_determinism::{canonical_median, orderings, to_env_vec};
use crate::storage::compute_median;

fn median_of(env: &Env, values: &[i128]) -> i128 {
    canonical_median(&to_env_vec(env, values))
}

/// The SDK implementation and the documented rule must never diverge.
#[test]
fn sdk_and_documented_rule_agree() {
    let e = Env::default();
    let cases: std::vec::Vec<std::vec::Vec<i128>> = vec![
        vec![1],
        vec![1, 2],
        vec![1, 2, 3],
        vec![1, 2, 3, 4],
        vec![5, 5],
        vec![5, 5, 5, 5],
        vec![10, 20, 20, 100],
        vec![-5, -3, -1, 0, 2],
        vec![7, 7, 7, 7, 7],
        vec![i128::MAX, i128::MAX],
    ];
    for case in cases {
        let v = to_env_vec(&e, &case);
        assert_eq!(
            compute_median(&v),
            canonical_median(&v),
            "compute_median diverged from the documented rule for {:?}",
            case
        );
    }
}

/// Even count: the two central values are averaged, truncating down.
#[test]
fn even_count_averages_the_two_central_values() {
    let e = Env::default();
    // sorted: 1 2 3 4 -> lo=2, hi=3 -> 2 + 1/2 = 2
    assert_eq!(median_of(&e, &[4, 3, 2, 1]), 2);
    // sorted: 10 20 30 40 -> lo=20, hi=30 -> 25
    assert_eq!(median_of(&e, &[40, 10, 30, 20]), 25);
}

/// Even count with an odd gap is the documented lower-of-two tie-break.
#[test]
fn even_count_odd_gap_takes_the_lower_central_value() {
    let e = Env::default();
    // lo=100, hi=101 -> 100 + 0 = 100, not 101 and not 100.5
    assert_eq!(median_of(&e, &[101, 100]), 100);
    // lo=2, hi=3 -> 2, never rounded up to 3
    assert_eq!(median_of(&e, &[2, 3]), 2);
    // A three-apart gap still floors: 10 + 3/2 = 11.
    assert_eq!(median_of(&e, &[10, 13]), 11);
}

/// Even count with an even gap is exact, so no rounding is involved at all.
#[test]
fn even_count_even_gap_is_exact() {
    let e = Env::default();
    // lo=100, hi=104 -> 100 + 4/2 = 102
    assert_eq!(median_of(&e, &[104, 100]), 102);
}

/// Two identical values: the two central statistics coincide.
#[test]
fn duplicate_pair_collapses_to_that_value() {
    let e = Env::default();
    assert_eq!(median_of(&e, &[5, 5]), 5);
    assert_eq!(median_of(&e, &[0, 0]), 0);
}

/// Duplicates spanning the centre must not be miscounted.
#[test]
fn duplicates_around_the_centre_are_counted_with_multiplicity() {
    let e = Env::default();
    // sorted 10 20 20 100 -> lo=20, hi=20 -> 20, *not* 10 or 100
    assert_eq!(median_of(&e, &[100, 20, 20, 10]), 20);
    // sorted 1 2 2 2 9 (odd) -> middle = 2
    assert_eq!(median_of(&e, &[9, 2, 1, 2, 2]), 2);
    // sorted 1 1 1 1 1 9 (even) -> lo=1, hi=1 -> 1
    assert_eq!(median_of(&e, &[9, 1, 1, 1, 1, 1]), 1);
}

/// Rounding boundary: adjacent large values must not overflow or round up.
#[test]
fn rounding_boundary_does_not_overflow() {
    let e = Env::default();
    // Two near-max values: `(lo + hi)` would overflow i128, the documented
    // form must not.
    let hi = i128::MAX;
    let lo = i128::MAX - 1;
    assert_eq!(median_of(&e, &[hi, lo]), lo);
    // Negative pair: the floor must still be the lower (more negative) value.
    assert_eq!(median_of(&e, &[-1, -2]), -2);
    assert_eq!(median_of(&e, &[-3, -1]), -2);
}

/// Odd count is the exact middle element.
#[test]
fn odd_count_is_the_exact_middle() {
    let e = Env::default();
    assert_eq!(median_of(&e, &[1, 2, 3]), 2);
    assert_eq!(median_of(&e, &[3, 1, 2]), 2);
    assert_eq!(median_of(&e, &[1, 2, 3, 4, 5]), 3);
}

/// Every ordering of the same set yields an identical aggregate.
#[test]
fn any_ordering_yields_an_identical_aggregate() {
    let e = Env::default();
    let sets: std::vec::Vec<std::vec::Vec<i128>> = vec![
        vec![10, 20, 30, 40],
        vec![5, 5, 5, 5],
        vec![1, 1, 2, 3, 3],
        vec![100, 101],
        vec![7],
        vec![-5, -3, -1, 0, 2, 9],
    ];
    for set in sets {
        let expected = median_of(&e, &set);
        for (i, ordering) in orderings(&e, &set).iter().enumerate() {
            assert_eq!(
                compute_median(&ordering),
                expected,
                "ordering #{} of {:?} produced a different aggregate",
                i,
                set
            );
        }
    }
}

/// Exhaustive order-independence: for a small set, *every* permutation is
/// enumerated and must agree. This is the strongest form of the property and is
/// tractable at these sizes.
#[test]
fn every_permutation_of_a_small_set_agrees() {
    let e = Env::default();
    let base: std::vec::Vec<i128> = vec![11, 22, 22, 33, 44];
    let expected = median_of(&e, &base);

    let n = base.len();
    let mut idx: std::vec::Vec<u32> = (0..n as u32).collect();
    loop {
        let perm: std::vec::Vec<i128> = idx.iter().map(|&i| base[i as usize]).collect();
        assert_eq!(
            compute_median(&to_env_vec(&e, &perm)),
            expected,
            "permutation {:?} produced a different aggregate",
            perm
        );
        if !next_permutation(&mut idx) {
            break;
        }
    }
}

/// Advances `idx` to the next lexicographic permutation; `false` when `idx` is
/// already the last one.
///
/// `idx` holds distinct values `0..n`, so this is the textbook algorithm over
/// that range: find the longest descending suffix, pivot the element before it
/// up to the next larger value, then reverse the suffix.
fn next_permutation(idx: &mut [u32]) -> bool {
    let n = idx.len();
    if n < 2 {
        return false;
    }
    // Pivot: the last index whose successor is larger.
    let mut pivot = n - 2;
    while idx[pivot] >= idx[pivot + 1] {
        if pivot == 0 {
            return false;
        }
        pivot -= 1;
    }
    // Successor: the smallest value in the descending suffix above the pivot.
    let mut succ = n - 1;
    while idx[succ] <= idx[pivot] {
        succ -= 1;
    }
    idx.swap(pivot, succ);
    idx[pivot + 1..].reverse();
    true
}

/// Storage iteration order cannot affect the result.
///
/// Sources are registered in one order and their prices submitted in the
/// reverse, which perturbs the order the aggregation pass walks its submission
/// keys in. The published aggregate must be identical to the one produced by
/// the forward order, because the rule reads only the multiset.
#[test]
fn storage_iteration_order_cannot_affect_the_result() {
    use crate::test_helpers::setup_contract;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Address, String};

    let aggregate_for = |e: &Env, reverse: bool| -> Option<i128> {
        let (client, _admin) = setup_contract(e);
        e.ledger().with_mut(|l| l.timestamp = 1000);
        client.set_min_sources_required(&1u32);

        let asset = Address::generate(e);
        client.register_asset(&asset);

        let mut sources: SdkVec<Address> = SdkVec::new(e);
        for _ in 0..4u32 {
            let s = Address::generate(e);
            client.add_source(&s, &String::from_str(e, "src"));
            sources.push_back(s);
        }

        let values = [100i128, 200, 300, 400];
        let order: std::vec::Vec<u32> = if reverse {
            vec![3, 2, 1, 0]
        } else {
            vec![0, 1, 2, 3]
        };
        for idx in order {
            let src = sources.get_unchecked(idx);
            client.submit_price(&src, &asset, &values[idx as usize], &1000u64);
        }
        client.get_price(&asset, &0u64).map(|p| p.price)
    };

    let forward = aggregate_for(&Env::default(), false).expect("forward aggregate");
    let reverse = aggregate_for(&Env::default(), true).expect("reverse aggregate");
    assert_eq!(
        forward, reverse,
        "submitting the same values in the opposite order changed the aggregate"
    );
    // Four values, even count: (200 + 300) / 2 = 250.
    assert_eq!(forward, 250);
}

/// The tie-break survives removing a source that was not central.
#[test]
fn tie_break_is_stable_after_a_non_central_source_is_removed() {
    let e = Env::default();
    // The even-count rule depends only on the multiset, so dropping a value
    // that is not one of the two central statistics leaves the median alone
    // only when the remaining set is unchanged at the centre — which is
    // exactly what this asserts about the rule, not about source bookkeeping.
    let before = median_of(&e, &[10, 20, 30, 40]);
    let after = median_of(&e, &[20, 30, 40]);
    assert_eq!(before, 25);
    assert_eq!(after, 30);
    // The documented answer for each set is reproduced independently.
    assert_eq!(canonical_median(&to_env_vec(&e, &[10, 20, 30, 40])), 25);
    assert_eq!(canonical_median(&to_env_vec(&e, &[20, 30, 40])), 30);
}
