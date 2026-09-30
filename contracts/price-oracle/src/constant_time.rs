//! #507 — Constant-shape comparison helpers for auth & crypto paths
//!
//! On a Soroban VM the caller cannot read a wall clock, but it *can* read the
//! metered instruction count a call consumed (from the fee it is charged, from
//! simulation, or from a cross-call that observes its own budget). A comparison
//! that returns as soon as it finds a difference therefore leaks, through
//! metering, *where* the first difference was. That is enough to recover a
//! secret one byte at a time even though no clock is readable.
//!
//! The helpers here remove the input-shape dependence: they always inspect
//! every byte position of the longer input and fold the per-byte differences
//! into an accumulator with a branch-free `|=`. The only data-dependent branch
//! left is the loop bound, which is a function of the *declared* lengths —
//! which for every call site in this contract are fixed-width (`BytesN<32>`,
//! `BytesN<64>`) or already public.
//!
//! See `docs/security/timing-side-channel-review.md` for the full site-by-site
//! classification (public vs secret) and the accepted residual exposure.

use soroban_sdk::{Bytes, BytesN};

/// Compares two byte buffers without an early exit.
///
/// Both buffers are scanned over `max(a.len(), b.len())` positions. A position
/// beyond the end of one buffer contributes that buffer's implicit zero bytes,
/// so a length mismatch is reported as inequality without short-circuiting.
///
/// # Arguments
///
/// * `a` - First buffer.
/// * `b` - Second buffer.
///
/// # Returns
///
/// `true` if the buffers have identical contents and length.
pub fn bytes_eq(a: &Bytes, b: &Bytes) -> bool {
    // Length is folded into the accumulator rather than tested, so a length
    // mismatch does not skip the scan below.
    let mut diff: u8 = ((a.len() ^ b.len()) != 0) as u8;
    for i in 0..max(a.len(), b.len()) {
        diff |= byte_at(a, i) ^ byte_at(b, i);
    }
    diff == 0
}

/// Compares two fixed-width 32-byte digests without an early exit.
///
/// Fixed width means the loop bound carries no information about the contents,
/// so the scan length is identical for every pair of inputs.
pub fn digest_eq(a: &BytesN<32>, b: &BytesN<32>) -> bool {
    let (la, lb) = (a.to_array(), b.to_array());
    let mut diff: u8 = 0;
    for i in 0..32 {
        diff |= la[i] ^ lb[i];
    }
    diff == 0
}

/// Compares two fixed-width 64-byte values (signatures, commitments) without
/// an early exit. Same reasoning as [`digest_eq`].
pub fn sig_eq(a: &BytesN<64>, b: &BytesN<64>) -> bool {
    let (la, lb) = (a.to_array(), b.to_array());
    let mut diff: u8 = 0;
    for i in 0..64 {
        diff |= la[i] ^ lb[i];
    }
    diff == 0
}

/// Compares two arbitrary byte arrays in constant shape, returning the verdict
/// together with the number of byte positions inspected.
///
/// The counter is what makes the constant-shape property *testable*: a test
/// asserts it is `max(a.len(), b.len())` for a mismatch in the first position,
/// a mismatch in the last position, and for equal buffers alike. `bytes_eq` is
/// this function with the counter discarded.
pub fn bytes_eq_counted(a: &[u8], b: &[u8]) -> (bool, u32) {
    // The scan covers the longer buffer, so the iteration count is a function
    // of the two declared lengths and never of *where* a difference is.
    let n = if a.len() >= b.len() { a.len() } else { b.len() };
    // Length is folded into the accumulator rather than tested, so a length
    // mismatch does not skip the scan.
    let mut diff: u32 = (a.len() ^ b.len()) as u32;
    for i in 0..n {
        let lhs = if i < a.len() { a[i] } else { 0 };
        let rhs = if i < b.len() { b[i] } else { 0 };
        diff |= (lhs ^ rhs) as u32;
    }
    // `diff` is non-zero iff any byte or the length differs; the fold above can
    // never wrap because every term is at most 0xff.
    (diff == 0, n as u32)
}

/// Reads byte `i` of `buf`, treating an out-of-range index as a zero byte.
fn byte_at(buf: &Bytes, i: u32) -> u8 {
    if i < buf.len() {
        buf.get_unchecked(i)
    } else {
        0
    }
}

fn max(a: u32, b: u32) -> u32 {
    if a >= b {
        a
    } else {
        b
    }
}
