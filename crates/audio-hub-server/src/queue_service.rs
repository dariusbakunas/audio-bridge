//! Queue management + auto-advance logic.
//!
//! Owns queue mutations and decides when to dispatch the next track.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::events::EventBus;
use crate::playback_transport::PlaybackTransport;
use crate::state::QueueState;
use crate::status_store::StatusStore;
use audio_bridge_types::PlaybackEndReason;

/// Result of attempting to dispatch the next queued track.
pub(crate) enum NextDispatchResult {
    /// Next track was dispatched successfully.
    Dispatched,
    /// Queue was empty.
    Empty,
    /// Dispatch failed (transport/channel error).
    Failed,
}

/// Snapshot of status fields used to decide auto-advance behavior.
pub(crate) struct AutoAdvanceInputs {
    /// Duration from previous known local status.
    pub last_duration_ms: Option<u64>,
    /// Duration from latest remote status.
    pub remote_duration_ms: Option<u64>,
    /// Elapsed from latest remote status.
    pub remote_elapsed_ms: Option<u64>,
    /// Reported playback end reason.
    pub end_reason: Option<PlaybackEndReason>,
    /// Effective elapsed from merged status.
    pub elapsed_ms: Option<u64>,
    /// Effective duration from merged status.
    pub duration_ms: Option<u64>,
    /// Whether user explicitly paused playback.
    pub user_paused: bool,
    /// Whether a seek operation is still in flight.
    pub seek_in_flight: bool,
    /// Whether auto-advance already dispatched.
    pub auto_advance_in_flight: bool,
    /// Whether manual next/previous is in flight.
    pub manual_advance_in_flight: bool,
    /// Whether status currently has a playing track.
    pub now_playing: bool,
}

fn should_auto_advance(inputs: &AutoAdvanceInputs) -> bool {
    if inputs.manual_advance_in_flight {
        return false;
    }
    let ended = matches!(inputs.end_reason, Some(PlaybackEndReason::Eof))
        && !inputs.user_paused
        && !inputs.seek_in_flight
        && inputs.now_playing;
    if ended && !inputs.auto_advance_in_flight {
        return true;
    }
    false
}

#[derive(Clone)]
/// Queue mutation service and auto-advance coordinator.
pub(crate) struct QueueService {
    queue: Arc<Mutex<QueueState>>,
    status: StatusStore,
    events: EventBus,
}

impl QueueService {
    /// Create a queue service backed by the shared queue + status store.
    pub(crate) fn new(
        queue: Arc<Mutex<QueueState>>,
        status: StatusStore,
        events: EventBus,
    ) -> Self {
        Self {
            queue,
            status,
            events,
        }
    }

    /// Return the shared queue state (for inspection/testing).
    pub(crate) fn queue(&self) -> &Arc<Mutex<QueueState>> {
        &self.queue
    }

    /// Add path to played history (deduplicated tail, bounded to 100 entries).
    pub(crate) fn record_played_path(&self, path: &Path) {
        const MAX_HISTORY: usize = 100;
        let mut queue = self.queue.lock().unwrap();
        if queue
            .history
            .back()
            .map(|last| last == path)
            .unwrap_or(false)
        {
            return;
        }
        queue.history.push_back(path.to_path_buf());
        if queue.history.len() > MAX_HISTORY {
            queue.history.pop_front();
        }
    }

    /// Returns `true` when history contains an item different from current.
    pub(crate) fn has_previous(&self, current: Option<&Path>) -> bool {
        let queue = self.queue.lock().unwrap();
        for path in queue.history.iter().rev() {
            if current.map(|c| c == path).unwrap_or(false) {
                continue;
            }
            return true;
        }
        false
    }

    /// Add paths to the queue, skipping duplicates.
    pub(crate) fn add_paths(&self, paths: Vec<PathBuf>) -> usize {
        let mut added = 0usize;
        let mut queue = self.queue.lock().unwrap();
        for path in paths {
            if queue.items.iter().any(|p| p == &path) {
                continue;
            }
            queue.items.push(path);
            added += 1;
        }
        if added > 0 {
            tracing::debug!(added, total = queue.items.len(), "queue add paths");
            self.events.queue_changed();
        }
        added
    }

    /// Clear the queue.
    pub(crate) fn clear(&self, clear_queue: bool, clear_history: bool) -> bool {
        let mut changed = false;
        let mut cleared_history = false;
        {
            let mut queue = self.queue.lock().unwrap();
            if clear_queue && !queue.items.is_empty() {
                tracing::debug!(count = queue.items.len(), "queue cleared");
                queue.items.clear();
                changed = true;
            }
            if clear_history && !queue.history.is_empty() {
                tracing::debug!(count = queue.history.len(), "queue history cleared");
                queue.history.clear();
                changed = true;
                cleared_history = true;
            }
        }
        if cleared_history {
            self.status.set_has_previous(false);
        }
        if changed {
            self.events.queue_changed();
        }
        changed
    }

    /// Dispatch the next track (if any) via the provided transport.
    pub(crate) fn dispatch_next(
        &self,
        transport: &dyn PlaybackTransport,
        mark_auto_advance: bool,
    ) -> NextDispatchResult {
        let path = {
            let mut q = match self.queue.lock() {
                Ok(q) => q,
                Err(_) => return NextDispatchResult::Failed,
            };
            if q.items.is_empty() {
                tracing::debug!("queue dispatch requested but queue is empty");
                None
            } else {
                let path = q.items.remove(0);
                tracing::info!(
                    path = %path.display(),
                    remaining = q.items.len(),
                    auto_advance = mark_auto_advance,
                    "queue dispatching next track"
                );
                self.events.queue_changed();
                Some(path)
            }
        };

        let Some(path) = path else {
            return NextDispatchResult::Empty;
        };

        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if transport.play(path.clone(), ext_hint, None, false).is_ok() {
            if mark_auto_advance {
                self.status.set_auto_advance_in_flight(true);
            }
            self.record_played_path(&path);
            self.status.set_has_previous(self.has_previous(Some(&path)));
            NextDispatchResult::Dispatched
        } else {
            NextDispatchResult::Failed
        }
    }

    /// Decide whether to auto-advance and dispatch if needed.
    pub(crate) fn maybe_auto_advance(
        &self,
        transport: &dyn PlaybackTransport,
        inputs: AutoAdvanceInputs,
    ) -> bool {
        tracing::debug!(
            last_duration_ms = ?inputs.last_duration_ms,
            remote_duration_ms = ?inputs.remote_duration_ms,
            remote_elapsed_ms = ?inputs.remote_elapsed_ms,
            elapsed_ms = ?inputs.elapsed_ms,
            duration_ms = ?inputs.duration_ms,
            user_paused = inputs.user_paused,
            seek_in_flight = inputs.seek_in_flight,
            auto_advance_in_flight = inputs.auto_advance_in_flight,
            manual_advance_in_flight = inputs.manual_advance_in_flight,
            now_playing = inputs.now_playing,
            "auto-advance check"
        );
        let should_dispatch = should_auto_advance(&inputs);

        if !should_dispatch {
            return false;
        }

        tracing::debug!("auto-advance dispatching next track");
        matches!(
            self.dispatch_next(transport, true),
            NextDispatchResult::Dispatched
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct TestTransport {
        plays: Arc<Mutex<Vec<(PathBuf, String, Option<u64>, bool)>>>,
        should_succeed: bool,
    }

    impl TestTransport {
        fn new(should_succeed: bool) -> Self {
            Self {
                plays: Arc::new(Mutex::new(Vec::new())),
                should_succeed,
            }
        }
    }

    impl PlaybackTransport for TestTransport {
        fn play(
            &self,
            path: PathBuf,
            ext_hint: String,
            seek_ms: Option<u64>,
            start_paused: bool,
        ) -> Result<(), crate::playback_transport::PlaybackTransportError> {
            self.plays
                .lock()
                .unwrap()
                .push((path, ext_hint, seek_ms, start_paused));
            if self.should_succeed {
                Ok(())
            } else {
                Err(crate::playback_transport::PlaybackTransportError::Offline)
            }
        }

        fn pause_toggle(&self) -> Result<(), crate::playback_transport::PlaybackTransportError> {
            Ok(())
        }

        fn stop(&self) -> Result<(), crate::playback_transport::PlaybackTransportError> {
            Ok(())
        }

        fn seek(&self, _ms: u64) -> Result<(), crate::playback_transport::PlaybackTransportError> {
            Ok(())
        }
    }

    fn make_service() -> QueueService {
        make_service_with_status().0
    }

    fn make_service_with_status() -> (QueueService, StatusStore) {
        let status = StatusStore::new(
            Arc::new(Mutex::new(crate::state::PlayerStatus::default())),
            crate::events::EventBus::new(),
        );
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let service = QueueService::new(queue, status.clone(), crate::events::EventBus::new());
        (service, status)
    }

    fn make_inputs() -> AutoAdvanceInputs {
        AutoAdvanceInputs {
            last_duration_ms: None,
            remote_duration_ms: None,
            remote_elapsed_ms: None,
            end_reason: None,
            elapsed_ms: None,
            duration_ms: None,
            user_paused: false,
            seek_in_flight: false,
            auto_advance_in_flight: false,
            manual_advance_in_flight: false,
            now_playing: true,
        }
    }

    #[test]
    fn add_paths_skips_duplicates() {
        let service = make_service();
        let path = PathBuf::from("/music/a.flac");
        let added = service.add_paths(vec![path.clone(), path.clone()]);
        assert_eq!(added, 1);
        assert_eq!(service.queue.lock().unwrap().items.len(), 1);
    }

    #[test]
    fn dispatch_next_returns_empty_when_queue_empty() {
        let service = make_service();
        let transport = TestTransport::new(true);
        assert!(matches!(
            service.dispatch_next(&transport, false),
            NextDispatchResult::Empty
        ));
    }

    #[test]
    fn dispatch_next_sends_play_and_marks_auto_advance() {
        let service = make_service();
        let status = service.status.clone();
        let transport = TestTransport::new(true);
        let path = PathBuf::from("/music/a.flac");
        service.add_paths(vec![path.clone()]);

        let result = service.dispatch_next(&transport, true);

        assert!(matches!(result, NextDispatchResult::Dispatched));
        let plays = transport.plays.lock().unwrap();
        assert_eq!(plays.len(), 1);
        assert_eq!(plays[0].0, path);
        assert_eq!(plays[0].1, "flac");
        assert!(status.inner().lock().unwrap().auto_advance_in_flight);
    }

    #[test]
    fn clear_returns_true_only_when_items_exist() {
        let service = make_service();
        assert!(!service.clear(true, false));
        service.add_paths(vec![PathBuf::from("/music/a.flac")]);
        assert!(service.clear(true, false));
        let queue = service.queue.lock().unwrap();
        assert!(queue.items.is_empty());
    }

    #[test]
    fn maybe_auto_advance_dispatches_on_remote_end() {
        let service = make_service();
        let transport = TestTransport::new(true);
        let path = PathBuf::from("/music/a.flac");
        service.add_paths(vec![path.clone()]);

        let inputs = AutoAdvanceInputs {
            end_reason: Some(PlaybackEndReason::Eof),
            ..make_inputs()
        };

        assert!(service.maybe_auto_advance(&transport, inputs));
    }

    #[test]
    fn maybe_auto_advance_does_not_dispatch_near_end_without_eof() {
        let service = make_service();
        let transport = TestTransport::new(true);
        let path = PathBuf::from("/music/a.flac");
        service.add_paths(vec![path.clone()]);

        let inputs = AutoAdvanceInputs {
            elapsed_ms: Some(9950),
            duration_ms: Some(10000),
            ..make_inputs()
        };

        assert!(!service.maybe_auto_advance(&transport, inputs));
    }

    #[test]
    fn maybe_auto_advance_respects_user_pause_and_seek() {
        let service = make_service();
        let transport = TestTransport::new(true);
        service.add_paths(vec![PathBuf::from("/music/a.flac")]);

        let paused_inputs = AutoAdvanceInputs {
            end_reason: Some(PlaybackEndReason::Eof),
            user_paused: true,
            ..make_inputs()
        };
        assert!(!service.maybe_auto_advance(&transport, paused_inputs));

        let seek_inputs = AutoAdvanceInputs {
            end_reason: Some(PlaybackEndReason::Eof),
            seek_in_flight: true,
            ..make_inputs()
        };
        assert!(!service.maybe_auto_advance(&transport, seek_inputs));
    }

    #[test]
    fn maybe_auto_advance_respects_manual_next_in_flight() {
        let service = make_service();
        let transport = TestTransport::new(true);
        service.add_paths(vec![PathBuf::from("/music/a.flac")]);
        let inputs = AutoAdvanceInputs {
            end_reason: Some(PlaybackEndReason::Eof),
            manual_advance_in_flight: true,
            now_playing: true,
            ..make_inputs()
        };
        assert!(!service.maybe_auto_advance(&transport, inputs));
    }

    #[test]
    fn maybe_auto_advance_respects_user_pause_for_remote_end() {
        let service = make_service();
        let transport = TestTransport::new(true);
        service.add_paths(vec![PathBuf::from("/music/a.flac")]);
        let inputs = AutoAdvanceInputs {
            end_reason: Some(PlaybackEndReason::Eof),
            user_paused: true,
            now_playing: true,
            ..make_inputs()
        };
        assert!(!service.maybe_auto_advance(&transport, inputs));
    }

    #[test]
    fn has_previous_ignores_current_repeat() {
        let service = make_service();
        let a = PathBuf::from("/music/a.flac");
        let b = PathBuf::from("/music/b.flac");
        service.record_played_path(&a);
        service.record_played_path(&b);
        service.record_played_path(&b);

        assert!(service.has_previous(Some(&b)));
        assert!(service.has_previous(Some(&a)));

        let service = make_service();
        service.record_played_path(&a);
        service.record_played_path(&a);
        assert!(!service.has_previous(Some(&a)));
    }

    #[test]
    fn dispatch_next_updates_has_previous() {
        let (service, status) = make_service_with_status();
        let transport = TestTransport::new(true);
        let a = PathBuf::from("/music/a.flac");
        let b = PathBuf::from("/music/b.flac");
        service.add_paths(vec![a.clone(), b.clone()]);

        assert!(matches!(
            service.dispatch_next(&transport, false),
            NextDispatchResult::Dispatched
        ));
        let has_previous = status.inner().lock().unwrap().has_previous;
        assert_eq!(has_previous, Some(false));

        assert!(matches!(
            service.dispatch_next(&transport, false),
            NextDispatchResult::Dispatched
        ));
        let has_previous = status.inner().lock().unwrap().has_previous;
        assert_eq!(has_previous, Some(true));
    }

    #[test]
    fn clear_history_resets_previous_flag() {
        let (service, status) = make_service_with_status();
        let a = PathBuf::from("/music/a.flac");
        service.record_played_path(&a);
        status.set_has_previous(true);

        assert!(service.clear(true, true));
        let status = status.inner().lock().unwrap();
        assert_eq!(status.has_previous, Some(false));
        let history = service.queue.lock().unwrap().history.clone();
        assert!(history.is_empty());
    }
}
