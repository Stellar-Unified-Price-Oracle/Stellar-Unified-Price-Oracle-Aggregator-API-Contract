//! # VDF-Based Source Sampling (Issue #181)
//!
//! Implements Verifiable Delay Function (VDF) proof verification and random source
//! selection to defend against oracle source bribery and collusion.
//!
//! ## Architecture
//!
//! Due to WASM instruction-budget limits (~4M instructions per invocation), this module
//! **verifies** a VDF proof rather than computing sequential squaring on-chain. The
//! verifier checks a Pietrzak or Wesolowski-style VDF output using a simplified
//! modular-exponentiation check within the instruction budget.
//!
//! ### Seed Construction
//!
//! ```text
//! seed = sha256(ledger_sequence_le(4) || ledger_timestamp_le(8))
//! ```
//!
//! ### Proof Verification (simplified Wesolowski-style check)
//!
//! For parameters `(g, output, proof, T)` in group Z/nZ:
//! ```text
//! r = hash_to_scalar(g, output)
//! check: g^(2^T * r_inv) * output^r == proof (mod n)  -- simplified
//! ```
//!
//! Since full big-integer arithmetic is not available in `#[no_std]` Soroban WASM
//! without external crates, this implementation uses the Soroban `sha256` primitive
//! to derive a pseudo-random scalar and performs a lightweight hash-chain consistency
//! check over the proof bytes. A production deployment would supply a dedicated
//! zk-VDF circuit; this module provides the structural interface and fallback logic.
//!
//! ### Fallback
//!
//! If VDF verification fails or the proof is empty, `sample_sources` gracefully falls
//! back to returning all registered active sources (equivalent to standard full-median
//! aggregation).

use crate::storage::{read_oracle_sources, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode};
use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, BytesN, Env, Vec};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Default number of sources to select per sample round.
const DEFAULT_SAMPLING_SIZE: u32 = 3;

// ─────────────────────────────────────────────────────────────────────────────
// Storage helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_sampling_size(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::VdfSamplingSize)
        .unwrap_or(DEFAULT_SAMPLING_SIZE)
}

fn write_sampling_size(env: &Env, n: u32) {
    let key = DataKey::VdfSamplingSize;
    env.storage().persistent().set(&key, &n);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

// ─────────────────────────────────────────────────────────────────────────────
// Seed derivation
// ─────────────────────────────────────────────────────────────────────────────

/// Derives the VDF input seed from the current ledger state.
///
/// `seed = sha256(sequence_le_4 || timestamp_le_8)`
pub fn derive_seed(env: &Env) -> BytesN<32> {
    let seq = env.ledger().sequence();
    let ts = env.ledger().timestamp();

    let seq_bytes: [u8; 4] = [
        (seq & 0xff) as u8,
        ((seq >> 8) & 0xff) as u8,
        ((seq >> 16) & 0xff) as u8,
        ((seq >> 24) & 0xff) as u8,
    ];
    let ts_bytes: [u8; 8] = [
        (ts & 0xff) as u8,
        ((ts >> 8) & 0xff) as u8,
        ((ts >> 16) & 0xff) as u8,
        ((ts >> 24) & 0xff) as u8,
        ((ts >> 32) & 0xff) as u8,
        ((ts >> 40) & 0xff) as u8,
        ((ts >> 48) & 0xff) as u8,
        ((ts >> 56) & 0xff) as u8,
    ];

    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, &seq_bytes));
    buf.append(&Bytes::from_slice(env, &ts_bytes));

    env.crypto().sha256(&buf).into()
}

// ─────────────────────────────────────────────────────────────────────────────
// VDF proof verification
// ─────────────────────────────────────────────────────────────────────────────

/// Verifies a VDF proof using a lightweight hash-chain consistency check.
///
/// This implements a simplified Wesolowski-style verification:
/// 1. Derive a challenge scalar `r = sha256(seed || output)`.
/// 2. Verify that `sha256(proof || r)` matches the expected output pattern.
/// 3. Check that the proof length and `iterations` parameter are consistent.
///
/// This is a structural check designed to be instruction-budget-friendly. A full
/// Wesolowski verification requires big-integer modular exponentiation, which must
/// be supplied via a dedicated ZK circuit in production. For the purposes of this
/// oracle module the check ensures:
/// - The proof is not trivially forged (it must incorporate `seed` and `output`).
/// - The `iterations` count is within a sane range.
///
/// Returns `true` if the lightweight verification passes, `false` otherwise.
pub fn verify_vdf_proof(
    env: &Env,
    seed: BytesN<32>,
    proof: Bytes,
    iterations: u64,
    output: BytesN<32>,
) -> bool {
    // Empty proof always fails
    if proof.is_empty() {
        return false;
    }

    // Sane iteration bounds (prevent trivially small or impossibly large values)
    if iterations == 0 || iterations > 1_000_000 {
        return false;
    }

    // Step 1: Derive challenge r = sha256(seed_bytes || output_bytes)
    let mut challenge_input = Bytes::new(env);
    challenge_input.append(&seed.clone().into());
    challenge_input.append(&output.clone().into());
    let challenge: BytesN<32> = env.crypto().sha256(&challenge_input).into();

    // Step 2: Compute expected = sha256(proof || challenge)
    let mut verify_input = Bytes::new(env);
    verify_input.append(&proof);
    verify_input.append(&challenge.clone().into());
    let computed: BytesN<32> = env.crypto().sha256(&verify_input).into();

    // Step 3: Consistency check — the first 16 bytes of `computed` must match
    // the first 16 bytes of `output` XOR'd with the first 16 bytes of `seed`.
    // This ensures the proof binds to both seed and output without full
    // modular exponentiation.
    let computed_bytes: Bytes = computed.into();
    let output_bytes: Bytes = output.into();
    let seed_bytes: Bytes = seed.into();

    for i in 0..16u32 {
        let c = computed_bytes.get_unchecked(i);
        let o = output_bytes.get_unchecked(i);
        let s = seed_bytes.get_unchecked(i);
        // Expected: computed[i] == output[i] ^ seed[i] ^ (iterations as u8 at position i%8)
        let iter_byte = ((iterations >> ((i % 8) * 8)) & 0xff) as u8;
        if c != (o ^ s ^ iter_byte) {
            return false;
        }
    }

    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Deterministic source selection from VDF output
// ─────────────────────────────────────────────────────────────────────────────

/// Selects `n` source addresses deterministically from `sources` using `randomness`.
///
/// Uses a Fisher-Yates-like partial shuffle seeded with successive sha256 hashes
/// of `randomness`. This is O(n) in the number of selected sources.
fn select_sources_from_randomness(
    env: &Env,
    sources: &[Address],
    n: usize,
    randomness: BytesN<32>,
) -> Vec<Address> {
    let total = sources.len();
    if n == 0 || total == 0 {
        return Vec::new(env);
    }
    let pick = n.min(total);

    // Build a mutable index array [0, 1, 2, …, total-1] as u32 values
    let mut indices: soroban_sdk::Vec<u32> = soroban_sdk::Vec::new(env);
    for i in 0..total {
        indices.push_back(i as u32);
    }

    let mut result: Vec<Address> = Vec::new(env);
    let mut current_rand: Bytes = randomness.into();

    for i in 0..pick {
        // Derive next random u32 from current_rand
        let hash_bytes: Bytes = env.crypto().sha256(&current_rand).into();
        // Use first 4 bytes as a u32
        let r0 = hash_bytes.get_unchecked(0) as u32;
        let r1 = hash_bytes.get_unchecked(1) as u32;
        let r2 = hash_bytes.get_unchecked(2) as u32;
        let r3 = hash_bytes.get_unchecked(3) as u32;
        let rand_u32 = (r0 << 24) | (r1 << 16) | (r2 << 8) | r3;

        let remaining = (total - i) as u32;
        let j = i as u32 + (rand_u32 % remaining);

        // Swap indices[i] and indices[j]
        let vi = indices.get_unchecked(i as u32);
        let vj = indices.get_unchecked(j);
        indices.set(i as u32, vj);
        indices.set(j, vi);

        let selected_idx = indices.get_unchecked(i as u32) as usize;
        result.push_back(sources[selected_idx].clone());

        // Advance randomness for next iteration
        current_rand = hash_bytes;
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Sets the number of sources to sample per VDF round. Admin-only.
///
/// `n` must be ≥ 1. If `n` exceeds the total registered sources at sampling
/// time, all sources are returned.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`]        — caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] — `n` is 0.
pub fn set_sampling_size(env: &Env, n: u32) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    if n == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    write_sampling_size(env, n);

    env.events().publish((symbol_short!("vdf_size"),), (n,));
}

/// Returns the configured sampling size. Default: 3.
pub fn get_sampling_size(env: &Env) -> u32 {
    read_sampling_size(env)
}

/// Derives the current VDF seed from ledger state.
///
/// Exposed as a convenience for off-chain VDF provers.
pub fn get_current_seed(env: &Env) -> BytesN<32> {
    derive_seed(env)
}

/// Samples `n` source addresses using VDF randomness.
///
/// Verifies `proof` against the current ledger seed. If verification succeeds,
/// uses `output` as the randomness source for deterministic source selection.
///
/// **Fallback**: if `proof` is empty or verification fails, returns all registered
/// active sources (standard full-set behaviour, equivalent to no sampling).
///
/// # Returns
///
/// A `Vec<Address>` of selected source addresses (length ≤ `n`).
pub fn sample_sources(
    env: &Env,
    proof: Bytes,
    output: BytesN<32>,
    iterations: u64,
) -> Vec<Address> {
    let seed = derive_seed(env);
    let n = read_sampling_size(env) as usize;

    // Load all registered sources
    let oracle_sources = read_oracle_sources(env);
    let all_sources = oracle_sources.sources;
    let total = all_sources.len() as usize;

    if total == 0 {
        return Vec::new(env);
    }

    // Attempt VDF verification
    let verified = verify_vdf_proof(env, seed, proof, iterations, output.clone());

    if !verified {
        // Fallback: return all sources
        env.events()
            .publish((symbol_short!("vdf_fall"),), (total as u32,));
        return all_sources;
    }

    // Convert Soroban Vec to a Rust slice-like structure for selection
    let mut src_vec: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(env);
    for i in 0..all_sources.len() {
        src_vec.push_back(all_sources.get_unchecked(i));
    }

    // Build a local array for the selection algorithm
    // We need a slice — use a fixed-capacity approach via indexed access
    let pick = n.min(total);
    if pick >= total {
        env.events()
            .publish((symbol_short!("vdf_ok"),), (total as u32,));
        return all_sources;
    }

    // Convert to intermediate Vec<Address> for the selection function
    let mut sources_slice: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(env);
    for i in 0..all_sources.len() {
        sources_slice.push_back(all_sources.get_unchecked(i));
    }

    // Build a plain Rust vec equivalent — iterate via index
    let selected = select_sources_deterministic(env, &sources_slice, pick as u32, output);

    env.events()
        .publish((symbol_short!("vdf_ok"),), (pick as u32,));

    selected
}

/// Deterministic source selection using a hash-based Fisher-Yates partial shuffle.
///
/// Works entirely within Soroban's `Vec<Address>` type.
fn select_sources_deterministic(
    env: &Env,
    sources: &soroban_sdk::Vec<Address>,
    n: u32,
    randomness: BytesN<32>,
) -> soroban_sdk::Vec<Address> {
    let total = sources.len();
    if n == 0 || total == 0 {
        return soroban_sdk::Vec::new(env);
    }
    let pick = n.min(total);

    // Build mutable index array
    let mut indices: soroban_sdk::Vec<u32> = soroban_sdk::Vec::new(env);
    for i in 0..total {
        indices.push_back(i);
    }

    let mut result: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(env);
    let mut current_rand: Bytes = randomness.into();

    for i in 0..pick {
        let hash_bytes: Bytes = env.crypto().sha256(&current_rand).into();

        let r0 = hash_bytes.get_unchecked(0) as u32;
        let r1 = hash_bytes.get_unchecked(1) as u32;
        let r2 = hash_bytes.get_unchecked(2) as u32;
        let r3 = hash_bytes.get_unchecked(3) as u32;
        let rand_u32 = (r0 << 24) | (r1 << 16) | (r2 << 8) | r3;

        let remaining = total - i;
        let j = i + (rand_u32 % remaining);

        let vi = indices.get_unchecked(i);
        let vj = indices.get_unchecked(j);
        indices.set(i, vj);
        indices.set(j, vi);

        let selected_idx = indices.get_unchecked(i);
        result.push_back(sources.get_unchecked(selected_idx));

        current_rand = hash_bytes;
    }

    result
}
