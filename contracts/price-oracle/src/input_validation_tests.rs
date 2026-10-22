#![cfg(test)]

//! Tests for the input canonicalization and validation audit (#506).
//!
//! The audit has three parts, and this suite covers all three:
//!
//! 1. **Completeness** — every `String`/`Bytes` parameter in `lib.rs` is
//!    classified in `STRING_PARAMS`/`BYTES_PARAMS`. The completeness tests
//!    parse the real `lib.rs` and fail if a parameter is added or removed
//!    without a matching table row, so the audit cannot silently rot.
//! 2. **Boundary** — each parameter's documented byte cap is tested at
//!    `cap` (accepted) and `cap + 1` (rejected), and at-limit values are
//!    shown to round-trip untruncated.
//! 3. **Adversarial** — empty, whitespace-padded, control-character,
//!    over-long Unicode and multi-byte inputs are rejected with the specific
//!    error code for their class.

use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, String, Vec};
// The crate is `no_std`; the test module still needs `alloc`'s formatting and
// `ToString` for building failure messages.
use std::string::ToString;

use crate::input_validation::{
    allows_empty, bytes_cap_for, cap_for, is_canonical_ascii, is_canonical_text, list_cap_for,
    validate_bytes, validate_list_len, validate_string, BYTES_PARAMS, LIST_PARAMS, STRING_PARAMS,
};
use crate::test_helpers::*;
use crate::types::ErrorCode;

/// Runs `validate_string` on a fresh `Env` and returns the contract error it
/// rejected with.
///
/// Validation panics by design (that is how the contract signals a bad
/// argument), so the panic is caught and its `Error(Contract, #N)` code
/// compared. A fresh `Env` is used per call because the panic unwinds the
/// environment that was passed in.
fn validate_expect_err(
    env: &Env,
    endpoint: &'static str,
    param: &'static str,
    value: &String,
) -> u32 {
    let bytes: std::vec::Vec<u8> = {
        let mut v = std::vec::Vec::new();
        let mut buf = [0u8; 8192];
        let n = core::cmp::min(value.len() as usize, buf.len());
        value.copy_into_slice(&mut buf[..n]);
        v.extend_from_slice(&buf[..n]);
        v
    };
    let e2 = Env::default();
    let v2 = String::from_str(&e2, &std::string::String::from_utf8_lossy(&bytes));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        validate_string(&e2, endpoint, param, &v2)
    }));
    match result {
        Ok(()) => panic!("{endpoint}::{param} unexpectedly accepted malformed input"),
        Err(payload) => code_from_panic(&payload),
    }
}

/// The codes the canonical-form validator can produce. Kept explicit so a
/// test can map a panic payload's discriminant back to the variant.
/// The `soroban_sdk::Error` a `panic_with_error!` surfaces for a given code.
fn code(c: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(c as u32)
}

const EXPECTED_CODES: &[ErrorCode] = &[
    ErrorCode::TextTooLong,
    ErrorCode::InvalidTextContent,
    ErrorCode::PayloadTooLarge,
];

/// A Soroban `String` of exactly `n` ASCII 'a' characters.
fn ascii(e: &Env, n: usize) -> String {
    let buf = [b'a'; 8192];
    String::from_str(e, core::str::from_utf8(&buf[..n]).unwrap())
}

// ────────────────────────────────────────────────────────────────────────────
// 1. Completeness — the audit table cannot drift from lib.rs
// ────────────────────────────────────────────────────────────────────────────

/// Every `String` parameter in the contract's public interface appears in
/// [`STRING_PARAMS`] with a cap and an emptiness policy.
#[test]
fn every_string_parameter_is_classified() {
    let lib = include_str!("lib.rs");
    let body = &lib[lib.find("impl PriceOracleContract {").unwrap()..];
    let listed: std::vec::Vec<(&str, &str)> =
        STRING_PARAMS.iter().map(|(e, p, _, _)| (*e, *p)).collect();

    let mut missing: std::vec::Vec<std::string::String> = std::vec::Vec::new();
    for (endpoint, params) in string_and_bytes_params(body) {
        for param in params {
            let is_string = param_is_string(&endpoint, &param);
            if is_string && !listed.contains(&(endpoint.as_str(), param.as_str())) {
                missing.push(format!("{endpoint}::{param}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "String parameters missing from STRING_PARAMS: {missing:?}"
    );
}

/// The reverse direction: the table must not list a parameter that no longer
/// exists, otherwise a removed endpoint would leave a stale cap behind.
#[test]
fn string_param_table_has_no_stale_rows() {
    let lib = include_str!("lib.rs");
    let body = &lib[lib.find("impl PriceOracleContract {").unwrap()..];
    let mut stale: std::vec::Vec<std::string::String> = std::vec::Vec::new();
    for (endpoint, param, _, _) in STRING_PARAMS {
        if !string_and_bytes_params(body)
            .iter()
            .any(|(e, ps)| e == endpoint && ps.iter().any(|p| p == param))
        {
            stale.push(format!("{endpoint}::{param}"));
        }
    }
    assert!(
        stale.is_empty(),
        "STRING_PARAMS lists parameters no longer in lib.rs: {stale:?}"
    );
}

/// Every `Bytes` parameter in the public interface is bounded.
#[test]
fn every_bytes_parameter_is_bounded() {
    let lib = include_str!("lib.rs");
    let body = &lib[lib.find("impl PriceOracleContract {").unwrap()..];
    let listed: std::vec::Vec<(&str, &str)> =
        BYTES_PARAMS.iter().map(|(e, p, _)| (*e, *p)).collect();

    let mut missing: std::vec::Vec<std::string::String> = std::vec::Vec::new();
    for (endpoint, params) in string_and_bytes_params(body) {
        for param in params {
            if param_is_bytes(&endpoint, &param)
                && !listed.contains(&(endpoint.as_str(), param.as_str()))
            {
                missing.push(format!("{endpoint}::{param}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "Bytes parameters missing from BYTES_PARAMS: {missing:?}"
    );
}

/// Every documented cap is non-zero, and a `Bytes` cap is never larger than
/// the corresponding `String` cap for the same endpoint (a byte payload that
/// could have been text is being treated as text).
#[test]
fn caps_are_sane() {
    for (endpoint, param, cap, _) in STRING_PARAMS {
        assert!(*cap > 0, "{endpoint}::{param} has a zero cap");
    }
    for (endpoint, param, cap) in BYTES_PARAMS {
        assert!(*cap > 0, "{endpoint}::{param} has a zero cap");
    }
    for (endpoint, param, cap) in LIST_PARAMS {
        assert!(*cap > 0, "{endpoint}::{param} has a zero element cap");
    }
}

/// Every public `Vec` parameter whose elements are fixed-size is registered in
/// [`LIST_PARAMS`], so a newly added batch or proof list cannot skip its count
/// cap.
///
/// This is the count-cap counterpart of `every_bytes_parameter_is_bounded`: it
/// only considers parameters whose declared type is a `Vec<...>` of a fixed
/// width, since a `Vec<String>` or `Vec<Bytes>` is covered by the text/byte
/// tables instead.
#[test]
fn every_fixed_element_list_parameter_is_count_capped() {
    let lib = include_str!("lib.rs");
    let body = &lib[lib.find("impl PriceOracleContract {").unwrap()..];

    let mut missing: std::vec::Vec<std::string::String> = std::vec::Vec::new();
    for (endpoint, params) in all_params(body) {
        for (pname, pty) in params {
            if !is_fixed_element_list(&pty) {
                continue;
            }
            if list_cap_for(&endpoint, &pname).is_none() {
                missing.push(format!("{endpoint}::{pname}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "fixed-element Vec parameters missing from LIST_PARAMS: {missing:?}"
    );

    // The reverse direction: no stale list rows.
    let mut stale: std::vec::Vec<std::string::String> = std::vec::Vec::new();
    for (endpoint, param, _) in LIST_PARAMS {
        let ty = param_type(endpoint, param);
        match ty {
            Some(t) if is_fixed_element_list(&t) => {}
            _ => stale.push(format!("{endpoint}::{param}")),
        }
    }
    assert!(
        stale.is_empty(),
        "LIST_PARAMS lists parameters that are not fixed-element Vecs: {stale:?}"
    );
}

/// A `Vec<T>` where `T` is a fixed-width type, so the only caller-controlled
/// dimension is the element count.
///
/// Excluded, because a *variable-length element anywhere in the vector* makes
/// the byte total caller-controlled and it belongs in the text/byte tables
/// instead:
///
/// * `Vec<String>` / `Vec<Bytes>` — variable-width elements.
/// * `Vec<(Address, i128, Bytes, u32)>` — `reveals` is such a tuple, and
///   `Vec<prices::MerkleProof>` nests `Bytes` inside a struct. These must be
///   classified by the byte rule, not by element count, or a per-element byte
///   cap would be missed.
/// * `Vec<BatchOperation>` / `Vec<BatchItem>` — nested collections whose own
///   length is the caller-controlled dimension.
fn is_fixed_element_list(ty: &str) -> bool {
    let ty = ty.trim();
    let inner = match ty
        .strip_prefix("soroban_sdk::Vec<")
        .or_else(|| ty.strip_prefix("Vec<"))
    {
        Some(rest) => rest.strip_suffix('>').unwrap_or(rest),
        None => return false,
    };
    // Any nested `Vec<` means a nested list: bounded by its own row, not this.
    if inner.contains("Vec<") {
        return false;
    }
    // Any nested variable-width container, at any nesting depth, disqualifies
    // the whole vector from being "fixed width".
    if inner.contains("String") || inner.contains("Bytes") && !is_only_bytesn(inner) {
        return false;
    }
    true
}

/// True when every occurrence of `Bytes` in the type is part of a fixed-width
/// `BytesN<..>`, so no unbounded element hides inside the vector element type.
fn is_only_bytesn(inner: &str) -> bool {
    let mut rest = inner;
    while let Some(i) = rest.find("Bytes") {
        let tail = &rest[i + "Bytes".len()..];
        // `BytesN<32>` is fixed width; a bare `Bytes` (optionally plural) is not.
        if !tail.starts_with('N') {
            return false;
        }
        rest = tail;
    }
    true
}

/// A list parameter at its cap is accepted; one element over is refused with
/// `PayloadTooLarge`. Asserted on the validator directly, since building a
/// 128-element SDK vector for every endpoint would dominate the test runtime.
#[test]
fn list_count_cap_boundary_is_exact() {
    let e = Env::default();
    for (endpoint, param, cap) in LIST_PARAMS {
        validate_list_len(&e, endpoint, param, *cap);
        let over = cap + 1;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            validate_list_len(&e, endpoint, param, over)
        }));
        assert!(
            result.is_err(),
            "{endpoint}::{param} must reject a list of {over} (cap {cap})"
        );
        assert_eq!(
            code_from_panic(&result.unwrap_err()),
            ErrorCode::PayloadTooLarge as u32,
            "{endpoint}::{param} must report PayloadTooLarge when over the count cap"
        );
    }
}

// ────────────────────────────────────────────────────────────────────────────
// lib.rs parsing helpers (shared by the completeness tests above)
// ────────────────────────────────────────────────────────────────────────────

/// A parameter of a public entrypoint: its name and its declared type text.
type Param = (std::string::String, std::string::String);

/// Extract `(endpoint, [param, ...])` for every `pub fn` in the contract impl
/// block, restricted to parameters whose declared type mentions `String` or
/// `Bytes`. Returned as owned `String`s because the crate is `no_std` in
/// non-test builds.
///
/// Signatures are joined across lines first: most endpoints in this contract
/// are written as a multi-line parameter list, and a line-at-a-time parse
/// silently misses every one of them.
fn string_and_bytes_params(
    body: &str,
) -> std::vec::Vec<(std::string::String, std::vec::Vec<std::string::String>)> {
    all_params(body)
        .into_iter()
        .map(|(name, params)| {
            let kept: std::vec::Vec<std::string::String> = params
                .into_iter()
                .filter(|(_, ty)| ty_mentions_bytes_or_text(ty))
                .map(|(pname, _)| pname)
                .collect();
            (name, kept)
        })
        .filter(|(_, ps)| !ps.is_empty())
        .collect()
}

/// Every parameter of every public entrypoint, with its declared type.
fn all_params(body: &str) -> std::vec::Vec<(std::string::String, std::vec::Vec<Param>)> {
    let mut out = std::vec::Vec::new();
    let lines: std::vec::Vec<&str> = body.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        // Only top-level entrypoints of the contract impl block; nested helpers
        // inside a function body are indented further and are not the surface.
        let rest = match trimmed.strip_prefix("pub fn ") {
            Some(r) => r,
            None => {
                i += 1;
                continue;
            }
        };
        if !lines[i].starts_with("    pub fn ") {
            i += 1;
            continue;
        }
        let name = rest
            .split(['(', '<', ' '])
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();

        // Accumulate the signature until the parameter list closes. Angle
        // brackets and nested generics are counted so `Vec<Env, Foo>` style
        // input cannot end the list early.
        let mut sig = std::string::String::new();
        let mut depth = 0i32;
        let mut closed = false;
        while i < lines.len() {
            let line = lines[i];
            for c in line.chars() {
                match c {
                    '(' | '<' | '[' => depth += 1,
                    ')' | '>' | ']' => depth -= 1,
                    _ => {}
                }
            }
            sig.push_str(line);
            sig.push(' ');
            i += 1;
            if depth <= 0 && sig.contains('(') {
                closed = true;
                break;
            }
        }
        if !closed {
            continue;
        }

        let open = match sig.find('(') {
            Some(k) => k,
            None => continue,
        };
        // The signature is now balanced, so the *last* ')' terminates the
        // parameter list (the return type may itself contain parentheses).
        let close = match sig.rfind(')') {
            Some(k) if k > open => k,
            _ => continue,
        };

        let mut collected = std::vec::Vec::new();
        for part in split_top_level(&sig[open + 1..close]) {
            let part = part.trim();
            let (pname, ptype) = match part.split_once(':') {
                Some(x) => x,
                None => continue,
            };
            let pname = pname.trim();
            let ptype = ptype.trim();
            if pname.is_empty() || pname == "env" || !is_plain_ident(pname) {
                continue;
            }
            collected.push((pname.to_string(), ptype.to_string()));
        }
        out.push((name, collected));
    }
    out
}

/// True when a declared type is a bare identifier (`price`, `reason`) rather
/// than a pattern binding or destructuring form.
fn is_plain_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.chars().next().unwrap().is_ascii_digit()
}

/// True when the declared type carries text or byte-payload content.
///
/// `BytesN<N>` is deliberately **excluded**: it is a fixed-size hash or key
/// (`BytesN<32>`, `BytesN<4>`), so it cannot grow with attacker input and needs
/// no length cap. Only variable-length `Bytes` and `String` (including
/// `Vec<String>`) are in scope for this audit.
///
/// A `Vec<...>` of fixed-width elements is also excluded: its elements cannot
/// grow, and the caller-controlled dimension is the element count, which
/// `LIST_PARAMS` bounds instead. A `Vec` that nests a variable-width element
/// (e.g. `Vec<(Address, i128, Bytes, u32)>`) *is* included, because the total
/// byte count is then caller-controlled.
fn ty_mentions_bytes_or_text(ty: &str) -> bool {
    if is_bytesn(ty) {
        return false;
    }
    if ty.contains("Vec<") && is_fixed_element_list(ty) {
        return false;
    }
    ty.contains("String") || ty.contains("Bytes")
}

/// True when a type is the fixed-size `BytesN<..>` form, tolerating the
/// `soroban_sdk::` path prefix that `#[contractimpl]` signatures use.
fn is_bytesn(ty: &str) -> bool {
    let ty = ty.trim();
    let ty = ty.strip_prefix("soroban_sdk::").unwrap_or(ty);
    let ty = ty.strip_prefix("crate::").unwrap_or(ty);
    ty.starts_with("BytesN<") || ty == "BytesN"
}

/// The declared type of `endpoint::param`, or `None` when no such parameter
/// exists. Resolving the type this way — rather than guessing from the
/// parameter *name* — is what keeps `Bytes` and `BytesN` apart.
fn param_type(endpoint: &str, param: &str) -> Option<std::string::String> {
    let lib = include_str!("lib.rs");
    let body = &lib[lib.find("impl PriceOracleContract {").unwrap()..];
    all_params(body)
        .into_iter()
        .find(|(e, _)| e == endpoint)
        .and_then(|(_, params)| {
            params
                .into_iter()
                .find(|(p, _)| p == param)
                .map(|(_, ty)| ty)
        })
}

/// Split a parameter list on commas that are not nested inside `<...>`.
fn split_top_level(params: &str) -> std::vec::Vec<&str> {
    let mut out = std::vec::Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in params.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&params[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&params[start..]);
    out
}

/// True when `endpoint::param` is declared as text (`String`, including
/// `Vec<String>`) rather than as a byte payload.
fn param_is_string(endpoint: &str, param: &str) -> bool {
    match param_type(endpoint, param) {
        Some(ty) => !is_bytesn(&ty) && ty.contains("String"),
        None => false,
    }
}

/// Byte-payload counterpart of [`param_is_string`]. Fixed-size `BytesN<..>`
/// parameters return `false`: they cannot grow and are out of scope.
fn param_is_bytes(endpoint: &str, param: &str) -> bool {
    match param_type(endpoint, param) {
        Some(ty) => !is_bytesn(&ty) && ty.contains("Bytes"),
        None => false,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// 2. Boundary behaviour of the validators themselves
// ────────────────────────────────────────────────────────────────────────────

/// A value of exactly `cap` bytes is accepted; `cap + 1` is refused.
///
/// The oversized case is driven through the client so the real contract error
/// code is observed rather than a caught panic. `add_source` is used as the
/// driver because its name is the one persisted field reachable without any
/// extra fixture state.
///
/// Note the expected code: `add_source` predates the shared validator and
/// carries its own `MAX_SOURCE_NAME_LENGTH` guard, which runs first and reports
/// the endpoint-specific [`ErrorCode::SourceNameTooLong`]. Both codes reject
/// the same input for the same reason (over-length); the audit does not
/// overwrite an existing, more specific error to impose a generic one. The
/// shared validator's own length boundary is pinned by
/// `every_cap_accepts_a_value_of_exactly_that_length` and
/// `oversized_text_is_rejected_as_text_too_long`.
#[test]
fn text_cap_boundary_is_exact() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let cap = cap_for("add_source", "name").unwrap();

    // Exactly at the cap: accepted and stored untruncated.
    let at_cap = ascii(&e, cap as usize);
    let source = Address::generate(&e);
    client.add_source(&source, &at_cap);
    let stored = client.get_oracle_sources().metadata.get(source).unwrap();
    assert_eq!(stored.len(), cap, "at-cap value is stored in full");
    assert_eq!(stored, at_cap);

    // One byte over: refused, with a length-specific code.
    let over = ascii(&e, cap as usize + 1);
    assert_eq!(over.len(), cap + 1);
    let other = Address::generate(&e);
    // `try_*` returns Result<_, Result<Error, InvokeError>>: the outer layer
    // is the host result, the inner one the contract error.
    let err = client.try_add_source(&other, &over).unwrap_err();
    assert_eq!(
        err,
        Ok(code(ErrorCode::SourceNameTooLong)),
        "cap+1 must be rejected with the endpoint's length error"
    );
}

/// Every classified parameter accepts a value at its own cap, and the
/// emptiness flag in the table matches what the lookup the validator uses
/// reports. This is the test that fails first if a cap or an emptiness flag is
/// changed without a decision.
///
/// Emptiness is deliberately *not* asserted through `is_canonical_text`: that
/// predicate answers "is this the canonical alphabet with no padding", which is
/// vacuously true for the empty string. Whether an empty value is acceptable
/// is a per-parameter policy, so it is checked against `allows_empty` — the
/// same function `validate_string` consults. `empty_string_policy_is_enforced`
/// then exercises the enforcement path itself.
#[test]
fn every_cap_accepts_a_value_of_exactly_that_length() {
    let e = Env::default();
    for (endpoint, param, cap, allow_empty) in STRING_PARAMS {
        let at_cap = ascii(&e, *cap as usize);
        assert_eq!(
            at_cap.len(),
            *cap,
            "{endpoint}::{param} fixture must be exactly at the cap"
        );
        // A value at the cap is canonical and must be accepted.
        validate_string(&e, endpoint, param, &at_cap);

        // The table's flag and the validator's lookup must not disagree.
        assert_eq!(
            allows_empty(endpoint, param),
            *allow_empty,
            "{endpoint}::{param} emptiness policy disagrees with allows_empty"
        );
        // An unclassified pair must not silently report "empty is fine".
        assert!(
            !allows_empty("no_such_endpoint", param),
            "an unclassified endpoint must default to rejecting empty"
        );
    }
}

/// The empty string is refused for every field that requires a value, and
/// accepted for the two free-text fields that permit it.
#[test]
fn empty_string_policy_is_enforced() {
    let e = Env::default();
    for (endpoint, param, _cap, allow_empty) in STRING_PARAMS {
        let empty = String::from_str(&e, "");
        if *allow_empty {
            validate_string(&e, endpoint, param, &empty);
        } else {
            let err = validate_expect_err(&e, endpoint, param, &empty);
            assert_eq!(
                err,
                ErrorCode::InvalidTextContent as u32,
                "{endpoint}::{param} must reject the empty string"
            );
        }
    }
}

/// A value is never truncated: at-cap input round-trips byte-for-byte.
#[test]
fn at_cap_value_is_not_truncated() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let name = ascii(&e, 64);
    let source = Address::generate(&e);
    client.add_source(&source, &name);
    let stored = client.get_oracle_sources().metadata.get(source).unwrap();
    assert_eq!(
        stored.len(),
        64,
        "stored length must equal the submitted length"
    );
    assert_eq!(stored, name, "stored value must equal the submitted value");
}

// ────────────────────────────────────────────────────────────────────────────
// 3. Adversarial inputs
// ────────────────────────────────────────────────────────────────────────────

/// Each malformed class maps to its own error code, so a caller can tell
/// "too big" from "not a legal value" without parsing a message.
///
/// Each case runs on a fresh `Env`: a rejected validation panics, and the
/// panic unwinds the environment, so the same `Env` cannot be reused.
#[test]
fn malformed_and_oversized_are_distinct_errors() {
    // Oversized → TextTooLong.
    let e = Env::default();
    let over = ascii(&e, 65);
    assert_eq!(
        validate_expect_err(&e, "add_source", "name", &over),
        ErrorCode::TextTooLong as u32
    );

    // Control character → InvalidTextContent (not TextTooLong).
    let e = Env::default();
    let control = String::from_str(&e, "a\u{7}b");
    assert_eq!(
        validate_expect_err(&e, "add_source", "name", &control),
        ErrorCode::InvalidTextContent as u32
    );

    // Whitespace padding → InvalidTextContent.
    for padded in [" a", "a "] {
        let e = Env::default();
        let s = String::from_str(&e, padded);
        assert_eq!(
            validate_expect_err(&e, "add_source", "name", &s),
            ErrorCode::InvalidTextContent as u32,
            "padded {padded:?} must be rejected"
        );
    }
}

/// Byte cap is measured in bytes, so a multi-byte character that *looks*
/// short can still exceed the cap. `MAX` is 64 bytes: 32 two-byte characters
/// fit, 33 do not.
#[test]
fn multibyte_length_is_measured_in_bytes() {
    let e = Env::default();
    // Each 'é' is two UTF-8 bytes.
    let fits: std::string::String = std::iter::repeat('é').take(32).collect();
    let overflows: std::string::String = std::iter::repeat('é').take(33).collect();
    assert_eq!(fits.len(), 64);
    assert_eq!(overflows.len(), 66);

    // 64 bytes is exactly at the cap and is non-ASCII, so it is rejected for
    // charset rather than length.
    let s = String::from_str(&e, &fits);
    assert_eq!(
        validate_expect_err(&e, "add_source", "name", &s),
        ErrorCode::InvalidTextContent as u32,
        "non-ASCII is rejected on charset even when it fits the cap"
    );

    // 66 bytes is both oversized and non-ASCII; length is checked first.
    let s = String::from_str(&e, &overflows);
    assert_eq!(
        validate_expect_err(&e, "add_source", "name", &s),
        ErrorCode::TextTooLong as u32,
        "the length check runs before the charset check"
    );
}

/// Unicode confusables — zero-width space, bidi override, a look-alike
/// Cyrillic 'а' — are all rejected, so two visually identical strings can
/// never be stored under different keys.
#[test]
fn unicode_confusables_are_rejected() {
    let e = Env::default();
    for (label, s) in [
        ("zero-width space", "ab\u{200B}c"),
        ("bidi override", "abc\u{202E}"),
        ("cyrillic a", "bcа"),
        ("fullwidth a", "ｂｃ"),
        ("combining accent", "e\u{0301}"),
        ("emoji", "bc🙂"),
    ] {
        let v = String::from_str(&e, s);
        assert_eq!(
            validate_expect_err(&e, "add_source", "name", &v),
            ErrorCode::InvalidTextContent as u32,
            "{label} must be rejected"
        );
    }
}

/// Printable ASCII with interior spaces is accepted — the canonical form is
/// not "no spaces at all", it is "no padding".
#[test]
fn interior_spaces_are_accepted() {
    let e = Env::default();
    let v = String::from_str(&e, "price out of band");
    validate_string(&e, "override_price", "reason", &v);
}

/// DEL (`0x7F`) and the C0 range are rejected even though they are single
/// bytes, so a stored value can never contain a terminal control sequence.
#[test]
fn control_characters_are_rejected() {
    let e = Env::default();
    for byte in [0x00u8, 0x07, 0x09, 0x0A, 0x0D, 0x1B, 0x7F] {
        let raw = [b'a', byte, b'b'];
        let s = core::str::from_utf8(&raw).unwrap();
        let v = String::from_str(&e, s);
        assert_eq!(
            validate_expect_err(&e, "add_source", "name", &v),
            ErrorCode::InvalidTextContent as u32,
            "byte {byte:#04x} must be rejected"
        );
    }
}

// ────────────────────────────────────────────────────────────────────────────
// 4. Byte payloads
// ────────────────────────────────────────────────────────────────────────────

/// A `Bytes` payload of exactly its cap is accepted; `cap + 1` is
/// `PayloadTooLarge` — a different code from the text path, so a caller can
/// tell a blob that is too big from a string that is too big.
#[test]
fn bytes_cap_boundary_is_exact_and_distinct() {
    let e = Env::default();
    let cap = bytes_cap_for("challenge_price", "proof_data").unwrap() as usize;

    let at_cap = Bytes::from_slice(&e, &[7u8; 1024]);
    assert_eq!(at_cap.len(), 1024);
    validate_bytes(&e, "challenge_price", "proof_data", &at_cap);

    // One byte over: refused with the payload code, which is distinct from
    // the text length code so a caller can tell the two apart.
    let e = Env::default();
    let over = Bytes::from_slice(&e, &[7u8; 1025]);
    assert_eq!(
        bytes_expect_err("challenge_price", "proof_data", over.len() as u32),
        ErrorCode::PayloadTooLarge as u32,
        "an oversized byte payload must be PayloadTooLarge, not TextTooLong"
    );
}

/// Runs `validate_bytes` with an all-`0x01` payload of `len` bytes on a fresh
/// `Env` and returns the error it rejected with.
fn bytes_expect_err(endpoint: &'static str, param: &'static str, len: u32) -> u32 {
    let e = Env::default();
    let payload = Bytes::from_slice(&e, &vec![1u8; len as usize]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        validate_bytes(&e, endpoint, param, &payload)
    }));
    match result {
        Ok(()) => panic!("{endpoint}::{param} unexpectedly accepted an oversized payload"),
        Err(payload) => code_from_panic(&payload),
    }
}

/// Extracts the `ErrorCode` from a caught `panic_with_error!` payload.
fn code_from_panic(payload: &std::boxed::Box<dyn core::any::Any + Send>) -> u32 {
    let msg = if let Some(s) = payload.downcast_ref::<std::string::String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        panic!("panic payload is not a string");
    };
    let start = msg
        .find("#")
        .unwrap_or_else(|| panic!("no error code in panic payload: {msg}"))
        + 1;
    let digits: std::string::String = msg[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let code: u32 = digits.parse().expect("numeric code");
    // Guard against the payload coming from some other panic: only the codes
    // this module can raise are accepted.
    assert!(
        EXPECTED_CODES.iter().any(|c| c.clone() as u32 == code),
        "panic payload carried an unexpected error code: {code}"
    );
    code
}

/// Every classified byte parameter accepts a payload at its own cap.
#[test]
fn every_bytes_cap_accepts_a_value_of_exactly_that_length() {
    let e = Env::default();
    for (endpoint, param, cap) in BYTES_PARAMS {
        let at_cap = Bytes::from_slice(&e, &vec![1u8; *cap as usize]);
        assert_eq!(at_cap.len(), *cap);
        validate_bytes(&e, endpoint, param, &at_cap);
    }
}

// ────────────────────────────────────────────────────────────────────────────
// 5. Storage exhaustion
// ────────────────────────────────────────────────────────────────────────────

/// A rejected oversized value writes nothing to storage: the call fails
/// before the entry is created, so an attacker cannot grow a ledger entry
/// with a string the contract refuses to accept.
#[test]
fn oversized_name_writes_nothing_to_storage() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = Address::generate(&e);
    let over = ascii(&e, 65);

    let result = client.try_add_source(&source, &over);
    assert!(result.is_err(), "an oversized name must be refused");

    let sources = client.get_oracle_sources();
    assert!(
        !sources.sources.contains(&source),
        "a refused source must not be registered"
    );
    assert!(
        !sources.metadata.get(source).is_some(),
        "a refused name must not be stored"
    );
}

/// The same for an unbounded `Vec<String>` dependency list: a long list of
/// individually-valid ids is still storage growth, so the list length is
/// capped too.
#[test]
fn oversized_dependency_list_is_refused() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let mut deps: Vec<String> = Vec::new(&e);
    for i in 0..64 {
        deps.push_back(String::from_str(&e, &format!("dep{i}")));
    }
    let op = String::from_str(&e, "op1");
    assert!(client.try_create_operation(&op, &deps).is_err());

    // A short list still works, so the cap is a bound and not a ban.
    let mut few: Vec<String> = Vec::new(&e);
    few.push_back(String::from_str(&e, "dep0"));
    let op2 = String::from_str(&e, "op2");
    assert!(client.try_create_operation(&op2, &few).is_ok());
}

/// Unvalidated endpoints used to store these strings without a cap. Now that
/// the check is wired in, an oversized chain name is refused at the
/// endpoint rather than reaching the storage key.
#[test]
fn oversized_chain_name_is_refused_at_the_endpoint() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let over = ascii(&e, 33); // chain cap is 32
    assert!(client.try_set_lz_chain_name(&1u32, &over).is_err());
}

#[test]
fn oversized_governor_operation_is_refused_at_the_endpoint() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let over = ascii(&e, 65); // operation cap is 64
    assert!(client.try_allow_governor_op(&over).is_err());
}
