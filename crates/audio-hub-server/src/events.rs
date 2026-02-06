//! In-process event bus for server-side updates.
//!
//! Provides a lightweight broadcast channel for UI subscriptions.

use tokio::sync::broadcast;

/// Server event payloads published by core services.
#[derive(Debug, Clone)]
pub enum HubEvent {
    QueueChanged,
    StatusChanged,
    OutputsChanged,
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<HubEvent>,
}

impl EventBus {
    /// Create a new event bus with a bounded broadcast channel.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(64);
        Self { sender }
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.sender.subscribe()
    }

    /// Notify subscribers that the queue has changed.
    pub fn queue_changed(&self) {
        let _ = self.sender.send(HubEvent::QueueChanged);
    }

    /// Notify subscribers that playback status has changed.
    pub fn status_changed(&self) {
        let _ = self.sender.send(HubEvent::StatusChanged);
    }

    /// Notify subscribers that outputs or selection have changed.
    pub fn outputs_changed(&self) {
        let _ = self.sender.send(HubEvent::OutputsChanged);
    }
}
