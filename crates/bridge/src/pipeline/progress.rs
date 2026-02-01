//! Progress reporting back to the sender over TCP.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Background thread that periodically reports playback position to the sender.
pub(crate) struct ProgressReporter {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl ProgressReporter {
    pub(crate) fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

/// Spawn a reporter thread that sends periodic PLAYBACK_POS frames.
///
/// When `stop` is set, the thread emits a final "paused=true" frame and exits.
pub(crate) fn start_progress_reporter(
    mut peer_tx: std::net::TcpStream,
    played_frames: Arc<AtomicU64>,
    paused: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> ProgressReporter {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_thread = stop.clone();

    peer_tx.set_nodelay(true).ok();
    let handle = thread::spawn(move || loop {
        if stop_thread.load(Ordering::Relaxed) {
            let frames = played_frames.load(Ordering::Relaxed);
            let payload = audio_bridge_proto::encode_playback_pos(frames, true);
            let _ = audio_bridge_proto::write_frame(
                &mut peer_tx,
                audio_bridge_proto::FrameKind::PlaybackPos,
                &payload,
            );
            break;
        }

        let is_paused = paused
            .as_ref()
            .map(|p| p.load(Ordering::Relaxed))
            .unwrap_or(false);

        if is_paused {
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        let frames = played_frames.load(Ordering::Relaxed);
        let payload = audio_bridge_proto::encode_playback_pos(frames, false);
        if audio_bridge_proto::write_frame(
            &mut peer_tx,
            audio_bridge_proto::FrameKind::PlaybackPos,
            &payload,
        )
        .is_err()
        {
            break;
        }

        thread::sleep(Duration::from_millis(200));
    });

    ProgressReporter { stop, handle }
}
