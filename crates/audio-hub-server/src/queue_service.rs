use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::models::{QueueItem, QueueResponse};
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
    pub now_playing: bool,
}

#[derive(Clone)]
pub(crate) struct QueueService {
    queue: Arc<Mutex<QueueState>>,
    status: StatusStore,
}

impl QueueService {
    pub(crate) fn new(queue: Arc<Mutex<QueueState>>, status: StatusStore) -> Self {
        Self { queue, status }
    }

    pub(crate) fn queue(&self) -> &Arc<Mutex<QueueState>> {
        &self.queue
    }

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
        added
    }

    pub(crate) fn remove_path(&self, path: &PathBuf) -> bool {
        let mut queue = self.queue.lock().unwrap();
        if let Some(pos) = queue.items.iter().position(|p| p == path) {
            queue.items.remove(pos);
            return true;
        }
        false
    }

    pub(crate) fn clear(&self) {
        let mut queue = self.queue.lock().unwrap();
        queue.items.clear();
    }

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
                Some(q.items.remove(0))
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

    pub(crate) fn maybe_auto_advance(
        &self,
        transport: &dyn PlaybackTransport,
        inputs: AutoAdvanceInputs,
    ) -> bool {
        let ended = inputs.last_duration_ms.is_some()
            && inputs.remote_duration_ms.is_none()
            && inputs.remote_elapsed_ms.is_none()
            && !inputs.user_paused
            && !inputs.seek_in_flight
            && inputs.now_playing;
        let should_dispatch = if ended && !inputs.auto_advance_in_flight {
            true
        } else if !inputs.auto_advance_in_flight && !inputs.seek_in_flight {
            if let (Some(elapsed), Some(duration)) = (inputs.elapsed_ms, inputs.duration_ms) {
                elapsed + 50 >= duration && !inputs.user_paused
            } else {
                false
            }
        } else {
            false
        };

        if !should_dispatch {
            return false;
        }

        matches!(
            self.dispatch_next(transport, true),
            NextDispatchResult::Dispatched
        )
    }
}
