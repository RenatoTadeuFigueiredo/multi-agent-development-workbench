//! Deterministic, ordered routing.

use serde::{Deserialize, Serialize};

use crate::{
    CoreError, FailureCategory,
    value::{DataSourceId, ModelAlias, ProviderId, RoleId, ToolId},
};

/// Risk assigned before dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

/// Effective permission granted to a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionScope {
    ReadOnly,
    ApprovalRequired,
    Denied,
}

/// Destination selected independently of provider-specific behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDestination {
    pub role: RoleId,
    pub model_alias: ModelAlias,
    pub provider: ProviderId,
    pub runtime_model: String,
}

/// Context made available to the selected executor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteContext {
    pub tools: Vec<ToolId>,
    pub data_sources: Vec<DataSourceId>,
    pub permission: PermissionScope,
}

/// Why a route won the ordered resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectedRule {
    Explicit,
    Workflow,
    Resolver,
    Coordinator,
}

/// Auditable plan emitted before provider dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingPlan {
    pub intent: String,
    pub destination: RouteDestination,
    pub context: RouteContext,
    pub risk: Risk,
    pub confidence: f64,
    pub selected_by: SelectedRule,
}

/// A potential route produced by one rule.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteCandidate {
    intent: String,
    destination: RouteDestination,
    context: RouteContext,
    risk: Risk,
    confidence: f64,
}

impl RouteCandidate {
    /// Creates a validated candidate.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` for an empty intent or out-of-range confidence.
    pub fn new(
        intent: impl Into<String>,
        destination: RouteDestination,
        context: RouteContext,
        risk: Risk,
        confidence: f64,
    ) -> Result<Self, CoreError> {
        let intent = intent.into();
        if intent.is_empty() || !(0.0..=1.0).contains(&confidence) || confidence.is_nan() {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "route intent and confidence are invalid",
            ));
        }
        Ok(Self {
            intent,
            destination,
            context,
            risk,
            confidence,
        })
    }

    fn into_plan(self, selected_by: SelectedRule) -> RoutingPlan {
        RoutingPlan {
            intent: self.intent,
            destination: self.destination,
            context: self.context,
            risk: self.risk,
            confidence: self.confidence,
            selected_by,
        }
    }
}

/// Inputs to the fixed-priority rule chain.
#[derive(Debug, Clone, Default)]
pub struct RoutingInputs {
    pub explicit: Option<RouteCandidate>,
    pub workflow: Option<RouteCandidate>,
    pub deterministic: Option<RouteCandidate>,
    pub coordinator: Option<RouteCandidate>,
}

/// A single route or a request for clarification.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutingOutcome {
    Selected(RoutingPlan),
    NeedsClarification {
        reason: &'static str,
        confidence: Option<f64>,
    },
}

/// Resolves at most one executor using the documented rule order.
#[derive(Debug, Clone, Copy)]
pub struct OrderedRouter {
    confidence_threshold: f64,
}

impl OrderedRouter {
    /// Creates a router with a validated coordinator threshold.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` when the threshold is outside zero through one.
    pub fn new(confidence_threshold: f64) -> Result<Self, CoreError> {
        if (0.0..=1.0).contains(&confidence_threshold) && !confidence_threshold.is_nan() {
            Ok(Self {
                confidence_threshold,
            })
        } else {
            Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "routing threshold must be between zero and one",
            ))
        }
    }

    /// Selects the first successful route and never broadcasts.
    #[must_use]
    pub fn resolve(self, inputs: RoutingInputs) -> RoutingOutcome {
        if let Some(candidate) = inputs.explicit {
            return RoutingOutcome::Selected(candidate.into_plan(SelectedRule::Explicit));
        }
        if let Some(candidate) = inputs.workflow {
            return RoutingOutcome::Selected(candidate.into_plan(SelectedRule::Workflow));
        }
        if let Some(candidate) = inputs.deterministic {
            return RoutingOutcome::Selected(candidate.into_plan(SelectedRule::Resolver));
        }
        if let Some(candidate) = inputs.coordinator {
            if candidate.confidence >= self.confidence_threshold {
                return RoutingOutcome::Selected(candidate.into_plan(SelectedRule::Coordinator));
            }
            return RoutingOutcome::NeedsClarification {
                reason: "coordinator confidence is below the configured threshold",
                confidence: Some(candidate.confidence),
            };
        }
        RoutingOutcome::NeedsClarification {
            reason: "no routing rule selected an executor",
            confidence: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        OrderedRouter, PermissionScope, Risk, RouteCandidate, RouteContext, RouteDestination,
        RoutingInputs, RoutingOutcome, SelectedRule,
    };
    use crate::value::{ModelAlias, ProviderId, RoleId};

    fn candidate(role: &str, confidence: f64) -> RouteCandidate {
        RouteCandidate::new(
            "test",
            RouteDestination {
                role: RoleId::parse(role).expect("role"),
                model_alias: ModelAlias::parse("model").expect("model"),
                provider: ProviderId::parse("provider").expect("provider"),
                runtime_model: "runtime".to_owned(),
            },
            RouteContext {
                tools: vec![],
                data_sources: vec![],
                permission: PermissionScope::ReadOnly,
            },
            Risk::Low,
            confidence,
        )
        .expect("candidate")
    }

    #[test]
    fn explicit_route_prevents_coordinator_selection() {
        let outcome = OrderedRouter::new(0.85)
            .expect("threshold")
            .resolve(RoutingInputs {
                explicit: Some(candidate("explicit", 1.0)),
                coordinator: Some(candidate("coordinator", 1.0)),
                ..RoutingInputs::default()
            });
        let RoutingOutcome::Selected(plan) = outcome else {
            panic!("expected route");
        };
        assert_eq!(plan.selected_by, SelectedRule::Explicit);
        assert_eq!(plan.destination.role.as_str(), "explicit");
    }

    proptest! {
        #[test]
        fn coordinator_below_threshold_never_dispatches(confidence in 0.0_f64..0.85) {
            let outcome = OrderedRouter::new(0.85).expect("threshold").resolve(RoutingInputs {
                coordinator: Some(candidate("coordinator", confidence)),
                ..RoutingInputs::default()
            });
            let asks_for_clarification =
                matches!(outcome, RoutingOutcome::NeedsClarification { .. });
            prop_assert!(asks_for_clarification);
        }
    }
}
