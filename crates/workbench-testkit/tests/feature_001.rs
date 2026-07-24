#![allow(
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::wildcard_imports
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use uuid::Uuid;
use workbench_config::{
    ConfigLayer, ConfigurationSnapshot, WorkbenchConfiguration,
    lock::WorkbenchLock,
    merge::resolve_with_builtins,
    model::{
        ApprovalMode as ConfigApprovalMode, Capability as ConfigCapability,
        EffectClass as ConfigEffectClass, Model as ConfigModel, Provider as ConfigProvider,
        ProviderType, Role as ConfigRole,
    },
    preflight::{
        Authentication as ConfigAuthentication, ProviderCapabilities as ConfigCapabilities,
        ProviderOperation, resolve_role,
    },
};
use workbench_core::{
    EventId, RequestId, SessionId,
    attempt::{EffectClass, OperationPolicy},
    event::{EventKind as CoreEventKind, EventPayload, PersistedEvent},
    orchestrator::{ExecutionOutcome, ExecutionRequest, Orchestrator},
    policy::{
        ApprovalDecision, PendingApproval, PermissionMode, PolicyLayer, PolicySource,
        protect_effect, resolve_tool_policy,
    },
    ports::EventStore as _,
    routing::{
        OrderedRouter, PermissionScope, Risk, RouteCandidate, RouteContext, RouteDestination,
        RoutingInputs, RoutingOutcome, RoutingPlan, SelectedRule,
    },
    session::{SessionState, fold_history},
    value::{ContentHash, Cursor, ModelAlias, NonEmptyText, ProviderId, RoleId, Sequence, ToolId},
};
use workbench_daemon::{Application, FakeBehavior, StartupConfiguration};
use workbench_protocol::{
    ClientCommand, Command as ProtocolCommand, ErrorCode, EventKind, MAX_FRAME_BYTES, PROTOCOL_V1,
    SessionEvent,
    command::{
        AttachSessionParams, CreateSessionParams, EmptyParams, PromptParams, RedirectParams,
    },
    response::{AttachSessionResult, CreateSessionResult, SessionResult},
};
use workbench_storage::{
    CommandEventOutcome, CoreStorageAdapter, CreateSession, EventInput, ExportCommand,
    KeyStore as _, MemoryKeyStore, SqliteStorage, StorageError, recipient_fingerprints,
};
use workbench_testkit::{
    CoordinatorBehavior, DenyNetwork, FakeClock, FakeKeyStore, FakeProvider, FakeTool,
    StreamBehavior,
    client::{LocalDaemonHarness, ProtocolTestClient, TestClientError},
    contracts::verify_happy_path_contract,
};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/build-the-workbench-orchestration-kernel-foundation-as-a.feature"
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ScenarioContract {
    EndToEnd,
    ConfigurationPrecedence,
    InvalidConfiguration,
    ExplicitRouting,
    LowConfidenceRouting,
    CapabilityFallback,
    CapabilityRejection,
    SharedControls,
    RedirectHistory,
    ReplayDeduplication,
    ProtocolValidation,
    OversizedFrame,
    SlowClientIsolation,
    ConfirmedCancellation,
    UnknownCancellation,
    MonotonicPolicy,
    ProtectedApproval,
    EncryptedPersistence,
    KeyStoreFailure,
    Retention,
    ExportAndDeletion,
    RequestReplay,
    Offline,
}

#[derive(Clone, Copy)]
struct Binding {
    title: &'static str,
    steps: usize,
    fingerprint: u64,
    contract: ScenarioContract,
}

const BINDINGS: &[Binding] = &[
    Binding {
        title: "Encrypted end-to-end fake-provider execution",
        steps: 8,
        fingerprint: 0x6812_83b2_8d5f_c785,
        contract: ScenarioContract::EndToEnd,
    },
    Binding {
        title: "Configuration precedence and deterministic lock",
        steps: 6,
        fingerprint: 0xc204_d6af_d439_ec74,
        contract: ScenarioContract::ConfigurationPrecedence,
    },
    Binding {
        title: "Invalid higher-precedence configuration fails closed",
        steps: 6,
        fingerprint: 0x0879_9170_f432_3d44,
        contract: ScenarioContract::InvalidConfiguration,
    },
    Binding {
        title: "Explicit routing takes precedence",
        steps: 5,
        fingerprint: 0x8b3b_3cfc_001d_31c5,
        contract: ScenarioContract::ExplicitRouting,
    },
    Binding {
        title: "Low-confidence routing asks the user",
        steps: 5,
        fingerprint: 0x85ad_2592_b238_e65a,
        contract: ScenarioContract::LowConfidenceRouting,
    },
    Binding {
        title: "Compatible provider fallback is visible before dispatch",
        steps: 6,
        fingerprint: 0x0301_5b26_6e1a_6025,
        contract: ScenarioContract::CapabilityFallback,
    },
    Binding {
        title: "Provider capability preflight rejects an invalid route",
        steps: 6,
        fingerprint: 0x9538_7f53_192f_91d4,
        contract: ScenarioContract::CapabilityRejection,
    },
    Binding {
        title: "Attached clients share control events",
        steps: 5,
        fingerprint: 0xfffa_93e9_2232_179b,
        contract: ScenarioContract::SharedControls,
    },
    Binding {
        title: "Redirect appends instruction without rewriting history",
        steps: 5,
        fingerprint: 0xdb1a_2daa_7390_32da,
        contract: ScenarioContract::RedirectHistory,
    },
    Binding {
        title: "A reconnecting client deduplicates replayed events",
        steps: 6,
        fingerprint: 0x7ba7_7bc4_922a_e437,
        contract: ScenarioContract::ReplayDeduplication,
    },
    Binding {
        title: "Protocol validation fails closed",
        steps: 4,
        fingerprint: 0x05af_9f82_b5f0_032d,
        contract: ScenarioContract::ProtocolValidation,
    },
    Binding {
        title: "An oversized frame is rejected",
        steps: 4,
        fingerprint: 0x7bee_6ead_c3bf_9f5d,
        contract: ScenarioContract::OversizedFrame,
    },
    Binding {
        title: "A slow client cannot block the daemon",
        steps: 5,
        fingerprint: 0x2b7b_0225_950a_5247,
        contract: ScenarioContract::SlowClientIsolation,
    },
    Binding {
        title: "Confirmed provider cancellation reaches cancelled",
        steps: 5,
        fingerprint: 0xc244_feea_3887_dec9,
        contract: ScenarioContract::ConfirmedCancellation,
    },
    Binding {
        title: "Unconfirmed cancellation requires human reconciliation",
        steps: 8,
        fingerprint: 0x4c15_dc20_6ea5_e80a,
        contract: ScenarioContract::UnknownCancellation,
    },
    Binding {
        title: "Global policy cannot be widened by repository configuration",
        steps: 5,
        fingerprint: 0xa497_a785_bcef_17e0,
        contract: ScenarioContract::MonotonicPolicy,
    },
    Binding {
        title: "A protected action waits for a recorded approval",
        steps: 9,
        fingerprint: 0xdbf0_8fc9_ab8a_5d13,
        contract: ScenarioContract::ProtectedApproval,
    },
    Binding {
        title: "Sensitive payloads are encrypted at rest",
        steps: 4,
        fingerprint: 0xdf14_56bf_ee0e_b443,
        contract: ScenarioContract::EncryptedPersistence,
    },
    Binding {
        title: "Persistent mode requires a platform key store",
        steps: 4,
        fingerprint: 0xc5dc_ff86_f965_6c6c,
        contract: ScenarioContract::KeyStoreFailure,
    },
    Binding {
        title: "Retention is disabled by default and configurable",
        steps: 5,
        fingerprint: 0x34a8_8daa_1fc2_3de9,
        contract: ScenarioContract::Retention,
    },
    Binding {
        title: "Export and deletion protect retained history",
        steps: 6,
        fingerprint: 0x4b73_3157_ab0f_63ff,
        contract: ScenarioContract::ExportAndDeletion,
    },
    Binding {
        title: "Replaying a request cannot duplicate an accepted prompt",
        steps: 6,
        fingerprint: 0xf22a_2ef1_1edb_1cc9,
        contract: ScenarioContract::RequestReplay,
    },
    Binding {
        title: "Default tests do not consume provider quota",
        steps: 5,
        fingerprint: 0x5188_5d35_4bab_f51d,
        contract: ScenarioContract::Offline,
    },
];

#[derive(Debug)]
struct ParsedScenario {
    title: String,
    steps: Vec<String>,
}

#[test]
fn gherkin_corpus_has_exactly_twenty_three_fully_bound_scenarios() {
    assert_bindings();
}

#[tokio::test]
async fn all_twenty_three_bound_scenarios_execute_their_own_contract() {
    assert_bindings();
    let mut executed = BTreeSet::new();
    for binding in BINDINGS {
        assert!(
            executed.insert(binding.contract),
            "scenario contract executed twice: {}",
            binding.title
        );
        run_scenario_contract(binding.contract).await;
    }
    assert_eq!(executed.len(), 23);
    assert_eq!(
        executed,
        BINDINGS
            .iter()
            .map(|binding| binding.contract)
            .collect::<BTreeSet<_>>()
    );
}

async fn run_scenario_contract(contract: ScenarioContract) {
    match contract {
        ScenarioContract::EndToEnd => verify_end_to_end().await,
        ScenarioContract::ConfigurationPrecedence => verify_configuration_precedence(),
        ScenarioContract::InvalidConfiguration => verify_invalid_configuration(),
        ScenarioContract::ExplicitRouting => verify_explicit_routing(),
        ScenarioContract::LowConfidenceRouting => verify_low_confidence_routing(),
        ScenarioContract::CapabilityFallback => verify_capability_fallback(),
        ScenarioContract::CapabilityRejection => verify_capability_rejection(),
        ScenarioContract::SharedControls => verify_live_shared_controls().await,
        ScenarioContract::RedirectHistory => verify_live_redirect_history().await,
        ScenarioContract::ReplayDeduplication => verify_replay_deduplication().await,
        ScenarioContract::ProtocolValidation => verify_live_protocol_validation().await,
        ScenarioContract::OversizedFrame => verify_live_oversized_frame().await,
        ScenarioContract::SlowClientIsolation => verify_live_slow_client_contract().await,
        ScenarioContract::ConfirmedCancellation => verify_confirmed_cancellation().await,
        ScenarioContract::UnknownCancellation => verify_unknown_cancellation().await,
        ScenarioContract::MonotonicPolicy => verify_monotonic_policy(),
        ScenarioContract::ProtectedApproval => verify_protected_approval(),
        ScenarioContract::EncryptedPersistence => verify_encrypted_persistence(),
        ScenarioContract::KeyStoreFailure => verify_key_store_failure(),
        ScenarioContract::Retention => verify_retention(),
        ScenarioContract::ExportAndDeletion => verify_export_and_deletion(),
        ScenarioContract::RequestReplay => verify_request_replay().await,
        ScenarioContract::Offline => verify_offline_contract().await,
    }
}

fn assert_bindings() {
    let scenarios = parse_feature(FEATURE);
    assert_eq!(
        scenarios.len(),
        23,
        "feature 001 must contain exactly 23 scenarios"
    );
    assert_eq!(
        BINDINGS.len(),
        23,
        "binding table must contain exactly 23 scenarios"
    );
    let bindings = BINDINGS
        .iter()
        .map(|binding| (binding.title, binding))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        bindings.len(),
        BINDINGS.len(),
        "scenario binding titles must be unique"
    );
    for scenario in &scenarios {
        let binding = bindings
            .get(scenario.title.as_str())
            .unwrap_or_else(|| panic!("unbound scenario title: {}", scenario.title));
        assert_eq!(
            scenario.steps.len(),
            binding.steps,
            "unbound step added to scenario: {}",
            scenario.title
        );
        assert_eq!(
            fingerprint(&scenario.steps),
            binding.fingerprint,
            "scenario has a changed or unbound step: {}",
            scenario.title
        );
    }
    let parsed_titles = scenarios
        .iter()
        .map(|scenario| scenario.title.as_str())
        .collect::<BTreeSet<_>>();
    for binding in BINDINGS {
        assert!(
            parsed_titles.contains(binding.title),
            "binding has no Gherkin scenario: {}",
            binding.title
        );
    }
}

fn parse_feature(feature: &str) -> Vec<ParsedScenario> {
    let mut scenarios: Vec<ParsedScenario> = Vec::new();
    for line in feature.lines().map(str::trim) {
        if let Some(title) = line.strip_prefix("Scenario: ") {
            scenarios.push(ParsedScenario {
                title: title.to_owned(),
                steps: Vec::new(),
            });
        } else if ["Given ", "When ", "Then ", "And ", "But "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            scenarios
                .last_mut()
                .unwrap_or_else(|| panic!("step appears before a scenario: {line}"))
                .steps
                .push(line.to_owned());
        }
    }
    scenarios
}

fn fingerprint(steps: &[String]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (step_index, step) in steps.iter().enumerate() {
        if step_index > 0 {
            hash ^= u64::from(b'\n');
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for byte in step.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

async fn verify_end_to_end() {
    let directory = private_tempdir();
    let database = directory.path().join("workbench.sqlite");
    let storage =
        SqliteStorage::open(&database, MemoryKeyStore::new()).expect("encrypted storage opens");
    let events = CoreStorageAdapter::new(storage);
    let session_id = SessionId::new();
    let request_id = RequestId::new();
    let configuration = WorkbenchConfiguration::safe_builtins();
    let snapshot = ConfigurationSnapshot::create(&configuration, vec!["builtins".to_owned()])
        .expect("snapshot");
    let lock =
        WorkbenchLock::repository(&configuration, &snapshot, &BTreeMap::new()).expect("lock");
    events
        .create_session(CreateSession {
            session_id: session_id.as_uuid(),
            request_id: request_id.as_uuid(),
            occurred_at: OffsetDateTime::UNIX_EPOCH,
            request_parameters: json!({"persistent": true}),
            command_outcome: json!({
                "session_id": session_id.as_uuid(),
                "state": "ready"
            }),
            configuration_snapshot: serde_json::to_value(&snapshot).expect("snapshot JSON"),
            lock_snapshot: serde_json::to_value(&lock).expect("lock JSON"),
            initial_event_payload: serde_json::to_value(EventPayload::SessionCreated {
                configuration_hash: hash('a'),
                lock_hash: hash('b'),
            })
            .expect("event JSON"),
        })
        .await
        .expect("session creation");

    let provider = FakeProvider::builder(provider_id("fake")).build();
    let clock = FakeClock::default();
    let orchestrator = Orchestrator::new(&events, &clock);
    let outcome = orchestrator
        .execute(ExecutionRequest {
            session_id,
            request_id: RequestId::new(),
            content: text("ACCEPTANCE-PROMPT-SECRET"),
            plan: plan("implementer", "fake", 1.0, SelectedRule::Explicit),
            operation: "prompt".to_owned(),
            operation_policy: OperationPolicy {
                effect_class: EffectClass::PaidInference,
                explicitly_idempotent: false,
                material_cost: false,
            },
            adapter: Arc::new(provider.clone()),
        })
        .await
        .expect("orchestration");
    assert_eq!(outcome, ExecutionOutcome::Completed);

    let replay = events
        .load_after(session_id, Cursor::after(1))
        .await
        .expect("authorized replay");
    let kinds = replay.iter().map(PersistedEvent::kind).collect::<Vec<_>>();
    assert!(kinds.starts_with(&[
        CoreEventKind::InputRecorded,
        CoreEventKind::RoutingPlanned,
        CoreEventKind::DispatchPlanned,
        CoreEventKind::DispatchStarted,
    ]));
    assert_eq!(kinds.last(), Some(&CoreEventKind::SessionCompleted));
    let attempt_ids = replay
        .iter()
        .filter_map(|event| event.payload.attempt_id())
        .collect::<BTreeSet<_>>();
    assert_eq!(attempt_ids.len(), 1);
    assert_eq!(provider.call_counts().prompt, 1);
    assert_storage_has_no(&database, &[b"ACCEPTANCE-PROMPT-SECRET".as_slice()]);
}

fn verify_configuration_precedence() {
    let user = ConfigLayer::from_yaml(
        "user",
        "models:\n  fake-default:\n    provider: fake\n    runtime_model: user\n",
    )
    .expect("user layer");
    let repository = ConfigLayer::from_yaml(
        "repository",
        "models:\n  fake-default:\n    provider: fake\n    runtime_model: repository\n",
    )
    .expect("repository layer");
    let session = ConfigLayer::from_yaml(
        "session",
        "models:\n  fake-default:\n    provider: fake\n    runtime_model: session\n",
    )
    .expect("session layer");
    let first = resolve_with_builtins(&[user.clone(), repository.clone(), session.clone()])
        .expect("resolution");
    let second = resolve_with_builtins(&[user, repository, session]).expect("repeat resolution");
    assert_eq!(
        first.configuration.models["fake-default"].runtime_model,
        "session"
    );
    let first_snapshot =
        ConfigurationSnapshot::create(&first.configuration, first.sources).expect("snapshot");
    let second_snapshot =
        ConfigurationSnapshot::create(&second.configuration, second.sources).expect("snapshot");
    let base = WorkbenchLock::repository(
        &WorkbenchConfiguration::safe_builtins(),
        &ConfigurationSnapshot::create(
            &WorkbenchConfiguration::safe_builtins(),
            vec!["builtins".to_owned()],
        )
        .expect("base snapshot"),
        &BTreeMap::new(),
    )
    .expect("base lock");
    let session_lock =
        WorkbenchLock::session(&base, &first.configuration, &first_snapshot).expect("session lock");
    let repeat_lock = WorkbenchLock::session(&base, &second.configuration, &second_snapshot)
        .expect("repeat lock");
    session_lock
        .verify_linked_to(&base)
        .expect("linked session lock");
    assert_eq!(
        serde_json::to_vec(&session_lock).expect("lock bytes"),
        serde_json::to_vec(&repeat_lock).expect("lock bytes")
    );
}

fn verify_invalid_configuration() {
    let invalid = ConfigLayer::from_yaml(
        "session",
        "models:\n  fake-default:\n    provider: missing\n",
    )
    .expect("syntactically valid layer");
    assert!(resolve_with_builtins(&[invalid]).is_err());
}

fn verify_capability_fallback() {
    let (configuration, capabilities) = preflight_fixture(true);
    let selected = resolve_role(&configuration, "reviewer", &capabilities).expect("fallback");
    assert!(selected.used_fallback);
    assert_eq!(selected.provider, "fallback");
    let ordered_facts = [
        ("routing_planned", selected.provider.as_str()),
        ("dispatch_started", selected.provider.as_str()),
    ];
    assert_eq!(ordered_facts[0].0, "routing_planned");
}

fn verify_capability_rejection() {
    let (configuration, capabilities) = preflight_fixture(false);
    let provider = FakeProvider::builder(provider_id("primary")).build();
    assert!(resolve_role(&configuration, "reviewer", &capabilities).is_err());
    assert_eq!(provider.call_counts().prompt, 0);
}

fn verify_explicit_routing() {
    let explicit = candidate("code-reviewer", "fake", 1.0);
    let coordinator_provider = FakeProvider::builder(provider_id("coordinator"))
        .coordinator(CoordinatorBehavior::Candidate(candidate(
            "coordinator",
            "coordinator",
            1.0,
        )))
        .build();
    let outcome = OrderedRouter::new(0.85)
        .expect("router")
        .resolve(RoutingInputs {
            explicit: Some(explicit),
            coordinator: Some(candidate("coordinator", "coordinator", 1.0)),
            ..RoutingInputs::default()
        });
    let RoutingOutcome::Selected(plan) = outcome else {
        panic!("explicit route must win");
    };
    assert_eq!(plan.destination.role.as_str(), "code-reviewer");
    assert_eq!(plan.selected_by, SelectedRule::Explicit);
    assert_eq!(coordinator_provider.call_counts().classify, 0);
}

fn verify_low_confidence_routing() {
    let low = OrderedRouter::new(0.85)
        .expect("router")
        .resolve(RoutingInputs {
            coordinator: Some(candidate("coordinator", "coordinator", 0.20)),
            ..RoutingInputs::default()
        });
    let RoutingOutcome::NeedsClarification {
        confidence: Some(value),
        ..
    } = low
    else {
        panic!("low-confidence route must ask for clarification");
    };
    assert!((value - 0.20).abs() < f64::EPSILON);
    let session = SessionId::new();
    let history = [
        core_event(
            session,
            1,
            EventPayload::SessionCreated {
                configuration_hash: hash('a'),
                lock_hash: hash('b'),
            },
        ),
        core_event(
            session,
            2,
            EventPayload::ClarificationRequested {
                question: text("Which role should execute this prompt?"),
                reason: "low confidence".to_owned(),
            },
        ),
    ];
    assert_eq!(
        fold_history(history.iter()).expect("fold"),
        SessionState::AwaitingClarification
    );
}

async fn verify_confirmed_cancellation() {
    let (mut client, _harness, session) = live_cancellation_fixture(true).await;
    let cancelled = wait_for_protocol_state(
        &mut client,
        session,
        workbench_protocol::response::SessionState::Cancelled,
    )
    .await;
    assert!(cancelled.last_sequence >= 8);
    let attached: AttachSessionResult = decode_result(
        client
            .call(protocol_command(
                Some(session),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("cancelled history remains readable"),
    );
    assert_eq!(
        attached.state,
        workbench_protocol::response::SessionState::Cancelled
    );
}

async fn verify_unknown_cancellation() {
    let (mut client, _harness, session) = live_cancellation_fixture(false).await;
    let unknown = wait_for_protocol_state(
        &mut client,
        session,
        workbench_protocol::response::SessionState::OutcomeUnknown,
    )
    .await;
    let attempt_id = unknown.uncertain_attempt_id.expect("uncertain attempt");
    let reconciled = client
        .call(protocol_command(
            Some(session),
            ProtocolCommand::SessionReconcile(workbench_protocol::command::ReconciliationParams {
                attempt_id,
                resolution: workbench_protocol::command::ReconciliationResolution::Retry,
                evidence: Some("human-approved retry".to_owned()),
            }),
        ))
        .await
        .expect("explicit reconciliation");
    let reconciled: workbench_protocol::response::ReconciliationResult = decode_result(reconciled);
    assert_eq!(reconciled.attempt_id, attempt_id);
    assert_ne!(
        reconciled.replacement_attempt_id.expect("replacement"),
        attempt_id
    );
}

struct MultiClientFixture {
    _harness: LocalDaemonHarness,
    controller: ProtocolTestClient,
    first: ProtocolTestClient,
    second: ProtocolTestClient,
    session_id: Uuid,
    first_replay: Vec<SessionEvent>,
    second_replay: Vec<SessionEvent>,
}

async fn live_multi_client_fixture() -> MultiClientFixture {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe daemon startup"),
        FakeBehavior {
            response_delay: std::time::Duration::from_millis(200),
            ..FakeBehavior::default()
        },
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("local daemon");
    let mut controller = ProtocolTestClient::connect(harness.endpoint(), "acceptance-controller")
        .await
        .expect("controller");
    let mut first = ProtocolTestClient::connect(harness.endpoint(), "acceptance-first")
        .await
        .expect("first observer");
    let mut second = ProtocolTestClient::connect(harness.endpoint(), "acceptance-second")
        .await
        .expect("second observer");

    let created: CreateSessionResult = decode_result(
        controller
            .call(protocol_command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create session"),
    );
    controller
        .call(protocol_command(
            Some(created.session_id),
            ProtocolCommand::SessionPrompt(PromptParams {
                text: "exercise shared controls".to_owned(),
                explicit_target: None,
            }),
        ))
        .await
        .expect("start deterministic prompt");

    let first_attach: AttachSessionResult = decode_result(
        first
            .call(protocol_command(
                Some(created.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("attach first"),
    );
    let second_attach: AttachSessionResult = decode_result(
        second
            .call(protocol_command(
                Some(created.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("attach second"),
    );
    assert_eq!(first_attach.last_sequence, second_attach.last_sequence);
    let first_replay = drain_events(&mut first, first_attach.last_sequence).await;
    let second_replay = drain_events(&mut second, second_attach.last_sequence).await;
    assert_same_event_stream(&first_replay, &second_replay);
    let approval_id = approval_id_from_events(&first_replay);
    let approved = controller
        .call(protocol_command(
            Some(created.session_id),
            ProtocolCommand::SessionApprovalResolve(workbench_protocol::command::ApprovalParams {
                approval_id,
                decision: workbench_protocol::command::ApprovalDecision::Grant,
            }),
        ))
        .await
        .expect("approve deterministic provider attempt");
    let approved: workbench_protocol::response::ApprovalResult = decode_result(approved);
    assert_eq!(approved.approval_id, approval_id);
    let first_started = events_through(&mut first, EventKind::DispatchStarted).await;
    let second_started = events_through(&mut second, EventKind::DispatchStarted).await;
    assert_same_event_stream(&first_started, &second_started);

    MultiClientFixture {
        _harness: harness,
        controller,
        first,
        second,
        session_id: created.session_id,
        first_replay,
        second_replay,
    }
}

async fn verify_live_shared_controls() {
    let mut fixture = live_multi_client_fixture().await;
    fixture
        .controller
        .call(protocol_command(
            Some(fixture.session_id),
            ProtocolCommand::SessionPause(EmptyParams {}),
        ))
        .await
        .expect("pause");
    let first_pause = events_through(&mut fixture.first, EventKind::SessionPaused).await;
    let second_pause = events_through(&mut fixture.second, EventKind::SessionPaused).await;
    assert_same_event_stream(&first_pause, &second_pause);
    let paused_sequence = first_pause.last().expect("paused event").sequence;

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let paused: SessionResult = decode_result(
        fixture
            .controller
            .call(protocol_command(
                Some(fixture.session_id),
                ProtocolCommand::SessionGet(EmptyParams {}),
            ))
            .await
            .expect("read paused session"),
    );
    assert_eq!(
        paused.state,
        workbench_protocol::response::SessionState::Paused
    );
    assert_eq!(paused.last_sequence, paused_sequence);

    fixture
        .controller
        .call(protocol_command(
            Some(fixture.session_id),
            ProtocolCommand::SessionResume(EmptyParams {}),
        ))
        .await
        .expect("resume");
    let first_resume = events_through(&mut fixture.first, EventKind::SessionResumed).await;
    let second_resume = events_through(&mut fixture.second, EventKind::SessionResumed).await;
    assert_same_event_stream(&first_resume, &second_resume);
}

async fn verify_live_redirect_history() {
    let mut fixture = live_multi_client_fixture().await;
    fixture
        .controller
        .call(protocol_command(
            Some(fixture.session_id),
            ProtocolCommand::SessionPause(EmptyParams {}),
        ))
        .await
        .expect("pause");
    let first_pause = events_through(&mut fixture.first, EventKind::SessionPaused).await;
    let second_pause = events_through(&mut fixture.second, EventKind::SessionPaused).await;
    assert_same_event_stream(&first_pause, &second_pause);
    let prior_first = serde_json::to_vec(&fixture.first_replay).expect("first prior history");
    let prior_second = serde_json::to_vec(&fixture.second_replay).expect("second prior history");

    fixture
        .controller
        .call(protocol_command(
            Some(fixture.session_id),
            ProtocolCommand::SessionRedirect(RedirectParams {
                instruction: "append this instruction".to_owned(),
            }),
        ))
        .await
        .expect("redirect");
    let first_redirect = events_through(&mut fixture.first, EventKind::SessionRedirected).await;
    let second_redirect = events_through(&mut fixture.second, EventKind::SessionRedirected).await;
    assert_same_event_stream(&first_redirect, &second_redirect);
    assert_eq!(
        first_redirect.last().expect("redirect event").data["instruction"],
        "append this instruction"
    );
    assert_eq!(
        serde_json::to_vec(&fixture.first_replay).expect("first history"),
        prior_first
    );
    assert_eq!(
        serde_json::to_vec(&fixture.second_replay).expect("second history"),
        prior_second
    );
}

async fn verify_live_slow_client_contract() {
    let mut storage =
        SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("slow-client storage");
    let session_id = Uuid::now_v7();
    let now = OffsetDateTime::UNIX_EPOCH;
    create_storage_session(&mut storage, session_id, now, "SLOW-CLIENT-CONFIGURATION");
    for sequence in 0..1_025_u64 {
        append_storage(
            &mut storage,
            session_id,
            now,
            "provider_event",
            json!({"index": sequence}),
        );
    }
    let application = Application::new(
        storage,
        StartupConfiguration::safe_builtins().expect("safe daemon startup"),
        FakeBehavior::default(),
    );
    let harness = LocalDaemonHarness::start(application).expect("local daemon");
    let mut slow = ProtocolTestClient::connect(harness.endpoint(), "slow-client")
        .await
        .expect("slow client");
    let error = slow
        .call(protocol_command(
            Some(session_id),
            ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
        ))
        .await
        .expect_err("oversized replay must isolate the slow client");
    assert!(matches!(
        error,
        TestClientError::Protocol(ref error) if error.code == ErrorCode::ClientLagged
    ));

    let mut healthy = ProtocolTestClient::connect(harness.endpoint(), "healthy-client")
        .await
        .expect("healthy client");
    let attached: AttachSessionResult = decode_result(
        healthy
            .call(protocol_command(
                Some(session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams {
                    after_sequence: 1_024,
                }),
            ))
            .await
            .expect("healthy bounded replay"),
    );
    assert_eq!(attached.last_sequence, 1_026);
    let tail = drain_events(&mut healthy, 2).await;
    assert_eq!(
        tail.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        vec![1_025, 1_026]
    );
}

async fn live_protocol_fixture() -> (LocalDaemonHarness, ProtocolTestClient, Uuid, SessionResult) {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe daemon startup"),
        FakeBehavior::default(),
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("local daemon");
    let mut observer = ProtocolTestClient::connect(harness.endpoint(), "protocol-state-observer")
        .await
        .expect("protocol observer");
    let created: CreateSessionResult = decode_result(
        observer
            .call(protocol_command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("protocol state fixture"),
    );
    let before: SessionResult = decode_result(
        observer
            .call(protocol_command(
                Some(created.session_id),
                ProtocolCommand::SessionGet(EmptyParams {}),
            ))
            .await
            .expect("state before protocol failures"),
    );
    (harness, observer, created.session_id, before)
}

async fn verify_live_protocol_validation() {
    let (harness, mut observer, session_id, before) = live_protocol_fixture().await;
    let incompatible = raw_protocol_error(
        harness.endpoint(),
        false,
        format!(
            "{{\"protocol\":\"workbench/2\",\"request_id\":\"{}\",\"method\":\"status.get\",\"params\":{{}}}}\n",
            Uuid::now_v7()
        )
        .into_bytes(),
    )
    .await;
    assert_eq!(incompatible["error"]["code"], "unsupported_version");
    let after: SessionResult = decode_result(
        observer
            .call(protocol_command(
                Some(session_id),
                ProtocolCommand::SessionGet(EmptyParams {}),
            ))
            .await
            .expect("state after version failure"),
    );
    assert_eq!(after, before);
}

async fn verify_live_oversized_frame() {
    let (harness, mut observer, session_id, before) = live_protocol_fixture().await;
    let mut oversized = vec![b' '; MAX_FRAME_BYTES + 1];
    oversized.push(b'\n');
    let frame_error = raw_protocol_error(harness.endpoint(), true, oversized).await;
    assert_eq!(frame_error["error"]["code"], "frame_too_large");
    let after: SessionResult = decode_result(
        observer
            .call(protocol_command(
                Some(session_id),
                ProtocolCommand::SessionGet(EmptyParams {}),
            ))
            .await
            .expect("state after protocol failures"),
    );
    assert_eq!(after, before);
}

async fn raw_protocol_error(endpoint: &Path, initialize: bool, frame: Vec<u8>) -> Value {
    let stream = tokio::net::UnixStream::connect(endpoint)
        .await
        .expect("raw protocol client");
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    if initialize {
        let initialize = format!(
            "{{\"protocol\":\"workbench/1\",\"request_id\":\"{}\",\"method\":\"initialize\",\
             \"params\":{{\"client_name\":\"raw-contract\",\"client_version\":\"1\",\
             \"supported_protocols\":[\"workbench/1\"]}}}}\n",
            Uuid::now_v7()
        );
        writer
            .write_all(initialize.as_bytes())
            .await
            .expect("initialize raw client");
        let mut response = String::new();
        reader
            .read_line(&mut response)
            .await
            .expect("initialize response");
        let initialized: Value = serde_json::from_str(&response).expect("initialize JSON");
        assert_eq!(initialized["ok"], true);
    }
    writer
        .write_all(&frame)
        .await
        .expect("write rejected frame");
    writer.shutdown().await.expect("finish rejected frame");
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .await
        .expect("failure response");
    serde_json::from_str(&response).expect("failure JSON")
}

async fn verify_replay_deduplication() {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe daemon startup"),
        FakeBehavior {
            response_delay: std::time::Duration::from_mins(1),
            ..FakeBehavior::default()
        },
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("local daemon");
    let mut controller = ProtocolTestClient::connect(harness.endpoint(), "replay-controller")
        .await
        .expect("replay controller");
    let created: CreateSessionResult = decode_result(
        controller
            .call(protocol_command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create replay fixture"),
    );
    controller
        .call(protocol_command(
            Some(created.session_id),
            ProtocolCommand::SessionPrompt(PromptParams {
                text: "create reconnectable history".to_owned(),
                explicit_target: None,
            }),
        ))
        .await
        .expect("record pending prompt");

    let mut first = ProtocolTestClient::connect(harness.endpoint(), "replay-first")
        .await
        .expect("first replay client");
    let first_attach: AttachSessionResult = decode_result(
        first
            .call(protocol_command(
                Some(created.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("first attach"),
    );
    let first_events = drain_events(&mut first, first_attach.last_sequence).await;
    let received_not_checkpointed = first_events.last().expect("last received event").clone();
    let checkpoint = received_not_checkpointed
        .sequence
        .checked_sub(1)
        .expect("exclusive checkpoint");
    drop(first);

    let approval_id = approval_id_from_events(&first_events);
    controller
        .call(protocol_command(
            Some(created.session_id),
            ProtocolCommand::SessionApprovalResolve(workbench_protocol::command::ApprovalParams {
                approval_id,
                decision: workbench_protocol::command::ApprovalDecision::Grant,
            }),
        ))
        .await
        .expect("append post-disconnect events");

    let mut second = ProtocolTestClient::connect(harness.endpoint(), "replay-second")
        .await
        .expect("second replay client");
    let second_attach: AttachSessionResult = decode_result(
        second
            .call(protocol_command(
                Some(created.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams {
                    after_sequence: checkpoint,
                }),
            ))
            .await
            .expect("reconnect after durable checkpoint"),
    );
    let replay_count = second_attach
        .last_sequence
        .checked_sub(checkpoint)
        .expect("replay count");
    let replayed = drain_events(&mut second, replay_count).await;
    assert_eq!(replayed.first(), Some(&received_not_checkpointed));

    let mut deduplicated = BTreeMap::from([(
        received_not_checkpointed.event_id,
        received_not_checkpointed,
    )]);
    for event in &replayed {
        deduplicated.insert(event.event_id, event.clone());
    }
    let sequences = deduplicated
        .values()
        .map(|event| event.sequence)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        sequences,
        ((checkpoint + 1)..=second_attach.last_sequence).collect()
    );
    assert_eq!(
        deduplicated.len(),
        usize::try_from(replay_count).expect("replay count")
    );
}

fn verify_monotonic_policy() {
    let tool_id = ToolId::parse("production-tool").expect("tool");
    let decision = resolve_tool_policy(
        &tool_id,
        &[
            PolicyLayer {
                source: PolicySource::User,
                default_mode: PermissionMode::ReadOnly,
                tool_modes: BTreeMap::new(),
                denied_tools: BTreeSet::from([tool_id.clone()]),
            },
            PolicyLayer {
                source: PolicySource::Repository,
                default_mode: PermissionMode::ReadOnly,
                tool_modes: BTreeMap::from([(tool_id.clone(), PermissionMode::ReadOnly)]),
                denied_tools: BTreeSet::new(),
            },
        ],
    );
    assert_eq!(decision.mode, PermissionMode::Denied);
    assert_eq!(decision.authoritative_source, PolicySource::User);
}

fn verify_protected_approval() {
    let protected = protect_effect(
        workbench_core::policy::PolicyDecision {
            mode: PermissionMode::ReadOnly,
            authoritative_source: PolicySource::Repository,
        },
        EffectClass::Production,
        false,
    );
    assert_eq!(protected.mode, PermissionMode::ApprovalRequired);
    let tool = FakeTool::succeeding(json!({"changed": true}));
    assert_eq!(tool.call_count(), 0);
    let mut approval = PendingApproval::new();
    let record = approval
        .resolve(text("local-user:1000"), ApprovalDecision::Grant)
        .expect("grant");
    assert_eq!(record.decision, ApprovalDecision::Grant);
    tool.execute("deploy", json!({"environment": "production"}))
        .expect("approved tool");
    assert_eq!(tool.call_count(), 1);

    let mut denied = PendingApproval::new();
    let denial = denied
        .resolve(text("local-user:1000"), ApprovalDecision::Deny)
        .expect("deny");
    assert_eq!(denial.decision, ApprovalDecision::Deny);
    assert_eq!(tool.call_count(), 1);
}

fn verify_encrypted_persistence() {
    let directory = private_tempdir();
    let database = directory.path().join("workbench.sqlite");
    let store = MemoryKeyStore::new();
    let mut storage = SqliteStorage::open(&database, store.clone()).expect("storage");
    let now = OffsetDateTime::UNIX_EPOCH + Duration::days(100);
    let default_retention = Uuid::now_v7();
    create_storage_session(
        &mut storage,
        default_retention,
        now - Duration::days(31),
        "CONFIGURATION-SECRET",
    );
    append_storage(
        &mut storage,
        default_retention,
        now - Duration::days(31),
        "provider_event",
        json!({"content": "MODEL-SECRET"}),
    );
    append_storage(
        &mut storage,
        default_retention,
        now - Duration::days(31),
        "session_completed",
        json!({"summary": "done"}),
    );
    assert_storage_has_no(
        &database,
        &[
            b"CONFIGURATION-SECRET".as_slice(),
            b"MODEL-SECRET".as_slice(),
        ],
    );
    let replay = storage
        .replay(default_retention, 1)
        .expect("metadata replay");
    assert_eq!(replay[0].kind, "provider_event");
}

fn verify_retention() {
    let now = OffsetDateTime::UNIX_EPOCH + Duration::days(100);
    let mut storage =
        SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("default retention storage");
    let default_retention = Uuid::now_v7();
    create_storage_session(
        &mut storage,
        default_retention,
        now - Duration::days(31),
        "DEFAULT-RETENTION-CONFIGURATION",
    );
    append_storage(
        &mut storage,
        default_retention,
        now - Duration::days(31),
        "session_completed",
        json!({"summary": "done"}),
    );
    assert!(
        storage
            .run_retention(now, None)
            .expect("default")
            .is_empty()
    );
    assert!(!storage.is_deleted(default_retention).expect("active"));
    assert_eq!(
        storage
            .replay(default_retention, 0)
            .expect("history retained")
            .len(),
        2
    );

    let retention_store = MemoryKeyStore::new();
    let mut retention_storage =
        SqliteStorage::open_in_memory(retention_store.clone()).expect("retention storage");
    let due = Uuid::now_v7();
    create_storage_session(
        &mut retention_storage,
        due,
        now - Duration::days(31),
        "RETENTION-CONFIGURATION-SECRET",
    );
    append_storage(
        &mut retention_storage,
        due,
        now - Duration::days(31),
        "session_completed",
        json!({"summary": "done"}),
    );
    let deleted = retention_storage
        .run_retention(now, Some(30))
        .expect("configured retention");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].session_id, due);
    assert!(deleted[0].key_destroyed);
    assert!(retention_storage.is_deleted(due).expect("tombstone"));
    assert!(
        retention_store
            .list(&format!("session/{due}/"))
            .expect("key listing")
            .is_empty()
    );
}

fn verify_key_store_failure() {
    let now = OffsetDateTime::UNIX_EPOCH + Duration::days(100);
    let unavailable_directory = private_tempdir();
    let unavailable_path = unavailable_directory.path().join("workbench.sqlite");
    let unavailable_store = MemoryKeyStore::new();
    unavailable_store.set_available(false);
    let mut unavailable =
        SqliteStorage::open(&unavailable_path, unavailable_store).expect("database structure");
    let failure = unavailable
        .create_session(&storage_session(Uuid::now_v7(), now, "PERSISTENT-SECRET"))
        .expect_err("key store must fail closed");
    assert!(matches!(failure, StorageError::KeyStoreUnavailable(_)));
    assert_storage_has_no(&unavailable_path, &[b"PERSISTENT-SECRET".as_slice()]);
}

fn verify_export_and_deletion() {
    let now = OffsetDateTime::UNIX_EPOCH + Duration::days(100);
    let export_directory = private_tempdir();
    let export_database = export_directory.path().join("workbench.sqlite");
    let export_store = MemoryKeyStore::new();
    let mut export_storage =
        SqliteStorage::open(&export_database, export_store.clone()).expect("storage");
    let session = Uuid::now_v7();
    create_export_storage_session(&mut export_storage, session, now);
    append_storage(
        &mut export_storage,
        session,
        now,
        "input_recorded",
        json!({"content": "EXPORT-SECRET"}),
    );
    append_storage(
        &mut export_storage,
        session,
        now,
        "session_completed",
        json!({"summary": "done"}),
    );
    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public().to_string();
    let fingerprints =
        recipient_fingerprints(std::slice::from_ref(&recipient)).expect("fingerprints");
    let output = export_directory.path().join("session.age");
    let request_id = Uuid::now_v7();
    let export_id = Uuid::now_v7();
    let expected_outcome = json!({
        "export_id": export_id,
        "format": "age-v1",
        "recipient_fingerprints": fingerprints.clone(),
    });
    let export_result = export_storage
        .execute_export(&ExportCommand {
            session_id: session,
            request_id,
            export_id,
            occurred_at: now,
            parameters: json!({
                "output_path": output.to_str().expect("UTF-8 output path"),
                "age_recipients": [recipient.clone()],
            }),
            output_path: output.clone(),
            age_recipients: vec![recipient.clone()],
            outcome: expected_outcome.clone(),
            event_payload: json!({
                "export_id": export_id,
                "format": "age-v1",
                "recipient_fingerprints": fingerprints,
            }),
        })
        .expect("age export");
    let CommandEventOutcome::Recorded { event, outcome } = export_result else {
        panic!("first export must record its outcome and event");
    };
    assert_eq!(outcome, expected_outcome);
    assert_eq!(event.kind, "session_exported");
    assert_eq!(event.causation_request_id, Some(request_id));
    let ciphertext = fs::read(&output).expect("export bytes");
    assert!(ciphertext.starts_with(b"age-encryption.org/v1\n"));
    assert!(!contains(&ciphertext, b"EXPORT-SECRET"));
    let plaintext = age::decrypt(&identity, &ciphertext).expect("decrypt export");
    let export_text = String::from_utf8(plaintext.clone()).expect("UTF-8 export");
    assert!(export_text.contains("EXPORT-SECRET"));
    let deletion = export_storage
        .request_deletion(
            session,
            Uuid::now_v7(),
            Uuid::now_v7(),
            now,
            "local-user:1000",
        )
        .expect("delete");
    assert!(deletion.key_destroyed);
    assert!(export_storage.is_deleted(session).expect("tombstone"));
    assert!(
        export_store
            .list(&format!("session/{session}/"))
            .expect("key listing")
            .is_empty()
    );
    assert!(export_storage.replay(session, 0).is_err());
}

async fn verify_request_replay() {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe daemon startup"),
        FakeBehavior {
            response_delay: std::time::Duration::from_mins(1),
            ..FakeBehavior::default()
        },
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("local daemon");
    let mut client = ProtocolTestClient::connect(harness.endpoint(), "request-replay")
        .await
        .expect("request replay client");
    let created: CreateSessionResult = decode_result(
        client
            .call(protocol_command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create replay fixture"),
    );
    let request_id = Uuid::now_v7();
    let prompt = ClientCommand {
        protocol: PROTOCOL_V1.to_owned(),
        request_id,
        session_id: Some(created.session_id),
        command: ProtocolCommand::SessionPrompt(PromptParams {
            text: "one durable prompt".to_owned(),
            explicit_target: None,
        }),
    };
    let first = client.call(prompt.clone()).await.expect("first prompt");
    let replay = client.call(prompt.clone()).await.expect("prompt replay");
    assert_eq!(replay, first);

    let conflict = client
        .call(ClientCommand {
            command: ProtocolCommand::SessionPrompt(PromptParams {
                text: "changed prompt".to_owned(),
                explicit_target: None,
            }),
            ..prompt
        })
        .await
        .expect_err("changed parameters");
    assert!(matches!(
        conflict,
        TestClientError::Protocol(ref error) if error.code == ErrorCode::InvalidRequest
    ));
    approve_pending_prompt(&mut client, created.session_id).await;
    let attached: AttachSessionResult = decode_result(
        client
            .call(protocol_command(
                Some(created.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("replay history"),
    );
    let events = drain_events(&mut client, attached.last_sequence).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::InputRecorded)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::DispatchStarted)
            .count(),
        1
    );
}

async fn verify_offline_contract() {
    let network = DenyNetwork::default();
    let provider = FakeProvider::builder(provider_id("fake"))
        .network_guard(network.clone())
        .stream(StreamBehavior::Emit(vec![
            Ok(workbench_core::ports::ProviderOutput::Acknowledged {
                provider_request_id: Some("offline".to_owned()),
            }),
            Ok(workbench_core::ports::ProviderOutput::Content {
                event_type: "text_delta".to_owned(),
                content: text("offline output"),
            }),
            Ok(workbench_core::ports::ProviderOutput::Tool {
                event_type: "tool_result".to_owned(),
                content: text("offline tool output"),
            }),
            Ok(workbench_core::ports::ProviderOutput::Completed {
                summary: "offline complete".to_owned(),
            }),
        ]))
        .build();
    verify_happy_path_contract(&provider)
        .await
        .expect("fake provider contract");
    assert_eq!(provider.call_counts().prompt, 1);
    network.assert_unused().expect("no network request");

    let store = FakeKeyStore::new();
    store
        .put("acceptance/key", b"synthetic")
        .expect("memory put");
    assert_eq!(
        store
            .get("acceptance/key")
            .expect("memory get")
            .expect("key")
            .as_slice(),
        b"synthetic"
    );
}

async fn live_cancellation_fixture(
    confirms: bool,
) -> (ProtocolTestClient, LocalDaemonHarness, Uuid) {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe daemon startup"),
        FakeBehavior {
            response_delay: std::time::Duration::from_mins(1),
            confirms_cancellation: confirms,
            cancellation_deadline: std::time::Duration::from_millis(10),
            report_findings: false,
        },
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("local daemon");
    let mut client = ProtocolTestClient::connect(harness.endpoint(), "cancellation-contract")
        .await
        .expect("cancellation client");
    let created: CreateSessionResult = decode_result(
        client
            .call(protocol_command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create cancellation fixture"),
    );
    client
        .call(protocol_command(
            Some(created.session_id),
            ProtocolCommand::SessionPrompt(PromptParams {
                text: "cancel deterministic work".to_owned(),
                explicit_target: None,
            }),
        ))
        .await
        .expect("start cancellable work");
    approve_pending_prompt(&mut client, created.session_id).await;
    client
        .call(protocol_command(
            Some(created.session_id),
            ProtocolCommand::SessionCancel(EmptyParams {}),
        ))
        .await
        .expect("cancel request");
    (client, harness, created.session_id)
}

async fn approve_pending_prompt(client: &mut ProtocolTestClient, session_id: Uuid) {
    let session: SessionResult = decode_result(
        client
            .call(protocol_command(
                Some(session_id),
                ProtocolCommand::SessionGet(EmptyParams {}),
            ))
            .await
            .expect("read pending approval"),
    );
    let approval_id = session.pending_approval_id.expect("pending approval");
    client
        .call(protocol_command(
            Some(session_id),
            ProtocolCommand::SessionApprovalResolve(workbench_protocol::command::ApprovalParams {
                approval_id,
                decision: workbench_protocol::command::ApprovalDecision::Grant,
            }),
        ))
        .await
        .expect("grant pending approval");
}

async fn wait_for_protocol_state(
    client: &mut ProtocolTestClient,
    session_id: Uuid,
    expected: workbench_protocol::response::SessionState,
) -> SessionResult {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let session: SessionResult = decode_result(
                client
                    .call(protocol_command(
                        Some(session_id),
                        ProtocolCommand::SessionGet(EmptyParams {}),
                    ))
                    .await
                    .expect("read cancellation state"),
            );
            if session.state == expected {
                return session;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("cancellation state deadline")
}

fn approval_id_from_events(events: &[SessionEvent]) -> Uuid {
    events
        .iter()
        .find(|event| event.kind == EventKind::ApprovalRequested)
        .and_then(|event| event.data.get("approval_id"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("approval request event")
}

fn protocol_command(session_id: Option<Uuid>, command: ProtocolCommand) -> ClientCommand {
    ClientCommand {
        protocol: PROTOCOL_V1.to_owned(),
        request_id: Uuid::now_v7(),
        session_id,
        command,
    }
}

fn decode_result<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("method result schema")
}

async fn drain_events(client: &mut ProtocolTestClient, count: u64) -> Vec<SessionEvent> {
    let mut events = Vec::with_capacity(usize::try_from(count).expect("event count"));
    for _ in 0..count {
        events.push(
            tokio::time::timeout(std::time::Duration::from_secs(2), client.next_event())
                .await
                .expect("event deadline")
                .expect("session event"),
        );
    }
    events
}

async fn events_through(
    client: &mut ProtocolTestClient,
    terminal_kind: EventKind,
) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), client.next_event())
            .await
            .expect("event deadline")
            .expect("session event");
        let finished = event.kind == terminal_kind;
        events.push(event);
        if finished {
            return events;
        }
    }
}

fn assert_same_event_stream(first: &[SessionEvent], second: &[SessionEvent]) {
    assert_eq!(
        first
            .iter()
            .map(|event| (event.sequence, event.event_id, event.kind))
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|event| (event.sequence, event.event_id, event.kind))
            .collect::<Vec<_>>()
    );
}

fn preflight_fixture(
    include_fallback: bool,
) -> (WorkbenchConfiguration, BTreeMap<String, ConfigCapabilities>) {
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "fallback".to_owned(),
        ConfigProvider {
            kind: ProviderType::Fake,
            driver: None,
            executable: None,
            credential_ref: None,
            privacy: None,
        },
    );
    configuration.models.insert(
        "fallback-model".to_owned(),
        ConfigModel {
            provider: "fallback".to_owned(),
            runtime_model: "fallback-runtime".to_owned(),
        },
    );
    configuration.roles.insert(
        "reviewer".to_owned(),
        ConfigRole {
            model: "fake-default".to_owned(),
            tools: Vec::new(),
            data_sources: Vec::new(),
            required_capabilities: vec![ConfigCapability::StructuredOutput],
            fallback_models: if include_fallback {
                vec!["fallback-model".to_owned()]
            } else {
                Vec::new()
            },
        },
    );
    let capabilities = BTreeMap::from([
        ("fake".to_owned(), config_capabilities(Vec::new())),
        (
            "fallback".to_owned(),
            config_capabilities(vec![ConfigCapability::StructuredOutput]),
        ),
    ]);
    (configuration, capabilities)
}

fn config_capabilities(capabilities: Vec<ConfigCapability>) -> ConfigCapabilities {
    ConfigCapabilities {
        adapter_id: "fake".to_owned(),
        adapter_version: "1".to_owned(),
        protocol: "test/1".to_owned(),
        authentication: ConfigAuthentication::Available,
        capabilities,
        context_window_tokens: Some(8_192),
        operations: vec![ProviderOperation {
            name: "prompt".to_owned(),
            effect_class: ConfigEffectClass::PaidInference,
            idempotent: false,
            material_cost: false,
            approval: ConfigApprovalMode::Never,
        }],
    }
}

fn candidate(role: &str, provider: &str, confidence: f64) -> RouteCandidate {
    RouteCandidate::new(
        "acceptance",
        RouteDestination {
            role: RoleId::parse(role).expect("role"),
            model_alias: ModelAlias::parse("fake-default").expect("model"),
            provider: provider_id(provider),
            runtime_model: "deterministic-v1".to_owned(),
        },
        RouteContext {
            tools: Vec::new(),
            data_sources: Vec::new(),
            permission: PermissionScope::ReadOnly,
        },
        Risk::Low,
        confidence,
    )
    .expect("candidate")
}

fn plan(role: &str, provider: &str, confidence: f64, selected_by: SelectedRule) -> RoutingPlan {
    let RoutingOutcome::Selected(plan) =
        OrderedRouter::new(0.85)
            .expect("router")
            .resolve(match selected_by {
                SelectedRule::Explicit => RoutingInputs {
                    explicit: Some(candidate(role, provider, confidence)),
                    ..RoutingInputs::default()
                },
                SelectedRule::Workflow => RoutingInputs {
                    workflow: Some(candidate(role, provider, confidence)),
                    ..RoutingInputs::default()
                },
                SelectedRule::Resolver => RoutingInputs {
                    deterministic: Some(candidate(role, provider, confidence)),
                    ..RoutingInputs::default()
                },
                SelectedRule::Coordinator => RoutingInputs {
                    coordinator: Some(candidate(role, provider, confidence)),
                    ..RoutingInputs::default()
                },
            })
    else {
        panic!("selected route");
    };
    plan
}

fn core_event(session_id: SessionId, sequence: u64, payload: EventPayload) -> PersistedEvent {
    PersistedEvent {
        event_id: EventId::new(),
        session_id,
        sequence: Sequence::new(sequence).expect("sequence"),
        causation_request_id: None,
        occurred_at: OffsetDateTime::UNIX_EPOCH,
        payload,
    }
}

fn storage_session(session_id: Uuid, now: OffsetDateTime, secret: &str) -> CreateSession {
    CreateSession {
        session_id,
        request_id: Uuid::now_v7(),
        occurred_at: now,
        request_parameters: json!({"persistent": true}),
        command_outcome: json!({"session_id": session_id, "state": "ready"}),
        configuration_snapshot: json!({"content": secret}),
        lock_snapshot: json!({"hash": "synthetic"}),
        initial_event_payload: json!({
            "kind": "session_created",
            "data": {
                "configuration_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "lock_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }
        }),
    }
}

fn create_storage_session(
    storage: &mut SqliteStorage<MemoryKeyStore>,
    session_id: Uuid,
    now: OffsetDateTime,
    secret: &str,
) {
    storage
        .create_session(&storage_session(session_id, now, secret))
        .expect("session creation");
}

fn create_export_storage_session(
    storage: &mut SqliteStorage<MemoryKeyStore>,
    session_id: Uuid,
    now: OffsetDateTime,
) {
    let configuration = WorkbenchConfiguration::safe_builtins();
    let snapshot = ConfigurationSnapshot::create(&configuration, vec!["testkit".to_owned()])
        .expect("configuration snapshot");
    let base_lock = WorkbenchLock::repository(&configuration, &snapshot, &BTreeMap::new())
        .expect("repository lock");
    let session_lock =
        WorkbenchLock::session(&base_lock, &configuration, &snapshot).expect("session lock");
    let configuration_hash = snapshot.content_hash.clone();
    let lock_hash = session_lock.hash().expect("lock hash");
    storage
        .create_session(&CreateSession {
            session_id,
            request_id: Uuid::now_v7(),
            occurred_at: now,
            request_parameters: json!({"persistent": true}),
            command_outcome: json!({"session_id": session_id, "state": "ready"}),
            configuration_snapshot: serde_json::to_value(snapshot).expect("snapshot JSON"),
            lock_snapshot: serde_json::to_value(session_lock).expect("lock JSON"),
            initial_event_payload: json!({
                "configuration_hash": configuration_hash,
                "lock_hash": lock_hash,
            }),
        })
        .expect("export session creation");
}

fn append_storage(
    storage: &mut SqliteStorage<MemoryKeyStore>,
    session_id: Uuid,
    occurred_at: OffsetDateTime,
    kind: &str,
    payload: Value,
) {
    storage
        .append_event(&EventInput {
            event_id: Uuid::now_v7(),
            session_id,
            occurred_at,
            kind: kind.to_owned(),
            causation_request_id: None,
            attempt_id: None,
            effect_class: None,
            payload,
        })
        .expect("event append");
}

fn private_tempdir() -> TempDir {
    let directory = TempDir::new().expect("temporary directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("private temporary directory");
    }
    directory
}

fn assert_storage_has_no(database: &Path, forbidden: &[&[u8]]) {
    for path in storage_files(database) {
        let bytes = fs::read(path).expect("storage bytes");
        for value in forbidden {
            assert!(
                !contains(&bytes, value),
                "sensitive plaintext leaked into persistent storage"
            );
        }
    }
}

fn storage_files(database: &Path) -> Vec<PathBuf> {
    fs::read_dir(database.parent().expect("database parent"))
        .expect("storage directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("workbench.sqlite"))
        })
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn provider_id(value: &str) -> ProviderId {
    ProviderId::parse(value).expect("provider")
}

fn text(value: &str) -> NonEmptyText {
    NonEmptyText::parse(value).expect("non-empty text")
}

fn hash(character: char) -> ContentHash {
    ContentHash::parse(character.to_string().repeat(64)).expect("content hash")
}
