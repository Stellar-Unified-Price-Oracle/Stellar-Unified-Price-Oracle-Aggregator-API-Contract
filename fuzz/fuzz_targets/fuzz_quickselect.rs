//! # Fuzz target: `fuzz_quickselect`  (#189 / #190)
//!
//! Coverage-guided fuzzer that verifies `quickselect_core` from `core_pricing.rs`
//! against a fully-sorted reference for every generated (array, k) pair.
//!
//! ## Invariants checked
//!
//! 1. `arr[k]` after `quickselect_core(&mut arr, k)` equals `sorted[k]`.
//! 2. All elements in `arr[..k]` are ≤ `arr[k]`.
//! 3. All elements in `arr[k+1..]` are ≥ `arr[k]`.
//! 4. The multiset of values is unchanged (no elements dropped or duplicated).
//!
//! ## Running
//!
//! ```sh
//! cargo fuzz run fuzz_quickselect -- -runs=1000000
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use price_oracle::core_pricing::quickselect_core;

fuzz_target!(|data: &[u8]| {
    // Minimum: 8 bytes for at least one i64 + 1 byte for k.
    if data.len() < 9 {
        return;
    }

    // Last byte encodes k (modulo n, computed below).
    let k_raw = *data.last().unwrap() as usize;
    let payload = &data[..data.len() - 1];

    // Decode payload as i64 little-endian words.
    let mut arr: Vec<i128> = payload
        .chunks_exact(8)
        .map(|b| {
            let bytes: [u8; 8] = b.try_into().unwrap();
            i64::from_le_bytes(bytes) as i128
        })
        .collect();
    arr.truncate(100);
    let n = arr.len();
    if n == 0 {
        return;
    }

    let k = k_raw % n;

    // Reference: fully sorted copy.
    let mut sorted = arr.clone();
    sorted.sort_unstable();
    let expected_kth = sorted[k];

    // ── invariant 1: k-th element correctness ────────────────────────────────
    quickselect_core(&mut arr, k);
    assert_eq!(
        arr[k], expected_kth,
        "QUICKSELECT k={} wrong: got {} expected {}, original={:?}",
        k, arr[k], expected_kth, &sorted[..n.min(8)]
    );

    // ── invariant 2: lower partition ≤ pivot ─────────────────────────────────
    let pivot = arr[k];
    for (i, &v) in arr[..k].iter().enumerate() {
        assert!(
            v <= pivot,
            "PARTITION LOWER violated at i={}: {} > pivot={}", i, v, pivot
        );
    }

    // ── invariant 3: upper partition ≥ pivot ─────────────────────────────────
    for (i, &v) in arr[k + 1..].iter().enumerate() {
        assert!(
            v >= pivot,
            "PARTITION UPPER violated at i={}: {} < pivot={}", k + 1 + i, v, pivot
        );
    }

    // ── invariant 4: multiset preserved ──────────────────────────────────────
    // Sort arr after quickselect and compare with sorted reference.
    arr.sort_unstable();
    assert_eq!(
        arr, sorted,
        "MULTISET CHANGED: quickselect dropped or added elements"
    );
});
