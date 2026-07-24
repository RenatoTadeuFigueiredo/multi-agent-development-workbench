use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncWriteExt, BufWriter},
    process::{ChildStdin, ChildStdout},
    sync::{mpsc, oneshot, watch},
};
use uuid::Uuid;

use crate::{
    AcpError, AcpErrorKind,
    codec::{FrameReader, encode_frame},
    protocol::{AdapterHealth, NormalizedUpdate, PromptControl, PromptExecution, PromptOutcome},
};

const WRITE_QUEUE_DEPTH: usize = 64;
const UPDATE_QUEUE_DEPTH: usize = 256;

pub(crate) enum WriteCommand {
    Frame(Vec<u8>),
    Close(oneshot::Sender<()>),
}

#[derive(Debug, Clone)]
pub(crate) enum PromptState {
    Pending,
    Terminal(Result<PromptOutcome, AcpError>),
}

struct PromptSink {
    updates: mpsc::Sender<NormalizedUpdate>,
    state: watch::Sender<PromptState>,
    acknowledged: bool,
}

pub(crate) struct Connection {
    writer: mpsc::Sender<WriteCommand>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value, AcpError>>>>,
    prompts: Mutex<HashMap<String, PromptSink>>,
    closed: AtomicBool,
    health: watch::Sender<AdapterHealth>,
    fatal: mpsc::Sender<AcpError>,
}

impl Connection {
    pub(crate) fn new(
        writer: mpsc::Sender<WriteCommand>,
        health: watch::Sender<AdapterHealth>,
        fatal: mpsc::Sender<AcpError>,
    ) -> Arc<Self> {
        Arc::new(Self {
            writer,
            pending: Mutex::new(HashMap::new()),
            prompts: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            health,
            fatal,
        })
    }

    pub(crate) async fn request(
        self: &Arc<Self>,
        method: &'static str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value, AcpError> {
        let deadline = tokio::time::Instant::now() + deadline;
        let (id, response) = self.begin_request(method, params, deadline).await?;
        if let Ok(result) = tokio::time::timeout_at(deadline, response).await {
            result.map_err(|_| transport_closed())?
        } else {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            Err(AcpError::new(
                AcpErrorKind::Timeout,
                "ACP request deadline expired",
            ))
        }
    }

    async fn begin_request(
        &self,
        method: &'static str,
        params: Value,
        deadline: tokio::time::Instant,
    ) -> Result<(String, oneshot::Receiver<Result<Value, AcpError>>), AcpError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(shutting_down());
        }
        let id = Uuid::now_v7().to_string();
        let frame = encode_frame(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))?;
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| internal_transport())?
            .insert(id.clone(), sender);
        let sent =
            tokio::time::timeout_at(deadline, self.writer.send(WriteCommand::Frame(frame))).await;
        let failure = match sent {
            Ok(Ok(())) => None,
            Ok(Err(_)) => Some(transport_closed()),
            Err(_) => Some(AcpError::new(
                AcpErrorKind::Timeout,
                "ACP request deadline expired",
            )),
        };
        if let Some(failure) = failure {
            self.pending
                .lock()
                .map_err(|_| internal_transport())?
                .remove(&id);
            return Err(failure);
        }
        Ok((id, receiver))
    }

    pub(crate) async fn notify(&self, method: &'static str, params: Value) -> Result<(), AcpError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(shutting_down());
        }
        let frame = encode_frame(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))?;
        self.writer
            .send(WriteCommand::Frame(frame))
            .await
            .map_err(|_| transport_closed())
    }

    pub(crate) async fn cancel(&self, session_id: &str) -> Result<(), AcpError> {
        self.notify("session/cancel", json!({"sessionId": session_id}))
            .await
    }

    pub(crate) fn set_health(&self, health: AdapterHealth) {
        let _ignored = self.health.send(health);
    }

    pub(crate) async fn start_prompt(
        self: &Arc<Self>,
        session_id: &str,
        prompt: Value,
        enqueue_deadline: Duration,
    ) -> Result<PromptExecution, AcpError> {
        let (update_tx, update_rx) = mpsc::channel(UPDATE_QUEUE_DEPTH);
        let (state_tx, state_rx) = watch::channel(PromptState::Pending);
        {
            let mut prompts = self.prompts.lock().map_err(|_| internal_transport())?;
            if prompts.contains_key(session_id) {
                return Err(AcpError::new(
                    AcpErrorKind::ProtocolViolation,
                    "ACP session already has an active prompt",
                ));
            }
            prompts.insert(
                session_id.to_owned(),
                PromptSink {
                    updates: update_tx,
                    state: state_tx,
                    acknowledged: false,
                },
            );
        }
        let control = PromptControl {
            connection: Arc::downgrade(self),
            session_id: Arc::from(session_id),
            state: state_rx.clone(),
            cancel_sent: Arc::new(AtomicBool::new(false)),
        };
        let connection = Arc::clone(self);
        let session = session_id.to_owned();
        let response = match self
            .begin_request(
                "session/prompt",
                prompt,
                tokio::time::Instant::now() + enqueue_deadline,
            )
            .await
        {
            Ok((_, response)) => response,
            Err(error) => {
                self.finish_prompt(session_id, Err(error.clone()));
                return Err(error);
            }
        };
        tokio::spawn(async move {
            let outcome = response
                .await
                .map_err(|_| transport_closed())
                .and_then(|result| result)
                .and_then(|value| crate::protocol::parse_prompt_outcome(&value));
            connection.finish_prompt(&session, outcome);
        });
        Ok(PromptExecution::new(update_rx, state_rx, control))
    }

    fn finish_prompt(&self, session_id: &str, outcome: Result<PromptOutcome, AcpError>) {
        if let Ok(mut prompts) = self.prompts.lock()
            && let Some(mut sink) = prompts.remove(session_id)
        {
            if !sink.acknowledged {
                let _ignored = sink.updates.try_send(crate::protocol::acknowledged());
                sink.acknowledged = true;
            }
            let _ignored = sink.state.send(PromptState::Terminal(outcome));
        }
    }

    pub(crate) async fn close_writer(&self) {
        self.closed.store(true, Ordering::Release);
        let (sender, receiver) = oneshot::channel();
        if self.writer.send(WriteCommand::Close(sender)).await.is_ok() {
            let _ignored = receiver.await;
        }
        self.fail(&shutting_down(), AdapterHealth::ShuttingDown);
    }

    pub(crate) fn fail(&self, error: &AcpError, health: AdapterHealth) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let _ignored = self.health.send(health);
        }
        if let Ok(mut pending) = self.pending.lock() {
            for (_, sender) in pending.drain() {
                let _ignored = sender.send(Err(error.clone()));
            }
        }
        if let Ok(mut prompts) = self.prompts.lock() {
            for (_, sink) in prompts.drain() {
                let _ignored = sink.state.send(PromptState::Terminal(Err(error.clone())));
            }
        }
    }

    fn handle_incoming(self: &Arc<Self>, value: &Value) -> Result<(), AcpError> {
        crate::protocol::validate_jsonrpc(value)?;
        let object = value.as_object().ok_or_else(protocol_violation)?;
        match (
            object.get("id"),
            object.get("method").and_then(Value::as_str),
        ) {
            (Some(id), Some(method)) => self.handle_reverse_request(id, method),
            (Some(id), None) => self.handle_response(id, object),
            (None, Some(method)) => self.handle_notification(method, object.get("params")),
            (None, None) => Err(protocol_violation()),
        }
    }

    fn handle_reverse_request(&self, id: &Value, method: &str) -> Result<(), AcpError> {
        crate::protocol::validate_jsonrpc_id(id)?;
        let value = if method == "session/request_permission" {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"outcome": {"outcome": "cancelled"}}
            })
        } else {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "method not found"}
            })
        };
        let frame = encode_frame(&value)?;
        self.writer
            .try_send(WriteCommand::Frame(frame))
            .map_err(|_| transport_closed())
    }

    fn handle_response(
        &self,
        id: &Value,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), AcpError> {
        let id = id.as_str().ok_or_else(protocol_violation)?;
        let result = match (object.get("result"), object.get("error")) {
            (Some(result), None) => Ok(result.clone()),
            (None, Some(error)) if error.is_object() => Err(AcpError::new(
                AcpErrorKind::RequestFailed,
                "ACP request was rejected",
            )),
            _ => return Err(protocol_violation()),
        };
        let sender = self
            .pending
            .lock()
            .map_err(|_| internal_transport())?
            .remove(id)
            .ok_or_else(protocol_violation)?;
        let _ignored = sender.send(result);
        Ok(())
    }

    fn handle_notification(&self, method: &str, params: Option<&Value>) -> Result<(), AcpError> {
        if method != "session/update" {
            return Ok(());
        }
        let (session_id, update) =
            crate::protocol::parse_session_update(params.ok_or_else(protocol_violation)?)?;
        let mut prompts = self.prompts.lock().map_err(|_| internal_transport())?;
        let Some(sink) = prompts.get_mut(&session_id) else {
            return Ok(());
        };
        if !sink.acknowledged {
            sink.updates
                .try_send(crate::protocol::acknowledged())
                .map_err(|_| transport_closed())?;
            sink.acknowledged = true;
        }
        if let Some(update) = update {
            sink.updates
                .try_send(update)
                .map_err(|_| transport_closed())?;
        }
        Ok(())
    }
}

pub(crate) fn spawn_writer(
    stdin: ChildStdin,
    mut receiver: mpsc::Receiver<WriteCommand>,
    connection: Arc<Connection>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut writer = BufWriter::new(stdin);
        while let Some(command) = receiver.recv().await {
            match command {
                WriteCommand::Frame(frame) => {
                    if writer.write_all(&frame).await.is_err()
                        || writer.write_all(b"\n").await.is_err()
                        || writer.flush().await.is_err()
                    {
                        let error = transport_closed();
                        connection.fail(&error, AdapterHealth::Crashed);
                        let _ignored = connection.fatal.try_send(error);
                        return;
                    }
                }
                WriteCommand::Close(sender) => {
                    let _ignored = writer.shutdown().await;
                    let _ignored = sender.send(());
                    return;
                }
            }
        }
    })
}

pub(crate) fn spawn_reader(
    stdout: ChildStdout,
    connection: Arc<Connection>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = FrameReader::new(stdout);
        loop {
            match reader.next_frame().await {
                Ok(Some(value)) => {
                    if let Err(error) = connection.handle_incoming(&value) {
                        connection.fail(&error, AdapterHealth::Crashed);
                        let _ignored = connection.fatal.try_send(error);
                        return;
                    }
                }
                Ok(None) => {
                    let error = transport_closed();
                    connection.fail(&error, AdapterHealth::Crashed);
                    let _ignored = connection.fatal.try_send(error);
                    return;
                }
                Err(error) => {
                    connection.fail(&error, AdapterHealth::Crashed);
                    let _ignored = connection.fatal.try_send(error);
                    return;
                }
            }
        }
    })
}

pub(crate) fn channel() -> (mpsc::Sender<WriteCommand>, mpsc::Receiver<WriteCommand>) {
    mpsc::channel(WRITE_QUEUE_DEPTH)
}

fn transport_closed() -> AcpError {
    AcpError::new(
        AcpErrorKind::TransportClosed,
        "ACP transport is unavailable",
    )
}

fn shutting_down() -> AcpError {
    AcpError::new(AcpErrorKind::ShuttingDown, "ACP adapter is shutting down")
}

fn protocol_violation() -> AcpError {
    AcpError::new(
        AcpErrorKind::ProtocolViolation,
        "ACP peer violated the protocol",
    )
}

fn internal_transport() -> AcpError {
    AcpError::new(
        AcpErrorKind::TransportClosed,
        "ACP transport state is unavailable",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use serde_json::json;
    use tokio::sync::{mpsc, watch};

    use super::{Connection, PromptState, WRITE_QUEUE_DEPTH, WriteCommand, channel};
    use crate::{AcpErrorKind, AdapterHealth, CancellationOutcome, protocol::PromptControl};

    fn saturated_connection() -> (Arc<Connection>, mpsc::Receiver<WriteCommand>) {
        let (writer, receiver) = channel();
        for _ in 0..WRITE_QUEUE_DEPTH {
            writer
                .try_send(WriteCommand::Frame(Vec::new()))
                .expect("fill writer queue");
        }
        let (health, _) = watch::channel(AdapterHealth::Available);
        let (fatal, _) = mpsc::channel(1);
        (Connection::new(writer, health, fatal), receiver)
    }

    #[tokio::test]
    async fn saturated_writer_respects_request_and_cancellation_deadlines() {
        let (connection, _receiver) = saturated_connection();
        let request = connection
            .request("initialize", json!({}), Duration::from_millis(20))
            .await
            .expect_err("saturated request");
        assert_eq!(request.kind(), AcpErrorKind::Timeout);
        assert!(
            connection
                .pending
                .lock()
                .expect("pending requests")
                .is_empty()
        );

        let (_state_sender, state) = watch::channel(PromptState::Pending);
        let control = PromptControl {
            connection: Arc::downgrade(&connection),
            session_id: Arc::from("session"),
            state,
            cancel_sent: Arc::new(AtomicBool::new(false)),
        };
        assert_eq!(
            control.cancel(Duration::from_millis(20)).await,
            CancellationOutcome::Unconfirmed
        );
        assert!(control.cancel_sent.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn reverse_requests_fail_closed_instead_of_waiting_on_a_full_writer() {
        let (connection, _receiver) = saturated_connection();

        let error = connection
            .handle_reverse_request(&json!("permission"), "session/request_permission")
            .expect_err("full reverse response queue");

        assert_eq!(error.kind(), AcpErrorKind::TransportClosed);
    }
}
