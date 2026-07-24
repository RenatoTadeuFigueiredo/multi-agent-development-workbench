//! Bounded, content-free daemon metrics.

use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;
use workbench_core::ports::Telemetry;

const ROUTE_RULE_COUNT: usize = 5;
const OUTCOME_COUNT: usize = 8;

/// Fixed routing labels accepted by daemon metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteRule {
    Explicit,
    Workflow,
    Resolver,
    Coordinator,
    Clarification,
}

impl RouteRule {
    const fn index(self) -> usize {
        match self {
            Self::Explicit => 0,
            Self::Workflow => 1,
            Self::Resolver => 2,
            Self::Coordinator => 3,
            Self::Clarification => 4,
        }
    }

    fn from_static_label(label: &'static str) -> Option<Self> {
        match label {
            "explicit" => Some(Self::Explicit),
            "workflow" => Some(Self::Workflow),
            "resolver" => Some(Self::Resolver),
            "coordinator" => Some(Self::Coordinator),
            "clarification" => Some(Self::Clarification),
            _ => None,
        }
    }
}

/// Fixed lifecycle outcomes accepted by daemon metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryOutcome {
    Success,
    Denied,
    Failed,
    Cancelled,
    Abandoned,
    Timeout,
    OutcomeUnknown,
    ClientLagged,
}

impl TelemetryOutcome {
    const fn index(self) -> usize {
        match self {
            Self::Success => 0,
            Self::Denied => 1,
            Self::Failed => 2,
            Self::Cancelled => 3,
            Self::Abandoned => 4,
            Self::Timeout => 5,
            Self::OutcomeUnknown => 6,
            Self::ClientLagged => 7,
        }
    }

    fn from_static_label(label: &'static str) -> Option<Self> {
        match label {
            "success" => Some(Self::Success),
            "denied" => Some(Self::Denied),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "abandoned" => Some(Self::Abandoned),
            "timeout" => Some(Self::Timeout),
            "outcome_unknown" => Some(Self::OutcomeUnknown),
            "client_lagged" => Some(Self::ClientLagged),
            _ => None,
        }
    }
}

/// External telemetry delivery mode.
///
/// The default is local-only metrics. OTLP is reserved for a future exporter
/// and cannot be silently selected while that exporter is unavailable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExternalTelemetryExport {
    #[default]
    Disabled,
    Otlp,
}

/// Telemetry initialization failure.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum TelemetryError {
    #[error("OTLP telemetry export was requested but no exporter is available")]
    ExternalExporterUnavailable,
}

/// Immutable copy of the bounded in-process counters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetrySnapshot {
    route_decisions: [[u64; OUTCOME_COUNT]; ROUTE_RULE_COUNT],
    attempts: [u64; OUTCOME_COUNT],
    rejected_records: u64,
    external_export: ExternalTelemetryExport,
}

impl TelemetrySnapshot {
    /// Returns the route decision count for one fixed label pair.
    #[must_use]
    pub const fn route_decisions(&self, rule: RouteRule, outcome: TelemetryOutcome) -> u64 {
        self.route_decisions[rule.index()][outcome.index()]
    }

    /// Returns the attempt count for one fixed outcome.
    #[must_use]
    pub const fn attempts(&self, outcome: TelemetryOutcome) -> u64 {
        self.attempts[outcome.index()]
    }

    /// Returns the number of records dropped because a label was not allowed.
    #[must_use]
    pub const fn rejected_records(&self) -> u64 {
        self.rejected_records
    }

    /// Reports whether external delivery was enabled for this sink.
    #[must_use]
    pub const fn external_export(&self) -> ExternalTelemetryExport {
        self.external_export
    }
}

/// Thread-safe telemetry sink with a compile-time-bounded label space.
#[derive(Debug)]
pub struct BoundedTelemetry {
    route_decisions: [[AtomicU64; OUTCOME_COUNT]; ROUTE_RULE_COUNT],
    attempts: [AtomicU64; OUTCOME_COUNT],
    rejected_records: AtomicU64,
    external_export: ExternalTelemetryExport,
}

impl BoundedTelemetry {
    /// Initializes telemetry with the requested external delivery policy.
    ///
    /// # Errors
    ///
    /// Returns `ExternalExporterUnavailable` when OTLP is requested because
    /// this build has no external exporter. No partially enabled sink is
    /// returned.
    pub fn initialize(external_export: ExternalTelemetryExport) -> Result<Self, TelemetryError> {
        match external_export {
            ExternalTelemetryExport::Disabled => Ok(Self::local_only()),
            ExternalTelemetryExport::Otlp => Err(TelemetryError::ExternalExporterUnavailable),
        }
    }

    fn local_only() -> Self {
        Self {
            route_decisions: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            attempts: std::array::from_fn(|_| AtomicU64::new(0)),
            rejected_records: AtomicU64::new(0),
            external_export: ExternalTelemetryExport::Disabled,
        }
    }

    /// Records a route using only typed, bounded labels.
    pub fn record_bounded_route(&self, rule: RouteRule, outcome: TelemetryOutcome) {
        increment(&self.route_decisions[rule.index()][outcome.index()]);
    }

    /// Records an attempt using only a typed, bounded outcome.
    pub fn record_bounded_attempt(&self, outcome: TelemetryOutcome) {
        increment(&self.attempts[outcome.index()]);
    }

    /// Captures the current counters.
    ///
    /// Concurrent increments may appear on either side of this non-transactional
    /// snapshot, but every individual counter is read atomically.
    #[must_use]
    pub fn snapshot(&self) -> TelemetrySnapshot {
        TelemetrySnapshot {
            route_decisions: std::array::from_fn(|rule| {
                std::array::from_fn(|outcome| {
                    self.route_decisions[rule][outcome].load(Ordering::Relaxed)
                })
            }),
            attempts: std::array::from_fn(|outcome| self.attempts[outcome].load(Ordering::Relaxed)),
            rejected_records: self.rejected_records.load(Ordering::Relaxed),
            external_export: self.external_export,
        }
    }

    fn reject_record(&self) {
        increment(&self.rejected_records);
    }
}

impl Default for BoundedTelemetry {
    fn default() -> Self {
        Self::local_only()
    }
}

impl Telemetry for BoundedTelemetry {
    fn record_route(&self, selected_rule: &'static str, outcome: &'static str) {
        let (Some(rule), Some(outcome)) = (
            RouteRule::from_static_label(selected_rule),
            TelemetryOutcome::from_static_label(outcome),
        ) else {
            self.reject_record();
            return;
        };
        self.record_bounded_route(rule, outcome);
    }

    fn record_attempt(&self, outcome: &'static str) {
        let Some(outcome) = TelemetryOutcome::from_static_label(outcome) else {
            self.reject_record();
            return;
        };
        self.record_bounded_attempt(outcome);
    }
}

fn increment(counter: &AtomicU64) {
    let _previous = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn external_export_is_explicitly_disabled_by_default() {
        let telemetry = BoundedTelemetry::default();

        assert_eq!(
            telemetry.snapshot().external_export(),
            ExternalTelemetryExport::Disabled
        );
    }

    #[test]
    fn unavailable_external_export_fails_closed() {
        let error = BoundedTelemetry::initialize(ExternalTelemetryExport::Otlp)
            .expect_err("unavailable exporter must fail");

        assert_eq!(error, TelemetryError::ExternalExporterUnavailable);
    }

    #[test]
    fn records_only_fixed_route_and_attempt_labels() {
        let telemetry = BoundedTelemetry::default();

        telemetry.record_route("explicit", "success");
        telemetry.record_route("explicit", "success");
        telemetry.record_route("coordinator", "denied");
        telemetry.record_attempt("failed");

        let snapshot = telemetry.snapshot();
        assert_eq!(
            snapshot.route_decisions(RouteRule::Explicit, TelemetryOutcome::Success),
            2
        );
        assert_eq!(
            snapshot.route_decisions(RouteRule::Coordinator, TelemetryOutcome::Denied),
            1
        );
        assert_eq!(snapshot.attempts(TelemetryOutcome::Failed), 1);
        assert_eq!(snapshot.rejected_records(), 0);
    }

    #[test]
    fn rejects_unknown_labels_without_retaining_them() {
        let telemetry = BoundedTelemetry::default();

        telemetry.record_route("session-01999999", "success");
        telemetry.record_route("explicit", "request-01999999");
        telemetry.record_attempt("prompt-body");

        let snapshot = telemetry.snapshot();
        assert_eq!(
            snapshot.route_decisions(RouteRule::Explicit, TelemetryOutcome::Success),
            0
        );
        assert_eq!(snapshot.attempts(TelemetryOutcome::Success), 0);
        assert_eq!(snapshot.rejected_records(), 3);
    }

    #[test]
    fn concurrent_updates_are_counted_without_dynamic_labels() {
        let telemetry = Arc::new(BoundedTelemetry::default());
        let threads = (0..4)
            .map(|_| {
                let telemetry = Arc::clone(&telemetry);
                std::thread::spawn(move || {
                    for _ in 0..1_000 {
                        telemetry.record_bounded_attempt(TelemetryOutcome::Success);
                    }
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().expect("telemetry thread");
        }

        assert_eq!(
            telemetry.snapshot().attempts(TelemetryOutcome::Success),
            4_000
        );
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let counter = AtomicU64::new(u64::MAX);

        increment(&counter);

        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}
