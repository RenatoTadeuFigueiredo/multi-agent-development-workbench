use std::sync::Arc;

use serde_json::json;
use time::{Duration, OffsetDateTime};
use workbench_core::{
    AttemptId, FailureCategory,
    ports::{
        CancellationStatus, Clock, ProviderAdapter, ProviderFailure, ProviderRegistry, Telemetry,
    },
    routing::{
        PermissionScope, Risk, RouteCandidate, RouteContext, RouteDestination, SelectedRule,
    },
    value::{ModelAlias, NonEmptyText, ProviderId, RoleId},
};
use workbench_protocol::{Command, command::EmptyParams};
use workbench_storage::KeyStore;
use workbench_testkit::{
    CoordinatorBehavior, DenyNetwork, FakeClock, FakeKeyStore, FakeProvider, FakeProviderRegistry,
    FakeTool, StreamBehavior, TelemetryRecord, TelemetrySink,
    client::{ClientCommandFactory, round_trip_command},
    contracts::{verify_failure_contract, verify_happy_path_contract},
};

#[tokio::test]
async fn fake_provider_passes_the_reusable_happy_contract_offline() {
    let provider = FakeProvider::builder(provider_id("fake")).build();
    let report = verify_happy_path_contract(&provider)
        .await
        .expect("provider contract");

    assert!(report.acknowledged);
    assert_eq!(report.content_events, 1);
    assert_eq!(report.tool_events, 1);
    assert!(report.completed);
    assert_eq!(report.cancellation, CancellationStatus::Confirmed);
    assert_eq!(provider.prompts().len(), 1);
    assert_eq!(
        provider.call_counts(),
        workbench_testkit::ProviderCallCounts {
            capabilities: 1,
            authentication: 1,
            start: 1,
            resume: 1,
            prompt: 1,
            cancel: 1,
            classify: 0,
        }
    );
    provider
        .network_guard()
        .assert_unused()
        .expect("provider stayed offline");
}

#[tokio::test]
async fn failure_contract_accepts_only_normalized_safe_failures() {
    let failure = ProviderFailure {
        category: FailureCategory::ProviderTimeout,
        user_safe_message: "provider timed out".to_owned(),
        definite: false,
    };
    let provider = FakeProvider::builder(provider_id("failure"))
        .stream(StreamBehavior::Emit(vec![Err(failure.clone())]))
        .build();

    let observed = verify_failure_contract(&provider)
        .await
        .expect("normalized failure");
    assert_eq!(observed, failure);
    assert_eq!(provider.call_counts().prompt, 1);
}

#[tokio::test]
async fn coordinator_registry_and_call_counters_are_deterministic() {
    let provider_id = provider_id("coordinator");
    let candidate = RouteCandidate::new(
        "implementation",
        RouteDestination {
            role: RoleId::parse("implementer").expect("role"),
            model_alias: ModelAlias::parse("implementation").expect("model"),
            provider: provider_id.clone(),
            runtime_model: "fake-runtime".to_owned(),
        },
        RouteContext {
            tools: Vec::new(),
            data_sources: Vec::new(),
            permission: PermissionScope::ReadOnly,
        },
        Risk::Low,
        0.95,
    )
    .expect("candidate");
    let provider = FakeProvider::builder(provider_id.clone())
        .coordinator(CoordinatorBehavior::Candidate(candidate))
        .build();
    let registry = FakeProviderRegistry::default();
    registry.register(provider_id.clone(), Arc::new(provider.clone()));

    let adapter = registry.adapter(&provider_id).expect("registered provider");
    let classified = adapter
        .classify(NonEmptyText::parse("route this").expect("text"))
        .await
        .expect("classification");
    let plan = workbench_core::routing::OrderedRouter::new(0.85)
        .expect("router")
        .resolve(workbench_core::routing::RoutingInputs {
            coordinator: Some(classified),
            ..workbench_core::routing::RoutingInputs::default()
        });
    let workbench_core::routing::RoutingOutcome::Selected(plan) = plan else {
        panic!("expected selected route");
    };
    assert_eq!(plan.selected_by, SelectedRule::Coordinator);
    assert_eq!(provider.call_counts().classify, 1);
    assert_eq!(registry.len(), 1);
    assert!(registry.remove(&provider_id).is_some());
    assert!(registry.is_empty());
}

#[test]
fn deterministic_support_fakes_are_observable_and_offline() {
    let clock = FakeClock::new(OffsetDateTime::UNIX_EPOCH);
    assert_eq!(
        clock.advance(Duration::seconds(5)),
        OffsetDateTime::UNIX_EPOCH + Duration::seconds(5)
    );
    assert_eq!(
        clock.now(),
        OffsetDateTime::UNIX_EPOCH + Duration::seconds(5)
    );

    let tool = FakeTool::succeeding(json!({"ok": true}));
    let result = tool
        .execute("inspect", json!({"path": "fixture"}))
        .expect("tool");
    assert_eq!(result.content, json!({"ok": true}));
    assert_eq!(tool.call_count(), 1);
    assert_eq!(tool.calls()[0].operation, "inspect");

    let telemetry = TelemetrySink::default();
    telemetry.record_route("explicit", "success");
    telemetry.record_attempt("completed");
    assert_eq!(
        telemetry.records(),
        [
            TelemetryRecord::Route {
                selected_rule: "explicit",
                outcome: "success"
            },
            TelemetryRecord::Attempt {
                outcome: "completed"
            }
        ]
    );

    let network = DenyNetwork::default();
    assert!(network.request("redacted").is_err());
    assert_eq!(network.attempts(), 1);

    let key_store = FakeKeyStore::new();
    key_store.put("test/key", b"secret").expect("put");
    assert_eq!(
        key_store
            .get("test/key")
            .expect("get")
            .expect("entry")
            .as_slice(),
        b"secret"
    );
}

#[test]
fn client_helper_generates_strict_v7_session_commands() {
    let factory = ClientCommandFactory::with_new_session();
    let command = factory.command(Command::SessionPause(EmptyParams {}));
    assert_eq!(command.session_id, Some(factory.session_id()));
    assert_eq!(command.request_id.get_version_num(), 7);
    let decoded = round_trip_command(&command).expect("strict round trip");
    assert_eq!(decoded, command);
}

#[tokio::test]
async fn cancellation_and_classification_failures_are_configurable() {
    let provider = FakeProvider::builder(provider_id("unavailable"))
        .cancellation(Err(workbench_core::CoreError::new(
            FailureCategory::ProviderUnavailable,
            "cancel unavailable",
        )))
        .coordinator(CoordinatorBehavior::Unsupported)
        .build();
    let handle = provider.start_session().await.expect("start");
    let cancel = provider.cancel(&handle, AttemptId::new()).await;
    assert!(cancel.is_err());
    let classify = provider
        .classify(NonEmptyText::parse("classify").expect("text"))
        .await;
    assert!(classify.is_err());
    assert_eq!(provider.call_counts().cancel, 1);
    assert_eq!(provider.call_counts().classify, 1);
}

fn provider_id(value: &str) -> ProviderId {
    ProviderId::parse(value).expect("provider ID")
}
