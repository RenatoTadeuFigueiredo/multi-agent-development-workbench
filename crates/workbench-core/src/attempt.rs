//! External side-effect attempt lifecycle and reconciliation.

use serde::{Deserialize, Serialize};

use crate::{AttemptId, CoreError, FailureCategory};

/// Classification used to determine approval and retry behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectClass {
    None,
    IdempotentRead,
    IdempotentWrite,
    PaidInference,
    NonIdempotentWrite,
    Production,
    Credential,
}

/// The declared safety properties of an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPolicy {
    pub effect_class: EffectClass,
    pub explicitly_idempotent: bool,
    pub material_cost: bool,
}

impl OperationPolicy {
    /// Rejects declarations that would permit unsafe retry assumptions.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` when an unsafe effect is declared idempotent.
    pub fn validate(&self) -> Result<(), CoreError> {
        let forbidden_idempotency = matches!(
            self.effect_class,
            EffectClass::PaidInference
                | EffectClass::NonIdempotentWrite
                | EffectClass::Production
                | EffectClass::Credential
        );
        if self.explicitly_idempotent && forbidden_idempotency {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "effect class cannot be declared idempotent",
            ));
        }
        Ok(())
    }

    /// Returns whether the operation can retry before dispatch starts.
    #[must_use]
    pub const fn permits_automatic_retry(&self) -> bool {
        matches!(self.effect_class, EffectClass::IdempotentRead)
            && self.explicitly_idempotent
            && !self.material_cost
    }
}

/// Durable progress of one external attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptProgress {
    Planned,
    Started,
    Acknowledged,
    Completed,
    Failed,
    OutcomeUnknown,
    Abandoned,
}

impl AttemptProgress {
    /// Returns whether a definite terminal fact exists.
    #[must_use]
    pub const fn is_definite_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Abandoned)
    }

    /// Returns whether external dispatch has started.
    #[must_use]
    pub const fn dispatch_started(self) -> bool {
        !matches!(self, Self::Planned)
    }
}

/// One durable external operation attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    id: AttemptId,
    operation: String,
    policy: OperationPolicy,
    progress: AttemptProgress,
    predecessor: Option<AttemptId>,
}

impl Attempt {
    /// Creates a planned attempt after validating its policy.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` for an empty operation or unsafe policy.
    pub fn plan(operation: impl Into<String>, policy: OperationPolicy) -> Result<Self, CoreError> {
        policy.validate()?;
        let operation = operation.into();
        if operation.is_empty() {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "operation must not be empty",
            ));
        }
        Ok(Self {
            id: AttemptId::new(),
            operation,
            policy,
            progress: AttemptProgress::Planned,
            predecessor: None,
        })
    }

    /// Returns the attempt ID.
    #[must_use]
    pub const fn id(&self) -> AttemptId {
        self.id
    }

    /// Returns the operation name.
    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// Returns the effect policy.
    #[must_use]
    pub const fn policy(&self) -> &OperationPolicy {
        &self.policy
    }

    /// Returns the durable progress.
    #[must_use]
    pub const fn progress(&self) -> AttemptProgress {
        self.progress
    }

    /// Returns the uncertain predecessor for a human-authorized retry.
    #[must_use]
    pub const fn predecessor(&self) -> Option<AttemptId> {
        self.predecessor
    }

    /// Advances to `started`.
    ///
    /// # Errors
    ///
    /// Returns `invalid_transition` unless the attempt is planned.
    pub fn mark_started(&mut self) -> Result<(), CoreError> {
        self.advance(AttemptProgress::Planned, AttemptProgress::Started)
    }

    /// Advances to `acknowledged`.
    ///
    /// # Errors
    ///
    /// Returns `invalid_transition` unless the attempt is started.
    pub fn mark_acknowledged(&mut self) -> Result<(), CoreError> {
        self.advance(AttemptProgress::Started, AttemptProgress::Acknowledged)
    }

    /// Records a definite completion.
    ///
    /// # Errors
    ///
    /// Returns `invalid_transition` unless dispatch started.
    pub fn mark_completed(&mut self) -> Result<(), CoreError> {
        if matches!(
            self.progress,
            AttemptProgress::Started | AttemptProgress::Acknowledged
        ) {
            self.progress = AttemptProgress::Completed;
            Ok(())
        } else {
            Err(invalid_attempt_transition())
        }
    }

    /// Records a definite failure.
    ///
    /// # Errors
    ///
    /// Returns `invalid_transition` after a terminal or uncertain outcome.
    pub fn mark_failed(&mut self) -> Result<(), CoreError> {
        if matches!(
            self.progress,
            AttemptProgress::Planned | AttemptProgress::Started | AttemptProgress::Acknowledged
        ) {
            self.progress = AttemptProgress::Failed;
            Ok(())
        } else {
            Err(invalid_attempt_transition())
        }
    }

    /// Records an uncertain external outcome.
    ///
    /// # Errors
    ///
    /// Returns `invalid_transition` unless dispatch started.
    pub fn mark_outcome_unknown(&mut self) -> Result<(), CoreError> {
        if matches!(
            self.progress,
            AttemptProgress::Started | AttemptProgress::Acknowledged
        ) {
            self.progress = AttemptProgress::OutcomeUnknown;
            Ok(())
        } else {
            Err(invalid_attempt_transition())
        }
    }

    /// Applies conservative crash recovery.
    pub fn recover(&mut self) {
        if matches!(
            self.progress,
            AttemptProgress::Started | AttemptProgress::Acknowledged
        ) {
            self.progress = AttemptProgress::OutcomeUnknown;
        }
    }

    /// Whether an automatic retry is safe at the current progress.
    #[must_use]
    pub fn may_retry_automatically(&self) -> bool {
        self.progress == AttemptProgress::Planned && self.policy.permits_automatic_retry()
    }

    fn advance(
        &mut self,
        expected: AttemptProgress,
        next: AttemptProgress,
    ) -> Result<(), CoreError> {
        if self.progress == expected {
            self.progress = next;
            Ok(())
        } else {
            Err(invalid_attempt_transition())
        }
    }
}

/// Human decision for an uncertain external outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationResolution {
    Retry,
    AcceptResult,
    Abandon,
}

/// Result of reconciling an uncertain attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    Retry(Attempt),
    Accepted,
    Abandoned,
}

/// Reconciles an uncertain attempt without silently repeating its effect.
///
/// # Errors
///
/// Returns `invalid_transition` unless the attempt has an unknown outcome.
pub fn reconcile(
    attempt: &mut Attempt,
    resolution: ReconciliationResolution,
) -> Result<ReconciliationOutcome, CoreError> {
    if attempt.progress != AttemptProgress::OutcomeUnknown {
        return Err(invalid_attempt_transition());
    }

    match resolution {
        ReconciliationResolution::Retry => {
            let mut replacement = Attempt::plan(attempt.operation.clone(), attempt.policy.clone())?;
            replacement.predecessor = Some(attempt.id);
            Ok(ReconciliationOutcome::Retry(replacement))
        }
        ReconciliationResolution::AcceptResult => {
            attempt.progress = AttemptProgress::Completed;
            Ok(ReconciliationOutcome::Accepted)
        }
        ReconciliationResolution::Abandon => {
            attempt.progress = AttemptProgress::Abandoned;
            Ok(ReconciliationOutcome::Abandoned)
        }
    }
}

fn invalid_attempt_transition() -> CoreError {
    CoreError::new(
        FailureCategory::InvalidTransition,
        "attempt transition is not allowed",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        Attempt, AttemptProgress, EffectClass, OperationPolicy, ReconciliationOutcome,
        ReconciliationResolution, reconcile,
    };

    fn policy(effect_class: EffectClass, explicitly_idempotent: bool) -> OperationPolicy {
        OperationPolicy {
            effect_class,
            explicitly_idempotent,
            material_cost: false,
        }
    }

    #[test]
    fn only_unstarted_explicit_idempotent_reads_retry_automatically() {
        let mut attempt =
            Attempt::plan("read", policy(EffectClass::IdempotentRead, true)).expect("valid");
        assert!(attempt.may_retry_automatically());
        attempt.mark_started().expect("valid transition");
        assert!(!attempt.may_retry_automatically());
    }

    #[test]
    fn started_attempt_recovers_as_unknown() {
        let mut attempt =
            Attempt::plan("read", policy(EffectClass::IdempotentRead, true)).expect("valid");
        attempt.mark_started().expect("valid transition");
        attempt.recover();
        assert_eq!(attempt.progress(), AttemptProgress::OutcomeUnknown);
    }

    #[test]
    fn human_retry_links_a_new_attempt() {
        let mut attempt =
            Attempt::plan("write", policy(EffectClass::IdempotentWrite, false)).expect("valid");
        attempt.mark_started().expect("valid transition");
        attempt.mark_outcome_unknown().expect("valid transition");
        let original = attempt.id();
        let outcome = reconcile(&mut attempt, ReconciliationResolution::Retry).expect("reconciled");
        let ReconciliationOutcome::Retry(replacement) = outcome else {
            panic!("expected retry");
        };
        assert_ne!(replacement.id(), original);
        assert_eq!(replacement.predecessor(), Some(original));
    }
}
