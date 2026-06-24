#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::Env;

use crate::storage::compute_median;

fn to_sdk_vec(env: &Env, v: &[i64]) -> soroban_sdk::Vec<i128> {
    let mut sv = soroban_sdk::Vec::new(env);
    for &x in v {
        sv.push_back(x as i128);
    }
    sv
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Median of an odd-length sorted list equals the middle element.
    #[test]
    fn median_odd_is_middle(mut vals in proptest::collection::vec(any::<i64>(), 1..=49usize).prop_filter("odd length", |v| v.len() % 2 == 1)) {
        let env = Env::default();
        vals.sort();
        let mid = vals[vals.len() / 2] as i128;
        let sv = to_sdk_vec(&env, &vals);
        prop_assert_eq!(compute_median(&sv), mid);
    }

    /// Median of an even-length sorted list equals the average of the two middle elements.
    #[test]
    fn median_even_is_avg_of_middles(mut vals in proptest::collection::vec(any::<i64>(), 2..=50usize).prop_filter("even length", |v| v.len() % 2 == 0)) {
        let env = Env::default();
        vals.sort();
        let mid = vals.len() / 2;
        let a = vals[mid - 1] as i128;
        let b = vals[mid] as i128;
        let expected = a + (b - a) / 2;
        let sv = to_sdk_vec(&env, &vals);
        prop_assert_eq!(compute_median(&sv), expected);
    }

    /// Median is always between min and max.
    #[test]
    fn median_between_min_and_max(vals in proptest::collection::vec(any::<i64>(), 1..=50usize)) {
        let env = Env::default();
        let min = *vals.iter().min().unwrap() as i128;
        let max = *vals.iter().max().unwrap() as i128;
        let sv = to_sdk_vec(&env, &vals);
        let m = compute_median(&sv);
        prop_assert!(m >= min && m <= max);
    }

    /// Adding the median value to the set does not change the median.
    #[test]
    fn median_stable_when_median_added(vals in proptest::collection::vec(any::<i64>(), 1..=49usize)) {
        let env = Env::default();
        let sv = to_sdk_vec(&env, &vals);
        let m = compute_median(&sv);
        // m fits in i64 range since inputs are i64-based
        let mut extended = vals.clone();
        extended.push(m as i64);
        let sv2 = to_sdk_vec(&env, &extended);
        prop_assert_eq!(compute_median(&sv2), m);
    }

    /// Median is invariant under permutation (reverse as a proxy).
    #[test]
    fn median_invariant_under_permutation(vals in proptest::collection::vec(any::<i64>(), 1..=50usize)) {
        let env = Env::default();
        let sv = to_sdk_vec(&env, &vals);
        let m = compute_median(&sv);
        let mut rev = vals.clone();
        rev.reverse();
        let sv_rev = to_sdk_vec(&env, &rev);
        prop_assert_eq!(compute_median(&sv_rev), m);
    }
}
