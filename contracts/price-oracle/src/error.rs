use soroban_sdk::contracterror;

/// Error codes returned by the price oracle contract.
///
/// Consumers should classify these into four categories to decide whether a
/// failed call is worth retrying:
///
/// - **Retryable**: transient conditions (e.g. not yet initialized, paused,
///   stale price that may refresh). Retry with bounded backoff.
/// - **StaleData**: the price exists but is older than the configured
///   threshold. Retry after a short delay; if it persists, surface to operators.
/// - **Configuration**: the caller or deployment is misconfigured (unknown
///   asset, missing feed). Do not retry blindly; fix configuration first.
/// - **Terminal**: invalid input or authorization failures. Never retry.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    // --- Configuration ---
    /// The requested asset has no configured price feed.
    AssetNotFound = 1,
    /// The contract has not been initialized with an admin/config.
    NotInitialized = 2,

    // --- Terminal (invalid input / authorization) ---
    /// Caller is not authorized to perform this action.
    Unauthorized = 3,
    /// An input argument failed validation.
    InvalidInput = 4,
    /// The price value is invalid (e.g. non-positive).
    InvalidPrice = 5,

    // --- Retryable (transient) ---
    /// The contract is paused; retry once unpaused.
    Paused = 6,
    /// A transient storage or ledger condition; safe to retry.
    TransientFailure = 7,

    // --- Stale data ---
    /// The stored price is older than the allowed staleness threshold.
    StalePrice = 8,
}

/// Category used by consumers to decide retry behavior.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    Retryable,
    StaleData,
    Configuration,
    Terminal,
}

impl Error {
    /// Classify a contract error into a consumer-facing category.
    pub fn category(self) -> ErrorCategory {
        match self {
            Error::Paused | Error::TransientFailure => ErrorCategory::Retryable,
            Error::StalePrice => ErrorCategory::StaleData,
            Error::AssetNotFound | Error::NotInitialized => ErrorCategory::Configuration,
            Error::Unauthorized | Error::InvalidInput | Error::InvalidPrice => {
                ErrorCategory::Terminal
            }
        }
    }

    /// Whether a consumer should retry the failed call.
    ///
    /// Retryable and stale-data errors may succeed on a later attempt;
    /// configuration and terminal errors will not.
    pub fn is_retryable(self) -> bool {
        matches!(
            self.category(),
            ErrorCategory::Retryable | ErrorCategory::StaleData
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors_are_retryable() {
        assert!(Error::Paused.is_retryable());
        assert!(Error::TransientFailure.is_retryable());
        assert_eq!(Error::Paused.category(), ErrorCategory::Retryable);
    }

    #[test]
    fn stale_price_is_stale_data() {
        assert!(Error::StalePrice.is_retryable());
        assert_eq!(Error::StalePrice.category(), ErrorCategory::StaleData);
    }

    #[test]
    fn invalid_input_does_not_retry() {
        assert!(!Error::InvalidInput.is_retryable());
        assert!(!Error::InvalidPrice.is_retryable());
        assert_eq!(Error::InvalidInput.category(), ErrorCategory::Terminal);
    }

    #[test]
    fn authorization_does_not_retry() {
        assert!(!Error::Unauthorized.is_retryable());
        assert_eq!(Error::Unauthorized.category(), ErrorCategory::Terminal);
    }

    #[test]
    fn configuration_errors_are_not_retryable() {
        assert!(!Error::AssetNotFound.is_retryable());
        assert!(!Error::NotInitialized.is_retryable());
        assert_eq!(Error::AssetNotFound.category(), ErrorCategory::Configuration);
    }

    #[test]
    fn bounded_retry_policy_terminates() {
        // Simulate a consumer retrying a transient failure with a bounded budget.
        const MAX_ATTEMPTS: u32 = 3;
        let mut attempts = 0u32;
        let mut err = Error::TransientFailure;
        while err.is_retryable() && attempts < MAX_ATTEMPTS {
            attempts += 1;
        }
        assert_eq!(attempts, MAX_ATTEMPTS);

        // Terminal errors never enter the retry loop.
        let mut terminal_attempts = 0u32;
        let terminal = Error::Unauthorized;
        while terminal.is_retryable() && terminal_attempts < MAX_ATTEMPTS {
            terminal_attempts += 1;
        }
        assert_eq!(terminal_attempts, 0);
    }
}
