//! Durable multi-stage workflow run state machine.

use serde::{Deserialize, Serialize};

use crate::{
    CoreError, FailureCategory,
    value::{NonEmptyText, WorkflowId},
};

/// Hard ceiling for automatic correction iterations (FR-008-001 / FR-008-005).
pub const MAX_CORRECTION_ITERATIONS: u32 = 8;

/// Closed public phase set for one workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPhase {
    Planned,
    Running,
    Paused,
    AwaitingHuman,
    Completed,
    Cancelled,
    Failed,
}

impl WorkflowPhase {
    /// Whether the run may dispatch a provider attempt.
    #[must_use]
    pub const fn permits_dispatch(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Whether the run has reached a terminal phase.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

/// Durable identity and progress for one workflow execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub workflow_id: WorkflowId,
    pub run_id: NonEmptyText,
    pub step_ids: Vec<NonEmptyText>,
    pub active_step_index: usize,
    pub iteration: u32,
    pub phase: WorkflowPhase,
    pub max_iterations: u32,
    pub on_findings_step_index: Option<usize>,
}

impl WorkflowRun {
    /// Starts a run on the first step of a validated workflow.
    ///
    /// # Errors
    ///
    /// Returns when the workflow has no steps or identifiers are empty.
    pub fn start(
        workflow_id: WorkflowId,
        run_id: NonEmptyText,
        step_ids: Vec<NonEmptyText>,
        max_iterations: u32,
        on_findings_step_index: Option<usize>,
    ) -> Result<Self, CoreError> {
        if step_ids.is_empty() {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "workflow has no steps",
            ));
        }
        if max_iterations == 0 || max_iterations > MAX_CORRECTION_ITERATIONS {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "workflow max_iterations must be between 1 and 8",
            ));
        }
        if let Some(index) = on_findings_step_index
            && index >= step_ids.len()
        {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "on_findings step index is out of range",
            ));
        }
        Ok(Self {
            workflow_id,
            run_id,
            step_ids,
            active_step_index: 0,
            iteration: 1,
            phase: WorkflowPhase::Running,
            max_iterations,
            on_findings_step_index,
        })
    }

    /// Returns the active step identifier.
    #[must_use]
    pub fn active_step_id(&self) -> Option<&NonEmptyText> {
        self.step_ids.get(self.active_step_index)
    }

    /// Advances after a successful step without findings.
    ///
    /// # Errors
    ///
    /// Returns when the run cannot dispatch or is already terminal.
    pub fn advance_after_success(&mut self) -> Result<(), CoreError> {
        self.require_running()?;
        if self.active_step_index + 1 >= self.step_ids.len() {
            self.phase = WorkflowPhase::Completed;
            return Ok(());
        }
        self.active_step_index += 1;
        self.iteration = 1;
        Ok(())
    }

    /// Routes to the correction target when findings are present.
    ///
    /// # Errors
    ///
    /// Returns when findings cannot be applied or the iteration ceiling is hit
    /// (phase becomes `awaiting_human`).
    pub fn apply_findings(&mut self) -> Result<(), CoreError> {
        self.require_running()?;
        let Some(target) = self.on_findings_step_index else {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "active step has no on_findings target",
            ));
        };
        if self.iteration >= self.max_iterations {
            self.phase = WorkflowPhase::AwaitingHuman;
            return Ok(());
        }
        self.active_step_index = target;
        self.iteration = self.iteration.saturating_add(1);
        Ok(())
    }

    /// Pauses advancement.
    ///
    /// # Errors
    ///
    /// Returns when the run is terminal.
    pub fn pause(&mut self) -> Result<(), CoreError> {
        if self.phase.is_terminal() {
            return Err(invalid_transition());
        }
        if matches!(self.phase, WorkflowPhase::Paused) {
            return Ok(());
        }
        self.phase = WorkflowPhase::Paused;
        Ok(())
    }

    /// Resumes a paused or human-awaiting run.
    ///
    /// # Errors
    ///
    /// Returns when the run is terminal or not resumable.
    pub fn resume(&mut self) -> Result<(), CoreError> {
        if self.phase.is_terminal() {
            return Err(invalid_transition());
        }
        if !matches!(
            self.phase,
            WorkflowPhase::Paused | WorkflowPhase::AwaitingHuman
        ) {
            return Err(invalid_transition());
        }
        self.phase = WorkflowPhase::Running;
        Ok(())
    }

    /// Cancels the run without inventing success.
    ///
    /// # Errors
    ///
    /// Returns when the run is already terminal with a different outcome.
    pub fn cancel(&mut self) -> Result<(), CoreError> {
        if matches!(self.phase, WorkflowPhase::Cancelled) {
            return Ok(());
        }
        if self.phase.is_terminal() {
            return Err(invalid_transition());
        }
        self.phase = WorkflowPhase::Cancelled;
        Ok(())
    }

    /// Marks the run failed after a definite step failure.
    ///
    /// # Errors
    ///
    /// Returns when the run is already terminal.
    pub fn fail(&mut self) -> Result<(), CoreError> {
        if self.phase.is_terminal() {
            return Err(invalid_transition());
        }
        self.phase = WorkflowPhase::Failed;
        Ok(())
    }

    fn require_running(&self) -> Result<(), CoreError> {
        if self.phase.permits_dispatch() {
            Ok(())
        } else {
            Err(invalid_transition())
        }
    }
}

fn invalid_transition() -> CoreError {
    CoreError::new(
        FailureCategory::InvalidTransition,
        "illegal workflow run transition",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_run(max_iterations: u32) -> WorkflowRun {
        WorkflowRun::start(
            WorkflowId::parse("primary").expect("id"),
            NonEmptyText::parse("run-1").expect("id"),
            vec![
                NonEmptyText::parse("specify").expect("id"),
                NonEmptyText::parse("review").expect("id"),
                NonEmptyText::parse("implement").expect("id"),
                NonEmptyText::parse("validate").expect("id"),
            ],
            max_iterations,
            Some(1),
        )
        .expect("start")
    }

    #[test]
    fn sequential_advance_completes_primary_path() {
        let mut run = sample_run(2);
        assert_eq!(
            run.active_step_id().map(NonEmptyText::as_str),
            Some("specify")
        );
        run.advance_after_success().expect("1");
        assert_eq!(
            run.active_step_id().map(NonEmptyText::as_str),
            Some("review")
        );
        run.advance_after_success().expect("2");
        assert_eq!(
            run.active_step_id().map(NonEmptyText::as_str),
            Some("implement")
        );
        run.advance_after_success().expect("3");
        assert_eq!(
            run.active_step_id().map(NonEmptyText::as_str),
            Some("validate")
        );
        run.advance_after_success().expect("4");
        assert_eq!(run.phase, WorkflowPhase::Completed);
    }

    #[test]
    fn findings_loop_is_bounded() {
        let mut run = sample_run(2);
        run.advance_after_success().expect("to review");
        run.apply_findings().expect("first correction");
        assert_eq!(run.iteration, 2);
        assert_eq!(run.phase, WorkflowPhase::Running);
        run.apply_findings().expect("ceiling");
        assert_eq!(run.phase, WorkflowPhase::AwaitingHuman);
    }

    #[test]
    fn pause_resume_and_cancel() {
        let mut run = sample_run(1);
        run.pause().expect("pause");
        assert!(!run.phase.permits_dispatch());
        run.resume().expect("resume");
        assert!(run.phase.permits_dispatch());
        run.cancel().expect("cancel");
        assert_eq!(run.phase, WorkflowPhase::Cancelled);
        assert!(run.cancel().is_ok());
    }
}
