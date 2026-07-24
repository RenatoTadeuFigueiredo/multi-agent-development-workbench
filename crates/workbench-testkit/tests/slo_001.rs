#![allow(clippy::missing_panics_doc, clippy::too_many_lines)]

use std::{
    fs,
    time::{Duration, Instant},
};

use serde_json::json;
use tempfile::TempDir;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;
use workbench_daemon::{Application, FakeBehavior, StartupConfiguration};
use workbench_protocol::{
    ClientCommand, Command, EventKind, PROTOCOL_V1,
    command::{AttachSessionParams, CreateSessionParams, EmptyParams, PromptParams},
    response::{AttachSessionResult, CreateSessionResult, SessionResult, SessionState},
};
use workbench_storage::{CommandOutcome, CreateSession, EventInput, MemoryKeyStore, SqliteStorage};
use workbench_testkit::client::{LocalDaemonHarness, ProtocolTestClient};

const LATENCY_OBJECTIVE: Duration = Duration::from_millis(100);
const REPLAY_OBJECTIVE: Duration = Duration::from_secs(2);
const CANCELLATION_OBJECTIVE: Duration = Duration::from_secs(5);
const SAMPLE_COUNT: usize = 1_000;
const REPLAY_EVENT_COUNT: usize = 10_000;

#[tokio::test]
#[ignore = "run through make test-slo for serialized latency measurements"]
async fn one_thousand_durable_daemon_routes_meet_the_p95_objective() {
    let mut samples = measure_daemon_routes().await;
    if percentile_95(&mut samples) > LATENCY_OBJECTIVE {
        println!("SLO routing_plan_latency: retrying once on a clean daemon fixture");
        samples = measure_daemon_routes().await;
    }
    assert_p95("routing_plan_latency", &mut samples, LATENCY_OBJECTIVE);
}

#[tokio::test]
#[ignore = "run through make test-slo for serialized latency measurements"]
async fn healthy_ipc_fan_out_and_control_acknowledgements_meet_p95() {
    let (mut fan_out, mut controls) = measure_controls_and_fan_out().await;
    if percentile_95(&mut fan_out) > LATENCY_OBJECTIVE
        || percentile_95(&mut controls) > LATENCY_OBJECTIVE
    {
        println!("SLO daemon_controls: retrying once on a clean daemon fixture");
        (fan_out, controls) = measure_controls_and_fan_out().await;
    }
    assert_p95("healthy_fan_out_latency", &mut fan_out, LATENCY_OBJECTIVE);
    assert_p95(
        "control_acknowledgement_latency",
        &mut controls,
        LATENCY_OBJECTIVE,
    );
}

#[test]
#[ignore = "run through make test-slo for serialized latency measurements"]
fn ten_thousand_encrypted_sqlite_events_replay_in_order_within_two_seconds() {
    let mut elapsed = measure_encrypted_replay();
    if elapsed > REPLAY_OBJECTIVE {
        println!("SLO encrypted_replay_10000: retrying once on a clean SQLite fixture");
        elapsed = measure_encrypted_replay();
    }
    report_single("encrypted_replay_10000", elapsed, REPLAY_OBJECTIVE);
    assert!(
        elapsed <= REPLAY_OBJECTIVE,
        "encrypted replay took {elapsed:?}, objective is {REPLAY_OBJECTIVE:?}"
    );
}

fn measure_encrypted_replay() -> Duration {
    let directory = private_tempdir();
    let database = directory.path().join("workbench.sqlite");
    let mut storage =
        SqliteStorage::open(&database, MemoryKeyStore::new()).expect("encrypted SQLite opens");
    let session_id = Uuid::now_v7();
    let occurred_at = OffsetDateTime::UNIX_EPOCH;
    let creation = storage
        .create_session(&CreateSession {
            session_id,
            request_id: Uuid::now_v7(),
            occurred_at,
            request_parameters: json!({"persistent": true}),
            command_outcome: json!({"session_id": session_id, "state": "ready"}),
            configuration_snapshot: json!({"model": "fake"}),
            lock_snapshot: json!({"configuration_hash": "synthetic"}),
            initial_event_payload: json!({"kind": "session_created"}),
        })
        .expect("create replay fixture");
    assert!(matches!(creation, CommandOutcome::Recorded(_)));

    for index in 0..REPLAY_EVENT_COUNT {
        storage
            .append_event(&EventInput {
                event_id: Uuid::now_v7(),
                session_id,
                occurred_at,
                kind: "provider_event".to_owned(),
                causation_request_id: None,
                attempt_id: None,
                effect_class: Some("paid-inference".to_owned()),
                payload: json!({
                    "index": index,
                    "content": "encrypted deterministic SLO fixture"
                }),
            })
            .expect("append encrypted replay event");
    }

    let started = Instant::now();
    let replay = storage
        .replay(session_id, 1)
        .expect("ordered encrypted replay");
    let elapsed = started.elapsed();

    assert_eq!(replay.len(), REPLAY_EVENT_COUNT);
    for (index, event) in replay.iter().enumerate() {
        let expected_sequence = u64::try_from(index).expect("index fits in u64") + 2;
        assert_eq!(event.sequence, expected_sequence);
        assert_eq!(
            event.payload["index"].as_u64(),
            Some(u64::try_from(index).expect("index fits in u64"))
        );
    }
    elapsed
}

#[tokio::test]
#[ignore = "run through make test-slo for serialized latency measurements"]
async fn cancellation_confirms_or_becomes_unknown_within_five_seconds() {
    let confirmed_elapsed = measure_cancellation(true, Duration::from_secs(5)).await;
    let unconfirmed_elapsed = measure_cancellation(false, Duration::from_millis(4_500)).await;

    println!(
        "SLO cancellation_resolution: confirmed_ms={:.3} unconfirmed_ms={:.3} objective_ms={}",
        milliseconds(confirmed_elapsed),
        milliseconds(unconfirmed_elapsed),
        CANCELLATION_OBJECTIVE.as_millis()
    );
    assert!(confirmed_elapsed <= CANCELLATION_OBJECTIVE);
    assert!(unconfirmed_elapsed <= CANCELLATION_OBJECTIVE);
}

async fn measure_daemon_routes() -> Vec<Duration> {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe startup"),
        FakeBehavior {
            response_delay: Duration::from_hours(1),
            ..FakeBehavior::default()
        },
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("daemon fixture");
    let mut client = ProtocolTestClient::connect(harness.endpoint(), "slo-routing")
        .await
        .expect("routing client");
    let mut sessions = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let created: CreateSessionResult = decode(
            client
                .call(command(
                    None,
                    Command::SessionCreate(CreateSessionParams {
                        persistent: true,
                        configuration_overrides: None,
                        workflow: None,
                    }),
                ))
                .await
                .expect("create route fixture"),
        );
        sessions.push(created.session_id);
    }

    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for (index, session_id) in sessions.into_iter().enumerate() {
        let started = Instant::now();
        client
            .call(command(
                Some(session_id),
                Command::SessionPrompt(PromptParams {
                    text: "measure durable local routing".to_owned(),
                    explicit_target: (index % 2 == 0).then(|| "workspace-coordinator".to_owned()),
                }),
            ))
            .await
            .expect("durable routed prompt");
        samples.push(started.elapsed());
    }
    samples
}

async fn measure_controls_and_fan_out() -> (Vec<Duration>, Vec<Duration>) {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe startup"),
        FakeBehavior {
            response_delay: Duration::from_hours(1),
            ..FakeBehavior::default()
        },
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("daemon fixture");
    let mut controller = ProtocolTestClient::connect(harness.endpoint(), "slo-controller")
        .await
        .expect("controller");
    let mut observer = ProtocolTestClient::connect(harness.endpoint(), "slo-observer")
        .await
        .expect("observer");
    let created: CreateSessionResult = decode(
        controller
            .call(command(
                None,
                Command::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create control fixture"),
    );
    controller
        .call(command(
            Some(created.session_id),
            Command::SessionPrompt(PromptParams {
                text: "measure controls".to_owned(),
                explicit_target: None,
            }),
        ))
        .await
        .expect("start control fixture");
    approve_pending_prompt(&mut controller, created.session_id).await;
    let attached: AttachSessionResult = decode(
        observer
            .call(command(
                Some(created.session_id),
                Command::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("attach observer"),
    );
    for _ in 0..attached.last_sequence {
        observer.next_event().await.expect("initial replay");
    }

    let mut fan_out = Vec::with_capacity(SAMPLE_COUNT);
    let mut controls = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..(SAMPLE_COUNT / 2) {
        let started = Instant::now();
        controller
            .call(command(
                Some(created.session_id),
                Command::SessionPause(EmptyParams {}),
            ))
            .await
            .expect("pause acknowledgement");
        controls.push(started.elapsed());
        let event = next_kind(&mut observer, EventKind::SessionPaused).await;
        fan_out.push(event_fan_out_latency(&event));

        let started = Instant::now();
        controller
            .call(command(
                Some(created.session_id),
                Command::SessionResume(EmptyParams {}),
            ))
            .await
            .expect("resume acknowledgement");
        controls.push(started.elapsed());
        let event = next_kind(&mut observer, EventKind::SessionResumed).await;
        fan_out.push(event_fan_out_latency(&event));
    }
    (fan_out, controls)
}

async fn measure_cancellation(confirms: bool, deadline: Duration) -> Duration {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe startup"),
        FakeBehavior {
            response_delay: Duration::from_hours(1),
            confirms_cancellation: confirms,
            cancellation_deadline: deadline,
            report_findings: false,
        },
    )
    .expect("in-memory daemon");
    let harness = LocalDaemonHarness::start(application).expect("daemon fixture");
    let mut client = ProtocolTestClient::connect(harness.endpoint(), "slo-cancellation")
        .await
        .expect("cancellation client");
    let created: CreateSessionResult = decode(
        client
            .call(command(
                None,
                Command::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create cancellation fixture"),
    );
    client
        .call(command(
            Some(created.session_id),
            Command::SessionPrompt(PromptParams {
                text: "measure cancellation".to_owned(),
                explicit_target: None,
            }),
        ))
        .await
        .expect("start cancellation fixture");
    approve_pending_prompt(&mut client, created.session_id).await;

    let expected = if confirms {
        SessionState::Cancelled
    } else {
        SessionState::OutcomeUnknown
    };
    let started = Instant::now();
    client
        .call(command(
            Some(created.session_id),
            Command::SessionCancel(EmptyParams {}),
        ))
        .await
        .expect("cancel acknowledgement");
    tokio::time::timeout(CANCELLATION_OBJECTIVE + Duration::from_millis(250), async {
        loop {
            let session: SessionResult = decode(
                client
                    .call(command(
                        Some(created.session_id),
                        Command::SessionGet(EmptyParams {}),
                    ))
                    .await
                    .expect("poll cancellation state"),
            );
            if session.state == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("cancellation terminal-state deadline");
    started.elapsed()
}

async fn approve_pending_prompt(client: &mut ProtocolTestClient, session_id: Uuid) {
    let session: SessionResult = decode(
        client
            .call(command(
                Some(session_id),
                Command::SessionGet(EmptyParams {}),
            ))
            .await
            .expect("read pending approval"),
    );
    let approval_id = session.pending_approval_id.expect("pending approval");
    client
        .call(command(
            Some(session_id),
            Command::SessionApprovalResolve(workbench_protocol::command::ApprovalParams {
                approval_id,
                decision: workbench_protocol::command::ApprovalDecision::Grant,
            }),
        ))
        .await
        .expect("grant pending approval");
}

async fn next_kind(
    client: &mut ProtocolTestClient,
    expected: EventKind,
) -> workbench_protocol::SessionEvent {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), client.next_event())
            .await
            .expect("fan-out deadline")
            .expect("fan-out event");
        if event.kind == expected {
            return event;
        }
    }
}

fn event_fan_out_latency(event: &workbench_protocol::SessionEvent) -> Duration {
    let appended_at = OffsetDateTime::parse(&event.occurred_at, &Rfc3339).expect("event timestamp");
    (OffsetDateTime::now_utc() - appended_at)
        .try_into()
        .expect("event timestamp must not be in the future")
}

fn command(session_id: Option<Uuid>, command: Command) -> ClientCommand {
    ClientCommand {
        protocol: PROTOCOL_V1.to_owned(),
        request_id: Uuid::now_v7(),
        session_id,
        command,
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).expect("method result schema")
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

fn assert_p95(label: &str, samples: &mut [Duration], objective: Duration) {
    assert!(!samples.is_empty(), "latency sample set must not be empty");
    let p95 = percentile_95(samples);
    println!(
        "SLO {label}: samples={} p95_ms={:.3} objective_ms={}",
        samples.len(),
        milliseconds(p95),
        objective.as_millis()
    );
    assert!(
        p95 <= objective,
        "{label} p95 was {p95:?}, objective is {objective:?}"
    );
}

fn percentile_95(samples: &mut [Duration]) -> Duration {
    assert!(!samples.is_empty(), "latency sample set must not be empty");
    samples.sort_unstable();
    let rank = samples.len().saturating_mul(95).div_ceil(100);
    samples[rank.saturating_sub(1)]
}

fn report_single(label: &str, elapsed: Duration, objective: Duration) {
    println!(
        "SLO {label}: elapsed_ms={:.3} objective_ms={}",
        milliseconds(elapsed),
        objective.as_millis()
    );
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
