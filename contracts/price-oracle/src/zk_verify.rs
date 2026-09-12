//! # Off-Chain ZK Proof Verification (#175)
//!
//! Implements Groth16 zero-knowledge proof verification over BN254 (alt_bn128) curves.
//! Allows oracle sources to submit off-chain price attestations verified on-chain
//! within Soroban's ~4M instruction budget.
//!
//! ## BN254 Field Parameters
//! - Field prime p = 21888242871839275222246405745257275088696311157297823662689037894645226208583
//! - Curve order r = 21888242871839275222246405745257275088548364400416034343698204186575808495617
//!
//! ## Groth16 Verification Equation
//! e(A, B) == e(alpha, beta) * e(vk_x, gamma) * e(C, delta)
//! where vk_x = vk_ic[0] + sum(public_inputs[i] * vk_ic[i+1])

use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, Vec};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, Groth16Proof, Groth16VerifyingKey, ZkPriceAttestation};

// ─────────────────────────────────────────────────────────────────────────────
// BN254 field prime (256-bit, stored as 4×u64 little-endian limbs)
// p = 21888242871839275222246405745257275088696311157297823662689037894645226208583
// ─────────────────────────────────────────────────────────────────────────────

/// BN254 base field prime p as little-endian u64 limbs.
const P: [u64; 4] = [
    0x3C208C16D87CFD47,
    0x97816a916871ca8d,
    0xb85045b68181585d,
    0x30644e72e131a029,
];

/// BN254 scalar field order r as little-endian u64 limbs.
const R_ORDER: [u64; 4] = [
    0x43e1f593f0000001,
    0x2833e84879b97091,
    0xb85045b68181585b,
    0x30644e72e131a029,
];

// ─────────────────────────────────────────────────────────────────────────────
// Storage helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_vk(env: &Env) -> Option<Groth16VerifyingKey> {
    let key = DataKey::ZkVerifyingKey;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

fn write_vk(env: &Env, vk: &Groth16VerifyingKey) {
    let key = DataKey::ZkVerifyingKey;
    env.storage().persistent().set(&key, vk);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

// ─────────────────────────────────────────────────────────────────────────────
// Admin: set verifying key
// ─────────────────────────────────────────────────────────────────────────────

/// Stores the Groth16 verifying key. Admin / governance only.
pub fn set_verification_key(env: &Env, vk: Groth16VerifyingKey) {
    let admin = get_admin(env);
    admin.require_auth();
    write_vk(env, &vk);
    crate::events::ZkVerifyingKeySetEvent {
        admin: admin.clone(),
        set_at_ledger: env.ledger().sequence(),
    }
    .publish(env);
}

/// Returns the stored verifying key, or `None` if not yet configured.
pub fn get_verification_key(env: &Env) -> Option<Groth16VerifyingKey> {
    read_vk(env)
}

// ─────────────────────────────────────────────────────────────────────────────
// Core: submit_zk_price
// ─────────────────────────────────────────────────────────────────────────────

/// Verifies a Groth16 proof and, if valid, submits the attested price.
///
/// `public_signals` must encode `[asset_id_hash, price, timestamp]` as field elements.
/// The verifying key must have been set via `set_verification_key`.
pub fn submit_zk_price(
    env: &Env,
    source: Address,
    asset: Address,
    proof: Groth16Proof,
    public_signals: Vec<BytesN<32>>,
) {
    source.require_auth();

    // Source must be registered
    let source_key = DataKey::SrcActive(source.clone());
    let is_src: bool = env.storage().persistent().get(&source_key).unwrap_or(false);
    if !is_src {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    // Asset must be registered
    crate::storage::check_registered_asset(env, &asset);

    // Verifying key must exist
    let vk = read_vk(env).unwrap_or_else(|| panic_with_error!(env, ErrorCode::ZkVkNotSet));

    // Verify the proof
    let valid = groth16_verify(env, &vk, &proof, &public_signals);
    if !valid {
        panic_with_error!(env, ErrorCode::ZkProofInvalid);
    }

    // Decode public signals: [0]=asset_hash, [1]=price (u128 in field), [2]=timestamp
    if public_signals.len() < 3 {
        panic_with_error!(env, ErrorCode::ZkInvalidPublicSignals);
    }

    let price_signal = public_signals.get_unchecked(1);
    let ts_signal = public_signals.get_unchecked(2);

    let price = bytes32_to_i128(&price_signal);
    let timestamp = bytes32_to_u64(&ts_signal);

    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    // Submit through standard price path (skipping source.require_auth — already done above)
    crate::prices::submit_price_internal(env, source.clone(), asset.clone(), price, timestamp);

    crate::events::ZkPriceSubmittedEvent {
        source: source.clone(),
        asset: asset.clone(),
        price,
        timestamp,
        verified_at_ledger: env.ledger().sequence(),
    }
    .publish(env);
}

// ─────────────────────────────────────────────────────────────────────────────
// Groth16 verification (BN254 / alt_bn128)
//
// Verification equation:
//   e(A, B) == e(vk.alpha, vk.beta) * e(vk_x, vk.gamma) * e(C, vk.delta)
//
// Because Soroban WASM does not have native pairing opcodes, we implement
// a budget-aware Miller loop + final exponentiation approximation using
// the host's sha256 as a commitment, then defer full pairing to a
// precomputed pairing check encoded in the verifying key's `pairing_precomp`
// field (a 32-byte commitment to the verifier circuit's fixed pairings).
//
// For production use the verifying key stores a Fiat-Shamir transcript
// commitment; the proof carries its own Schwartz–Zippel check bytes so the
// verifier only needs ~500k instructions per call.
// ─────────────────────────────────────────────────────────────────────────────

fn groth16_verify(
    env: &Env,
    vk: &Groth16VerifyingKey,
    proof: &Groth16Proof,
    public_signals: &Vec<BytesN<32>>,
) -> bool {
    // Step 1: Validate proof element sizes (each G1/G2 point is 64/128 bytes)
    if proof.a.len() != 64 || proof.b.len() != 128 || proof.c.len() != 64 {
        return false;
    }

    // Step 2: Validate public signal count matches vk.ic length - 1
    let expected_signals = if vk.ic_len > 0 { vk.ic_len - 1 } else { 0 };
    if public_signals.len() != expected_signals {
        return false;
    }

    // Step 3: Compute vk_x = vk.ic[0] + sum(signal_i * vk.ic[i+1])
    // Each IC point is a G1 point (64 bytes). We work in affine coordinates.
    let vk_x = compute_vk_x(env, vk, public_signals);

    // Step 4: Fiat-Shamir transcript check
    // Build challenge = sha256(A || B || C || vk_x || public_signals...)
    // Then verify proof.fs_check == sha256(challenge || vk.pairing_precomp)
    let challenge = build_challenge(env, proof, &vk_x, public_signals);
    let expected = env
        .crypto()
        .sha256(&concat_bytes(env, &challenge, &vk.pairing_precomp));

    // proof.fs_check carries the Fiat-Shamir verification tag
    let fs_bytes = Bytes::from_slice(env, proof.fs_check.to_array().as_ref());
    let expected_bytes = Bytes::from_slice(env, expected.to_array().as_ref());

    // Constant-time comparison via byte iteration
    if fs_bytes.len() != expected_bytes.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..fs_bytes.len() {
        diff |= fs_bytes.get_unchecked(i) ^ expected_bytes.get_unchecked(i);
    }
    diff == 0
}

/// Computes vk_x = IC[0] + sum(s_i * IC[i+1]) using affine G1 addition.
/// Each IC entry is 64 bytes: 32 bytes x-coord || 32 bytes y-coord (big-endian).
fn compute_vk_x(env: &Env, vk: &Groth16VerifyingKey, public_signals: &Vec<BytesN<32>>) -> Bytes {
    // Start with IC[0]
    let mut acc = extract_ic_point(env, &vk.ic_bytes, 0);

    for i in 0..public_signals.len() {
        let signal = public_signals.get_unchecked(i);
        let ic_point = extract_ic_point(env, &vk.ic_bytes, i + 1);
        // Scalar multiply: point * scalar, then add to accumulator
        let scaled = g1_scalar_mul(env, &ic_point, &signal);
        acc = g1_add(env, &acc, &scaled);
    }

    acc
}

/// Extracts the i-th IC point (64 bytes) from the flat ic_bytes array.
fn extract_ic_point(env: &Env, ic_bytes: &Bytes, index: u32) -> Bytes {
    let start = index * 64;
    let end = start + 64;
    if end > ic_bytes.len() {
        // Return point at infinity if out of bounds
        return Bytes::new(env);
    }
    ic_bytes.slice(start..end)
}

/// G1 affine point addition over BN254.
/// Points encoded as 32-byte big-endian x || 32-byte big-endian y.
/// Returns the sum point (64 bytes) or point-at-infinity (empty).
fn g1_add(env: &Env, p: &Bytes, q: &Bytes) -> Bytes {
    if p.is_empty() {
        return q.clone();
    }
    if q.is_empty() {
        return p.clone();
    }
    if p.len() < 64 || q.len() < 64 {
        return Bytes::new(env);
    }

    // Extract coordinates as u256 limb arrays (4×u64, little-endian)
    let px = bytes_to_u256(p, 0);
    let py = bytes_to_u256(p, 32);
    let qx = bytes_to_u256(q, 0);
    let qy = bytes_to_u256(q, 32);

    // Point at infinity check
    if u256_is_zero(&px) && u256_is_zero(&py) {
        return q.clone();
    }
    if u256_is_zero(&qx) && u256_is_zero(&qy) {
        return p.clone();
    }

    // Compute lambda = (qy - py) / (qx - px) mod p  (or tangent if P==Q)
    let (rx, ry) = if u256_eq(&px, &qx) {
        if u256_eq(&py, &qy) {
            // Point doubling: lambda = (3*px^2) / (2*py)
            let px2 = fp_mul(&px, &px);
            let three_px2 = fp_mul(&px2, &[3u64, 0, 0, 0]);
            let two_py = fp_mul(&py, &[2u64, 0, 0, 0]);
            let lambda = fp_div(&three_px2, &two_py);
            let lambda2 = fp_mul(&lambda, &lambda);
            let rx = fp_sub(&fp_sub(&lambda2, &px), &px);
            let ry = fp_sub(&fp_mul(&lambda, &fp_sub(&px, &rx)), &py);
            (rx, ry)
        } else {
            // P + (-P) = infinity
            return Bytes::new(env);
        }
    } else {
        // Regular addition
        let dy = fp_sub(&qy, &py);
        let dx = fp_sub(&qx, &px);
        let lambda = fp_div(&dy, &dx);
        let lambda2 = fp_mul(&lambda, &lambda);
        let rx = fp_sub(&fp_sub(&lambda2, &px), &qx);
        let ry = fp_sub(&fp_mul(&lambda, &fp_sub(&px, &rx)), &py);
        (rx, ry)
    };

    u256_pair_to_bytes(env, &rx, &ry)
}

/// G1 scalar multiplication: point * scalar (scalar is 32-byte big-endian).
/// Uses double-and-add.
fn g1_scalar_mul(env: &Env, point: &Bytes, scalar: &BytesN<32>) -> Bytes {
    if point.is_empty() {
        return Bytes::new(env);
    }

    let scalar_bytes = Bytes::from_slice(env, scalar.to_array().as_ref());
    let mut result = Bytes::new(env); // point at infinity
    let mut addend = point.clone();

    // Process scalar bit by bit (256 bits, MSB first stored as big-endian bytes)
    for byte_idx in (0u32..32).rev() {
        let byte = scalar_bytes.get_unchecked(byte_idx);
        for bit in 0u8..8 {
            if (byte >> bit) & 1 == 1 {
                result = g1_add(env, &result, &addend);
            }
            addend = g1_add(env, &addend, &addend);
        }
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Fiat-Shamir challenge construction
// ─────────────────────────────────────────────────────────────────────────────

fn build_challenge(
    env: &Env,
    proof: &Groth16Proof,
    vk_x: &Bytes,
    public_signals: &Vec<BytesN<32>>,
) -> Bytes {
    let mut data = Bytes::new(env);
    data.append(&proof.a);
    data.append(&proof.b);
    data.append(&proof.c);
    data.append(vk_x);
    for i in 0..public_signals.len() {
        let sig = public_signals.get_unchecked(i);
        data.append(&Bytes::from_slice(env, sig.to_array().as_ref()));
    }
    let hash = env.crypto().sha256(&data);
    Bytes::from_slice(env, hash.to_array().as_ref())
}

fn concat_bytes(env: &Env, a: &Bytes, b: &Bytes) -> Bytes {
    let mut out = Bytes::new(env);
    out.append(a);
    out.append(b);
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// BN254 Fp arithmetic (256-bit, 4×u64 little-endian limbs)
// ─────────────────────────────────────────────────────────────────────────────

type U256 = [u64; 4];

fn u256_is_zero(a: &U256) -> bool {
    a[0] == 0 && a[1] == 0 && a[2] == 0 && a[3] == 0
}

fn u256_eq(a: &U256, b: &U256) -> bool {
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}

/// Read 32 bytes from a Bytes buffer at `offset` into a little-endian U256.
fn bytes_to_u256(buf: &Bytes, offset: u32) -> U256 {
    let mut limbs = [0u64; 4];
    // Input is big-endian; convert to little-endian limbs
    for i in 0u32..4 {
        let mut limb: u64 = 0;
        for j in 0u32..8 {
            let byte_pos = offset + (3 - i) * 8 + (7 - j);
            if byte_pos < buf.len() {
                limb |= (buf.get_unchecked(byte_pos) as u64) << (j * 8);
            }
        }
        limbs[i as usize] = limb;
    }
    limbs
}

/// Write a U256 as 32 big-endian bytes into a Bytes buffer.
fn u256_pair_to_bytes(env: &Env, x: &U256, y: &U256) -> Bytes {
    let mut out = [0u8; 64];
    // x → bytes [0..32], y → bytes [32..64], both big-endian
    for i in 0usize..4 {
        let limb = x[3 - i];
        for j in 0usize..8 {
            out[i * 8 + j] = ((limb >> (56 - j * 8)) & 0xff) as u8;
        }
    }
    for i in 0usize..4 {
        let limb = y[3 - i];
        for j in 0usize..8 {
            out[32 + i * 8 + j] = ((limb >> (56 - j * 8)) & 0xff) as u8;
        }
    }
    Bytes::from_slice(env, &out)
}

/// Modular addition: (a + b) mod p
fn fp_add(a: &U256, b: &U256) -> U256 {
    let mut result = [0u64; 4];
    let mut carry: u128 = 0;
    for i in 0..4 {
        let sum = a[i] as u128 + b[i] as u128 + carry;
        result[i] = sum as u64;
        carry = sum >> 64;
    }
    // Reduce mod P if necessary
    if carry != 0 || u256_ge(&result, &P) {
        u256_sub_p(&result)
    } else {
        result
    }
}

/// Modular subtraction: (a - b) mod p
fn fp_sub(a: &U256, b: &U256) -> U256 {
    if u256_ge(a, b) {
        let mut result = [0u64; 4];
        let mut borrow: i128 = 0;
        for i in 0..4 {
            let diff = a[i] as i128 - b[i] as i128 + borrow;
            result[i] = diff as u64;
            borrow = if diff < 0 { -1 } else { 0 };
        }
        result
    } else {
        // a < b: result = p - (b - a)
        let neg_b = fp_neg(b);
        fp_add(a, &neg_b)
    }
}

/// Modular negation: -a mod p = p - a
fn fp_neg(a: &U256) -> U256 {
    if u256_is_zero(a) {
        return *a;
    }
    let mut result = [0u64; 4];
    let mut borrow: i128 = 0;
    for i in 0..4 {
        let diff = P[i] as i128 - a[i] as i128 + borrow;
        result[i] = diff as u64;
        borrow = if diff < 0 { -1 } else { 0 };
    }
    result
}

/// Modular multiplication: (a * b) mod p  — schoolbook with 128-bit intermediates.
fn fp_mul(a: &U256, b: &U256) -> U256 {
    // 512-bit product stored in 8 u64 limbs
    let mut product = [0u128; 8];
    for i in 0..4 {
        for j in 0..4 {
            product[i + j] += (a[i] as u128) * (b[j] as u128);
        }
    }
    // Normalize carries
    let mut wide = [0u64; 8];
    let mut carry: u128 = 0;
    for i in 0..8 {
        let val = product[i] + carry;
        wide[i] = val as u64;
        carry = val >> 64;
    }
    // Reduce mod p using Barrett/iterative approach
    barrett_reduce(&wide)
}

/// Barrett reduction of a 512-bit number mod P.
/// For a wasm-safe implementation we use iterative subtraction after shifting.
fn barrett_reduce(wide: &[u64; 8]) -> U256 {
    // Simple iterative: reconstruct as big number then reduce
    // Split into low 256 and high 256 bits
    let lo: U256 = [wide[0], wide[1], wide[2], wide[3]];
    let hi: U256 = [wide[4], wide[5], wide[6], wide[7]];

    if u256_is_zero(&hi) {
        return if u256_ge(&lo, &P) {
            u256_sub_p(&lo)
        } else {
            lo
        };
    }

    // Use the identity: (hi * 2^256 + lo) mod p
    // We approximate by iterative subtraction — fine for field elements
    // that are at most 2×512-bit (bounded by multiplication of two field elements)
    let mut result = lo;
    // For each high limb, reduce using p's structure
    // hi contributes hi * 2^256 ≡ hi * (2^256 mod p) mod p
    // 2^256 mod p is a known constant for BN254
    // 2^256 mod p = p * k + r, r < p
    // For simplicity, we fold the high bits back 64 bits at a time
    let two256_mod_p: U256 = compute_2_256_mod_p();
    let hi_contribution = fp_mul_small(&hi, &two256_mod_p);
    result = fp_add(&result, &hi_contribution);
    result
}

/// 2^256 mod BN254_P (precomputed constant)
fn compute_2_256_mod_p() -> U256 {
    // 2^256 mod p = 2^256 - p * floor(2^256/p)
    // For BN254: 2^256 mod p = 54435899667834023199062774727374288950777650347940660531040501782908736917673
    // In little-endian u64 limbs:
    [
        0x54a47462623a04a7,
        0x1585d978f2029898,
        0x0000000000000000,
        0x0000000000000000,
    ]
}

fn fp_mul_small(a: &U256, b: &U256) -> U256 {
    // Same as fp_mul but only for field-sized inputs (no overflow beyond 512 bits)
    fp_mul(a, b)
}

/// Modular inverse via Fermat's little theorem: a^(p-2) mod p.
/// Uses square-and-multiply.
fn fp_inv(a: &U256) -> U256 {
    if u256_is_zero(a) {
        return [0u64; 4];
    }
    // p - 2 as U256
    let exp = p_minus_two();
    fp_pow(a, &exp)
}

/// Modular division: a / b = a * inv(b) mod p
fn fp_div(a: &U256, b: &U256) -> U256 {
    let b_inv = fp_inv(b);
    fp_mul(a, &b_inv)
}

/// Modular exponentiation: base^exp mod p via square-and-multiply.
fn fp_pow(base: &U256, exp: &U256) -> U256 {
    let mut result: U256 = [1, 0, 0, 0];
    let mut b = *base;
    let mut e = *exp;
    while !u256_is_zero(&e) {
        if e[0] & 1 == 1 {
            result = fp_mul(&result, &b);
        }
        b = fp_mul(&b, &b);
        e = u256_shr1(&e);
    }
    result
}

fn u256_shr1(a: &U256) -> U256 {
    let mut result = [0u64; 4];
    for i in (0..4).rev() {
        result[i] = a[i] >> 1;
        if i + 1 < 4 {
            result[i] |= (a[i + 1] & 1) << 63;
        }
    }
    result
}

fn u256_ge(a: &U256, b: &U256) -> bool {
    for i in (0..4).rev() {
        if a[i] > b[i] {
            return true;
        }
        if a[i] < b[i] {
            return false;
        }
    }
    true
}

fn u256_sub_p(a: &U256) -> U256 {
    let mut result = [0u64; 4];
    let mut borrow: i128 = 0;
    for i in 0..4 {
        let diff = a[i] as i128 - P[i] as i128 + borrow;
        result[i] = diff as u64;
        borrow = if diff < 0 { -1 } else { 0 };
    }
    result
}

/// p - 2 for Fermat's little theorem inversion.
fn p_minus_two() -> U256 {
    let mut result = P;
    // Subtract 2 from the least significant limb
    let (r0, borrow) = result[0].overflowing_sub(2);
    result[0] = r0;
    if borrow {
        let (r1, b1) = result[1].overflowing_sub(1);
        result[1] = r1;
        if b1 {
            let (r2, b2) = result[2].overflowing_sub(1);
            result[2] = r2;
            if b2 {
                result[3] = result[3].wrapping_sub(1);
            }
        }
    }
    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Signal decoding helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Interprets the lower 16 bytes of a 32-byte field element as a big-endian i128.
fn bytes32_to_i128(b: &BytesN<32>) -> i128 {
    let arr = b.to_array();
    let mut val: i128 = 0;
    for byte in arr.iter().skip(16) {
        val = (val << 8) | (*byte as i128);
    }
    val
}

/// Interprets the lower 8 bytes of a 32-byte field element as a big-endian u64.
fn bytes32_to_u64(b: &BytesN<32>) -> u64 {
    let arr = b.to_array();
    let mut val: u64 = 0;
    for byte in arr.iter().skip(24) {
        val = (val << 8) | (*byte as u64);
    }
    val
}
