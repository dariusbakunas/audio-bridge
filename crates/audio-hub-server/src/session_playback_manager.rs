//! Session-scoped playback dispatch helpers.
//!
//! Ensures a session owns its selected output before dispatching playback.

use std::path::{Path, PathBuf};

use actix_web::HttpResponse;
use crossbeam_channel::Sender;

use crate::bridge::BridgeCommand;
use crate::bridge_manager::{merge_bridges, parse_output_id};
use crate::bridge_transport::BridgeTransportClient;
use crate::models::QueueMode;
use crate::output_providers::cast_provider::CastProvider;
use crate::output_controller::OutputControllerError;
use crate::session_registry::BoundOutputError;
use crate::state::AppState;

#[derive(Debug)]
pub enum SessionPlaybackError {
    SessionNotFound,
    NoOutputSelected { session_id: String },
    OutputLockMissing { session_id: String, output_id: String },
    OutputInUse {
        session_id: String,
        output_id: String,
        held_by_session_id: String,
    },
    SelectFailed {
        session_id: String,
        output_id: String,
        reason: String,
    },
    DispatchFailed {
        session_id: String,
        output_id: String,
        reason: String,
    },
    StatusFailed {
        session_id: String,
        output_id: String,
        reason: String,
    },
    CommandFailed {
        session_id: String,
        output_id: String,
        reason: String,
    },
}

impl SessionPlaybackError {
    pub fn into_response(self) -> HttpResponse {
        match self {
            SessionPlaybackError::SessionNotFound => HttpResponse::NotFound()
                .body("session not found"),
            SessionPlaybackError::NoOutputSelected { session_id } => HttpResponse::ServiceUnavailable()
                .body(format!("session has no selected output: {session_id}")),
            SessionPlaybackError::OutputLockMissing {
                session_id,
                output_id,
            } => HttpResponse::ServiceUnavailable().body(format!(
                "session output lock missing: session_id={session_id} output_id={output_id}"
            )),
            SessionPlaybackError::OutputInUse {
                session_id,
                output_id,
                held_by_session_id,
            } => HttpResponse::Conflict().body(format!(
                "session output in use: session_id={session_id} output_id={output_id} held_by={held_by_session_id}"
            )),
            SessionPlaybackError::SelectFailed {
                session_id,
                output_id,
                reason,
            } => HttpResponse::ServiceUnavailable().body(format!(
                "failed to select output for session: session_id={session_id} output_id={output_id} reason={reason}"
            )),
            SessionPlaybackError::DispatchFailed {
                session_id,
                output_id,
                reason,
            } => HttpResponse::ServiceUnavailable().body(format!(
                "failed to dispatch playback for session: session_id={session_id} output_id={output_id} reason={reason}"
            )),
            SessionPlaybackError::StatusFailed {
                session_id,
                output_id,
                reason,
            } => HttpResponse::ServiceUnavailable().body(format!(
                "failed to fetch status for session: session_id={session_id} output_id={output_id} reason={reason}"
            )),
            SessionPlaybackError::CommandFailed {
                session_id,
                output_id,
                reason,
            } => HttpResponse::ServiceUnavailable().body(format!(
                "failed to execute session command: session_id={session_id} output_id={output_id} reason={reason}"
            )),
        }
    }
}

fn controller_error_reason(err: &OutputControllerError) -> String {
    match err {
        OutputControllerError::NoActiveOutput => "no_active_output".to_string(),
        OutputControllerError::UnsupportedOutput { requested, active } => {
            format!("unsupported_output requested={requested} active={active}")
        }
        OutputControllerError::OutputOffline { output_id } => {
            format!("output_offline output_id={output_id}")
        }
        OutputControllerError::PlayerOffline => "player_offline".to_string(),
        OutputControllerError::Http(_) => "http_error".to_string(),
    }
}

pub struct SessionPlaybackManager;

struct BridgeTarget {
    output_id: String,
    device_id: String,
    http_addr: std::net::SocketAddr,
}

impl SessionPlaybackManager {
    pub fn new() -> Self {
        Self
    }

    fn bound_output_id(&self, session_id: &str) -> Result<String, SessionPlaybackError> {
        match crate::session_registry::require_bound_output(session_id) {
            Ok(output_id) => Ok(output_id),
            Err(BoundOutputError::SessionNotFound) => Err(SessionPlaybackError::SessionNotFound),
            Err(BoundOutputError::NoOutputSelected) => Err(SessionPlaybackError::NoOutputSelected {
                session_id: session_id.to_string(),
            }),
            Err(BoundOutputError::OutputLockMissing { output_id }) => {
                Err(SessionPlaybackError::OutputLockMissing {
                    session_id: session_id.to_string(),
                    output_id,
                })
            }
            Err(BoundOutputError::OutputInUse {
                output_id,
                held_by_session_id,
            }) => Err(SessionPlaybackError::OutputInUse {
                session_id: session_id.to_string(),
                output_id,
                held_by_session_id,
            }),
        }
    }

    fn bridge_target(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Option<BridgeTarget> {
        let (bridge_id, device_id) = parse_output_id(output_id).ok()?;
        let http_addr = {
            let bridges_state = state.providers.bridge.bridges.lock().ok()?;
            let discovered = state.providers.bridge.discovered_bridges.lock().ok()?;
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            merged.iter().find(|b| b.id == bridge_id).map(|b| b.http_addr)?
        };
        Some(BridgeTarget {
            output_id: output_id.to_string(),
            device_id,
            http_addr,
        })
    }

    fn cast_worker(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Option<Sender<BridgeCommand>> {
        if !output_id.starts_with("cast:") {
            return None;
        }
        CastProvider::ensure_worker_for_output(state, output_id).ok()
    }

    async fn bridge_play_path(
        &self,
        state: &AppState,
        session_id: &str,
        target: BridgeTarget,
        path: PathBuf,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<String, SessionPlaybackError> {
        let client = BridgeTransportClient::new_with_base(
            target.http_addr,
            state.providers.bridge.public_base_url.clone(),
            Some(state.metadata.db.clone()),
        );
        let devices = client
            .list_devices()
            .await
            .map_err(|err| SessionPlaybackError::SelectFailed {
                session_id: session_id.to_string(),
                output_id: target.output_id.clone(),
                reason: format!("list_devices_failed {err:#}"),
            })?;
        let Some(device_name) = devices
            .iter()
            .find(|d| d.id == target.device_id)
            .map(|d| d.name.clone())
        else {
            return Err(SessionPlaybackError::SelectFailed {
                session_id: session_id.to_string(),
                output_id: target.output_id,
                reason: "unknown_device".to_string(),
            });
        };
        client
            .set_device(&device_name, None)
            .await
            .map_err(|err| SessionPlaybackError::SelectFailed {
                session_id: session_id.to_string(),
                output_id: target.output_id.clone(),
                reason: format!("set_device_failed {err:#}"),
            })?;

        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let title = Some(path.to_string_lossy().to_string());
        client
            .play_path(
                &path,
                if ext_hint.is_empty() {
                    None
                } else {
                    Some(ext_hint.as_str())
                },
                title.as_deref(),
                seek_ms,
                start_paused,
            )
            .await
            .map_err(|err| SessionPlaybackError::DispatchFailed {
                session_id: session_id.to_string(),
                output_id: target.output_id.clone(),
                reason: format!("play_failed {err:#}"),
            })?;
        state.events.status_changed();
        Ok(target.output_id)
    }

    pub async fn play_path(
        &self,
        state: &AppState,
        session_id: &str,
        path: PathBuf,
    ) -> Result<String, SessionPlaybackError> {
        self
            .play_path_with_options(state, session_id, path, None, false)
            .await
    }

    pub async fn play_path_with_options(
        &self,
        state: &AppState,
        session_id: &str,
        path: PathBuf,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<String, SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        if let Some(tx) = self.cast_worker(state, &output_id) {
            let ext_hint = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            tx.send(BridgeCommand::Play {
                path,
                ext_hint,
                seek_ms,
                start_paused,
            })
            .map_err(|err| SessionPlaybackError::DispatchFailed {
                session_id: session_id.to_string(),
                output_id: output_id.clone(),
                reason: format!("cast_send_failed {err}"),
            })?;
            state.events.status_changed();
            return Ok(output_id);
        }
        if let Some(target) = self.bridge_target(state, &output_id) {
            return self
                .bridge_play_path(state, session_id, target, path, seek_ms, start_paused)
                .await;
        }

        if let Err(err) = state.output.controller.select_output(state, &output_id).await {
            return Err(SessionPlaybackError::SelectFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            });
        }

        match state
            .output
            .controller
            .play_request(state, path, QueueMode::Keep, Some(output_id.as_str()))
            .await
        {
            Ok(active_output_id) => Ok(active_output_id),
            Err(err) => Err(SessionPlaybackError::DispatchFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            }),
        }
    }

    pub async fn status(
        &self,
        state: &AppState,
        session_id: &str,
    ) -> Result<crate::models::StatusResponse, SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        if let Some(target) = self.bridge_target(state, &output_id) {
            let bridge_id = parse_output_id(&target.output_id)
                .ok()
                .map(|(bridge_id, _)| bridge_id);
            let cached_status = bridge_id.as_ref().and_then(|bridge_id| {
                state
                    .providers
                    .bridge
                    .status_cache
                    .lock()
                    .ok()
                    .and_then(|cache| cache.get(bridge_id).cloned())
            });
            let live_status = BridgeTransportClient::new(target.http_addr).status().await.ok();
            let status = if let Some(fetched) = live_status {
                if let Some(bridge_id) = bridge_id.as_ref() {
                    if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                        cache.insert(bridge_id.clone(), fetched.clone());
                    }
                }
                fetched
            } else if let Some(cached) = cached_status {
                cached
            } else {
                return Err(SessionPlaybackError::StatusFailed {
                    session_id: session_id.to_string(),
                    output_id: target.output_id.clone(),
                    reason: "status_failed bridge status unavailable".to_string(),
                });
            };
            return Ok(self.build_bridge_status_response(
                state,
                session_id,
                target.output_id,
                status,
            ));
        }
        match state
            .output
            .controller
            .status_for_output(state, &output_id)
            .await
        {
            Ok(mut status) => {
                self.apply_session_now_playing_fallback(state, session_id, &mut status);
                Ok(status)
            }
            Err(err) => Ok(self.synthetic_status(state, session_id, &output_id, Some(err))),
        }
    }

    fn build_bridge_status_response(
        &self,
        state: &AppState,
        session_id: &str,
        output_id: String,
        status: audio_bridge_types::BridgeStatus,
    ) -> crate::models::StatusResponse {
        let should_use_queue_fallback = status.now_playing.is_none()
            && (!status.paused || status.elapsed_ms.is_some() || status.duration_ms.is_some());
        let queue_now_playing_track_id = if should_use_queue_fallback {
            crate::session_registry::queue_snapshot(session_id)
                .ok()
                .and_then(|snapshot| snapshot.now_playing)
        } else {
            None
        };
        let now_playing_track_id_from_status = status
            .now_playing
            .as_deref()
            .and_then(parse_now_playing_track_id);
        let session_has_previous = crate::session_registry::queue_snapshot(session_id)
            .ok()
            .map(|snapshot| !snapshot.history.is_empty());
        let resolved_path = status
            .now_playing
            .as_deref()
            .and_then(parse_now_playing_path)
            .map(PathBuf::from);
        let now_playing_track_id = now_playing_track_id_from_status
            .or(queue_now_playing_track_id)
            .or_else(|| {
                resolved_path.as_ref().and_then(|path| {
                    state
                        .metadata
                        .db
                        .track_id_for_path(&path.to_string_lossy())
                        .ok()
                        .flatten()
                })
            });
        let (title, artist, album, format) = now_playing_track_id
            .and_then(|track_id| state.metadata.db.track_record_by_id(track_id).ok().flatten())
            .map(|record| {
                (
                    record.title.or(Some(record.file_name)),
                    record.artist,
                    record.album,
                    record.format,
                )
            })
            .unwrap_or((None, None, None, None));
        crate::models::StatusResponse {
            now_playing_track_id,
            paused: status.paused,
            bridge_online: true,
            elapsed_ms: status.elapsed_ms,
            duration_ms: status.duration_ms,
            source_codec: status.source_codec,
            source_bit_depth: status.source_bit_depth,
            container: status.container,
            output_sample_format: status.output_sample_format,
            resampling: status.resampling,
            resample_from_hz: status.resample_from_hz,
            resample_to_hz: status.resample_to_hz,
            sample_rate: status.sample_rate,
            channels: status.channels,
            output_sample_rate: status.sample_rate,
            output_device: status.device,
            title,
            artist,
            album,
            format,
            output_id: Some(output_id),
            bitrate_kbps: None,
            underrun_frames: status.underrun_frames,
            underrun_events: status.underrun_events,
            buffer_size_frames: status.buffer_size_frames,
            buffered_frames: status.buffered_frames,
            buffer_capacity_frames: status.buffer_capacity_frames,
            has_previous: session_has_previous,
        }
    }

    pub async fn pause_toggle(
        &self,
        state: &AppState,
        session_id: &str,
    ) -> Result<(), SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        if let Some(tx) = self.cast_worker(state, &output_id) {
            tx.send(BridgeCommand::PauseToggle)
                .map_err(|err| SessionPlaybackError::CommandFailed {
                    session_id: session_id.to_string(),
                    output_id: output_id.clone(),
                    reason: format!("cast_send_failed {err}"),
                })?;
            state.events.status_changed();
            return Ok(());
        }
        if let Some(target) = self.bridge_target(state, &output_id) {
            BridgeTransportClient::new(target.http_addr)
                .pause_toggle()
                .await
                .map_err(|err| SessionPlaybackError::CommandFailed {
                    session_id: session_id.to_string(),
                    output_id: target.output_id,
                    reason: format!("pause_failed {err:#}"),
                })?;
            state.events.status_changed();
            return Ok(());
        }
        if let Err(err) = state.output.controller.select_output(state, &output_id).await {
            return Err(SessionPlaybackError::SelectFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            });
        }
        state
            .output
            .controller
            .pause_toggle(state)
            .await
            .map_err(|err| SessionPlaybackError::CommandFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            })
    }

    pub async fn seek(
        &self,
        state: &AppState,
        session_id: &str,
        ms: u64,
    ) -> Result<(), SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        if let Some(tx) = self.cast_worker(state, &output_id) {
            tx.send(BridgeCommand::Seek { ms })
                .map_err(|err| SessionPlaybackError::CommandFailed {
                    session_id: session_id.to_string(),
                    output_id: output_id.clone(),
                    reason: format!("cast_send_failed {err}"),
                })?;
            state.events.status_changed();
            return Ok(());
        }
        if let Some(target) = self.bridge_target(state, &output_id) {
            BridgeTransportClient::new(target.http_addr)
                .seek(ms)
                .await
                .map_err(|err| SessionPlaybackError::CommandFailed {
                    session_id: session_id.to_string(),
                    output_id: target.output_id,
                    reason: format!("seek_failed {err:#}"),
                })?;
            state.events.status_changed();
            return Ok(());
        }
        if let Err(err) = state.output.controller.select_output(state, &output_id).await {
            return Err(SessionPlaybackError::SelectFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            });
        }
        state
            .output
            .controller
            .seek(state, ms)
            .await
            .map_err(|err| SessionPlaybackError::CommandFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            })
    }

    pub async fn stop(
        &self,
        state: &AppState,
        session_id: &str,
    ) -> Result<(), SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        if let Some(tx) = self.cast_worker(state, &output_id) {
            tx.send(BridgeCommand::Stop)
                .map_err(|err| SessionPlaybackError::CommandFailed {
                    session_id: session_id.to_string(),
                    output_id: output_id.clone(),
                    reason: format!("cast_send_failed {err}"),
                })?;
            state.events.status_changed();
            return Ok(());
        }
        if let Some(target) = self.bridge_target(state, &output_id) {
            BridgeTransportClient::new(target.http_addr)
                .stop()
                .await
                .map_err(|err| SessionPlaybackError::CommandFailed {
                    session_id: session_id.to_string(),
                    output_id: target.output_id,
                    reason: format!("stop_failed {err:#}"),
                })?;
            state.events.status_changed();
            return Ok(());
        }
        if let Err(err) = state.output.controller.select_output(state, &output_id).await {
            return Err(SessionPlaybackError::SelectFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            });
        }
        state
            .output
            .controller
            .stop(state)
            .await
            .map_err(|err| SessionPlaybackError::CommandFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            })
    }

    pub async fn volume(
        &self,
        state: &AppState,
        session_id: &str,
    ) -> Result<crate::models::SessionVolumeResponse, SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        state
            .output
            .controller
            .volume_for_output(state, &output_id)
            .await
            .map_err(|err| SessionPlaybackError::CommandFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            })
    }

    pub async fn set_volume(
        &self,
        state: &AppState,
        session_id: &str,
        value: u8,
    ) -> Result<crate::models::SessionVolumeResponse, SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        state
            .output
            .controller
            .set_volume_for_output(state, &output_id, value)
            .await
            .map_err(|err| SessionPlaybackError::CommandFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            })
    }

    pub async fn set_mute(
        &self,
        state: &AppState,
        session_id: &str,
        muted: bool,
    ) -> Result<crate::models::SessionVolumeResponse, SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
        state
            .output
            .controller
            .set_mute_for_output(state, &output_id, muted)
            .await
            .map_err(|err| SessionPlaybackError::CommandFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            })
    }

    fn synthetic_status(
        &self,
        state: &AppState,
        session_id: &str,
        output_id: &str,
        source_error: Option<OutputControllerError>,
    ) -> crate::models::StatusResponse {
        let snapshot = crate::session_registry::queue_snapshot(session_id).ok();
        let now_track_id = snapshot.as_ref().and_then(|s| s.now_playing);
        let has_previous = snapshot
            .as_ref()
            .map(|s| !s.history.is_empty())
            .unwrap_or(false);
        let (now_playing_track_id, title, artist, album, format, sample_rate, duration_ms) =
            match now_track_id {
                Some(track_id) => match state.metadata.db.track_record_by_id(track_id).ok().flatten() {
                    Some(record) => (
                        Some(track_id),
                        record.title.or(Some(record.file_name)),
                        record.artist,
                        record.album,
                        record.format,
                        record.sample_rate,
                        record.duration_ms,
                    ),
                    None => (Some(track_id), None, None, None, None, None, None),
                },
                None => (None, None, None, None, None, None, None),
            };
        let provider_online = source_error.is_none();
        crate::models::StatusResponse {
            now_playing_track_id,
            paused: false,
            bridge_online: provider_online,
            elapsed_ms: None,
            duration_ms,
            source_codec: None,
            source_bit_depth: None,
            container: None,
            output_sample_format: None,
            resampling: None,
            resample_from_hz: None,
            resample_to_hz: None,
            sample_rate,
            channels: None,
            output_sample_rate: sample_rate,
            output_device: None,
            title,
            artist,
            album,
            format,
            output_id: Some(output_id.to_string()),
            bitrate_kbps: None,
            underrun_frames: None,
            underrun_events: None,
            buffer_size_frames: None,
            buffered_frames: None,
            buffer_capacity_frames: None,
            has_previous: Some(has_previous),
        }
    }

    fn apply_session_now_playing_fallback(
        &self,
        state: &AppState,
        session_id: &str,
        status: &mut crate::models::StatusResponse,
    ) {
        let Some(snapshot) = crate::session_registry::queue_snapshot(session_id).ok() else {
            return;
        };
        if status.has_previous.is_none() {
            status.has_previous = Some(!snapshot.history.is_empty());
        }
        if status.now_playing_track_id.is_some() {
            return;
        }
        if status.paused && status.elapsed_ms.is_none() && status.duration_ms.is_none() {
            return;
        }
        let Some(track_id) = snapshot.now_playing else {
            return;
        };
        status.now_playing_track_id = Some(track_id);
        let Some(record) = state.metadata.db.track_record_by_id(track_id).ok().flatten() else {
            return;
        };
        if status.sample_rate.is_none() {
            status.sample_rate = record.sample_rate;
        }
        if status.output_sample_rate.is_none() {
            status.output_sample_rate = record.sample_rate;
        }
        if status.duration_ms.is_none() {
            status.duration_ms = record.duration_ms;
        }
        if status.artist.is_none() {
            status.artist = record.artist;
        }
        if status.album.is_none() {
            status.album = record.album;
        }
        if status.format.is_none() {
            status.format = record.format;
        }
        if status.title.is_none() {
            status.title = record.title.or(Some(record.file_name));
        }
    }
}

fn parse_now_playing_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if Path::new(trimmed).is_absolute() {
        return Some(trimmed.to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let url = reqwest::Url::parse(trimmed).ok()?;
        return url
            .query_pairs()
            .find_map(|(k, v)| (k == "path").then_some(v.into_owned()))
            .and_then(|decoded| {
                let candidate = decoded.trim().to_string();
                Path::new(&candidate).is_absolute().then_some(candidate)
            });
    }
    None
}

fn parse_now_playing_track_id(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(id) = trimmed.parse::<i64>() {
        return Some(id);
    }
    let url = reqwest::Url::parse(trimmed).ok()?;
    let mut segments = url.path_segments()?;
    let mut prev = None::<String>;
    for segment in &mut segments {
        if prev.as_deref() == Some("track") {
            if let Ok(id) = segment.parse::<i64>() {
                return Some(id);
            }
        }
        prev = Some(segment.to_string());
    }
    None
}
