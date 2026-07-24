//! Monotonic policy resolution and durable approvals.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    ApprovalId, CoreError, FailureCategory,
    attempt::EffectClass,
    value::{NonEmptyText, ToolId},
};

/// Effective permission for one action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    ReadOnly,
    ApprovalRequired,
    Denied,
}

impl PermissionMode {
    const fn restriction_rank(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::ApprovalRequired => 1,
            Self::Denied => 2,
        }
    }

    const fn intersect(self, other: Self) -> Self {
        if self.restriction_rank() >= other.restriction_rank() {
            self
        } else {
            other
        }
    }
}

/// Origin of an effective policy restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    BuiltIn,
    User,
    Repository,
    Session,
    EffectClass,
}

/// One resolved policy layer.
#[derive(Debug, Clone)]
pub struct PolicyLayer {
    pub source: PolicySource,
    pub default_mode: PermissionMode,
    pub tool_modes: BTreeMap<ToolId, PermissionMode>,
    pub denied_tools: BTreeSet<ToolId>,
}

/// Explainable policy result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyDecision {
    pub mode: PermissionMode,
    pub authoritative_source: PolicySource,
}

/// Intersects every layer so later configuration cannot widen authority.
#[must_use]
pub fn resolve_tool_policy(tool: &ToolId, layers: &[PolicyLayer]) -> PolicyDecision {
    let mut decision = PolicyDecision {
        mode: PermissionMode::ReadOnly,
        authoritative_source: PolicySource::BuiltIn,
    };

    for layer in layers {
        let proposed = if layer.denied_tools.contains(tool) {
            PermissionMode::Denied
        } else {
            layer
                .tool_modes
                .get(tool)
                .copied()
                .unwrap_or(layer.default_mode)
        };
        let intersected = decision.mode.intersect(proposed);
        if intersected != decision.mode {
            decision = PolicyDecision {
                mode: intersected,
                authoritative_source: layer.source,
            };
        }
    }
    decision
}

/// Adds mandatory protection based on the external effect class.
#[must_use]
pub fn protect_effect(
    current: PolicyDecision,
    effect_class: EffectClass,
    material_cost: bool,
) -> PolicyDecision {
    let mandatory = match effect_class {
        EffectClass::None | EffectClass::IdempotentRead if !material_cost => {
            PermissionMode::ReadOnly
        }
        EffectClass::IdempotentRead
        | EffectClass::IdempotentWrite
        | EffectClass::PaidInference
        | EffectClass::NonIdempotentWrite
        | EffectClass::Production
        | EffectClass::Credential
        | EffectClass::None => PermissionMode::ApprovalRequired,
    };
    let protected = current.mode.intersect(mandatory);
    if protected == current.mode {
        current
    } else {
        PolicyDecision {
            mode: protected,
            authoritative_source: PolicySource::EffectClass,
        }
    }
}

/// A human approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Grant,
    Deny,
}

/// Immutable recorded approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approval_id: ApprovalId,
    pub actor: NonEmptyText,
    pub decision: ApprovalDecision,
}

/// A protected action awaiting a decision.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    id: ApprovalId,
    recorded: Option<ApprovalRecord>,
}

impl PendingApproval {
    /// Creates a new pending approval.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: ApprovalId::new(),
            recorded: None,
        }
    }

    /// Returns its stable ID.
    #[must_use]
    pub const fn id(&self) -> ApprovalId {
        self.id
    }

    /// Records the first decision, replays an identical decision, and rejects a conflict.
    ///
    /// # Errors
    ///
    /// Returns `invalid_transition` when a different decision was already recorded.
    pub fn resolve(
        &mut self,
        actor: NonEmptyText,
        decision: ApprovalDecision,
    ) -> Result<ApprovalRecord, CoreError> {
        if let Some(recorded) = self.recorded.clone() {
            if recorded.decision == decision {
                return Ok(recorded);
            }
            return Err(CoreError::new(
                FailureCategory::InvalidTransition,
                "approval already has a conflicting decision",
            ));
        }
        let record = ApprovalRecord {
            approval_id: self.id,
            actor,
            decision,
        };
        self.recorded = Some(record.clone());
        Ok(record)
    }
}

impl Default for PendingApproval {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use proptest::prelude::*;

    use super::{
        ApprovalDecision, PendingApproval, PermissionMode, PolicyLayer, PolicySource,
        resolve_tool_policy,
    };
    use crate::value::{NonEmptyText, ToolId};

    #[test]
    fn repository_cannot_widen_global_denial() {
        let tool = ToolId::parse("terminal").expect("tool");
        let decision = resolve_tool_policy(
            &tool,
            &[
                PolicyLayer {
                    source: PolicySource::User,
                    default_mode: PermissionMode::ReadOnly,
                    tool_modes: BTreeMap::new(),
                    denied_tools: BTreeSet::from([tool.clone()]),
                },
                PolicyLayer {
                    source: PolicySource::Repository,
                    default_mode: PermissionMode::ReadOnly,
                    tool_modes: BTreeMap::from([(tool.clone(), PermissionMode::ReadOnly)]),
                    denied_tools: BTreeSet::new(),
                },
            ],
        );
        assert_eq!(decision.mode, PermissionMode::Denied);
        assert_eq!(decision.authoritative_source, PolicySource::User);
    }

    #[test]
    fn repeated_approval_returns_original_actor_and_conflict_fails() {
        let mut approval = PendingApproval::new();
        let first = approval
            .resolve(
                NonEmptyText::parse("first-client").expect("actor"),
                ApprovalDecision::Grant,
            )
            .expect("first decision");
        let replay = approval
            .resolve(
                NonEmptyText::parse("second-client").expect("actor"),
                ApprovalDecision::Grant,
            )
            .expect("replay");
        assert_eq!(replay, first);
        assert!(
            approval
                .resolve(
                    NonEmptyText::parse("second-client").expect("actor"),
                    ApprovalDecision::Deny
                )
                .is_err()
        );
    }

    proptest! {
        #[test]
        fn adding_a_layer_never_reduces_restriction(deny in any::<bool>()) {
            let tool = ToolId::parse("repository").expect("tool");
            let initial = resolve_tool_policy(&tool, &[]);
            let next = resolve_tool_policy(&tool, &[PolicyLayer {
                source: PolicySource::Repository,
                default_mode: if deny { PermissionMode::Denied } else { PermissionMode::ReadOnly },
                tool_modes: BTreeMap::new(),
                denied_tools: BTreeSet::new(),
            }]);
            prop_assert!(next.mode.restriction_rank() >= initial.mode.restriction_rank());
        }
    }
}
