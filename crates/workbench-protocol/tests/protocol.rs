use bytes::BytesMut;
use serde_json::json;
use tokio_util::codec::Decoder;
use uuid::Uuid;
use workbench_protocol::{
    ClientCommand, EventKind, NdjsonCodec, ProtocolCodecError, SessionEvent, SubscriptionError,
    SubscriptionQueue, replay_after,
};

#[test]
fn parses_every_committed_method_with_strict_params() {
    let request = Uuid::now_v7();
    let session = Uuid::now_v7();
    let other = Uuid::now_v7();
    let cases = [
        (
            "initialize",
            json!({
                "client_name": "test",
                "client_version": "1",
                "supported_protocols": ["workbench/1"]
            }),
            None,
        ),
        ("status.get", json!({}), None),
        ("session.create", json!({"persistent": true}), None),
        ("session.get", json!({}), Some(session)),
        (
            "session.attach",
            json!({"after_sequence": 0}),
            Some(session),
        ),
        (
            "session.prompt",
            json!({"text": "hello", "explicit_target": "reviewer"}),
            Some(session),
        ),
        ("session.pause", json!({}), Some(session)),
        ("session.resume", json!({}), Some(session)),
        (
            "session.redirect",
            json!({"instruction": "continue"}),
            Some(session),
        ),
        ("session.cancel", json!({}), Some(session)),
        (
            "session.approval.resolve",
            json!({"approval_id": other, "decision": "grant"}),
            Some(session),
        ),
        (
            "session.reconcile",
            json!({"attempt_id": other, "resolution": "retry"}),
            Some(session),
        ),
        (
            "session.export",
            json!({"output_path": "/tmp/session.age", "age_recipients": ["age1test"]}),
            Some(session),
        ),
        (
            "session.delete",
            json!({"confirm_session_id": other}),
            Some(session),
        ),
    ];

    for (method, params, session_id) in cases {
        let mut value = json!({
            "protocol": "workbench/1",
            "request_id": request,
            "method": method,
            "params": params
        });
        if let Some(session_id) = session_id {
            value["session_id"] = json!(session_id);
        }
        let command: ClientCommand =
            serde_json::from_value(value).unwrap_or_else(|error| panic!("{method}: {error}"));
        assert_eq!(command.command.method(), method);
        let encoded = serde_json::to_value(&command).expect("command serializes");
        assert_eq!(encoded["method"], method);
    }
}

#[test]
fn rejects_unknown_fields_wrong_session_scope_and_non_v7_ids() {
    let request = Uuid::now_v7();
    let session = Uuid::now_v7();
    let unknown = json!({
        "protocol": "workbench/1",
        "request_id": request,
        "method": "session.pause",
        "session_id": session,
        "params": {"unexpected": true}
    });
    assert!(serde_json::from_value::<ClientCommand>(unknown).is_err());

    let missing_session = json!({
        "protocol": "workbench/1",
        "request_id": request,
        "method": "session.pause",
        "params": {}
    });
    assert!(serde_json::from_value::<ClientCommand>(missing_session).is_err());

    let old_uuid = json!({
        "protocol": "workbench/1",
        "request_id": Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000")
            .expect("valid UUIDv4"),
        "method": "status.get",
        "params": {}
    });
    assert!(serde_json::from_value::<ClientCommand>(old_uuid).is_err());
}

#[test]
fn codec_rejects_duplicate_keys_trailing_json_invalid_utf8_and_oversize() {
    type Codec = NdjsonCodec<ClientCommand, ClientCommand>;
    let mut codec = Codec::default();
    let request = Uuid::now_v7();
    let duplicate = format!(
        "{{\"protocol\":\"workbench/1\",\"request_id\":\"{request}\",\"method\":\"status.get\",\"method\":\"status.get\",\"params\":{{}}}}\n"
    );
    let error = codec
        .decode(&mut BytesMut::from(duplicate.as_bytes()))
        .expect_err("duplicate key");
    assert!(matches!(error, ProtocolCodecError::InvalidJson(_)));

    let trailing = format!(
        "{{\"protocol\":\"workbench/1\",\"request_id\":\"{request}\",\"method\":\"status.get\",\"params\":{{}}}}{{}}\n"
    );
    assert!(
        codec
            .decode(&mut BytesMut::from(trailing.as_bytes()))
            .is_err()
    );

    let mut invalid_utf8 = BytesMut::from(&b"{\"bad\":\"\xff\"}\n"[..]);
    assert!(codec.decode(&mut invalid_utf8).is_err());

    let mut oversized =
        BytesMut::from(vec![b' '; workbench_protocol::MAX_FRAME_BYTES + 1].as_slice());
    assert!(matches!(
        codec.decode(&mut oversized),
        Err(ProtocolCodecError::FrameTooLarge)
    ));
}

#[test]
fn codec_classifies_an_incompatible_protocol_before_command_decoding() {
    type Codec = NdjsonCodec<ClientCommand, ClientCommand>;
    let request = Uuid::now_v7();
    let frame = format!(
        "{{\"protocol\":\"workbench/2\",\"request_id\":\"{request}\",\"method\":\"status.get\",\"params\":{{}}}}\n"
    );

    let error = Codec::default()
        .decode(&mut BytesMut::from(frame.as_bytes()))
        .expect_err("incompatible protocol");

    assert!(matches!(error, ProtocolCodecError::UnsupportedVersion));
}

#[test]
fn queue_enforces_event_and_byte_limits_and_replay_is_exclusive() {
    let session = Uuid::now_v7();
    let events = (1..=3)
        .map(|sequence| event(session, sequence))
        .collect::<Vec<_>>();
    let replayed = replay_after(events.clone(), 1);
    assert_eq!(
        replayed
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [2, 3]
    );

    let mut count_limited = SubscriptionQueue::new(1, usize::MAX);
    count_limited.push(events[0].clone()).expect("first event");
    assert_eq!(
        count_limited.push(events[1].clone()),
        Err(SubscriptionError::ClientLagged)
    );
    assert!(count_limited.is_closed());

    let event_bytes = serde_json::to_vec(&events[0]).expect("event encodes").len() + 1;
    let mut byte_limited = SubscriptionQueue::new(10, event_bytes);
    byte_limited.push(events[0].clone()).expect("fits exactly");
    assert_eq!(
        byte_limited.push(events[1].clone()),
        Err(SubscriptionError::ClientLagged)
    );
}

fn event(session_id: Uuid, sequence: u64) -> SessionEvent {
    SessionEvent {
        protocol: "workbench/1".to_owned(),
        event_id: Uuid::now_v7(),
        session_id,
        sequence,
        causation_request_id: None,
        kind: EventKind::ProviderEvent,
        occurred_at: "2026-07-23T00:00:00Z".to_owned(),
        data: json!({"content": "test"}),
    }
}
