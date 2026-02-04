use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;

use crate::bridge::BridgeCommand;
use crate::state::{PlayerStatus, QueueState};

pub(crate) enum NextDispatchResult {
    Dispatched,
    Empty,
    Failed,
}

pub(crate) fn dispatch_next_from_queue(
    queue: &Arc<Mutex<QueueState>>,
    status: &Arc<Mutex<PlayerStatus>>,
    cmd_tx: &Sender<BridgeCommand>,
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
    if cmd_tx
        .send(BridgeCommand::Play {
            path,
            ext_hint,
            seek_ms: None,
            start_paused: false,
        })
        .is_ok()
    {
        if mark_auto_advance {
            if let Ok(mut s) = status.lock() {
                s.auto_advance_in_flight = true;
            }
        }
        NextDispatchResult::Dispatched
    } else {
        NextDispatchResult::Failed
    }
}
