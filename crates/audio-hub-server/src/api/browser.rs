//! Browser websocket API for playback control.

use actix::prelude::*;
use actix_web::{get, web, Error, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use serde::{Deserialize, Serialize};

use crate::browser::BrowserOutbound;
use crate::state::AppState;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserClientMessage {
    Hello { name: Option<String> },
    Status {
        paused: bool,
        elapsed_ms: Option<u64>,
        duration_ms: Option<u64>,
        now_playing: Option<String>,
    },
    Ended,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserServerMessage {
    Hello { session_id: String },
}

pub struct BrowserWs {
    session_id: Option<String>,
    state: web::Data<AppState>,
    base_url: Option<String>,
}

impl BrowserWs {
    pub fn new(state: web::Data<AppState>, base_url: Option<String>) -> Self {
        Self {
            session_id: None,
            state,
            base_url,
        }
    }

    fn is_active(&self, output_id: &str) -> bool {
        self.state
            .providers
            .bridge
            .bridges
            .lock()
            .ok()
            .and_then(|s| s.active_output_id.clone())
            .as_deref()
            == Some(output_id)
    }

    fn handle_status(
        &self,
        paused: bool,
        elapsed_ms: Option<u64>,
        duration_ms: Option<u64>,
        now_playing: Option<String>,
    ) {
        let Some(session_id) = self.session_id.as_deref() else { return };
        let output_id = format!("browser:{session_id}");
        if !self.is_active(&output_id) {
            return;
        }

        if now_playing.is_none() && elapsed_ms.is_none() && duration_ms.is_none() {
            self.state.playback.manager.status().on_stop();
            self.state.providers.browser.update_last_duration(session_id, None);
            self.state.playback.manager.update_has_previous();
            return;
        }

        let mut status = audio_bridge_types::BridgeStatus::default();
        status.now_playing = now_playing;
        status.paused = paused;
        status.elapsed_ms = elapsed_ms;
        status.duration_ms = duration_ms;

        let last_duration = self.state.providers.browser.get_last_duration(session_id);
        let (inputs, changed) = self
            .state
            .playback
            .manager
            .status()
            .reduce_remote_and_inputs(&status, last_duration);
        self.state.playback.manager.status().emit_if_changed(changed);
        self.state.providers.browser.update_last_duration(session_id, status.duration_ms);

        let transport = crate::playback_transport::ChannelTransport::new(
            self.state.providers.bridge.player.lock().unwrap().cmd_tx.clone(),
        );
        let _ = self
            .state
            .playback
            .manager
            .queue_service()
            .maybe_auto_advance(&transport, inputs);
        self.state.playback.manager.update_has_previous();
    }

    fn handle_ended(&self) {
        let Some(session_id) = self.session_id.as_deref() else { return };
        let output_id = format!("browser:{session_id}");
        if !self.is_active(&output_id) {
            return;
        }
        let mut status = audio_bridge_types::BridgeStatus::default();
        status.paused = false;
        status.now_playing = None;
        status.elapsed_ms = None;
        status.duration_ms = None;

        let last_duration = self.state.providers.browser.get_last_duration(session_id);
        let (inputs, changed) = self
            .state
            .playback
            .manager
            .status()
            .reduce_remote_and_inputs(&status, last_duration);
        self.state.playback.manager.status().emit_if_changed(changed);
        self.state.providers.browser.update_last_duration(session_id, status.duration_ms);

        let transport = crate::playback_transport::ChannelTransport::new(
            self.state.providers.bridge.player.lock().unwrap().cmd_tx.clone(),
        );
        let _ = self
            .state
            .playback
            .manager
            .queue_service()
            .maybe_auto_advance(&transport, inputs);
        self.state.playback.manager.update_has_previous();
    }
}

impl Actor for BrowserWs {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let addr = ctx.address().recipient::<BrowserOutbound>();
        let session_id = self.state.providers.browser.register_session(
            "Browser".to_string(),
            addr,
            self.base_url.clone(),
        );
        self.session_id = Some(session_id.clone());
        self.state.events.outputs_changed();
        let msg = BrowserServerMessage::Hello { session_id };
        if let Ok(text) = serde_json::to_string(&msg) {
            ctx.text(text);
        }
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        if let Some(session_id) = self.session_id.take() {
            let _ = self.state.providers.browser.remove_session(&session_id);
            self.state.events.outputs_changed();
        }
    }
}

impl Handler<BrowserOutbound> for BrowserWs {
    type Result = ();

    fn handle(&mut self, msg: BrowserOutbound, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(msg.0);
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for BrowserWs {
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        let msg = match item {
            Ok(msg) => msg,
            Err(_) => {
                ctx.stop();
                return;
            }
        };
        match msg {
            ws::Message::Text(text) => {
                if let Ok(payload) = serde_json::from_str::<BrowserClientMessage>(&text) {
                    match payload {
                        BrowserClientMessage::Hello { name } => {
                            if let (Some(id), Some(name)) = (self.session_id.as_deref(), name) {
                                self.state.providers.browser.update_name(id, name);
                                self.state.events.outputs_changed();
                            }
                        }
                        BrowserClientMessage::Status {
                            paused,
                            elapsed_ms,
                            duration_ms,
                            now_playing,
                        } => {
                            self.handle_status(paused, elapsed_ms, duration_ms, now_playing);
                        }
                        BrowserClientMessage::Ended => {
                            self.handle_ended();
                        }
                    }
                }
            }
            ws::Message::Ping(bytes) => ctx.pong(&bytes),
            ws::Message::Pong(_) => {}
            ws::Message::Close(_) => ctx.stop(),
            ws::Message::Binary(_) => {}
            ws::Message::Continuation(_) => ctx.stop(),
            ws::Message::Nop => {}
        }
    }
}

#[get("/browser/ws")]
pub async fn browser_ws(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let conn = req.connection_info();
    let scheme = conn.scheme();
    if !matches!(scheme, "http" | "https") {
        return Err(actix_web::error::ErrorBadRequest("invalid scheme"));
    }
    let host = conn.host();
    if host.len() > 255 {
        return Err(actix_web::error::ErrorBadRequest("host too long"));
    }
    let base_url = Some(format!("{}://{}", scheme, host));
    ws::start(BrowserWs::new(state, base_url), &req, stream)
}
