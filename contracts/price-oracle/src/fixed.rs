//! # Canonical fixed-point arithmetic (#819)
//!
//! One rounding mode and one scaling helper for **all** price math. Every
//! multiplication/division on a price value must go through [`mul_div`] or
//! [`rescale`]; ad-hoc `*` / `/` on prices is a review defect.
//!
//! ## Rounding
//!
//! [`Rounding`] names the direction explicitly at every call site, so the
//! rounding of an endpoint is a property of its code, not of an accident of
//! integer truncation:
//!
//! | Mode | Result |
//! |---|---|
//! | [`Rounding::Down`] | toward negative infinity (floor) |
//! | [`Rounding::Up`] | toward positive infinity (ceil) |
//! | [`Rounding::TowardZero`] | truncation, the historical behaviour |
//! | [`Rounding::Nearest`] | nearest, ties away from zero |
//!
//! ## Overflow
//!
//! The product `a * b` is formed in `i128` with `checked_mul`; a product that
//! does not fit is refused with [`FixedError::Overflow`] rather than wrapped or
//! saturated. A zero divisor is [`FixedError::DivisionByZero`], and a quotient
//! outside `i128` is [`FixedError::Overflow`]. Callers map these onto their own
//! `ErrorCode` (see `derived_feeds::mul_div`), so this module stays `Env`-free
//! and is directly provable with Kani.

/// Rounding direction for a fixed-point division.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Toward negative infinity.
    Down,
    /// Toward positive infinity.
    Up,
    /// Toward zero (truncation).
    TowardZero,
    /// Nearest integer, ties away from zero.
    Nearest,
}

/// Failure of a fixed-point operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedError {
    /// The intermediate product or the final quotient does not fit `i128`.
    Overflow,
    /// The divisor was zero.
    DivisionByZero,
    /// A decimal count was outside the supported range.
    InvalidDecimals,
}

/// Largest decimal count [`rescale`] accepts: `10^38` still fits `i128`.
pub const MAX_DECIMALS: u32 = 38;

/// `10^decimals` as an `i128`, or [`FixedError::InvalidDecimals`].
pub fn pow10(decimals: u32) -> Result<i128, FixedError> {
    if decimals > MAX_DECIMALS {
        return Err(FixedError::InvalidDecimals);
    }
    let mut scale: i128 = 1;
    let mut i = 0;
    while i < decimals {
        scale = match scale.checked_mul(10) {
            Some(s) => s,
            None => return Err(FixedError::InvalidDecimals),
        };
        i += 1;
    }
    Ok(scale)
}

/// `a * b / c`, rounded as requested, with overflow checks.
///
/// The product is formed first (checked), then divided with the requested
/// rounding. `c == 0` is [`FixedError::DivisionByZero`].
pub fn mul_div(a: i128, b: i128, c: i128, rounding: Rounding) -> Result<i128, FixedError> {
    if c == 0 {
        return Err(FixedError::DivisionByZero);
    }
    let product = match a.checked_mul(b) {
        Some(p) => p,
        None => return Err(FixedError::Overflow),
    };
    let quotient = product / c;
    let remainder = product % c;
    if remainder == 0 {
        return Ok(quotient);
    }
    // `remainder` has the sign of `product`; `c` may be negative, so compare
    // magnitudes to decide whether the exact result lies above the quotient.
    let exact_is_above = (remainder > 0) == (c > 0);
    let step = match rounding {
        Rounding::TowardZero => 0,
        Rounding::Down => {
            if exact_is_above {
                0
            } else {
                -1
            }
        }
        Rounding::Up => {
            if exact_is_above {
                1
            } else {
                0
            }
        }
        Rounding::Nearest => {
            // |remainder| * 2 >= |c| means the fraction is at least one half.
            let twice = match remainder.unsigned_abs().checked_mul(2) {
                Some(t) => t,
                None => return Err(FixedError::Overflow),
            };
            if twice >= c.unsigned_abs() {
                if exact_is_above {
                    1
                } else {
                    -1
                }
            } else {
                0
            }
        }
    };
    match quotient.checked_add(step) {
        Some(q) => Ok(q),
        None => Err(FixedError::Overflow),
    }
}

/// Re-expresses `value` from `from_dec` decimals to `to_dec` decimals.
///
/// Scaling *up* multiplies by `10^(to_dec - from_dec)` (exact, checked);
/// scaling *down* divides by `10^(from_dec - to_dec)` with the requested
/// rounding. Both decimal counts must be `<= `[`MAX_DECIMALS`].
pub fn rescale(
    value: i128,
    from_dec: u32,
    to_dec: u32,
    rounding: Rounding,
) -> Result<i128, FixedError> {
    if from_dec == to_dec {
        return Ok(value);
    }
    if from_dec < to_dec {
        let factor = pow10(to_dec - from_dec)?;
        return match value.checked_mul(factor) {
            Some(v) => Ok(v),
            None => Err(FixedError::Overflow),
        };
    }
    let divisor = pow10(from_dec - to_dec)?;
    mul_div(value, 1, divisor, rounding)
}
