#![cfg(test)]
//! #507 — Timing side-channel shape tests.
//!
//! The contract cannot hide its metered instruction count from the caller who
//! pays for it, so a comparison that returns at the first difference leaks
//! *where* that difference was. These tests assert the **shape** of the
//! comparison path rather than its wall-clock cost: the number of byte
//! positions inspected must depend only on the declared lengths, never on where
//! a difference lies. See `docs/security/timing-side-channel-review.md`.

use crate::constant_time::{bytes_eq_counted, digest_eq, sig_eq};

// Slice `Vec`, deliberately *not* the Soroban one re-exported at the crate
// root: these helpers take plain host slices so the tests can drive them
// directly.
use std::vec::Vec;

/// A 32-byte buffer with a single bit flipped at `pos`, relative to `base`.
fn with_flip(base: &[u8; 32], pos: usize) -> [u8; 32] {
    let mut out = *base;
    out[pos] ^= 0x01;
    out
}

fn base32() -> [u8; 32] {
    let mut b = [0u8; 32];
    for (i, byte) in b.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    b
}

// ---------------------------------------------------------------------------
// bytes_eq_counted: the scan length must not depend on where the difference is
// ---------------------------------------------------------------------------

#[test]
fn mismatch_position_does_not_change_scan_length() {
    let base = base32();
    let reference: Vec<u8> = base.to_vec();

    // Every single-byte mismatch position must inspect the same number of
    // positions. An early-exit comparison would return fewer for a
    // first-position mismatch than for a last-position one.
    for pos in 0..32usize {
        let flipped = with_flip(&base, pos);
        let (equal, scanned) = bytes_eq_counted(&flipped, &reference);
        assert!(!equal, "a flipped byte at {pos} must not compare equal");
        assert_eq!(
            scanned, 32,
            "mismatch at byte {pos} scanned {scanned} positions, expected 32"
        );
    }
}

#[test]
fn equal_and_unequal_buffers_scan_the_same_number_of_positions() {
    let base = base32();
    let reference: Vec<u8> = base.to_vec();

    let (eq, scanned_eq) = bytes_eq_counted(&reference, &reference);
    assert!(eq);
    let (ne, scanned_ne) = bytes_eq_counted(&with_flip(&base, 31), &reference);
    assert!(!ne);

    assert_eq!(
        scanned_eq, scanned_ne,
        "equality must not be cheaper to determine than inequality"
    );
}

#[test]
fn length_mismatch_does_not_skip_the_scan() {
    let a: Vec<u8> = (0u8..16).collect();
    let b: Vec<u8> = (0u8..32).collect();

    let (equal, scanned) = bytes_eq_counted(&a, &b);
    assert!(!equal, "different lengths must not compare equal");
    assert_eq!(
        scanned, 32,
        "a length mismatch must still scan the longer buffer"
    );
}

#[test]
fn single_bit_difference_anywhere_is_detected() {
    let base = base32();
    let reference: Vec<u8> = base.to_vec();
    // Every bit position in the first and last byte must be caught, not just
    // bit 0 of each byte.
    for bit in 0..8usize {
        for pos in [0usize, 15, 31] {
            let mut flipped = base;
            flipped[pos] ^= 1 << bit;
            let (equal, _) = bytes_eq_counted(&flipped, &reference);
            assert!(!equal, "bit {bit} of byte {pos} went undetected");
        }
    }
}

// ---------------------------------------------------------------------------
// digest_eq / sig_eq: fixed width, full scan
// ---------------------------------------------------------------------------

#[test]
fn digest_eq_matches_on_equal_digests() {
    let e = soroban_sdk::Env::default();
    let base = base32();
    let a = soroban_sdk::BytesN::from_array(&e, &base);
    assert!(digest_eq(&a, &a));
}

#[test]
fn digest_eq_detects_a_flip_at_every_position() {
    let e = soroban_sdk::Env::default();
    let base = base32();
    let reference = soroban_sdk::BytesN::from_array(&e, &base);
    for pos in 0..32usize {
        let flipped = soroban_sdk::BytesN::from_array(&e, &with_flip(&base, pos));
        assert!(
            !digest_eq(&reference, &flipped),
            "digest_eq missed a flip at byte {pos}"
        );
    }
}

#[test]
fn sig_eq_detects_a_flip_at_every_position() {
    let e = soroban_sdk::Env::default();
    let base = [0x5au8; 64];
    let reference = soroban_sdk::BytesN::from_array(&e, &base);
    for pos in 0..64usize {
        let mut flipped = base;
        flipped[pos] ^= 0x80;
        let other = soroban_sdk::BytesN::from_array(&e, &flipped);
        assert!(
            !sig_eq(&reference, &other),
            "sig_eq missed a flip at byte {pos}"
        );
    }
}

// ---------------------------------------------------------------------------
// Source-level guard: no early exit inside a reviewed comparison loop
// ---------------------------------------------------------------------------

/// The reviewed comparison sites must not reintroduce an early return inside a
/// byte loop. `vdf_sampler`'s consistency check is the one secret-operand loop
/// that walks bytes, so its shape is asserted directly against the source.
#[test]
fn vdf_consistency_check_has_no_early_exit() {
    let src = include_str!("vdf_sampler.rs");
    let start = src.find("let computed_arr = computed.to_array();").expect(
        "the VDF consistency check should still assign the arrays it folds; \
             if this moved, update this test to the new location",
    );
    let end = src[start..]
        .find("diff == 0")
        .expect("the VDF consistency check should end in a diff accumulator test");
    let body = &src[start..start + end];

    assert!(
        !body.contains("return false"),
        "the VDF consistency check must not return early: an early exit \
         reveals, through the metered instruction count, how many leading \
         bytes of a forged proof matched (#507)"
    );
    assert!(
        body.contains("diff |="),
        "the VDF consistency check should fold differences into an accumulator"
    );
}

/// The ZK Fiat-Shamir comparison must route through the constant-shape helper
/// rather than comparing digests inline.
#[test]
fn zk_fiat_shamir_compare_uses_constant_shape_helper() {
    let src = include_str!("zk_verify.rs");
    assert!(
        src.contains("constant_time::bytes_eq_counted"),
        "the Fiat-Shamir tag comparison must use the constant-shape helper (#507)"
    );
    assert!(
        !src.contains("if fs_bytes.len() != expected_bytes.len()"),
        "the length early-exit in the Fiat-Shamir comparison must not come back (#507)"
    );
}
