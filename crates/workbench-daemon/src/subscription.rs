use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::mpsc;
use uuid::Uuid;
use workbench_protocol::{SessionEvent, SubscriptionError, SubscriptionQueue};

#[derive(Debug, Clone, Copy)]
enum Signal {
    Event,
    Lagged,
}

struct Subscriber {
    queue: Arc<Mutex<SubscriptionQueue>>,
    signals: mpsc::UnboundedSender<Signal>,
}

#[derive(Default)]
pub struct SubscriptionHub {
    sessions: Mutex<HashMap<Uuid, HashMap<Uuid, Subscriber>>>,
}

impl SubscriptionHub {
    pub fn subscribe(
        self: &Arc<Self>,
        session_id: Uuid,
        replay: impl IntoIterator<Item = SessionEvent>,
    ) -> Result<SessionSubscription, SubscriptionError> {
        let queue = Arc::new(Mutex::new(SubscriptionQueue::default()));
        {
            let mut queue_guard = queue.lock().map_err(|_| SubscriptionError::Closed)?;
            for event in replay {
                queue_guard.push(event)?;
            }
        }
        let (signals, receiver) = mpsc::unbounded_channel();
        let id = Uuid::now_v7();
        let initial_events = queue.lock().map_err(|_| SubscriptionError::Closed)?.len();
        for _ in 0..initial_events {
            signals
                .send(Signal::Event)
                .map_err(|_| SubscriptionError::Closed)?;
        }
        self.sessions
            .lock()
            .map_err(|_| SubscriptionError::Closed)?
            .entry(session_id)
            .or_default()
            .insert(
                id,
                Subscriber {
                    queue: Arc::clone(&queue),
                    signals,
                },
            );
        Ok(SessionSubscription {
            id,
            session_id,
            hub: Arc::clone(self),
            queue,
            receiver,
        })
    }

    pub fn publish(&self, event: &SessionEvent) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let Some(subscribers) = sessions.get_mut(&event.session_id) else {
            return;
        };
        subscribers.retain(|_, subscriber| {
            let pushed = subscriber
                .queue
                .lock()
                .ok()
                .and_then(|mut queue| queue.push(event.clone()).ok())
                .is_some();
            if pushed {
                subscriber.signals.send(Signal::Event).is_ok()
            } else {
                let _ignored = subscriber.signals.send(Signal::Lagged);
                false
            }
        });
        if subscribers.is_empty() {
            sessions.remove(&event.session_id);
        }
    }

    pub fn purge_session(&self, session_id: Uuid) -> Result<(), SubscriptionError> {
        let subscribers = self
            .sessions
            .lock()
            .map_err(|_| SubscriptionError::Closed)?
            .remove(&session_id);
        if let Some(subscribers) = subscribers {
            for subscriber in subscribers.into_values() {
                let mut queue = subscriber
                    .queue
                    .lock()
                    .map_err(|_| SubscriptionError::Closed)?;
                while queue.pop().is_some() {}
            }
        }
        Ok(())
    }

    fn remove(&self, session_id: Uuid, id: Uuid) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        if let Some(subscribers) = sessions.get_mut(&session_id) {
            subscribers.remove(&id);
            if subscribers.is_empty() {
                sessions.remove(&session_id);
            }
        }
    }
}

pub enum SubscriptionItem {
    Event(SessionEvent),
    Lagged,
}

pub struct SessionSubscription {
    id: Uuid,
    session_id: Uuid,
    hub: Arc<SubscriptionHub>,
    queue: Arc<Mutex<SubscriptionQueue>>,
    receiver: mpsc::UnboundedReceiver<Signal>,
}

impl SessionSubscription {
    pub async fn next(&mut self) -> Option<SubscriptionItem> {
        match self.receiver.recv().await? {
            Signal::Event => self
                .queue
                .lock()
                .ok()
                .and_then(|mut queue| queue.pop())
                .map(SubscriptionItem::Event),
            Signal::Lagged => Some(SubscriptionItem::Lagged),
        }
    }
}

impl Drop for SessionSubscription {
    fn drop(&mut self) {
        self.hub.remove(self.session_id, self.id);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use workbench_protocol::{EventKind, PROTOCOL_V1};

    use super::*;

    fn event(sequence: u64) -> SessionEvent {
        SessionEvent {
            protocol: PROTOCOL_V1.to_owned(),
            event_id: Uuid::now_v7(),
            session_id: Uuid::nil(),
            sequence,
            causation_request_id: None,
            kind: EventKind::ProviderEvent,
            occurred_at: "1970-01-01T00:00:00Z".to_owned(),
            data: json!({}),
        }
    }

    #[tokio::test]
    async fn publishes_to_multiple_subscribers() {
        let hub = Arc::new(SubscriptionHub::default());
        let mut first = hub.subscribe(Uuid::nil(), []).expect("subscriber");
        let mut second = hub.subscribe(Uuid::nil(), []).expect("subscriber");
        hub.publish(&event(1));
        assert!(matches!(
            first.next().await,
            Some(SubscriptionItem::Event(_))
        ));
        assert!(matches!(
            second.next().await,
            Some(SubscriptionItem::Event(_))
        ));
    }
}
