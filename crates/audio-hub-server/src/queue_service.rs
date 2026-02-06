//! Queue management + auto-advance logic.
//!
//! Owns queue mutations and decides when to dispatch the next track.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::models::{QueueItem, QueueResponse};
use crate::events::EventBus;
use crate::playback_transport::PlaybackTransport;
use crate::state::{QueueState};
use crate::status_store::StatusStore;

pub(crate) enum NextDispatchResult {
    Dispatched,
    Empty,
    Failed,
}

pub(crate) struct AutoAdvanceInputs {
    pub last_duration_ms: Option<u64>,
    pub remote_duration_ms: Option<u64>,
    pub remote_elapsed_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub user_paused: bool,
    pub seek_in_flight: bool,
    pub auto_advance_in_flight: bool,
    pub manual_advance_in_flight: bool,
    pub now_playing: bool,
}

fn should_auto_advance(inputs: &AutoAdvanceInputs) -> bool {
    if inputs.manual_advance_in_flight {
        return false;
    }
    let ended = inputs.last_duration_ms.is_some()
        && inputs.remote_duration_ms.is_none()
        && inputs.remote_elapsed_ms.is_none()
        && !inputs.user_paused
        && !inputs.seek_in_flight
        && inputs.now_playing;
    if ended && !inputs.auto_advance_in_flight {
        return true;
    }
    if !inputs.auto_advance_in_flight && !inputs.seek_in_flight {
        if let (Some(elapsed), Some(duration)) = (inputs.elapsed_ms, inputs.duration_ms) {
            return elapsed + 50 >= duration && !inputs.user_paused;
        }
    }
    false
}

#[derive(Clone)]
pub(crate) struct QueueService {
    queue: Arc<Mutex<QueueState>>,
    status: StatusStore,
    events: EventBus,
}

impl QueueService {
    /// Create a queue service backed by the shared queue + status store.
    pub(crate) fn new(queue: Arc<Mutex<QueueState>>, status: StatusStore, events: EventBus) -> Self {
        Self { queue, status, events }
    }

    /// Return the shared queue state (for inspection/testing).
    pub(crate) fn queue(&self) -> &Arc<Mutex<QueueState>> {
        &self.queue
    }

    /// Build an API response for the current queue.
    pub(crate) fn list(&self, library: &crate::library::LibraryIndex) -> QueueResponse {
        let queue = self.queue.lock().unwrap();
        let items = queue
            .items
            .iter()
            .map(|path| match library.find_track_by_path(path) {
                Some(crate::models::LibraryEntry::Track {
                    path,
                    file_name,
                    duration_ms,
                    sample_rate,
                    album,
                    artist,
                    format,
                    ..
                }) => QueueItem::Track {
                    path,
                    file_name,
                    duration_ms,
                    sample_rate,
                    album,
                    artist,
                    format,
                },
                _ => QueueItem::Missing {
                    path: path.to_string_lossy().to_string(),
                },
            })
            .collect();
        QueueResponse { items }
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
            self.events.queue_changed();
        }
        added
    }

    /// Insert paths at the front of the queue, preserving order and skipping duplicates.
    pub(crate) fn add_next_paths(&self, paths: Vec<PathBuf>) -> usize {
        let mut added = 0usize;
        let mut queue = self.queue.lock().unwrap();
        for path in paths {
            if queue.items.iter().any(|p| p == &path) {
                continue;
            }
            queue.items.insert(added, path);
            added += 1;
        }
        if added > 0 {
            self.events.queue_changed();
        }
        added
    }

    /// Remove a single path from the queue.
    pub(crate) fn remove_path(&self, path: &PathBuf) -> bool {
        let mut queue = self.queue.lock().unwrap();
        if let Some(pos) = queue.items.iter().position(|p| p == path) {
            queue.items.remove(pos);
            self.events.queue_changed();
            return true;
        }
        false
    }

    /// Clear the queue.
    pub(crate) fn clear(&self) {
        let mut queue = self.queue.lock().unwrap();
        if !queue.items.is_empty() {
            queue.items.clear();
            self.events.queue_changed();
        }
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
                None
            } else {
                let path = q.items.remove(0);
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

        if transport.play(path, ext_hint, None, false).is_ok() {
            if mark_auto_advance {
                self.status.set_auto_advance_in_flight(true);
            }
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
        let status = StatusStore::new(
            Arc::new(Mutex::new(crate::state::PlayerStatus::default())),
            crate::events::EventBus::new(),
        );
        let queue = Arc::new(Mutex::new(QueueState::default()));
        QueueService::new(queue, status, crate::events::EventBus::new())
    }

    fn make_inputs() -> AutoAdvanceInputs {
        AutoAdvanceInputs {
            last_duration_ms: None,
            remote_duration_ms: None,
            remote_elapsed_ms: None,
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
    fn remove_path_returns_true_when_found() {
        let service = make_service();
        let path = PathBuf::from("/music/a.flac");
        service.add_paths(vec![path.clone()]);
        assert!(service.remove_path(&path));
        assert!(service.queue.lock().unwrap().items.is_empty());
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
    fn add_next_paths_inserts_at_front() {
        let service = make_service();
        let a = PathBuf::from("/music/a.flac");
        let b = PathBuf::from("/music/b.flac");
        let c = PathBuf::from("/music/c.flac");
        service.add_paths(vec![a.clone(), b.clone()]);

        let added = service.add_next_paths(vec![c.clone()]);

        assert_eq!(added, 1);
        let queue = service.queue.lock().unwrap();
        assert_eq!(queue.items, vec![c, a, b]);
    }

    #[test]
    fn maybe_auto_advance_dispatches_on_remote_end() {
        let service = make_service();
        let transport = TestTransport::new(true);
        let path = PathBuf::from("/music/a.flac");
        service.add_paths(vec![path.clone()]);

        let inputs = AutoAdvanceInputs {
            last_duration_ms: Some(1000),
            remote_duration_ms: None,
            remote_elapsed_ms: None,
            ..make_inputs()
        };

        assert!(service.maybe_auto_advance(&transport, inputs));
    }

    #[test]
    fn maybe_auto_advance_dispatches_near_end() {
        let service = make_service();
        let transport = TestTransport::new(true);
        let path = PathBuf::from("/music/a.flac");
        service.add_paths(vec![path.clone()]);

        let inputs = AutoAdvanceInputs {
            elapsed_ms: Some(9950),
            duration_ms: Some(10000),
            ..make_inputs()
        };

        assert!(service.maybe_auto_advance(&transport, inputs));
    }

    #[test]
    fn maybe_auto_advance_respects_user_pause_and_seek() {
        let service = make_service();
        let transport = TestTransport::new(true);
        service.add_paths(vec![PathBuf::from("/music/a.flac")]);

        let paused_inputs = AutoAdvanceInputs {
            elapsed_ms: Some(9950),
            duration_ms: Some(10000),
            user_paused: true,
            ..make_inputs()
        };
        assert!(!service.maybe_auto_advance(&transport, paused_inputs));

        let seek_inputs = AutoAdvanceInputs {
            elapsed_ms: Some(9950),
            duration_ms: Some(10000),
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
            last_duration_ms: Some(1000),
            remote_duration_ms: None,
            remote_elapsed_ms: None,
            manual_advance_in_flight: true,
            now_playing: true,
            ..make_inputs()
        };
        assert!(!service.maybe_auto_advance(&transport, inputs));
    }
}
