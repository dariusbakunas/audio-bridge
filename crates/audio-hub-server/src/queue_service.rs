//! Queue management + auto-advance logic.
//!
//! Owns queue mutations and decides when to dispatch the next track.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::models::{QueueItem, QueueResponse};
use audio_bridge_types::PlaybackEndReason;
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
    pub end_reason: Option<PlaybackEndReason>,
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
pub(crate) struct QueueService {
    queue: Arc<Mutex<QueueState>>,
    status: StatusStore,
    events: EventBus,
}

impl QueueService {
    /// Create a queue service backed by the shared queue + status store.
    pub(crate) fn new(queue: Arc<Mutex<QueueState>>, status: StatusStore, events: EventBus) -> Self {
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

    pub(crate) fn record_played_path(&self, path: &Path) {
        const MAX_HISTORY: usize = 100;
        let mut queue = self.queue.lock().unwrap();
        if queue.history.back().map(|last| last == path).unwrap_or(false) {
            return;
        }
        queue.history.push_back(path.to_path_buf());
        if queue.history.len() > MAX_HISTORY {
            queue.history.pop_front();
        }
    }

    pub(crate) fn take_previous(&self, current: Option<&Path>) -> Option<PathBuf> {
        let mut queue = self.queue.lock().unwrap();
        while let Some(last) = queue.history.pop_back() {
            if current.map(|c| c == last).unwrap_or(false) {
                continue;
            }
            return Some(last);
        }
        None
    }

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

    /// Build an API response for the current queue.
    pub(crate) fn list(
        &self,
        library: &crate::library::LibraryIndex,
        metadata_db: Option<&crate::metadata_db::MetadataDb>,
    ) -> QueueResponse {
        let now_playing = self
            .status
            .inner()
            .lock()
            .ok()
            .and_then(|guard| guard.now_playing.clone());
        let now_playing_str = now_playing.as_ref().map(|p| p.to_string_lossy().to_string());
        let (queued_items, history_items) = {
            let queue = self.queue.lock().unwrap();
            (queue.items.clone(), queue.history.clone())
        };

        let mut items: Vec<QueueItem> = queued_items
            .iter()
            .map(|path| {
                let is_now_playing = now_playing_str
                    .as_deref()
                    .map(|current| current == path.to_string_lossy().as_ref())
                    .unwrap_or(false);
                match library.find_track_by_path(path) {
                    Some(crate::models::LibraryEntry::Track {
                        path,
                        file_name,
                        duration_ms,
                        sample_rate,
                        album,
                        artist,
                        format,
                        ..
                    }) => {
                        let title = metadata_db
                            .and_then(|db| {
                                db.track_record_by_path(path.as_str())
                                    .ok()
                                    .flatten()
                                    .and_then(|record| record.title)
                            });
                        let id = metadata_db
                            .and_then(|db| db.track_id_for_path(&path).ok().flatten());
                        QueueItem::Track {
                            id,
                            path,
                            file_name,
                            title,
                            duration_ms,
                            sample_rate,
                            album,
                            artist,
                            format,
                            now_playing: is_now_playing,
                            played: false,
                        }
                    }
                    _ => QueueItem::Missing {
                        path: path.to_string_lossy().to_string(),
                    },
                }
            })
            .collect();

        if let Some(current_path) = now_playing {
            let current_str = current_path.to_string_lossy();
            let index = items.iter().position(|item| match item {
                QueueItem::Track { path, .. } => path == current_str.as_ref(),
                QueueItem::Missing { path } => path == current_str.as_ref(),
            });
            if let Some(index) = index {
                if index != 0 {
                    let current = items.remove(index);
                    items.insert(0, current);
                }
            } else {
                let entry = match library.find_track_by_path(&current_path) {
                    Some(crate::models::LibraryEntry::Track {
                        path,
                        file_name,
                        duration_ms,
                        sample_rate,
                        album,
                        artist,
                        format,
                        ..
                    }) => {
                        let title = metadata_db
                            .and_then(|db| {
                                db.track_record_by_path(path.as_str())
                                    .ok()
                                    .flatten()
                                    .and_then(|record| record.title)
                            });
                        let id = metadata_db
                            .and_then(|db| db.track_id_for_path(&path).ok().flatten());
                        QueueItem::Track {
                            id,
                            path,
                            file_name,
                            title,
                            duration_ms,
                            sample_rate,
                            album,
                            artist,
                            format,
                            now_playing: true,
                            played: false,
                        }
                    }
                    _ => QueueItem::Missing {
                        path: current_path.to_string_lossy().to_string(),
                    },
                };
                items.insert(0, entry);
            }
        }

        let mut played_paths = Vec::new();
        for path in history_items.iter().rev() {
            if let Some(current) = now_playing_str.as_deref() {
                if current == path.to_string_lossy().as_ref() {
                    continue;
                }
            }
            played_paths.push(path.clone());
            if played_paths.len() >= 10 {
                break;
            }
        }

        if !played_paths.is_empty() {
            played_paths.reverse();
            let mut seen = std::collections::HashSet::new();
            for item in &items {
                match item {
                    QueueItem::Track { path, .. } => {
                        seen.insert(path.clone());
                    }
                    QueueItem::Missing { path } => {
                        seen.insert(path.clone());
                    }
                }
            }

            let mut played_items = Vec::new();
            for path in played_paths {
                let path_str = path.to_string_lossy().to_string();
                if seen.contains(&path_str) {
                    continue;
                }
                let entry = match library.find_track_by_path(&path) {
                    Some(crate::models::LibraryEntry::Track {
                        path,
                        file_name,
                        duration_ms,
                        sample_rate,
                        album,
                        artist,
                        format,
                        ..
                    }) => {
                        let title = metadata_db
                            .and_then(|db| {
                                db.track_record_by_path(path.as_str())
                                    .ok()
                                    .flatten()
                                    .and_then(|record| record.title)
                            });
                        let id = metadata_db
                            .and_then(|db| db.track_id_for_path(&path).ok().flatten());
                        QueueItem::Track {
                            id,
                            path,
                            file_name,
                            title,
                            duration_ms,
                            sample_rate,
                            album,
                            artist,
                            format,
                            now_playing: false,
                            played: true,
                        }
                    }
                    _ => QueueItem::Missing { path: path_str },
                };
                played_items.push(entry);
            }

            if !played_items.is_empty() {
                played_items.append(&mut items);
                items = played_items;
            }
        }

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
            tracing::debug!(added, total = queue.items.len(), "queue add paths");
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
            tracing::debug!(added, total = queue.items.len(), "queue add next paths");
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
    pub(crate) fn clear(&self, clear_queue: bool, clear_history: bool) -> bool {
        let mut queue = self.queue.lock().unwrap();
        let mut changed = false;
        if clear_queue && !queue.items.is_empty() {
            tracing::debug!(count = queue.items.len(), "queue cleared");
            queue.items.clear();
            changed = true;
        }
        if clear_history && !queue.history.is_empty() {
            tracing::debug!(count = queue.history.len(), "queue history cleared");
            queue.history.clear();
            changed = true;
            self.status.set_has_previous(false);
        }
        if changed {
            self.events.queue_changed();
        }
        changed
    }

    /// Drop all items up to and including the matching path.
    pub(crate) fn drain_through_path(&self, path: &Path) -> bool {
        let mut queue = self.queue.lock().unwrap();
        if let Some(pos) = queue.items.iter().position(|p| p == path) {
            queue.items.drain(0..=pos);
            self.events.queue_changed();
            return true;
        }
        false
    }

    /// Rebuild the queue starting after the supplied path, checking queue and history.
    pub(crate) fn play_from_any(&self, path: &Path) -> bool {
        if self.drain_through_path(path) {
            return true;
        }

        let (history_tail, queued_items) = {
            let mut queue = self.queue.lock().unwrap();
            let pos = match queue.history.iter().position(|p| p == path) {
                Some(pos) => pos,
                None => return false,
            };
            let tail = queue
                .history
                .iter()
                .skip(pos + 1)
                .cloned()
                .collect::<Vec<PathBuf>>();
            if pos + 1 < queue.history.len() {
                queue.history.drain((pos + 1)..);
            }
            let queued_items = queue.items.clone();
            (tail, queued_items)
        };

        let current = self
            .status
            .inner()
            .lock()
            .ok()
            .and_then(|guard| guard.now_playing.clone());

        let mut seen = std::collections::HashSet::new();
        let mut rebuilt = Vec::new();

        for item in history_tail {
            if seen.insert(item.clone()) {
                rebuilt.push(item);
            }
        }

        if let Some(current) = current {
            if seen.insert(current.clone()) {
                rebuilt.push(current);
            }
        }

        for item in queued_items.iter() {
            if seen.insert(item.clone()) {
                rebuilt.push(item.clone());
            }
        }

        if let Ok(mut queue) = self.queue.lock() {
            queue.items = rebuilt;
        }
        self.events.queue_changed();
        true
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
    fn drain_through_path_removes_up_to_match() {
        let service = make_service();
        let a = PathBuf::from("/music/a.flac");
        let b = PathBuf::from("/music/b.flac");
        let c = PathBuf::from("/music/c.flac");
        service.add_paths(vec![a.clone(), b.clone(), c.clone()]);

        assert!(service.drain_through_path(&b));
        let queue = service.queue.lock().unwrap();
        assert_eq!(queue.items, vec![c]);
    }

    #[test]
    fn drain_through_path_returns_false_when_missing() {
        let service = make_service();
        service.add_paths(vec![PathBuf::from("/music/a.flac")]);

        assert!(!service.drain_through_path(Path::new("/music/missing.flac")));
        let queue = service.queue.lock().unwrap();
        assert_eq!(queue.items.len(), 1);
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
    fn play_from_any_rebuilds_queue_from_history_and_trims() {
        let (service, status) = make_service_with_status();
        let a = PathBuf::from("/music/a.flac");
        let b = PathBuf::from("/music/b.flac");
        let c = PathBuf::from("/music/c.flac");
        let d = PathBuf::from("/music/d.flac");
        let e = PathBuf::from("/music/e.flac");
        let f = PathBuf::from("/music/f.flac");

        service.record_played_path(&a);
        service.record_played_path(&b);
        service.record_played_path(&c);
        status.on_play(d.clone(), false);
        service.add_paths(vec![e.clone(), f.clone()]);

        assert!(service.play_from_any(&b));

        let queue_items = service.queue.lock().unwrap().items.clone();
        assert_eq!(queue_items, vec![c, d, e, f]);

        let history = service.queue.lock().unwrap().history.clone();
        assert_eq!(history.into_iter().collect::<Vec<_>>(), vec![a, b]);
    }

    #[test]
    fn play_from_any_returns_false_when_missing() {
        let service = make_service();
        let missing = PathBuf::from("/music/missing.flac");
        service.add_paths(vec![PathBuf::from("/music/a.flac")]);
        assert!(!service.play_from_any(&missing));
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
        let has_previous = status
            .inner()
            .lock()
            .unwrap()
            .has_previous;
        assert_eq!(has_previous, Some(false));

        assert!(matches!(
            service.dispatch_next(&transport, false),
            NextDispatchResult::Dispatched
        ));
        let has_previous = status
            .inner()
            .lock()
            .unwrap()
            .has_previous;
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
