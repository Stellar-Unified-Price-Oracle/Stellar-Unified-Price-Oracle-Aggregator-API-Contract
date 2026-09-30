//! # Input canonicalization and validation — `input_validation.rs` (#506)
//!
//! Every public entrypoint that accepts a `String` or raw `Bytes` routes its
//! argument through this module first. It defines the single canonical form
//! for untrusted text and byte payloads and rejects — never truncates,
//! never normalizes — anything outside it.
//!
//! ## Canonical form
//!
//! **Text (`String`)** — the accepted alphabet is printable ASCII, encoded as
//! UTF-8. Every byte must be in `0x20..=0x7E` (space through tilde), and the
//! value must not begin or end with a space.
//!
//! Rejected: empty strings (where the field requires a value), leading or
//! trailing whitespace, C0/C1/DEL control characters, and all non-ASCII
//! bytes — which covers the Unicode confusables (zero-width joiners, bidi
//! overrides, homoglyphs) in the same rule rather than a fragile blocklist.
//! Because the accepted alphabet is one byte per character, two distinct
//! accepted values never compare equal on storage, so the "equal-looking
//! values compare unequal" class of bug cannot arise for text fields.
//!
//! This is a deliberately strict, single-charset policy. It is not a claim
//! that ASCII-only is the only reasonable policy — it is the policy the
//! contract commits to, enforced identically at every endpoint, so a stored
//! value always means exactly what the caller sent.
//!
//! **Length** — limits are measured in **bytes** (the unit the host charges
//! for and the unit the storage key encodes), not graphemes or code points.
//! `String::len()` in the Soroban SDK returns the UTF-8 byte length, so a cap
//! below is the number of bytes actually persisted. Because the canonical
//! alphabet is one byte per character, byte length and character count
//! coincide for every accepted value, and a boundary test therefore pins the
//! limit in both units at once.
//!
//! **Bytes (`Bytes`)** — opaque payloads are not text and are not
//! canonicalized, but they are bounded: a payload above the per-endpoint cap
//! is rejected before it reaches storage, which is what prevents an attacker
//! from growing ledger entries without bound.
//!
//! ## Errors
//!
//! | Condition                             | Error                             |
//! |---------------------------------------|-----------------------------------|
//! | over the length cap                   | [`ErrorCode::TextTooLong`]        |
//! | malformed (charset, empty, padding)   | [`ErrorCode::InvalidTextContent`] |
//! | `Bytes` payload over its cap          | [`ErrorCode::PayloadTooLarge`]    |
//!
//! The three conditions are distinct codes so a caller can tell an oversized
//! value from a malformed one without parsing a message.
//!
//! ## Storage-exhaustion rationale
//!
//! Every string or byte value that reaches persistent storage passes through
//! a cap here, so a caller can write at most `cap x (entries the endpoint
//! already rate-limits)` bytes per operation instead of an unbounded string
//! that could be replayed until the transaction fails for an unrelated
//! reason.
//!
//! ## Audit
//!
//! `input_validation_tests.rs` enumerates every string/`Bytes` parameter in
//! `lib.rs` and fails if a parameter is missing from [`STRING_PARAMS`] or
//! [`BYTES_PARAMS`], so a new endpoint cannot skip this module unnoticed.

use soroban_sdk::{panic_with_error, Bytes, Env, String};

use crate::types::ErrorCode;

// ────────────────────────────────────────────────────────────────────────────
// The audit tables
// ────────────────────────────────────────────────────────────────────────────

/// Per-endpoint classification of every `String` parameter accepted by a
/// public entrypoint. Each row names the endpoint, the parameter, its maximum
/// length **in bytes**, and whether the empty string is acceptable.
///
/// Caps are chosen per field meaning, not uniformly: an identifier that keys
/// storage is held to a short cap, a free-text reason to the existing 256-byte
/// budget, and the DID document — the only field intended to hold structured
/// content — to 4096 bytes.
pub const STRING_PARAMS: &[(&str, &str, u32, bool)] = &[
    // (endpoint, parameter, max_bytes, allow_empty)
    ("initialize", "description", 256, true),
    ("set_description", "new_description", 256, true),
    ("emergency_pause", "reason", 256, false),
    ("freeze_price", "reason", 256, false),
    ("override_price", "reason", 256, false),
    ("correct_price", "reason", 256, false),
    ("add_source", "name", 64, false),
    ("add_source_with_assets", "name", 64, false),
    ("propose_source", "name", 64, false),
    ("set_notification_preference", "channel", 256, false),
    ("set_notification_preference", "target", 256, false),
    ("set_source_verification", "verification_method", 64, false),
    ("set_source_diversity", "infra", 64, false),
    ("set_source_diversity", "upstream", 64, false),
    ("set_source_diversity", "owner", 64, false),
    ("create_operation", "op_id", 64, false),
    ("create_operation", "depends_on", 64, false),
    ("execute_dependent_operation", "op_id", 64, false),
    ("cancel_dependent_operation", "op_id", 64, false),
    ("get_operation_dependencies", "op_id", 64, false),
    ("get_operation_status", "op_id", 64, false),
    ("allow_governor_op", "operation", 64, false),
    ("disallow_governor_op", "operation", 64, false),
    ("is_governor_op_allowed", "operation", 64, false),
    ("set_lz_chain_name", "chain", 32, false),
    ("register_foreign_asset_mapping", "chain", 32, false),
    ("update_foreign_asset_mapping", "chain", 32, false),
    ("remove_foreign_asset_mapping", "chain", 32, false),
    ("get_foreign_asset_mapping", "chain", 32, false),
    ("submit_cross_chain_price", "chain_id", 32, false),
    ("set_axelar_trusted_source", "source_chain", 32, false),
    ("remove_axelar_trusted_source", "source_chain", 32, false),
    ("execute_axelar_message", "source_chain", 32, false),
    ("set_axelar_trusted_source", "source_address", 128, false),
    ("remove_axelar_trusted_source", "source_address", 128, false),
    ("execute_axelar_message", "source_address", 128, false),
    ("did_register", "document", 4096, false),
    ("add_relayer", "name", 64, false),
];

/// Every `Bytes` parameter accepted by a public entrypoint, with its maximum
/// length in bytes. Opaque payloads are not canonicalized, only bounded.
pub const BYTES_PARAMS: &[(&str, &str, u32)] = &[
    // (endpoint, parameter, max_bytes)
    ("challenge_price", "proof_data", 1024),
    ("challenge_relayed_submission", "proof_data", 1024),
    ("propose_operation", "data", 4096),
    ("propose_operation_with_priority", "data", 4096),
    ("ms_propose_operation", "data", 4096),
    ("reveal_price", "salt", 64),
    // Each batch element embeds a salt with the same 64-byte cap; the batch
    // count is separately bounded by `MAX_BATCH_REVEALS` in `prices.rs`.
    ("reveal_prices_batch", "reveals", 64),
    ("vdf_verify_proof", "proof", 1024),
    ("vdf_sample_sources", "proof", 1024),
    ("execute_axelar_message", "payload", 4096),
    ("lz_receive", "message", 4096),
    ("submit_price_merkle", "data", 4096),
];

// ────────────────────────────────────────────────────────────────────────────
// Validators
// ────────────────────────────────────────────────────────────────────────────

/// Returns the byte cap registered for `parameter` on `endpoint`, or `None`
/// if the pair is not in [`STRING_PARAMS`].
///
/// Called with a pair that is missing from the table the contract panics
/// with [`ErrorCode::InvalidTextContent`]: an unclassified parameter is
/// treated as malformed input rather than waved through.
pub fn cap_for(endpoint: &str, parameter: &str) -> Option<u32> {
    STRING_PARAMS
        .iter()
        .find(|(e, p, _, _)| *e == endpoint && *p == parameter)
        .map(|(_, _, cap, _)| *cap)
}

/// Returns whether `parameter` on `endpoint` accepts the empty string.
/// Defaults to `false` (reject) for an unclassified parameter.
pub fn allows_empty(endpoint: &str, parameter: &str) -> bool {
    STRING_PARAMS
        .iter()
        .find(|(e, p, _, _)| *e == endpoint && *p == parameter)
        .map(|(_, _, _, empty)| *empty)
        .unwrap_or(false)
}

/// Returns the byte cap registered for `parameter` on `endpoint`, or `None`.
pub fn bytes_cap_for(endpoint: &str, parameter: &str) -> Option<u32> {
    BYTES_PARAMS
        .iter()
        .find(|(e, p, _)| *e == endpoint && *p == parameter)
        .map(|(_, _, cap)| *cap)
}

/// True when every byte of `s` is printable ASCII (`0x20..=0x7E`).
///
/// Rejects C0 controls, DEL, C1 controls and every multi-byte UTF-8 sequence.
/// Non-ASCII rejection is what makes the canonical form normalization-free:
/// no accepted value has an alternative encoding.
pub fn is_canonical_ascii(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| (0x20..=0x7E).contains(b))
}

/// True when `bytes` is canonical ASCII and carries no leading or trailing
/// space.
///
/// Interior spaces are allowed (a reason may contain them); padding is not,
/// because `"a "` and `"a"` would otherwise be two different storage keys
/// for what a reader sees as one value.
pub fn is_canonical_text(bytes: &[u8]) -> bool {
    is_canonical_ascii(bytes) && !bytes.starts_with(b" ") && !bytes.ends_with(b" ")
}

/// Copies a Soroban `String` into a fixed buffer and returns how many bytes
/// were read. The buffer is sized well above the largest cap in
/// [`STRING_PARAMS`], so any input that fits the buffer is fully examined and
/// anything larger is rejected by the length check first.
const SCRATCH: usize = 8192;

fn to_bytes(value: &String) -> ([u8; SCRATCH], usize) {
    let mut buf = [0u8; SCRATCH];
    let n = core::cmp::min(value.len() as usize, SCRATCH);
    if n > 0 {
        value.copy_into_slice(&mut buf[..n]);
    }
    (buf, n)
}

/// Every `Vec` parameter whose elements are fixed-size (`BytesN<..>`, `Address`,
/// numeric), and therefore bounded by a **count** cap rather than a byte cap.
///
/// A vector of fixed-width elements cannot grow without bound per element, but
/// its *length* is caller-controlled and is charged to the ledger footprint, so
/// it needs the same treatment as an oversized `Bytes` payload. Count caps are
/// the domain limits the decoders already imply (a validator set larger than the
/// configured one, a reveal batch larger than the history window).
pub const LIST_PARAMS: &[(&str, &str, u32)] = &[
    // (endpoint, parameter, max_elements)
    //
    // Caps are the domain limits the surrounding code already implies where one
    // exists (a reveal batch no larger than the history window, a validator set
    // no larger than the consensus size); otherwise they are a conservative
    // storage bound. Every one of these is enforced by `validate_list_len` at
    // the endpoint, not merely documented here.
    ("zk_submit_price", "public_signals", 64),
    ("relay_verify_validator_set", "validators", 128),
    ("relay_verify_validator_set", "signatures", 128),
    ("relay_verify_event_proof", "proof", 128),
    // `MerkleProof` is `MerkleLeaf` + `Vec<BytesN<32>>` + `u32` — all fixed
    // width, so the proof count is the caller-controlled dimension.
    ("submit_price_merkle", "proofs", 64),
    ("simulate_aggregation", "hypothetical_prices", 128),
    // Address-only lists: bounded by the max-source ceiling (128).
    ("add_source_with_assets", "assets", 128),
    ("set_source_governance", "approvers", 32),
    ("ms_set_governors", "governors", 32),
    ("recovery_set_guardians", "guardians", 32),
    ("remove_sources", "sources", 128),
    // Batch endpoints: each item is a multi-field struct, so a tight cap keeps
    // the footprint bounded (this is the resource-exhaustion class, D6).
    ("propose_batch", "operations", 32),
    ("simulate_batch", "operations", 32),
    ("sc_submit_batch", "batch", 32),
    ("sc_dispute_channel", "last_known_batch", 32),
    ("submit_prices", "asset_prices", 64),
    ("submit_prices_relayed", "submissions", 64),
    ("get_storage_batch", "requests", 64),
];

/// Maximum number of dependency ids in a `Vec<String>` operation list.
pub const MAX_DEPENDENCY_COUNT: u32 = 32;

/// Returns the element cap registered for a list parameter, or `None`.
pub fn list_cap_for(endpoint: &str, parameter: &str) -> Option<u32> {
    LIST_PARAMS
        .iter()
        .find(|(e, p, _)| *e == endpoint && *p == parameter)
        .map(|(_, _, cap)| *cap)
}

/// Validates the **length** of a fixed-element list parameter.
///
/// `BytesN` payloads inside the list are fixed width and need no byte cap; what
/// an attacker controls is how many of them are supplied. An unclassified list
/// parameter is rejected rather than passed through, mirroring
/// [`validate_string`]'s treatment of an unknown text field.
pub fn validate_list_len(env: &Env, endpoint: &str, parameter: &str, len: u32) {
    let cap = list_cap_for(endpoint, parameter).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::InvalidTextContent);
    });
    if len > cap {
        panic_with_error!(env, ErrorCode::PayloadTooLarge);
    }
}

/// Validates a `String` parameter against the canonical form and its cap.
///
/// Order matters and is part of the contract: the length check runs first so
/// an oversized value is reported as [`ErrorCode::TextTooLong`] even when it
/// also contains a disallowed byte, and the charset check runs before the
/// emptiness check so a blank string is reported as malformed rather than as
/// merely missing.
///
/// Returns the input unchanged — this function never rewrites its argument.
pub fn validate_string(env: &Env, endpoint: &str, parameter: &str, value: &String) {
    let cap = cap_for(endpoint, parameter).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::InvalidTextContent);
    });
    // Soroban's `String::len()` is the UTF-8 byte length, i.e. the number of
    // bytes that will actually be written to the ledger.
    let len = value.len();
    if len > cap {
        panic_with_error!(env, ErrorCode::TextTooLong);
    }
    let (buf, n) = to_bytes(value);
    if !is_canonical_text(&buf[..n]) {
        panic_with_error!(env, ErrorCode::InvalidTextContent);
    }
    if len == 0 && !allows_empty(endpoint, parameter) {
        panic_with_error!(env, ErrorCode::InvalidTextContent);
    }
}

/// Validates a `Bytes` parameter against its cap.
///
/// Opaque payloads carry no charset requirement, so the only rule is size.
/// `Bytes::len()` is the byte length, which is also the storage cost.
pub fn validate_bytes(env: &Env, endpoint: &str, parameter: &str, value: &Bytes) {
    let cap = bytes_cap_for(endpoint, parameter).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::InvalidTextContent);
    });
    if value.len() > cap {
        panic_with_error!(env, ErrorCode::PayloadTooLarge);
    }
}

/// Validates every element of a `Vec<String>` dependency list, then the list
/// itself. A long list of individually-valid ids is still storage growth, so
/// the list length is bounded too.
pub fn validate_string_vec(
    env: &Env,
    endpoint: &str,
    parameter: &str,
    values: &soroban_sdk::Vec<String>,
) {
    if values.len() > MAX_DEPENDENCY_COUNT {
        panic_with_error!(env, ErrorCode::TextTooLong);
    }
    for i in 0..values.len() {
        validate_string(env, endpoint, parameter, &values.get_unchecked(i));
    }
}
