use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use crate::{OpenRouterError, OpenRouterErrorKind};

/// Configuration ceilings copied from resolved `policies.cost`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostPolicyConfig {
    pub max_session_usd_micros: u64,
    pub max_attempt_usd_micros: Option<u64>,
}

/// Pre-dispatch budget evaluation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetDecision {
    Allow { estimate_usd_micros: u64 },
    DenySession,
    DenyAttempt,
}

/// Process-local session spend ledger consulted before paid dispatch.
#[derive(Clone, Default)]
pub struct SessionCostLedger {
    spend_usd_micros: Arc<AtomicU64>,
    events: Arc<Mutex<Vec<u64>>>,
}

impl SessionCostLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn spend_usd_micros(&self) -> u64 {
        self.spend_usd_micros.load(Ordering::Relaxed)
    }

    /// Records a successful completion spend.
    pub fn record_spend(&self, usd_micros: u64) {
        if usd_micros == 0 {
            return;
        }
        self.spend_usd_micros
            .fetch_add(usd_micros, Ordering::Relaxed);
        if let Ok(mut events) = self.events.lock() {
            events.push(usd_micros);
        }
    }

    /// Seeds the ledger for offline over-budget tests.
    pub fn seed_spend(&self, usd_micros: u64) {
        self.spend_usd_micros.store(usd_micros, Ordering::Relaxed);
    }
}

/// Conservative default estimate when usage is unknown (one cent).
pub const DEFAULT_ATTEMPT_ESTIMATE_USD_MICROS: u64 = 10_000;

/// Evaluates whether a paid attempt may start.
#[must_use]
pub fn evaluate_budget(
    policy: CostPolicyConfig,
    ledger: &SessionCostLedger,
    estimate_usd_micros: u64,
) -> BudgetDecision {
    if let Some(max_attempt) = policy.max_attempt_usd_micros
        && estimate_usd_micros > max_attempt
    {
        return BudgetDecision::DenyAttempt;
    }
    let spend = ledger.spend_usd_micros();
    let projected = spend.saturating_add(estimate_usd_micros);
    if projected > policy.max_session_usd_micros {
        BudgetDecision::DenySession
    } else {
        BudgetDecision::Allow {
            estimate_usd_micros,
        }
    }
}

/// Converts a deny decision into a redacted pre-dispatch error.
///
/// # Errors
///
/// Always returns a budget error for deny variants.
pub fn deny_error(decision: BudgetDecision) -> Result<(), OpenRouterError> {
    match decision {
        BudgetDecision::Allow { .. } => Ok(()),
        BudgetDecision::DenySession => Err(OpenRouterError::new(
            OpenRouterErrorKind::BudgetExceeded,
            "session paid-inference budget would be exceeded",
        )),
        BudgetDecision::DenyAttempt => Err(OpenRouterError::new(
            OpenRouterErrorKind::BudgetExceeded,
            "attempt paid-inference budget would be exceeded",
        )),
    }
}

/// Estimates attempt cost using the conservative default, capped by session.
///
/// Optional `max_attempt_usd_micros` is enforced as a ceiling in
/// [`evaluate_budget`], not as the estimate itself.
#[must_use]
pub fn estimate_attempt_usd_micros(policy: CostPolicyConfig) -> u64 {
    DEFAULT_ATTEMPT_ESTIMATE_USD_MICROS.min(policy.max_session_usd_micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_when_session_budget_is_exhausted() {
        let ledger = SessionCostLedger::new();
        ledger.seed_spend(1_000_000);
        let decision = evaluate_budget(
            CostPolicyConfig {
                max_session_usd_micros: 1_000_000,
                max_attempt_usd_micros: Some(10_000),
            },
            &ledger,
            1,
        );
        assert_eq!(decision, BudgetDecision::DenySession);
    }

    #[test]
    fn denies_when_attempt_estimate_exceeds_ceiling() {
        let ledger = SessionCostLedger::new();
        let decision = evaluate_budget(
            CostPolicyConfig {
                max_session_usd_micros: 5_000_000,
                max_attempt_usd_micros: Some(5_000),
            },
            &ledger,
            5_001,
        );
        assert_eq!(decision, BudgetDecision::DenyAttempt);
    }

    #[test]
    fn allows_within_budget_and_records_spend() {
        let ledger = SessionCostLedger::new();
        let decision = evaluate_budget(
            CostPolicyConfig {
                max_session_usd_micros: 1_000_000,
                max_attempt_usd_micros: Some(50_000),
            },
            &ledger,
            10_000,
        );
        assert_eq!(
            decision,
            BudgetDecision::Allow {
                estimate_usd_micros: 10_000
            }
        );
        ledger.record_spend(12_345);
        assert_eq!(ledger.spend_usd_micros(), 12_345);
    }
}
