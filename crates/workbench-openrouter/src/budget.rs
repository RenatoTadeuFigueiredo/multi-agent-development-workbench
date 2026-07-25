use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use uuid::Uuid;

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

/// Redacted durable spend persistence (implemented by the daemon/storage edge).
pub trait DurableSpendStore: Send + Sync {
    /// Loads every non-zero per-session spend for ledger restore.
    ///
    /// # Errors
    ///
    /// Returns when durable storage cannot be read.
    fn load_spends(&self) -> Result<Vec<(Uuid, u64)>, OpenRouterError>;

    /// Persists the latest redacted spend total for one session.
    ///
    /// # Errors
    ///
    /// Returns when durable storage cannot be written.
    fn persist_spend(&self, session_id: Uuid, spend_usd_micros: u64) -> Result<(), OpenRouterError>;
}

/// Per-session paid-inference spend ledger consulted before paid dispatch.
#[derive(Clone, Default)]
pub struct SessionCostLedger {
    by_session: Arc<Mutex<HashMap<Uuid, u64>>>,
    durable: Option<Arc<dyn DurableSpendStore>>,
}

impl SessionCostLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a ledger that restores and persists redacted spend via `store`.
    ///
    /// # Errors
    ///
    /// Returns when the durable store cannot load prior spends.
    pub fn with_durable_store(store: Arc<dyn DurableSpendStore>) -> Result<Self, OpenRouterError> {
        let mut ledger = Self {
            by_session: Arc::new(Mutex::new(HashMap::new())),
            durable: Some(store),
        };
        ledger.restore_from_durable()?;
        Ok(ledger)
    }

    /// Reloads redacted spends from the durable store after restart.
    ///
    /// # Errors
    ///
    /// Returns when the durable store is missing or cannot load spends.
    pub fn restore_from_durable(&mut self) -> Result<(), OpenRouterError> {
        let Some(store) = self.durable.as_ref() else {
            return Ok(());
        };
        let spends = store.load_spends()?;
        let mut guard = self.by_session.lock().map_err(|_| {
            OpenRouterError::new(OpenRouterErrorKind::Unavailable, "cost ledger unavailable")
        })?;
        guard.clear();
        for (session_id, spend) in spends {
            if spend > 0 {
                guard.insert(session_id, spend);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn spend_usd_micros(&self, session_id: Uuid) -> u64 {
        self.by_session
            .lock()
            .ok()
            .and_then(|guard| guard.get(&session_id).copied())
            .unwrap_or(0)
    }

    /// Records a successful completion spend and optionally persists it.
    ///
    /// # Errors
    ///
    /// Returns when durable persistence fails after an in-memory update.
    pub fn record_spend(
        &self,
        session_id: Uuid,
        usd_micros: u64,
    ) -> Result<u64, OpenRouterError> {
        if usd_micros == 0 {
            return Ok(self.spend_usd_micros(session_id));
        }
        let total = {
            let mut guard = self.by_session.lock().map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "cost ledger unavailable")
            })?;
            let entry = guard.entry(session_id).or_insert(0);
            *entry = entry.saturating_add(usd_micros);
            *entry
        };
        if let Some(store) = self.durable.as_ref() {
            store.persist_spend(session_id, total)?;
        }
        Ok(total)
    }

    /// Seeds the ledger for offline over-budget tests (does not persist).
    pub fn seed_spend(&self, session_id: Uuid, usd_micros: u64) {
        if let Ok(mut guard) = self.by_session.lock() {
            guard.insert(session_id, usd_micros);
        }
    }
}

/// Conservative default estimate when usage is unknown (one cent).
pub const DEFAULT_ATTEMPT_ESTIMATE_USD_MICROS: u64 = 10_000;

/// Evaluates whether a paid attempt may start for one session.
#[must_use]
pub fn evaluate_budget(
    policy: CostPolicyConfig,
    ledger: &SessionCostLedger,
    session_id: Uuid,
    estimate_usd_micros: u64,
) -> BudgetDecision {
    if let Some(max_attempt) = policy.max_attempt_usd_micros
        && estimate_usd_micros > max_attempt
    {
        return BudgetDecision::DenyAttempt;
    }
    let spend = ledger.spend_usd_micros(session_id);
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
#[must_use]
pub fn estimate_attempt_usd_micros(policy: CostPolicyConfig) -> u64 {
    DEFAULT_ATTEMPT_ESTIMATE_USD_MICROS.min(policy.max_session_usd_micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_when_session_budget_is_exhausted() {
        let session = Uuid::now_v7();
        let ledger = SessionCostLedger::new();
        ledger.seed_spend(session, 1_000_000);
        let decision = evaluate_budget(
            CostPolicyConfig {
                max_session_usd_micros: 1_000_000,
                max_attempt_usd_micros: Some(10_000),
            },
            &ledger,
            session,
            1,
        );
        assert_eq!(decision, BudgetDecision::DenySession);
    }

    #[test]
    fn denies_when_attempt_estimate_exceeds_ceiling() {
        let session = Uuid::now_v7();
        let ledger = SessionCostLedger::new();
        let decision = evaluate_budget(
            CostPolicyConfig {
                max_session_usd_micros: 5_000_000,
                max_attempt_usd_micros: Some(5_000),
            },
            &ledger,
            session,
            5_001,
        );
        assert_eq!(decision, BudgetDecision::DenyAttempt);
    }

    #[test]
    fn allows_within_budget_and_records_spend_per_session() {
        let session = Uuid::now_v7();
        let other = Uuid::now_v7();
        let ledger = SessionCostLedger::new();
        let decision = evaluate_budget(
            CostPolicyConfig {
                max_session_usd_micros: 1_000_000,
                max_attempt_usd_micros: Some(50_000),
            },
            &ledger,
            session,
            10_000,
        );
        assert_eq!(
            decision,
            BudgetDecision::Allow {
                estimate_usd_micros: 10_000
            }
        );
        assert_eq!(ledger.record_spend(session, 12_345).expect("record"), 12_345);
        assert_eq!(ledger.spend_usd_micros(session), 12_345);
        assert_eq!(ledger.spend_usd_micros(other), 0);
    }

    #[test]
    fn durable_store_restores_and_persists_spend() {
        struct MemStore {
            rows: Mutex<HashMap<Uuid, u64>>,
        }
        impl DurableSpendStore for MemStore {
            fn load_spends(&self) -> Result<Vec<(Uuid, u64)>, OpenRouterError> {
                Ok(self
                    .rows
                    .lock()
                    .expect("lock")
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect())
            }
            fn persist_spend(
                &self,
                session_id: Uuid,
                spend_usd_micros: u64,
            ) -> Result<(), OpenRouterError> {
                self.rows
                    .lock()
                    .expect("lock")
                    .insert(session_id, spend_usd_micros);
                Ok(())
            }
        }

        let session = Uuid::now_v7();
        let store = Arc::new(MemStore {
            rows: Mutex::new(HashMap::from([(session, 42_000)])),
        });
        let ledger = SessionCostLedger::with_durable_store(store.clone()).expect("restore");
        assert_eq!(ledger.spend_usd_micros(session), 42_000);
        assert_eq!(ledger.record_spend(session, 8_000).expect("record"), 50_000);
        assert_eq!(store.rows.lock().expect("lock").get(&session), Some(&50_000));
    }
}
