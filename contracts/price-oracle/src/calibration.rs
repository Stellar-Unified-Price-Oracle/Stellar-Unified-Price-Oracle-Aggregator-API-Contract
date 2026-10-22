//! Calibration helpers for the price oracle.
//!
//! This module contains pure, deterministic helpers used to calibrate
//! operational objectives and alert thresholds. It is intentionally free of
//! storage access so it can be exercised by repeatable checks.

/// Lag objective and alert calibration for ingest pipelines.
///
/// Two lag metrics are tracked:
/// - `source_to_indexer`: time between a source event and it being indexed.
/// - `ledger_to_consumer`: time between a ledger closing and a consumer
///   observing it.
///
/// Each metric has a warning and a page threshold. A measurement at or above
/// the warning threshold raises `Warning`; at or above the page threshold it
/// raises `Page`. Anything below the warning threshold is `Ok`.

/// Severity of a lag alert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LagSeverity {
    Ok,
    Warning,
    Page,
}

/// A single lag measurement tagged with the affected network and asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LagMeasurement {
    pub network: u32,
    pub asset: u32,
    pub source_to_indexer_ms: u64,
    pub ledger_to_consumer_ms: u64,
}

/// Calibrated thresholds for a lag metric, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LagThresholds {
    pub warning_ms: u64,
    pub page_ms: u64,
}

impl LagThresholds {
    /// Classify a measurement against these thresholds.
    ///
    /// Boundary behavior: a value exactly equal to `warning_ms` is a warning,
    /// and a value exactly equal to `page_ms` is a page.
    pub fn classify(&self, value_ms: u64) -> LagSeverity {
        if value_ms >= self.page_ms {
            LagSeverity::Page
        } else if value_ms >= self.warning_ms {
            LagSeverity::Warning
        } else {
            LagSeverity::Ok
        }
    }
}

/// Default source-to-indexer thresholds: warn at 30s, page at 2m.
pub const SOURCE_TO_INDEXER: LagThresholds = LagThresholds {
    warning_ms: 30_000,
    page_ms: 120_000,
};

/// Default ledger-to-consumer thresholds: warn at 15s, page at 60s.
pub const LEDGER_TO_CONSUMER: LagThresholds = LagThresholds {
    warning_ms: 15_000,
    page_ms: 60_000,
};

/// The alert produced for a lag measurement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LagAlert {
    pub severity: LagSeverity,
    pub network: u32,
    pub asset: u32,
    pub source_to_indexer_ms: u64,
    pub ledger_to_consumer_ms: u64,
}

/// Evaluate a lag measurement and produce an alert.
///
/// The overall severity is the maximum of the two metric severities so that a
/// page on either metric pages the on-call. The alert always carries the
/// affected network and asset so responders can route it.
pub fn evaluate(measurement: &LagMeasurement) -> LagAlert {
    let source = SOURCE_TO_INDEXER.classify(measurement.source_to_indexer_ms);
    let ledger = LEDGER_TO_CONSUMER.classify(measurement.ledger_to_consumer_ms);
    let severity = max_severity(source, ledger);
    LagAlert {
        severity,
        network: measurement.network,
        asset: measurement.asset,
        source_to_indexer_ms: measurement.source_to_indexer_ms,
        ledger_to_consumer_ms: measurement.ledger_to_consumer_ms,
    }
}

fn max_severity(a: LagSeverity, b: LagSeverity) -> LagSeverity {
    match (a, b) {
        (LagSeverity::Page, _) | (_, LagSeverity::Page) => LagSeverity::Page,
        (LagSeverity::Warning, _) | (_, LagSeverity::Warning) => LagSeverity::Warning,
        _ => LagSeverity::Ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurement(source_ms: u64, ledger_ms: u64) -> LagMeasurement {
        LagMeasurement {
            network: 1,
            asset: 42,
            source_to_indexer_ms: source_ms,
            ledger_to_consumer_ms: ledger_ms,
        }
    }

    #[test]
    fn normal_operation_is_ok() {
        let alert = evaluate(&measurement(1_000, 1_000));
        assert_eq!(alert.severity, LagSeverity::Ok);
        assert_eq!(alert.network, 1);
        assert_eq!(alert.asset, 42);
    }

    #[test]
    fn warning_boundary_is_inclusive() {
        assert_eq!(SOURCE_TO_INDEXER.classify(29_999), LagSeverity::Ok);
        assert_eq!(SOURCE_TO_INDEXER.classify(30_000), LagSeverity::Warning);
        assert_eq!(LEDGER_TO_CONSUMER.classify(15_000), LagSeverity::Warning);
    }

    #[test]
    fn page_boundary_is_inclusive() {
        assert_eq!(SOURCE_TO_INDEXER.classify(119_999), LagSeverity::Warning);
        assert_eq!(SOURCE_TO_INDEXER.classify(120_000), LagSeverity::Page);
        assert_eq!(LEDGER_TO_CONSUMER.classify(60_000), LagSeverity::Page);
    }

    #[test]
    fn replayed_incident_pages_with_network_and_asset() {
        // Replay of a lag incident: indexer stalled past the page threshold.
        let alert = evaluate(&measurement(180_000, 5_000));
        assert_eq!(alert.severity, LagSeverity::Page);
        assert_eq!(alert.network, 1);
        assert_eq!(alert.asset, 42);
        assert_eq!(alert.source_to_indexer_ms, 180_000);
    }

    #[test]
    fn recovery_returns_to_ok() {
        let incident = evaluate(&measurement(180_000, 5_000));
        assert_eq!(incident.severity, LagSeverity::Page);
        let recovered = evaluate(&measurement(2_000, 2_000));
        assert_eq!(recovered.severity, LagSeverity::Ok);
    }
}
