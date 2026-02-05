use std::sync::{Arc, Mutex};

use crate::playback_transport::PlaybackTransport;
use crate::state::QueueState;
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

pub(crate) fn maybe_auto_advance(
    queue: &Arc<Mutex<QueueState>>,
    status: &StatusStore,
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
        dispatch_next_from_queue(queue, status, transport, true),
        NextDispatchResult::Dispatched
    )
}

pub(crate) fn dispatch_next_from_queue(
    queue: &Arc<Mutex<QueueState>>,
    status: &StatusStore,
    transport: &dyn PlaybackTransport,
    mark_auto_advance: bool,
) -> NextDispatchResult {
    let path = {
        let mut q = match queue.lock() {
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
            status.set_auto_advance_in_flight(true);
        }
        NextDispatchResult::Dispatched
    } else {
        NextDispatchResult::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback_transport::ChannelTransport;
    use crossbeam_channel::unbounded;
    use std::sync::Mutex;
    use crate::state::PlayerStatus;
    use crate::status_store::StatusStore;

    fn base_inputs() -> AutoAdvanceInputs {
        AutoAdvanceInputs {
            last_duration_ms: Some(1000),
            remote_duration_ms: Some(1000),
            remote_elapsed_ms: Some(100),
            elapsed_ms: Some(100),
            duration_ms: Some(1000),
            user_paused: false,
            seek_in_flight: false,
            auto_advance_in_flight: false,
            now_playing: true,
        }
    }

    #[test]
    fn auto_advance_triggers_on_end_condition() {
        let (tx, _rx) = unbounded();
        let queue = Arc::new(Mutex::new(QueueState {
            items: vec![std::path::PathBuf::from("track.flac")],
        }));
        let status = StatusStore::new(Arc::new(Mutex::new(PlayerStatus::default())));
        let transport = ChannelTransport::new(tx);

        let mut inputs = base_inputs();
        inputs.remote_duration_ms = None;
        inputs.remote_elapsed_ms = None;

        let dispatched = maybe_auto_advance(&queue, &status, &transport, inputs);
        assert!(dispatched);
    }

    #[test]
    fn auto_advance_skips_when_user_paused() {
        let (tx, _rx) = unbounded();
        let queue = Arc::new(Mutex::new(QueueState {
            items: vec![std::path::PathBuf::from("track.flac")],
        }));
        let status = StatusStore::new(Arc::new(Mutex::new(PlayerStatus::default())));
        let transport = ChannelTransport::new(tx);

        let mut inputs = base_inputs();
        inputs.user_paused = true;
        inputs.elapsed_ms = Some(995);
        inputs.duration_ms = Some(1000);

        let dispatched = maybe_auto_advance(&queue, &status, &transport, inputs);
        assert!(!dispatched);
    }
}
