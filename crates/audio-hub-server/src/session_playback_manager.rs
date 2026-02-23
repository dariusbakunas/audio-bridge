//! Session-scoped playback dispatch helpers.
//!
//! Ensures a session owns its selected output before dispatching playback.

use std::path::PathBuf;

use actix_web::HttpResponse;

use crate::models::QueueMode;
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

    pub async fn play_path(
        &self,
        state: &AppState,
        session_id: &str,
        path: PathBuf,
    ) -> Result<String, SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;

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
        state
            .output
            .controller
            .status_for_output(state, &output_id)
            .await
            .map_err(|err| SessionPlaybackError::StatusFailed {
                session_id: session_id.to_string(),
                output_id,
                reason: controller_error_reason(&err),
            })
    }

    pub async fn pause_toggle(
        &self,
        state: &AppState,
        session_id: &str,
    ) -> Result<(), SessionPlaybackError> {
        let output_id = self.bound_output_id(session_id)?;
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
}
