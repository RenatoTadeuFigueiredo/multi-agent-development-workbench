use std::collections::VecDeque;

use thiserror::Error;

use crate::SessionEvent;

pub const MAX_QUEUE_EVENTS: usize = 1_024;
pub const MAX_QUEUE_BYTES: usize = 8_388_608;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SubscriptionError {
    #[error("client_lagged")]
    ClientLagged,
    #[error("subscription is closed")]
    Closed,
    #[error("event encoding failed")]
    Encoding,
}

#[derive(Clone, Debug)]
pub struct SubscriptionQueue {
    events: VecDeque<(SessionEvent, usize)>,
    encoded_bytes: usize,
    max_events: usize,
    max_bytes: usize,
    closed: bool,
}

impl Default for SubscriptionQueue {
    fn default() -> Self {
        Self::new(MAX_QUEUE_EVENTS, MAX_QUEUE_BYTES)
    }
}

impl SubscriptionQueue {
    pub fn new(max_events: usize, max_bytes: usize) -> Self {
        Self {
            events: VecDeque::new(),
            encoded_bytes: 0,
            max_events,
            max_bytes,
            closed: false,
        }
    }

    pub fn push(&mut self, event: SessionEvent) -> Result<(), SubscriptionError> {
        if self.closed {
            return Err(SubscriptionError::Closed);
        }
        event.validate().map_err(|_| SubscriptionError::Encoding)?;
        let encoded_bytes = serde_json::to_vec(&event)
            .map_err(|_| SubscriptionError::Encoding)?
            .len()
            + 1;
        if self.events.len() == self.max_events
            || self.encoded_bytes.saturating_add(encoded_bytes) > self.max_bytes
        {
            self.closed = true;
            return Err(SubscriptionError::ClientLagged);
        }
        self.encoded_bytes += encoded_bytes;
        self.events.push_back((event, encoded_bytes));
        Ok(())
    }

    pub fn pop(&mut self) -> Option<SessionEvent> {
        self.events.pop_front().map(|(event, encoded_bytes)| {
            self.encoded_bytes -= encoded_bytes;
            event
        })
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

pub fn replay_after(
    events: impl IntoIterator<Item = SessionEvent>,
    after_sequence: u64,
) -> Vec<SessionEvent> {
    events
        .into_iter()
        .filter(|event| event.sequence > after_sequence)
        .collect()
}
