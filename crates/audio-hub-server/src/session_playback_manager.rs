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
                None,
                false,
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
                seek_ms: None,
                start_paused: false,
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
                .bridge_play_path(state, session_id, target, path)
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
            let status = BridgeTransportClient::new(target.http_addr)
                .status()
                .await
                .map_err(|err| SessionPlaybackError::StatusFailed {
                    session_id: session_id.to_string(),
                    output_id: target.output_id.clone(),
                    reason: format!("status_failed {err:#}"),
                })?;
            let should_use_queue_fallback = status.now_playing.is_none()
                && (!status.paused || status.elapsed_ms.is_some() || status.duration_ms.is_some());
            let queue_now_playing = if should_use_queue_fallback {
                crate::session_registry::queue_snapshot(session_id)
                    .ok()
                    .and_then(|snapshot| snapshot.now_playing)
                    .map(|path| path.to_string_lossy().to_string())
            } else {
                None
            };
            let now_playing_path = status.now_playing.clone().or(queue_now_playing);
            let session_has_previous = crate::session_registry::queue_snapshot(session_id)
                .ok()
                .map(|snapshot| !snapshot.history.is_empty());
            let resolved_path = now_playing_path
                .as_deref()
                .and_then(parse_now_playing_path)
                .map(PathBuf::from);
            let (title, artist, album, format) = if let Some(path) = resolved_path.as_ref() {
                let lib = state.library.read().unwrap();
                match lib.find_track_by_path(&path) {
                    Some(crate::models::LibraryEntry::Track {
                        file_name,
                        artist,
                        album,
                        format,
                        ..
                    }) => {
                        let path_str = path.to_string_lossy();
                        let title = state
                            .metadata
                            .db
                            .track_record_by_path(path_str.as_ref())
                            .ok()
                            .flatten()
                            .and_then(|record| record.title)
                            .or_else(|| Some(file_name));
                        (title, artist, album, Some(format))
                    }
                    _ => (None, None, None, None),
                }
            } else {
                (None, None, None, None)
            };
            return Ok(crate::models::StatusResponse {
                now_playing: now_playing_path,
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
                output_id: Some(target.output_id),
                bitrate_kbps: None,
                underrun_frames: status.underrun_frames,
                underrun_events: status.underrun_events,
                buffer_size_frames: status.buffer_size_frames,
                buffered_frames: status.buffered_frames,
                buffer_capacity_frames: status.buffer_capacity_frames,
                has_previous: session_has_previous,
            });
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

    fn synthetic_status(
        &self,
        state: &AppState,
        session_id: &str,
        output_id: &str,
        source_error: Option<OutputControllerError>,
    ) -> crate::models::StatusResponse {
        let snapshot = crate::session_registry::queue_snapshot(session_id).ok();
        let now_path = snapshot.as_ref().and_then(|s| s.now_playing.clone());
        let has_previous = snapshot
            .as_ref()
            .map(|s| !s.history.is_empty())
            .unwrap_or(false);
        let (now_playing, title, artist, album, format, sample_rate, duration_ms) = match now_path {
            Some(path) => {
                let now = path.to_string_lossy().to_string();
                let lib = state.library.read().unwrap();
                match lib.find_track_by_path(&path) {
                    Some(crate::models::LibraryEntry::Track {
                        file_name,
                        sample_rate,
                        duration_ms,
                        artist,
                        album,
                        format,
                        ..
                    }) => {
                        let title = state
                            .metadata
                            .db
                            .track_record_by_path(&now)
                            .ok()
                            .flatten()
                            .and_then(|record| record.title)
                            .or_else(|| Some(file_name));
                        (
                            Some(now),
                            title,
                            artist,
                            album,
                            Some(format),
                            sample_rate,
                            duration_ms,
                        )
                    }
                    _ => (Some(now), None, None, None, None, None, None),
                }
            }
            None => (None, None, None, None, None, None, None),
        };
        let provider_online = source_error.is_none();
        crate::models::StatusResponse {
            now_playing,
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
        if status.now_playing.is_some() {
            return;
        }
        if status.paused && status.elapsed_ms.is_none() && status.duration_ms.is_none() {
            return;
        }
        let Some(path) = snapshot.now_playing else {
            return;
        };
        let now = path.to_string_lossy().to_string();
        status.now_playing = Some(now.clone());
        let lib = state.library.read().unwrap();
        if let Some(crate::models::LibraryEntry::Track {
            file_name,
            sample_rate,
            duration_ms,
            artist,
            album,
            format,
            ..
        }) = lib.find_track_by_path(&path)
        {
            if status.sample_rate.is_none() {
                status.sample_rate = sample_rate;
            }
            if status.output_sample_rate.is_none() {
                status.output_sample_rate = sample_rate;
            }
            if status.duration_ms.is_none() {
                status.duration_ms = duration_ms;
            }
            if status.artist.is_none() {
                status.artist = artist;
            }
            if status.album.is_none() {
                status.album = album;
            }
            if status.format.is_none() {
                status.format = Some(format);
            }
            if status.title.is_none() {
                status.title = state
                    .metadata
                    .db
                    .track_record_by_path(&now)
                    .ok()
                    .flatten()
                    .and_then(|record| record.title)
                    .or_else(|| Some(file_name));
            }
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
