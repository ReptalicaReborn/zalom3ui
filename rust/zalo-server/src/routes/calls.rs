//! Track 1 (`feat/rust-calls`): 1:1 incoming/outgoing call state machine, ControlPoller +
//! VoIP WebSocket control ingestion, call REST endpoints, call debug log, and `/ws/call-audio`.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
#[cfg(test)]
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::debug;
use url::Url;

use zalo_media::call::{parse_server, MediaCall, MediaCallOptions, MediaEvent};
use zalo_proto::{
    incoming_call_of, media_params_of, new_call_id, outgoing_call_of, outgoing_media_of,
    parse_json_keeping_bigints, ringing_call_of, server_list, ControlPollEvent, ControlPoller,
    ControlPollerHandle, OfferedServer, OutgoingMedia, RingingCall, VoiceCallApi, ZaloClient,
    CALL_ENDING_ACTS, RINGING_MAX_AGE_MS,
};

use crate::errors::{ApiResult, HttpError};
use crate::routes::AppState;
use crate::session::ZaloSession;
use crate::types::{
    CallEvent, CallState, CurrentCall, IncomingCall as ServerIncomingCall, SessionState, ThreadKind,
};

const PEER_ACTS: &[&str] = &["onoff_camera", "mute_audio"];
const HANDLED_CONTROL_TTL_MS: i64 = 10 * 60_000;
const INCOMING_RING_TIMEOUT_MS: u64 = 45_000;
const OUTGOING_SETUP_TIMEOUT_MS: u64 = 20_000;
const OUTGOING_RING_TIMEOUT_MS: u64 = 60_000;
const ANSWER_TEST_SECONDS: u64 = 30;
const WS_HEARTBEAT_SECS: u64 = 30;
const MAX_AUDIO_FROM_APP_BYTES: usize = 1276;
const MAX_VIDEO_FRAME_BYTES: usize = 512 * 1024;
const AUDIO_WS_MAX_PAYLOAD: usize = MAX_VIDEO_FRAME_BYTES + 1024;
const AUDIO_WS_QUEUE_CAP: usize = 256;

/// Server -> app: peer requested a video key frame (`0x05`).
pub const KEY_FRAME_REQUEST: &[u8] = &[0x05];

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Server -> app audio frame: `0x01 | u16BE(seq) | u32BE(rtp_ts) | opus`.
pub fn encode_audio_to_app(seq: u16, timestamp: u32, opus: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(7 + opus.len());
    out.push(0x01);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&timestamp.to_be_bytes());
    out.extend_from_slice(opus);
    out
}

/// App -> server audio frame: `0x02 | opus` (`2..=1276` total bytes).
pub fn decode_audio_from_app(data: &[u8]) -> Option<&[u8]> {
    if data.len() >= 2 && data.len() <= MAX_AUDIO_FROM_APP_BYTES && data[0] == 0x02 {
        Some(&data[1..])
    } else {
        None
    }
}

/// Server -> app video frame: `0x03 | u8(flags: bit0=key) | u32BE(90kHz rtp_ts) | Annex-B H.264`.
pub fn encode_video_to_app(key: bool, timestamp: u32, annex_b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + annex_b.len());
    out.push(0x03);
    out.push(u8::from(key));
    out.extend_from_slice(&timestamp.to_be_bytes());
    out.extend_from_slice(annex_b);
    out
}

/// App -> server video frame: `0x04 | u8(flags: bit0=key) | Annex-B H.264` (`3..=524290` total bytes).
pub fn decode_video_from_app(data: &[u8]) -> Option<(bool, &[u8])> {
    if data.len() >= 3 && data.len() <= MAX_VIDEO_FRAME_BYTES + 2 && data[0] == 0x04 {
        Some(((data[1] & 1) != 0, &data[2..]))
    } else {
        None
    }
}

/// App -> server key-frame request: `0x06`.
pub fn is_key_frame_wanted(data: &[u8]) -> bool {
    !data.is_empty() && data[0] == 0x06
}

// ---- VoiceCall transport abstraction (enables unit tests without hitting live Zalo HTTPS) ----

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait VoiceCallTransport: Send + Sync {
    fn ring_ring<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>>;
    fn decline_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>>;
    fn accept_call<'a>(
        &'a self,
        call: &'a RingingCall,
        server: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>>;
    fn cam_mic_state<'a>(
        &'a self,
        call: &'a RingingCall,
        cam_on: Option<bool>,
        muted: Option<bool>,
    ) -> BoxFuture<'a, Result<Value, String>>;
    fn hang_up<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>>;
    fn request_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>>;
    fn send_call_request<'a>(
        &'a self,
        call: &'a RingingCall,
        media: &'a OutgoingMedia,
        server: &'a OfferedServer,
    ) -> BoxFuture<'a, Result<Value, String>>;
    fn cancel_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>>;
    fn answer_ack<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>>;
}

struct ProtoVoiceCallTransport {
    api: VoiceCallApi,
}

impl ProtoVoiceCallTransport {
    fn from_client(client: &Arc<ZaloClient>) -> Arc<dyn VoiceCallTransport> {
        Arc::new(Self {
            api: VoiceCallApi::new((**client).clone()),
        })
    }
}

impl VoiceCallTransport for ProtoVoiceCallTransport {
    fn ring_ring<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.api.ring_ring(call).await.map_err(|e| e.to_string()) })
    }

    fn decline_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.api.decline_call(call).await.map_err(|e| e.to_string()) })
    }

    fn accept_call<'a>(
        &'a self,
        call: &'a RingingCall,
        server: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.api
                .accept_call(call, server)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn cam_mic_state<'a>(
        &'a self,
        call: &'a RingingCall,
        cam_on: Option<bool>,
        muted: Option<bool>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.api
                .cam_mic_state(call, cam_on, muted)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn hang_up<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.api.hang_up(call).await.map_err(|e| e.to_string()) })
    }

    fn request_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.api.request_call(call).await.map_err(|e| e.to_string()) })
    }

    fn send_call_request<'a>(
        &'a self,
        call: &'a RingingCall,
        media: &'a OutgoingMedia,
        server: &'a OfferedServer,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.api
                .send_call_request(call, media, server)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn cancel_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.api.cancel_call(call).await.map_err(|e| e.to_string()) })
    }

    fn answer_ack<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.api.answer_ack(call).await.map_err(|e| e.to_string()) })
    }
}

// ---- CallManager state machine ----

/// One message from the server to the app's call-audio client, whatever the transport.
#[derive(Debug, Clone)]
pub enum AudioWsOut {
    /// A binary frame (`0x01` audio, `0x03` video, `0x05` key-frame request).
    Binary(Vec<u8>),
    /// The link is over: close code and reason (`1000` "replaced" / "call ended").
    Close(u16, &'static str),
}

/// Why no audio link could be opened: the WebSocket handler closes with this code and reason.
pub const NO_ACTIVE_CALL_CLOSE: (u16, &str) = (1008, "No active call");

/// Transport-agnostic call-audio link: frames in ([`AudioLink::handle_incoming`]), frames out
/// ([`AudioLink::recv`]), close-with-reason (an [`AudioWsOut::Close`] from `recv`). Used by the
/// `/ws/call-audio` WebSocket handler and by the in-process embedded handle. Only one link is
/// registered at a time: opening another closes the older one with `1000 "replaced"`. Frames to
/// the client are dropped when it lags (bounded queue). Unregisters itself on drop.
pub struct AudioLink {
    manager: Arc<CallManager>,
    id: u64,
    rx: tokio::sync::Mutex<mpsc::Receiver<AudioWsOut>>,
}

impl AudioLink {
    /// The next message for the client; `None` when the link was unregistered without a close.
    pub async fn recv(&self) -> Option<AudioWsOut> {
        self.rx.lock().await.recv().await
    }

    /// Handles one binary frame from the app: `0x02` audio, `0x04` video, `0x06` key-frame request.
    /// Anything else is ignored.
    pub fn handle_incoming(&self, data: &[u8]) {
        if let Some(opus) = decode_audio_from_app(data) {
            self.manager.send_call_audio(opus.to_vec());
        } else if let Some((key, annex_b)) = decode_video_from_app(data) {
            self.manager.send_call_video(key, annex_b.to_vec());
        } else if is_key_frame_wanted(data) {
            self.manager.request_call_key_frame();
        }
    }
}

impl Drop for AudioLink {
    fn drop(&mut self) {
        self.manager.unregister_audio_ws(self.id);
    }
}

struct AudioWsClient {
    id: u64,
    tx: mpsc::Sender<AudioWsOut>,
}

struct RingingEntry {
    call: RingingCall,
    timeout_task: Option<JoinHandle<()>>,
}

struct ActiveCall {
    id: u64,
    info: CurrentCall,
    call: RingingCall,
    transport: Arc<dyn VoiceCallTransport>,
    media: Option<MediaCall>,
    outgoing: bool,
    started_at: i64,
    accepted: bool,
    requested: bool,
    done: bool,
    want_hang_up: bool,
    cam_on: bool,
    peer_camera_on: bool,
    peer_muted: bool,
    peer_heard: HashSet<String>,
    server_address: Option<String>,
    ring_timeout_ms: u64,
    timer_task: Option<JoinHandle<()>>,
    media_task: Option<JoinHandle<()>>,
}

struct CallStateInner {
    ringing: HashMap<String, RingingEntry>,
    active: Option<ActiveCall>,
    handled_controls: HashMap<String, i64>,
    audio_ws: Option<AudioWsClient>,
    ctrl_poll_client: Option<Weak<ZaloClient>>,
    ctrl_poll_handle: Option<ControlPollerHandle>,
    ctrl_poll_task: Option<JoinHandle<()>>,
}

impl Default for CallStateInner {
    fn default() -> Self {
        Self {
            ringing: HashMap::new(),
            active: None,
            handled_controls: HashMap::new(),
            audio_ws: None,
            ctrl_poll_client: None,
            ctrl_poll_handle: None,
            ctrl_poll_task: None,
        }
    }
}

/// Manages 1:1 incoming and outgoing calls, ControlPoller ingestion, and `/ws/call-audio`.
pub struct CallManager {
    session: Arc<ZaloSession>,
    inner: Mutex<CallStateInner>,
    next_id: AtomicU64,
    ring_timeout_ms: u64,
    #[cfg(test)]
    test_transport: Mutex<Option<Arc<dyn VoiceCallTransport>>>,
}

impl Drop for CallManager {
    fn drop(&mut self) {
        if let Ok(mut s) = self.inner.lock() {
            for (_, mut entry) in s.ringing.drain() {
                if let Some(t) = entry.timeout_task.take() {
                    t.abort();
                }
            }
            if let Some(mut active) = s.active.take() {
                if let Some(t) = active.timer_task.take() {
                    t.abort();
                }
                if let Some(t) = active.media_task.take() {
                    t.abort();
                }
                if let Some(m) = active.media.take() {
                    m.close(false);
                }
            }
            if let Some(mut h) = s.ctrl_poll_handle.take() {
                h.stop();
            }
            if let Some(t) = s.ctrl_poll_task.take() {
                t.abort();
            }
        }
    }
}

impl CallManager {
    pub fn new(session: Arc<ZaloSession>) -> Arc<Self> {
        Arc::new(Self {
            session,
            inner: Mutex::new(CallStateInner::default()),
            next_id: AtomicU64::new(1),
            ring_timeout_ms: INCOMING_RING_TIMEOUT_MS,
            #[cfg(test)]
            test_transport: Mutex::new(None),
        })
    }

    fn spawn_background(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let session = Arc::clone(&self.session);
        let mut voip_rx = session.subscribe_voip_controls();
        let mut events_rx = session.subscribe_events();

        // Start ControlPoller immediately if already logged in.
        self.sync_control_poller();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    ev = voip_rx.recv() => {
                        let Some(mgr) = weak.upgrade() else { break };
                        match ev {
                            Ok(ctrl) => {
                                mgr.on_voip_control(&ctrl, "socket", now_ms(), false);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    ev = events_rx.recv() => {
                        let Some(mgr) = weak.upgrade() else { break };
                        match ev {
                            Ok(payload) => {
                                if payload.get("type").and_then(|v| v.as_str()) == Some("status") {
                                    let state_str = payload
                                        .get("status")
                                        .and_then(|s| s.get("state"))
                                        .and_then(|v| v.as_str());
                                    if state_str == Some("logged_out") {
                                        mgr.on_logout();
                                    } else {
                                        mgr.sync_control_poller();
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });
    }

    fn transport(&self) -> Result<Arc<dyn VoiceCallTransport>, HttpError> {
        #[cfg(test)]
        if let Ok(guard) = self.test_transport.lock() {
            if let Some(ref t) = *guard {
                return Ok(Arc::clone(t));
            }
        }
        let client = self.session.client()?;
        Ok(ProtoVoiceCallTransport::from_client(&client))
    }

    fn optional_transport(&self) -> Option<Arc<dyn VoiceCallTransport>> {
        self.transport().ok()
    }

    fn emit_call_event(&self, ev: CallEvent) {
        self.session
            .broadcast_event(json!({ "type": "call", "call": ev }));
    }

    fn send_audio_ws(&self, msg: AudioWsOut) {
        let tx = self
            .inner
            .lock()
            .ok()
            .and_then(|s| s.audio_ws.as_ref().map(|c| c.tx.clone()));
        if let Some(tx) = tx {
            let _ = tx.try_send(msg);
        }
    }

    fn on_logout(self: &Arc<Self>) {
        self.stop_control_poller();
        let active_id = self
            .inner
            .lock()
            .ok()
            .and_then(|s| s.active.as_ref().map(|a| a.id));
        if let Some(id) = active_id {
            self.finish_call(id, "logout", true);
        }
        if let Ok(mut s) = self.inner.lock() {
            for (_, mut entry) in s.ringing.drain() {
                if let Some(t) = entry.timeout_task.take() {
                    t.abort();
                }
            }
        }
    }

    fn stop_control_poller(&self) {
        if let Ok(mut s) = self.inner.lock() {
            if let Some(mut h) = s.ctrl_poll_handle.take() {
                h.stop();
            }
            if let Some(t) = s.ctrl_poll_task.take() {
                t.abort();
            }
            s.ctrl_poll_client = None;
        }
    }

    fn sync_control_poller(self: &Arc<Self>) {
        let Ok(client) = self.session.client() else {
            self.stop_control_poller();
            return;
        };
        let status = self.session.get_status();
        if !matches!(status.state, SessionState::Ready | SessionState::Disconnected) {
            self.stop_control_poller();
            return;
        }
        {
            let Ok(s) = self.inner.lock() else { return };
            if let Some(ref existing) = s.ctrl_poll_client {
                if let Some(existing_arc) = existing.upgrade() {
                    if Arc::ptr_eq(&existing_arc, &client) {
                        return;
                    }
                }
            }
        }

        self.stop_control_poller();

        let ctx = client.ctx();
        let servers = server_list(&ctx.ctrs_urls);
        let socket_cfg = ctx
            .settings
            .get("features")
            .and_then(|f| f.get("socket"));
        let enable_ctrl_socket = socket_cfg
            .and_then(|s| s.get("enable_ctrl_socket"))
            .and_then(|v| v.as_bool());
        let enable_chat_socket = socket_cfg
            .and_then(|s| s.get("enable_chat_socket"))
            .and_then(|v| v.as_bool());
        let disable_lp = socket_cfg
            .and_then(|s| s.get("disable_lp"))
            .and_then(|v| v.as_bool());

        let mut login_keys: Vec<String> = ctx
            .login_info
            .as_object()
            .map(|m| {
                m.keys()
                    .filter(|k| k.starts_with("zpw_"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        login_keys.sort();

        let server_hosts: Vec<String> = servers
            .iter()
            .map(|u| {
                Url::parse(u)
                    .ok()
                    .and_then(|p| p.host_str().map(str::to_string))
                    .unwrap_or_else(|| u.clone())
            })
            .collect();

        self.session.call_trace().record(&json!({
            "kind": "ctrl_setup",
            "loginKeys": login_keys,
            "servers": server_hosts,
            "socket": {
                "enable_ctrl_socket": enable_ctrl_socket,
                "enable_chat_socket": enable_chat_socket,
                "disable_lp": disable_lp,
            },
        }));

        if ctx.secret_key.is_empty() {
            self.session.call_trace().record(&json!({
                "kind": "ctrl_poll_unavailable",
                "reason": "no session key",
            }));
            return;
        }
        if enable_ctrl_socket == Some(true) && disable_lp == Some(true) {
            self.session.call_trace().record(&json!({
                "kind": "ctrl_poll_skipped",
                "reason": "controls come over the socket (disable_lp)",
            }));
            if let Ok(mut s) = self.inner.lock() {
                s.ctrl_poll_client = Some(Arc::downgrade(&client));
            }
            return;
        }

        let hold = ctx
            .login_info
            .get("zpw_conf")
            .and_then(|c| c.get("lp_ctl_timeout"))
            .and_then(|v| v.as_u64())
            .filter(|&h| h > 0 && h < 300)
            .unwrap_or(30);

        let poller = ControlPoller::new(Arc::clone(ctx), client.http().clone(), hold);
        let (mut rx, handle) = poller.start();

        let weak = Arc::downgrade(self);
        let client_weak = Arc::downgrade(&client);
        let task = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                let Some(mgr) = weak.upgrade() else { break };
                let Some(c) = client_weak.upgrade() else { break };
                // Ensure this poller still belongs to the active session client
                if mgr
                    .session
                    .client()
                    .ok()
                    .is_none_or(|cur| !Arc::ptr_eq(&cur, &c))
                {
                    break;
                }
                match ev {
                    ControlPollEvent::Controls(controls) => {
                        for ctrl in extract_voip_controls(&controls) {
                            mgr.on_voip_control(&ctrl, "poll", now_ms(), true);
                        }
                    }
                    ControlPollEvent::Refused { code, message } => {
                        mgr.session.call_trace().record(&json!({
                            "kind": "ctrl_poll_refused",
                            "code": code,
                            "message": message,
                        }));
                        break;
                    }
                    ControlPollEvent::Error { message, failures } => {
                        mgr.session.call_trace().record(&json!({
                            "kind": "ctrl_poll_error",
                            "error": message,
                            "failures": failures,
                        }));
                    }
                    ControlPollEvent::Unavailable(reason) => {
                        mgr.session.call_trace().record(&json!({
                            "kind": "ctrl_poll_unavailable",
                            "reason": reason,
                        }));
                        break;
                    }
                }
            }
        });

        if let Ok(mut s) = self.inner.lock() {
            s.ctrl_poll_client = Some(Arc::downgrade(&client));
            s.ctrl_poll_handle = Some(handle);
            s.ctrl_poll_task = Some(task);
        }
    }

    /// Returns the currently active call, if any.
    pub fn current_call(&self) -> Option<CurrentCall> {
        self.inner
            .lock()
            .ok()
            .and_then(|s| s.active.as_ref().map(|a| a.info.clone()))
    }

    fn resolve_caller_name(&self, user_id: &str, display_name: Option<&str>) -> String {
        if let Some(name) = self.session.friend_name(user_id) {
            if !name.is_empty() {
                return name;
            }
        }
        if let Some(thread) = self.session.store().get_thread(ThreadKind::User, user_id) {
            if !thread.name.is_empty() {
                return thread.name;
            }
        }
        if let Some(dn) = display_name {
            if !dn.is_empty() {
                return dn.to_string();
            }
        }
        "Someone".to_string()
    }

    /// Processes one decoded VoIP control (`{ act, data, raw }`) from WebSocket or ControlPoller.
    pub fn on_voip_control(
        self: &Arc<Self>,
        control: &Value,
        source: &str,
        now: i64,
        record_trace: bool,
    ) {
        let act = control
            .get("act")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let data_map = json_object(control.get("data")).unwrap_or_default();
        let raw = control.get("raw").cloned().unwrap_or_else(|| control.clone());

        if record_trace {
            self.session.call_trace().record(&json!({
                "kind": "control",
                "source": source,
                "act": act,
                "control": raw,
            }));
        }

        let call_id = text_of(data_map.get("callId").or_else(|| data_map.get("call_id")));
        let has_call_id = !call_id.is_empty();
        let is_peer_act = PEER_ACTS.contains(&act.as_str());

        // 1. Peer cam/mic toggle controls repeat legitimately; never dedupe by act:callId.
        if is_peer_act {
            self.on_peer_control(
                &act,
                &data_map,
                if has_call_id { Some(call_id.as_str()) } else { None },
            );
            return;
        }

        // 2. Active outgoing call controls (ring_ring, answer, busy, reject, cancel, endcall).
        if has_call_id {
            enum OutgoingAction {
                None,
                Ringing(u64, CurrentCall, u64),
                Connect(u64),
                Finish(u64, String),
                Duplicate,
            }

            let action = {
                let Ok(mut s) = self.inner.lock() else { return };
                let matches_outgoing = s
                    .active
                    .as_ref()
                    .is_some_and(|a| a.outgoing && !a.done && a.info.call_id == call_id);
                if matches_outgoing {
                    let key = format!("{act}:{call_id}");
                    if s.handled_controls.contains_key(&key) {
                        OutgoingAction::Duplicate
                    } else {
                        s.handled_controls.insert(key, now);
                        let active = s.active.as_mut().unwrap();
                        let active_id = active.id;
                        let params = json_object(data_map.get("params"));
                        match act.as_str() {
                            "ring_ring" => {
                                if active.info.state == CallState::Calling {
                                    active.info.state = CallState::Ringing;
                                    OutgoingAction::Ringing(
                                        active_id,
                                        active.info.clone(),
                                        active.ring_timeout_ms,
                                    )
                                } else {
                                    OutgoingAction::None
                                }
                            }
                            "answer" => {
                                if active.info.state == CallState::Connected {
                                    OutgoingAction::None
                                } else {
                                    let status = number_of(
                                        data_map
                                            .get("status")
                                            .or_else(|| params.as_ref().and_then(|p| p.get("status"))),
                                    )
                                    .unwrap_or(0);
                                    if status == 0 {
                                        if let Some(wanted) = params
                                            .as_ref()
                                            .and_then(|p| p.get("rtpSerIp"))
                                            .and_then(|v| v.as_str())
                                            .filter(|s| !s.is_empty())
                                        {
                                            let same = active
                                                .server_address
                                                .as_deref()
                                                .is_some_and(|a| a == wanted);
                                            self.session.call_trace().record(&json!({
                                                "kind": "peer_server",
                                                "callId": call_id,
                                                "sameAsOurs": same,
                                            }));
                                        }
                                        OutgoingAction::Connect(active_id)
                                    } else {
                                        let reason = match status {
                                            3 => "declined",
                                            1 => "busy",
                                            _ => "unreachable",
                                        };
                                        OutgoingAction::Finish(active_id, reason.to_string())
                                    }
                                }
                            }
                            "reject" => {
                                let reason = if active.info.state != CallState::Connected {
                                    "declined"
                                } else {
                                    "reject"
                                };
                                OutgoingAction::Finish(active_id, reason.to_string())
                            }
                            "busy" => OutgoingAction::Finish(active_id, "busy".to_string()),
                            "cancel" | "endcall" | "end_call" => {
                                OutgoingAction::Finish(active_id, act.clone())
                            }
                            _ => OutgoingAction::None,
                        }
                    }
                } else {
                    // Deduplicate once-per-call controls across socket + poll
                    s.handled_controls
                        .retain(|_, at| now - *at <= HANDLED_CONTROL_TTL_MS);
                    let key = format!("{act}:{call_id}");
                    if s.handled_controls.contains_key(&key) {
                        OutgoingAction::Duplicate
                    } else {
                        s.handled_controls.insert(key, now);
                        OutgoingAction::None
                    }
                }
            };

            match action {
                OutgoingAction::Duplicate => return,
                OutgoingAction::Ringing(active_id, info, ring_ms) => {
                    self.arm_active_timer(active_id, ring_ms, "no_answer");
                    self.emit_call_event(CallEvent::Ringing(info));
                    return;
                }
                OutgoingAction::Connect(active_id) => {
                    self.connect_outgoing_call(active_id);
                    return;
                }
                OutgoingAction::Finish(active_id, reason) => {
                    self.finish_call(active_id, &reason, false);
                    return;
                }
                OutgoingAction::None => {
                    // Check if it was consumed by an outgoing call with unknown act
                    let was_outgoing = self
                        .inner
                        .lock()
                        .ok()
                        .and_then(|s| {
                            s.active
                                .as_ref()
                                .map(|a| a.outgoing && !a.done && a.info.call_id == call_id)
                        })
                        .unwrap_or(false);
                    if was_outgoing {
                        return;
                    }
                }
            }
        }

        // 3. Incoming call (`request` / `group_request`)
        if let Some(incoming) = incoming_call_of(&act, &data_map, now) {
            if now - incoming.ts > RINGING_MAX_AGE_MS {
                self.session.call_trace().record(&json!({
                    "kind": "call_too_old",
                    "callId": incoming.call_id,
                    "ageSeconds": ((now - incoming.ts) as f64 / 1000.0).round() as i64,
                }));
                return;
            }

            let ringing = ringing_call_of(incoming.clone(), data_map, now);
            let call_id_str = incoming.call_id.clone();
            let ring_timeout_ms = self.ring_timeout_ms;
            let weak = Arc::downgrade(self);
            let cid_for_timer = call_id_str.clone();
            let timer_handle = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ring_timeout_ms)).await;
                let Some(mgr) = weak.upgrade() else { return };
                let expired = {
                    let Ok(mut s) = mgr.inner.lock() else { return };
                    match s.ringing.get(&cid_for_timer) {
                        Some(entry) if entry.call.received_at == now => {
                            s.ringing.remove(&cid_for_timer);
                            true
                        }
                        _ => false,
                    }
                };
                if expired {
                    mgr.emit_call_event(CallEvent::Ended {
                        call_id: cid_for_timer,
                        act: "timeout".to_string(),
                    });
                }
            });

            {
                let Ok(mut s) = self.inner.lock() else { return };
                let expired_ids: Vec<String> = s
                    .ringing
                    .iter()
                    .filter(|(_, e)| now - e.call.received_at > RINGING_MAX_AGE_MS)
                    .map(|(k, _)| k.clone())
                    .collect();
                for id in expired_ids {
                    if let Some(mut old) = s.ringing.remove(&id) {
                        if let Some(t) = old.timeout_task.take() {
                            t.abort();
                        }
                    }
                }
                if let Some(mut prev) = s.ringing.insert(
                    call_id_str.clone(),
                    RingingEntry {
                        call: ringing.clone(),
                        timeout_task: Some(timer_handle),
                    },
                ) {
                    if let Some(t) = prev.timeout_task.take() {
                        t.abort();
                    }
                }
            }

            // Auto-send ring_ring for 1:1 incoming calls so the caller hears ringing immediately.
            if !incoming.group {
                if let Some(transport) = self.optional_transport() {
                    let session = Arc::clone(&self.session);
                    let ringing_bg = ringing.clone();
                    let cid = call_id_str.clone();
                    tokio::spawn(async move {
                        match transport.ring_ring(&ringing_bg).await {
                            Ok(answer) => session.call_trace().record(&json!({
                                "kind": "ringring",
                                "callId": cid,
                                "answer": answer,
                            })),
                            Err(err) => session.call_trace().record(&json!({
                                "kind": "ringring_failed",
                                "callId": cid,
                                "error": err,
                            })),
                        }
                    });
                }
            }

            let caller_name =
                self.resolve_caller_name(&incoming.caller_id, incoming.display_name.as_deref());
            let group_name = incoming.group_id.as_deref().and_then(|gid| {
                self.session
                    .store()
                    .get_thread(ThreadKind::Group, gid)
                    .map(|t| t.name)
                    .filter(|n| !n.is_empty())
            });

            self.emit_call_event(CallEvent::Incoming {
                call: ServerIncomingCall {
                    call_id: incoming.call_id,
                    caller_id: incoming.caller_id,
                    video: incoming.video,
                    group: incoming.group,
                    group_id: incoming.group_id,
                    ts: incoming.ts,
                    display_name: incoming.display_name,
                    caller_name,
                    group_name,
                },
            });
            return;
        }

        // 4. Call-ending controls (`answer`, `cancel`, `endcall`, `end_call`, `reject`, `busy`, ...)
        if CALL_ENDING_ACTS.contains(&act.as_str()) {
            let active_to_finish = {
                let Ok(mut s) = self.inner.lock() else { return };
                if has_call_id {
                    if let Some(mut entry) = s.ringing.remove(&call_id) {
                        if let Some(t) = entry.timeout_task.take() {
                            t.abort();
                        }
                    }
                }
                if let Some(ref active) = s.active {
                    if has_call_id && active.info.call_id == call_id {
                        // Our own answer comes back as "answer" echo — do NOT end the call.
                        if act == "answer" {
                            return;
                        }
                        Some(active.id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(active_id) = active_to_finish {
                self.finish_call(active_id, &act, false);
                return;
            }
            self.emit_call_event(CallEvent::Ended { call_id, act });
        }
    }

    fn on_peer_control(&self, act: &str, data_map: &Map<String, Value>, call_id: Option<&str>) {
        let params = json_object(data_map.get("params"));
        let Some(value) = number_of(
            params
                .as_ref()
                .and_then(|p| p.get("type").or_else(|| p.get("status"))),
        ) else {
            return;
        };
        let event = {
            let Ok(mut s) = self.inner.lock() else { return };
            let Some(ref mut active) = s.active else { return };
            if let Some(cid) = call_id {
                if !cid.is_empty() && cid != active.info.call_id {
                    return;
                }
            }
            let before = (active.peer_camera_on, active.peer_muted);
            if act == "onoff_camera" {
                active.peer_camera_on = value != 0;
            } else {
                active.peer_muted = value == 1;
            }
            let after = (active.peer_camera_on, active.peer_muted);
            if before == after && active.peer_heard.contains(act) {
                return;
            }
            active.peer_heard.insert(act.to_string());
            CallEvent::Peer {
                call_id: active.info.call_id.clone(),
                camera_on: active.peer_camera_on,
                muted: active.peer_muted,
            }
        };
        self.emit_call_event(event);
    }

    /// Declines a ringing incoming call.
    pub async fn decline_call(&self, call_id: &str) -> Result<(), HttpError> {
        let transport = self.transport()?;
        let call = {
            let mut s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            let Some(entry) = s.ringing.get(call_id) else {
                return Err(HttpError::not_found("That call isn't ringing any more"));
            };
            if now_ms() - entry.call.received_at > RINGING_MAX_AGE_MS {
                if let Some(mut old) = s.ringing.remove(call_id) {
                    if let Some(t) = old.timeout_task.take() {
                        t.abort();
                    }
                }
                return Err(HttpError::not_found("That call isn't ringing any more"));
            }
            if entry.call.incoming.group {
                return Err(HttpError::new(422, "Group calls can't be declined here"));
            }
            let mut claimed = s.ringing.remove(call_id).unwrap();
            if let Some(t) = claimed.timeout_task.take() {
                t.abort();
            }
            claimed.call
        };

        match transport.ring_ring(&call).await {
            Ok(answer) => self.session.call_trace().record(&json!({
                "kind": "ringring",
                "callId": call_id,
                "answer": answer,
            })),
            Err(err) => self.session.call_trace().record(&json!({
                "kind": "ringring_failed",
                "callId": call_id,
                "error": err,
            })),
        }

        match transport.decline_call(&call).await {
            Ok(answer) => {
                self.session.call_trace().record(&json!({
                    "kind": "declined",
                    "callId": call_id,
                    "answer": answer,
                }));
            }
            Err(err) => {
                self.session.call_trace().record(&json!({
                    "kind": "decline_failed",
                    "callId": call_id,
                    "error": err,
                }));
                return Err(HttpError::new(502, format!("Zalo refused: {err}")));
            }
        }

        self.emit_call_event(CallEvent::Ended {
            call_id: call_id.to_string(),
            act: "declined".to_string(),
        });
        Ok(())
    }

    /// Picks up a ringing 1:1 call (or runs a silent `test_seconds` call test).
    pub async fn answer_call(
        self: &Arc<Self>,
        call_id: &str,
        test_seconds: Option<u64>,
    ) -> Result<CurrentCall, HttpError> {
        let transport = self.transport()?;
        let (call, params, server) = {
            let mut s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            let Some(entry) = s.ringing.get(call_id) else {
                return Err(HttpError::not_found("That call isn't ringing any more"));
            };
            if now_ms() - entry.call.received_at > RINGING_MAX_AGE_MS {
                if let Some(mut old) = s.ringing.remove(call_id) {
                    if let Some(t) = old.timeout_task.take() {
                        t.abort();
                    }
                }
                return Err(HttpError::not_found("That call isn't ringing any more"));
            }
            if entry.call.incoming.group {
                return Err(HttpError::new(422, "Group calls can't be answered here"));
            }
            if s.active.is_some() {
                return Err(HttpError::conflict("Already in a call"));
            }
            let Some(p) = media_params_of(&entry.call) else {
                self.session.call_trace().record(&json!({
                    "kind": "answer_unavailable",
                    "callId": call_id,
                    "reason": "the request has no media server, ids or session",
                }));
                return Err(HttpError::new(
                    422,
                    "This call doesn't carry what's needed to answer it",
                ));
            };
            let Some(srv) = parse_server(&p.server) else {
                self.session.call_trace().record(&json!({
                    "kind": "answer_unavailable",
                    "callId": call_id,
                    "reason": "the request has no media server, ids or session",
                }));
                return Err(HttpError::new(
                    422,
                    "This call doesn't carry what's needed to answer it",
                ));
            };
            let mut claimed = s.ringing.remove(call_id).unwrap();
            if let Some(t) = claimed.timeout_task.take() {
                t.abort();
            }
            (claimed.call, p, srv)
        };

        let caller_name = self.resolve_caller_name(
            &call.incoming.caller_id,
            call.incoming.display_name.as_deref(),
        );
        let current = CurrentCall {
            call_id: call_id.to_string(),
            caller_id: call.incoming.caller_id.clone(),
            caller_name,
            video: call.incoming.video,
            state: CallState::Connecting,
            connected_at: None,
            outgoing: false,
        };
        let active_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let started_at = now_ms();

        {
            let mut s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            if s.active.is_some() {
                return Err(HttpError::conflict("Already in a call"));
            }
            s.active = Some(ActiveCall {
                id: active_id,
                info: current.clone(),
                call: call.clone(),
                transport: Arc::clone(&transport),
                media: None,
                outgoing: false,
                started_at,
                accepted: false,
                requested: false,
                done: false,
                want_hang_up: false,
                cam_on: false,
                peer_camera_on: true,
                peer_muted: false,
                peer_heard: HashSet::new(),
                server_address: Some(params.server.clone()),
                ring_timeout_ms: OUTGOING_RING_TIMEOUT_MS,
                timer_task: None,
                media_task: None,
            });
        }

        let res = self
            .run_answer_flow(active_id, &call, &params, server, &transport, test_seconds)
            .await;

        if res.is_err() {
            let media_to_close = {
                let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                if s.active.as_ref().is_some_and(|a| a.id == active_id) {
                    let mut a = s.active.take().unwrap();
                    if let Some(t) = a.timer_task.take() {
                        t.abort();
                    }
                    if let Some(t) = a.media_task.take() {
                        t.abort();
                    }
                    a.media.take()
                } else {
                    None
                }
            };
            if let Some(m) = media_to_close {
                m.close(false);
            }
        }
        res
    }

    async fn run_answer_flow(
        self: &Arc<Self>,
        active_id: u64,
        call: &RingingCall,
        params: &zalo_proto::MediaParams,
        server: zalo_media::call::MediaServer,
        transport: &Arc<dyn VoiceCallTransport>,
        test_seconds: Option<u64>,
    ) -> Result<CurrentCall, HttpError> {
        let call_id = &call.incoming.call_id;

        match transport.ring_ring(call).await {
            Ok(answer) => self.session.call_trace().record(&json!({
                "kind": "ringring",
                "callId": call_id,
                "answer": answer,
            })),
            Err(err) => self.session.call_trace().record(&json!({
                "kind": "ringring_failed",
                "callId": call_id,
                "error": err,
            })),
        }

        if !self.is_active_alive(active_id) {
            return Err(HttpError::conflict("Call ended while connecting"));
        }

        let opts = MediaCallOptions::new(
            server,
            params.user_id,
            params.partner_id,
            params.call_id,
            params.session.clone(),
            false,
        );
        let (media, ev_rx) = MediaCall::new(opts)
            .await
            .map_err(|e| HttpError::new(502, e))?;
        let media_task = self.spawn_media_event_pump(call_id.clone(), ev_rx);

        let t0 = now_ms();
        {
            let Ok(mut s) = self.inner.lock() else {
                media.close(false);
                media_task.abort();
                return Err(HttpError::internal("Call state lock poisoned"));
            };
            if let Some(ref mut a) = s.active {
                if a.id == active_id && !a.done {
                    a.media = Some(media.clone());
                    a.media_task = Some(media_task);
                    a.started_at = t0;
                } else {
                    media.close(false);
                    media_task.abort();
                    return Err(HttpError::conflict("Call ended while connecting"));
                }
            } else {
                media.close(false);
                media_task.abort();
                return Err(HttpError::conflict("Call ended while connecting"));
            }
        }

        let token = media.join_default().await;
        self.session.call_trace().record(&json!({
            "kind": "media_join",
            "callId": call_id,
            "server": params.server,
            "joined": token.is_some(),
            "ms": now_ms() - t0,
        }));

        if !self.is_active_alive(active_id) {
            media.close(false);
            return Err(HttpError::conflict("Call ended while connecting"));
        }
        if token.is_none() {
            media.close(false);
            return Err(HttpError::new(
                502,
                "Zalo's media server didn't let us join the call",
            ));
        }

        match transport.accept_call(call, &params.server).await {
            Ok(answer) => self.session.call_trace().record(&json!({
                "kind": "answered",
                "callId": call_id,
                "answer": answer,
                "video": call.incoming.video,
                "test": test_seconds.is_some(),
            })),
            Err(err) => {
                media.close(true);
                self.session.call_trace().record(&json!({
                    "kind": "answer_failed",
                    "callId": call_id,
                    "error": err,
                }));
                return Err(HttpError::new(502, format!("Zalo refused: {err}")));
            }
        }

        // Check if ended while accept_call was in flight
        let (want_hang_up, still_alive) = {
            let Ok(s) = self.inner.lock() else {
                return Err(HttpError::internal("Call state lock poisoned"));
            };
            match s.active.as_ref() {
                Some(a) if a.id == active_id && !a.done => (false, true),
                Some(a) if a.id == active_id => (a.want_hang_up, false),
                _ => (true, false),
            }
        };
        if !still_alive {
            media.close(want_hang_up);
            if want_hang_up {
                let t = Arc::clone(transport);
                let c = call.clone();
                let session = Arc::clone(&self.session);
                let cid = call_id.clone();
                tokio::spawn(async move {
                    match t.hang_up(&c).await {
                        Ok(answer) => session.call_trace().record(&json!({
                            "kind": "hung_up",
                            "callId": cid,
                            "answer": answer,
                        })),
                        Err(err) => session.call_trace().record(&json!({
                            "kind": "hang_up_failed",
                            "callId": cid,
                            "error": err,
                        })),
                    }
                });
            }
            return Err(HttpError::conflict("Call ended while connecting"));
        }

        media.start_silence();

        let (connected_info, cam_on) = {
            let mut s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            let Some(ref mut a) = s.active else {
                return Err(HttpError::conflict("Call ended while connecting"));
            };
            if a.id != active_id || a.done {
                return Err(HttpError::conflict("Call ended while connecting"));
            }
            a.accepted = true;
            a.info.state = CallState::Connected;
            a.info.connected_at = Some(now_ms());
            (a.info.clone(), a.cam_on)
        };

        if call.incoming.video {
            self.send_cam_mic(Arc::clone(transport), call.clone(), Some(cam_on), None);
        }

        if let Some(secs) = test_seconds {
            self.arm_active_timer(active_id, secs.saturating_mul(1000), "time_up");
        }

        self.emit_call_event(CallEvent::Connected(connected_info.clone()));
        Ok(connected_info)
    }

    /// Places a 1:1 outgoing call.
    pub async fn start_call(
        self: &Arc<Self>,
        user_id: &str,
        video: bool,
        setup_ms: Option<u64>,
        ring_ms: Option<u64>,
    ) -> Result<CurrentCall, HttpError> {
        let transport = self.transport()?;
        if user_id.is_empty()
            || user_id.len() > 20
            || !user_id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(HttpError::bad_request(
                "That isn't a person Zalo can call",
            ));
        }
        if self.session.own_id().as_deref() == Some(user_id) {
            return Err(HttpError::bad_request("You can't call yourself"));
        }

        let call_number = new_call_id();
        let call_id = call_number.to_string();
        let call = outgoing_call_of(user_id, call_number, video);
        let peer_name = self.resolve_caller_name(user_id, None);
        let current = CurrentCall {
            call_id: call_id.clone(),
            caller_id: user_id.to_string(),
            caller_name: peer_name,
            video,
            state: CallState::Calling,
            connected_at: None,
            outgoing: true,
        };
        let active_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let setup_ms = setup_ms.unwrap_or(OUTGOING_SETUP_TIMEOUT_MS);
        let ring_ms = ring_ms.unwrap_or(OUTGOING_RING_TIMEOUT_MS);

        {
            let mut s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            if s.active.is_some() {
                return Err(HttpError::conflict("Already in a call"));
            }
            s.active = Some(ActiveCall {
                id: active_id,
                info: current.clone(),
                call: call.clone(),
                transport: Arc::clone(&transport),
                media: None,
                outgoing: true,
                started_at: now_ms(),
                accepted: false,
                requested: false,
                done: false,
                want_hang_up: false,
                cam_on: false,
                peer_camera_on: true,
                peer_muted: false,
                peer_heard: HashSet::new(),
                server_address: None,
                ring_timeout_ms: ring_ms,
                timer_task: None,
                media_task: None,
            });
        }

        self.emit_call_event(CallEvent::Outgoing(current.clone()));
        self.arm_active_timer(active_id, setup_ms, "unreachable");

        let answer = match transport.request_call(&call).await {
            Ok(v) => v,
            Err(err) => {
                self.session.call_trace().record(&json!({
                    "kind": "request_call_failed",
                    "callId": call_id,
                    "error": err,
                }));
                self.finish_call(active_id, "failed", false);
                return Err(HttpError::new(502, format!("Zalo refused the call: {err}")));
            }
        };

        if let Ok(mut s) = self.inner.lock() {
            if let Some(ref mut a) = s.active {
                if a.id == active_id {
                    a.requested = true;
                }
            }
        }

        let mut shape: Vec<String> = answer
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        shape.sort();

        let offered = match outgoing_media_of(&answer, call_number) {
            Ok(m) => {
                self.session.call_trace().record(&json!({
                    "kind": "request_call",
                    "callId": call_id,
                    "video": video,
                    "fields": shape,
                    "servers": m.servers.len(),
                }));
                m
            }
            Err(problem) => {
                self.session.call_trace().record(&json!({
                    "kind": "request_call",
                    "callId": call_id,
                    "video": video,
                    "fields": shape,
                    "servers": 0,
                    "problem": problem,
                }));
                self.finish_call(active_id, "failed", false);
                return Err(HttpError::new(
                    502,
                    format!("Zalo didn't set the call up: {problem}"),
                ));
            }
        };

        if !self.is_active_alive(active_id) {
            return Err(HttpError::conflict("Call ended while connecting"));
        }

        let mut chosen: Option<OfferedServer> = None;
        for candidate in offered.servers.iter().take(3) {
            let Some(srv) = parse_server(&candidate.rtp) else {
                continue;
            };
            let opts = MediaCallOptions::new(
                srv,
                offered.user_id,
                offered.partner_id,
                call_number,
                offered.session.clone(),
                true,
            );
            let Ok((media, ev_rx)) = MediaCall::new(opts).await else {
                continue;
            };
            let media_task = self.spawn_media_event_pump(call_id.clone(), ev_rx);
            {
                let Ok(mut s) = self.inner.lock() else {
                    media.close(false);
                    media_task.abort();
                    return Err(HttpError::internal("Call state lock poisoned"));
                };
                if let Some(ref mut a) = s.active {
                    if a.id == active_id && !a.done {
                        a.media = Some(media.clone());
                        a.media_task = Some(media_task);
                    } else {
                        media.close(false);
                        media_task.abort();
                        return Err(HttpError::conflict("Call ended while connecting"));
                    }
                } else {
                    media.close(false);
                    media_task.abort();
                    return Err(HttpError::conflict("Call ended while connecting"));
                }
            }

            let t0 = now_ms();
            let token = media.join_default().await;
            self.session.call_trace().record(&json!({
                "kind": "media_join",
                "callId": call_id,
                "server": candidate.rtp,
                "joined": token.is_some(),
                "ms": now_ms() - t0,
            }));

            if !self.is_active_alive(active_id) {
                media.close(false);
                return Err(HttpError::conflict("Call ended while connecting"));
            }
            if token.is_some() {
                chosen = Some(candidate.clone());
                break;
            }
            media.close(false);
            if let Ok(mut s) = self.inner.lock() {
                if let Some(ref mut a) = s.active {
                    if a.id == active_id {
                        a.media = None;
                        if let Some(t) = a.media_task.take() {
                            t.abort();
                        }
                    }
                }
            }
        }

        let Some(chosen) = chosen else {
            self.finish_call(active_id, "failed", false);
            return Err(HttpError::new(
                502,
                "Zalo's media server didn't let us join the call",
            ));
        };

        if let Ok(mut s) = self.inner.lock() {
            if let Some(ref mut a) = s.active {
                if a.id == active_id {
                    a.server_address = Some(chosen.rtp.clone());
                }
            }
        }

        match transport.send_call_request(&call, &offered, &chosen).await {
            Ok(reply) => self.session.call_trace().record(&json!({
                "kind": "call_requested",
                "callId": call_id,
                "video": video,
                "answer": reply,
            })),
            Err(err) => {
                self.session.call_trace().record(&json!({
                    "kind": "request_failed",
                    "callId": call_id,
                    "error": err,
                }));
                self.finish_call(active_id, "failed", false);
                return Err(HttpError::new(502, format!("Zalo refused the call: {err}")));
            }
        }

        let snapshot = {
            let Ok(s) = self.inner.lock() else {
                return Err(HttpError::internal("Call state lock poisoned"));
            };
            match s.active.as_ref() {
                Some(a) if a.id == active_id && !a.done => a.info.clone(),
                _ => return Err(HttpError::conflict("Call ended while connecting")),
            }
        };
        Ok(snapshot)
    }

    fn connect_outgoing_call(&self, active_id: u64) {
        let extracted = {
            let Ok(mut s) = self.inner.lock() else { return };
            let Some(ref mut a) = s.active else { return };
            if a.id != active_id || a.done || a.info.state == CallState::Connected {
                return;
            }
            let Some(ref media) = a.media else { return };
            if let Some(t) = a.timer_task.take() {
                t.abort();
            }
            media.start_silence();
            a.info.state = CallState::Connected;
            a.info.connected_at = Some(now_ms());
            (
                a.info.clone(),
                Arc::clone(&a.transport),
                a.call.clone(),
                a.cam_on,
            )
        };

        let (info, transport, call, cam_on) = extracted;
        let session = Arc::clone(&self.session);
        let t_ack = Arc::clone(&transport);
        let c_ack = call.clone();
        let cid = info.call_id.clone();
        tokio::spawn(async move {
            match t_ack.answer_ack(&c_ack).await {
                Ok(answer) => session.call_trace().record(&json!({
                    "kind": "answerack",
                    "callId": cid,
                    "answer": answer,
                })),
                Err(err) => session.call_trace().record(&json!({
                    "kind": "answerack_failed",
                    "callId": cid,
                    "error": err,
                })),
            }
        });

        if info.video {
            self.send_cam_mic(transport, call, Some(cam_on), None);
        }

        self.emit_call_event(CallEvent::Connected(info));
    }

    /// Hangs up (or cancels) the active call.
    pub fn hang_up_call(&self) -> Result<(), HttpError> {
        let active_id = {
            let s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            s.active
                .as_ref()
                .map(|a| a.id)
                .ok_or_else(|| HttpError::not_found("No call in progress"))?
        };
        self.finish_call(active_id, "hangup", true);
        Ok(())
    }

    /// Tells the peer our microphone mute state.
    pub fn set_call_muted(&self, muted: bool) -> Result<(), HttpError> {
        let target = {
            let s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            let a = s
                .active
                .as_ref()
                .ok_or_else(|| HttpError::not_found("No call in progress"))?;
            if a.info.state == CallState::Connected {
                Some((Arc::clone(&a.transport), a.call.clone()))
            } else {
                None
            }
        };
        if let Some((transport, call)) = target {
            self.send_cam_mic(transport, call, None, Some(muted));
        }
        Ok(())
    }

    /// Turns our camera on or off, telling the peer and gating outbound video frames.
    pub fn set_call_camera(&self, on: bool) -> Result<(), HttpError> {
        let target = {
            let mut s = self
                .inner
                .lock()
                .map_err(|_| HttpError::internal("Call state lock poisoned"))?;
            let a = s
                .active
                .as_mut()
                .ok_or_else(|| HttpError::not_found("No call in progress"))?;
            a.cam_on = on;
            if a.info.state == CallState::Connected {
                Some((Arc::clone(&a.transport), a.call.clone()))
            } else {
                None
            }
        };
        if let Some((transport, call)) = target {
            self.send_cam_mic(transport, call, Some(on), None);
        }
        Ok(())
    }

    pub fn send_call_audio(&self, opus: Vec<u8>) {
        let media = self.inner.lock().ok().and_then(|s| {
            s.active.as_ref().and_then(|a| {
                if a.info.state == CallState::Connected {
                    a.media.clone()
                } else {
                    None
                }
            })
        });
        if let Some(m) = media {
            m.send_audio_frame(opus);
        }
    }

    pub fn send_call_video(&self, key: bool, data: Vec<u8>) {
        let media = self.inner.lock().ok().and_then(|s| {
            s.active.as_ref().and_then(|a| {
                if a.info.state == CallState::Connected && a.cam_on {
                    a.media.clone()
                } else {
                    None
                }
            })
        });
        if let Some(m) = media {
            m.send_video_frame(key, data);
        }
    }

    pub fn request_call_key_frame(&self) {
        let media = self.inner.lock().ok().and_then(|s| {
            s.active.as_ref().and_then(|a| {
                if a.info.state == CallState::Connected {
                    a.media.clone()
                } else {
                    None
                }
            })
        });
        if let Some(m) = media {
            m.request_key_frame("app");
        }
    }

    fn send_cam_mic(
        &self,
        transport: Arc<dyn VoiceCallTransport>,
        call: RingingCall,
        cam_on: Option<bool>,
        muted: Option<bool>,
    ) {
        let session = Arc::clone(&self.session);
        let call_id = call.incoming.call_id.clone();
        tokio::spawn(async move {
            let mut base = Map::new();
            base.insert("callId".into(), json!(call_id));
            if let Some(c) = cam_on {
                base.insert("camOn".into(), json!(c));
            }
            if let Some(m) = muted {
                base.insert("muted".into(), json!(m));
            }
            match transport.cam_mic_state(&call, cam_on, muted).await {
                Ok(answer) => {
                    base.insert("kind".into(), json!("cam_mic"));
                    base.insert("answer".into(), answer);
                    session.call_trace().record(&Value::Object(base));
                }
                Err(err) => {
                    base.insert("kind".into(), json!("cam_mic_failed"));
                    base.insert("error".into(), json!(err));
                    session.call_trace().record(&Value::Object(base));
                }
            }
        });
    }

    fn is_active_alive(&self, active_id: u64) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|s| s.active.as_ref().map(|a| a.id == active_id && !a.done))
            .unwrap_or(false)
    }

    fn arm_active_timer(self: &Arc<Self>, active_id: u64, ms: u64, reason: &'static str) {
        let weak = Arc::downgrade(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            if let Some(mgr) = weak.upgrade() {
                mgr.finish_call(active_id, reason, true);
            }
        });
        if let Ok(mut s) = self.inner.lock() {
            if let Some(ref mut a) = s.active {
                if a.id == active_id && !a.done {
                    if let Some(old) = a.timer_task.replace(handle) {
                        old.abort();
                    }
                    return;
                }
            }
        }
        handle.abort();
    }

    fn spawn_media_event_pump(
        self: &Arc<Self>,
        call_id: String,
        mut ev_rx: mpsc::UnboundedReceiver<MediaEvent>,
    ) -> JoinHandle<()> {
        let weak = Arc::downgrade(self);
        let session = Arc::clone(&self.session);
        tokio::spawn(async move {
            while let Some(ev) = ev_rx.recv().await {
                match ev {
                    MediaEvent::Audio {
                        seq,
                        timestamp,
                        opus,
                    } => {
                        if let Some(mgr) = weak.upgrade() {
                            mgr.send_audio_ws(AudioWsOut::Binary(encode_audio_to_app(
                                seq, timestamp, &opus,
                            )));
                        }
                    }
                    MediaEvent::Video(frame) => {
                        if let Some(mgr) = weak.upgrade() {
                            mgr.send_audio_ws(AudioWsOut::Binary(encode_video_to_app(
                                frame.key,
                                frame.timestamp,
                                &frame.data,
                            )));
                        }
                    }
                    MediaEvent::KeyFrameRequest => {
                        if let Some(mgr) = weak.upgrade() {
                            mgr.send_audio_ws(AudioWsOut::Binary(KEY_FRAME_REQUEST.to_vec()));
                        }
                    }
                    MediaEvent::Log(mut entry) => {
                        if let Some(obj) = entry.as_object_mut() {
                            obj.insert("callId".to_string(), json!(call_id));
                        }
                        session.call_trace().record(&entry);
                    }
                }
            }
        })
    }

    fn finish_call(&self, active_id: u64, reason: &str, send_end: bool) {
        let finished = {
            let Ok(mut s) = self.inner.lock() else { return };
            let matches = s
                .active
                .as_ref()
                .is_some_and(|a| a.id == active_id && !a.done);
            if !matches {
                return;
            }
            let mut active = s.active.take().unwrap();
            active.done = true;
            active.want_hang_up = send_end;
            if let Some(t) = active.timer_task.take() {
                t.abort();
            }
            if let Some(t) = active.media_task.take() {
                t.abort();
            }
            let ws_to_close = s.audio_ws.take().map(|c| c.tx);
            (active, ws_to_close)
        };

        let (active, ws_to_close) = finished;
        let was_connected = active.info.state == CallState::Connected;
        let send_media_end = if active.outgoing {
            send_end && was_connected
        } else {
            send_end && active.accepted
        };
        let stats = active.media.as_ref().map(|m| m.stats());
        if let Some(ref media) = active.media {
            media.close(send_media_end);
        }

        let act = if active.outgoing && reason == "hangup" && !was_connected {
            "cancel".to_string()
        } else {
            reason.to_string()
        };

        let seconds = ((now_ms() - active.started_at) as f64 / 1000.0).round() as i64;
        let mut trace = json!({
            "kind": "call_done",
            "callId": active.info.call_id,
            "reason": act,
            "seconds": seconds,
        });
        if active.outgoing {
            trace["outgoing"] = json!(true);
        }
        if let Some(st) = stats {
            trace["stats"] = json!(st);
        }
        self.session.call_trace().record(&trace);

        if active.outgoing {
            if send_end && (was_connected || active.requested) {
                let t = Arc::clone(&active.transport);
                let c = active.call.clone();
                let session = Arc::clone(&self.session);
                let cid = active.info.call_id.clone();
                tokio::spawn(async move {
                    if was_connected {
                        match t.hang_up(&c).await {
                            Ok(answer) => session.call_trace().record(&json!({
                                "kind": "hung_up",
                                "callId": cid,
                                "answer": answer,
                            })),
                            Err(err) => session.call_trace().record(&json!({
                                "kind": "hang_up_failed",
                                "callId": cid,
                                "error": err,
                            })),
                        }
                    } else {
                        match t.cancel_call(&c).await {
                            Ok(answer) => session.call_trace().record(&json!({
                                "kind": "cancelled",
                                "callId": cid,
                                "answer": answer,
                            })),
                            Err(err) => session.call_trace().record(&json!({
                                "kind": "cancel_failed",
                                "callId": cid,
                                "error": err,
                            })),
                        }
                    }
                });
            }
        } else if send_end && active.accepted {
            let t = Arc::clone(&active.transport);
            let c = active.call.clone();
            let session = Arc::clone(&self.session);
            let cid = active.info.call_id.clone();
            tokio::spawn(async move {
                match t.hang_up(&c).await {
                    Ok(answer) => session.call_trace().record(&json!({
                        "kind": "hung_up",
                        "callId": cid,
                        "answer": answer,
                    })),
                    Err(err) => session.call_trace().record(&json!({
                        "kind": "hang_up_failed",
                        "callId": cid,
                        "error": err,
                    })),
                }
            });
        }

        self.emit_call_event(CallEvent::Ended {
            call_id: active.info.call_id,
            act,
        });

        if let Some(tx) = ws_to_close {
            let _ = tx.try_send(AudioWsOut::Close(1000, "call ended"));
        }
    }

    /// Opens the audio link for the call in progress, replacing an existing link with `1000`
    /// "replaced". `None` when there is no active call (close `1008`, [`NO_ACTIVE_CALL_CLOSE`]).
    pub fn open_audio_link(self: &Arc<Self>) -> Option<AudioLink> {
        let (id, rx) = self.register_audio_ws()?;
        Some(AudioLink {
            manager: Arc::clone(self),
            id,
            rx: tokio::sync::Mutex::new(rx),
        })
    }

    /// Registers a newly connected audio client, replacing any existing one with code `1000`.
    fn register_audio_ws(&self) -> Option<(u64, mpsc::Receiver<AudioWsOut>)> {
        let mut s = self.inner.lock().ok()?;
        s.active.as_ref()?;
        let (tx, rx) = mpsc::channel(AUDIO_WS_QUEUE_CAP);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Some(old) = s.audio_ws.replace(AudioWsClient { id, tx }) {
            let _ = old.tx.try_send(AudioWsOut::Close(1000, "replaced"));
        }
        Some((id, rx))
    }

    fn unregister_audio_ws(&self, client_id: u64) {
        if let Ok(mut s) = self.inner.lock() {
            if s.audio_ws.as_ref().is_some_and(|c| c.id == client_id) {
                s.audio_ws = None;
            }
        }
    }
}

// ---- Axum Router & Handlers ----

/// Builds the Track 1 calls & call-log router with an isolated `CallManager` per server instance.
pub fn router(state: AppState) -> (Router<AppState>, Arc<CallManager>) {
    let manager = CallManager::new(Arc::clone(&state.session));
    manager.spawn_background();

    let router = Router::new()
        .route("/api/debug/call-log", get(get_call_log).delete(clear_call_log))
        .route("/api/calls/current", get(current_call))
        .route("/api/calls/{callId}/decline", post(post_decline_call))
        .route("/api/calls/{callId}/answer", post(post_answer_call))
        .route("/api/calls/{callId}/answer-test", post(post_answer_test))
        .route("/api/calls/start", post(post_start_call))
        .route("/api/calls/hangup", post(post_hangup_call))
        .route("/api/calls/mute", post(post_mute_call))
        .route("/api/calls/camera", post(post_camera_call))
        .route("/ws/call-audio", get(ws_call_audio))
        .layer(Extension(Arc::clone(&manager)));
    (router, manager)
}

async fn get_call_log(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.session.call_trace().read();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
}

async fn clear_call_log(State(state): State<AppState>) -> StatusCode {
    state.session.call_trace().clear();
    StatusCode::NO_CONTENT
}

async fn current_call(Extension(manager): Extension<Arc<CallManager>>) -> impl IntoResponse {
    match manager.current_call() {
        Some(call) => (StatusCode::OK, Json(json!(call))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "No active call" })),
        )
            .into_response(),
    }
}

async fn post_decline_call(
    Extension(manager): Extension<Arc<CallManager>>,
    AxumPath(call_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    manager.decline_call(&call_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_answer_call(
    Extension(manager): Extension<Arc<CallManager>>,
    AxumPath(call_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    let _ = manager.answer_call(&call_id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_answer_test(
    Extension(manager): Extension<Arc<CallManager>>,
    AxumPath(call_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    let _ = manager
        .answer_call(&call_id, Some(ANSWER_TEST_SECONDS))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_start_call(
    Extension(manager): Extension<Arc<CallManager>>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let user_id = val
        .get("userId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let video = val.get("video").and_then(|v| v.as_bool()).unwrap_or(false);
    let call = manager.start_call(user_id, video, None, None).await?;
    Ok((StatusCode::OK, Json(call)))
}

async fn post_hangup_call(
    Extension(manager): Extension<Arc<CallManager>>,
) -> ApiResult<StatusCode> {
    manager.hang_up_call()?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_mute_call(
    Extension(manager): Extension<Arc<CallManager>>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let val = parse_json_body(&body)?;
    let muted = val.get("muted").and_then(|v| v.as_bool()).unwrap_or(false);
    manager.set_call_muted(muted)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_camera_call(
    Extension(manager): Extension<Arc<CallManager>>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let val = parse_json_body(&body)?;
    let on = val.get("on").and_then(|v| v.as_bool()).unwrap_or(false);
    manager.set_call_camera(on)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ws_call_audio(
    Extension(manager): Extension<Arc<CallManager>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.max_message_size(AUDIO_WS_MAX_PAYLOAD)
        .on_upgrade(move |socket| handle_ws_call_audio(socket, manager))
}

async fn handle_ws_call_audio(mut socket: WebSocket, manager: Arc<CallManager>) {
    let Some(link) = manager.open_audio_link() else {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: NO_ACTIVE_CALL_CLOSE.0,
                reason: NO_ACTIVE_CALL_CLOSE.1.into(),
            })))
            .await;
        return;
    };

    let mut heartbeat = tokio::time::interval(Duration::from_secs(WS_HEARTBEAT_SECS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut alive = true;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if !alive {
                    break;
                }
                alive = false;
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            out = link.recv() => {
                match out {
                    Some(AudioWsOut::Binary(data)) => {
                        if socket.send(Message::Binary(data.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(AudioWsOut::Close(code, reason)) => {
                        let _ = socket
                            .send(Message::Close(Some(CloseFrame {
                                code,
                                reason: reason.into(),
                            })))
                            .await;
                        break;
                    }
                    None => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Binary(data))) => {
                        alive = true;
                        link.handle_incoming(data.as_ref());
                    }
                    Some(Ok(Message::Pong(_))) => {
                        alive = true;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        alive = true;
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        break;
                    }
                    Some(Ok(_)) => {
                        alive = true;
                    }
                }
            }
        }
    }

    debug!(client_id = link.id, "Closed /ws/call-audio connection");
}

// ---- JSON & control helpers ----

fn parse_json_body(body: &Bytes) -> Result<Value, HttpError> {
    if body.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let val: Value =
        serde_json::from_slice(body).map_err(|_| HttpError::bad_request("Invalid JSON body"))?;
    if !val.is_object() {
        return Err(HttpError::bad_request("Request body must be a JSON object"));
    }
    Ok(val)
}

fn json_object(v: Option<&Value>) -> Option<Map<String, Value>> {
    match v? {
        Value::Object(m) => Some(m.clone()),
        Value::String(s) if s.trim_start().starts_with('{') => {
            parse_json_keeping_bigints(s).ok()?.as_object().cloned()
        }
        _ => None,
    }
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn number_of(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Extracts `{ act, data, raw }` VoIP controls from raw `ControlPoller` control entries.
fn extract_voip_controls(controls: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for control in controls {
        let Some(content) = control.get("content").and_then(|c| c.as_object()) else {
            continue;
        };
        if content.get("act_type").and_then(|v| v.as_str()) != Some("voip") {
            continue;
        }
        let act = content
            .get("act")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let data = match content.get("data") {
            Some(Value::String(s)) => {
                parse_json_keeping_bigints(s).unwrap_or_else(|_| json!({ "text": s }))
            }
            Some(other) => other.clone(),
            None => Value::Object(Map::new()),
        };
        out.push(json!({
            "act": act,
            "data": data,
            "raw": control,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    use futures_util::{SinkExt, StreamExt};
    use tokio::net::{TcpListener, UdpSocket};
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::Message as TMsg;
    use zalo_media::call::rtcp::{parse_rtcp, pli, RtcpPacket};
    use zalo_media::call::srtp::{rtp_header, SrtpContext};
    use zalo_media::call::zrtc::{self, cmd, kind, Packet};

    use crate::config::Config;
    use crate::store::ChatStore;

    const SESSION: &str = "TEST-made-up-session-0123456789abcdefghijklmnop";

    struct FakeMediaServer {
        port: u16,
        got: Arc<Mutex<FakeMediaGot>>,
        push_tx: mpsc::UnboundedSender<Vec<u8>>,
        done_tx: Option<oneshot::Sender<()>>,
    }

    #[derive(Default)]
    struct FakeMediaGot {
        joins: usize,
        caller_joins: usize,
        audio: Vec<Vec<u8>>,
        video: Vec<Vec<u8>>,
        rtcp_video: Vec<Vec<u8>>,
        ends: usize,
    }

    impl FakeMediaServer {
        async fn start(accept: bool) -> Self {
            let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let port = sock.local_addr().unwrap().port();
            let got = Arc::new(Mutex::new(FakeMediaGot::default()));
            let got_bg = Arc::clone(&got);
            let (push_tx, mut push_rx) = mpsc::unbounded_channel::<Vec<u8>>();
            let (done_tx, mut done_rx) = oneshot::channel::<()>();

            tokio::spawn(async move {
                let mut caller_audio = SrtpContext::from_session(SESSION).unwrap();
                let mut caller_video = SrtpContext::from_session(SESSION).unwrap();
                let mut reader_audio = SrtpContext::from_session(SESSION).unwrap();
                let mut reader_video = SrtpContext::from_session(SESSION).unwrap();
                let mut buf = vec![0u8; 4096];
                let mut peer = None;

                loop {
                    tokio::select! {
                        _ = &mut done_rx => break,
                        pkt = push_rx.recv() => {
                            if let (Some(data), Some(addr)) = (pkt, peer) {
                                let _ = sock.send_to(&data, addr).await;
                            }
                        }
                        res = sock.recv_from(&mut buf) => {
                            let Ok((n, from)) = res else { break };
                            peer = Some(from);
                            let msg = &buf[..n];
                            if msg.is_empty() { continue; }
                            match msg[0] {
                                kind::REQUEST => {
                                    if let Ok(Packet::Control { header, .. }) = zrtc::parse(msg) {
                                        if matches!(header.cmd, cmd::INIT_CALL_CALLEE | cmd::INIT_CALL_CALLER) {
                                            {
                                                let mut g = got_bg.lock().unwrap();
                                                g.joins += 1;
                                                if header.cmd == cmd::INIT_CALL_CALLER {
                                                    g.caller_joins += 1;
                                                }
                                            }
                                            if !accept { continue; }
                                            let mut reply = msg[..21].to_vec();
                                            reply[0] = kind::RESPONSE;
                                            reply.extend_from_slice(&0u32.to_le_bytes());
                                            reply.extend_from_slice(&1u32.to_le_bytes());
                                            reply.extend_from_slice(&7u32.to_le_bytes());
                                            reply.extend_from_slice(&0u16.to_le_bytes());
                                            let _ = sock.send_to(&reply, from).await;

                                            // Send 1 audio packet + 1 video keyframe
                                            let mut rtp = hex::decode("80700005000000640000beef").unwrap();
                                            rtp.extend_from_slice(b"opus-frame");
                                            let mut ap = vec![kind::AUDIO_IN];
                                            ap.extend_from_slice(&caller_audio.protect(&rtp).unwrap());
                                            let _ = sock.send_to(&ap, from).await;

                                            let vrtp = hex::decode("80e20000000023280000515100000001658880").unwrap();
                                            let mut vp = vec![kind::VIDEO_IN];
                                            vp.extend_from_slice(&caller_video.protect(&vrtp).unwrap());
                                            let _ = sock.send_to(&vp, from).await;
                                        } else if header.cmd == cmd::END_CALL {
                                            got_bg.lock().unwrap().ends += 1;
                                        }
                                    }
                                }
                                kind::AUDIO => {
                                    if msg.len() > 5 {
                                        if let Some(plain) = reader_audio.unprotect(&msg[5..]) {
                                            got_bg.lock().unwrap().audio.push(plain[12..].to_vec());
                                        }
                                    }
                                }
                                kind::VIDEO => {
                                    if msg.len() > 5 {
                                        if let Some(plain) = reader_video.unprotect(&msg[5..]) {
                                            got_bg.lock().unwrap().video.push(plain);
                                        }
                                    }
                                }
                                kind::VIDEO_RTCP => {
                                    if msg.len() > 5 {
                                        got_bg.lock().unwrap().rtcp_video.push(msg[5..].to_vec());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            });

            Self {
                port,
                got,
                push_tx,
                done_tx: Some(done_tx),
            }
        }

        fn push(&self, pkt: Vec<u8>) {
            let _ = self.push_tx.send(pkt);
        }
    }

    impl Drop for FakeMediaServer {
        fn drop(&mut self) {
            if let Some(tx) = self.done_tx.take() {
                let _ = tx.send(());
            }
        }
    }

    struct FakeTransport {
        sent: Mutex<Vec<(String, Value)>>,
        media_port: u16,
        request_call_override: Mutex<Option<Value>>,
    }

    impl FakeTransport {
        fn new(media_port: u16) -> Arc<Self> {
            Arc::new(Self {
                sent: Mutex::new(Vec::new()),
                media_port,
                request_call_override: Mutex::new(None),
            })
        }

        fn record(&self, ep: &str, v: Value) {
            self.sent.lock().unwrap().push((ep.to_string(), v));
        }

        fn endpoints(&self) -> Vec<String> {
            self.sent.lock().unwrap().iter().map(|(e, _)| e.clone()).collect()
        }
    }

    impl VoiceCallTransport for FakeTransport {
        fn ring_ring<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
            self.record("ringring", json!({ "callId": call.raw_call_id }));
            Box::pin(async { Ok(json!({})) })
        }
        fn decline_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
            self.record("answer", json!({ "callId": call.raw_call_id, "status": 3 }));
            Box::pin(async { Ok(json!({})) })
        }
        fn accept_call<'a>(
            &'a self,
            call: &'a RingingCall,
            server: &'a str,
        ) -> BoxFuture<'a, Result<Value, String>> {
            self.record(
                "answer",
                json!({ "callId": call.raw_call_id, "status": 0, "server": server }),
            );
            Box::pin(async { Ok(json!({})) })
        }
        fn cam_mic_state<'a>(
            &'a self,
            call: &'a RingingCall,
            cam_on: Option<bool>,
            muted: Option<bool>,
        ) -> BoxFuture<'a, Result<Value, String>> {
            self.record(
                "conf",
                json!({
                    "callId": call.raw_call_id,
                    "isOnCam": match cam_on { Some(true) => 1, Some(false) => 0, None => -1 },
                    "isOnMic": match muted { Some(true) => 1, Some(false) => 0, None => -1 },
                }),
            );
            Box::pin(async { Ok(json!({})) })
        }
        fn hang_up<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
            self.record("endcall", json!({ "callId": call.raw_call_id }));
            Box::pin(async { Ok(json!({})) })
        }
        fn request_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
            self.record(
                "requestcall",
                json!({
                    "calleeId": call.incoming.caller_id,
                    "callId": call.raw_call_id,
                    "typeRequest": call.call_type,
                }),
            );
            let ov = self.request_call_override.lock().unwrap().clone();
            let port = self.media_port;
            let raw_id = call.raw_call_id.clone();
            Box::pin(async move {
                if let Some(custom) = ov {
                    return Ok(custom);
                }
                Ok(json!({
                    "id": raw_id,
                    "status": 0,
                    "fromId": 438000002,
                    "toId": 465000001,
                    "sessId": SESSION,
                    "servers": [{
                        "rtpaddr": format!("127.0.0.1:{port}"),
                        "rtcpaddr": format!("127.0.0.1:{port}"),
                    }],
                }))
            })
        }
        fn send_call_request<'a>(
            &'a self,
            call: &'a RingingCall,
            _media: &'a OutgoingMedia,
            server: &'a OfferedServer,
        ) -> BoxFuture<'a, Result<Value, String>> {
            self.record(
                "request",
                json!({ "callId": call.raw_call_id, "rtpAddress": server.rtp }),
            );
            Box::pin(async { Ok(json!({})) })
        }
        fn cancel_call<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
            self.record("cancel", json!({ "callId": call.raw_call_id }));
            Box::pin(async { Ok(json!({})) })
        }
        fn answer_ack<'a>(&'a self, call: &'a RingingCall) -> BoxFuture<'a, Result<Value, String>> {
            self.record("answerack", json!({ "callId": call.raw_call_id }));
            Box::pin(async { Ok(json!({})) })
        }
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}-{:016x}", rand::random::<u64>()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_manager(port: u16) -> (Arc<ZaloSession>, Arc<CallManager>, Arc<FakeTransport>, PathBuf) {
        let dir = temp_dir("zalo-calls-test");
        let store = ChatStore::open(dir.clone());
        let session = Arc::new(ZaloSession::new(store, dir.clone(), true));
        let mgr = CallManager::new(Arc::clone(&session));
        let transport = FakeTransport::new(port);
        *mgr.test_transport.lock().unwrap() = Some(Arc::clone(&transport) as Arc<dyn VoiceCallTransport>);
        (session, mgr, transport, dir)
    }

    fn ring_control(port: u16, call_id: &str, video: bool) -> Value {
        json!({
            "act": "request",
            "data": {
                "uidFrom": "465000001",
                "uidN": "4459000000000000001",
                "uidTo": "438000002",
                "callId": call_id,
                "session": SESSION,
                "rtpAddress": format!("127.0.0.1:{port}"),
                "params": json!({
                    "userId": 438000002,
                    "Dname": "Bảo",
                    "extendData": json!({ "callType": if video { 1 } else { 0 } }).to_string(),
                }).to_string(),
            },
            "raw": {},
        })
    }

    #[test]
    fn binary_audio_and_video_framing_matches_spec() {
        assert_eq!(
            hex::encode(encode_audio_to_app(0xfffe, 0x1234_5678, &[0xaa, 0xbb])),
            "01fffe12345678aabb"
        );
        assert_eq!(
            decode_audio_from_app(&[2, 0xf8, 0xff, 0xfe]),
            Some(&[0xf8, 0xff, 0xfe][..])
        );
        assert_eq!(decode_audio_from_app(&[2]), None);
        assert_eq!(decode_audio_from_app(&[1, 2, 3]), None);
        assert_eq!(decode_audio_from_app(&vec![2u8; 2000]), None);

        assert_eq!(
            hex::encode(encode_video_to_app(true, 0x1234_5678, &[0, 0, 0, 1, 0x65])),
            "0301123456780000000165"
        );
        assert_eq!(KEY_FRAME_REQUEST, &[0x05]);
        assert_eq!(
            decode_video_from_app(&hex::decode("04010000000165").unwrap()),
            Some((true, &hex::decode("0000000165").unwrap()[..]))
        );
        assert_eq!(
            decode_video_from_app(&hex::decode("040041").unwrap()),
            Some((false, &[0x41u8][..]))
        );
        assert_eq!(decode_video_from_app(&[4, 1]), None);
        assert_eq!(decode_video_from_app(&[2, 1, 2, 3]), None);
        assert_eq!(decode_video_from_app(&vec![4u8; 600 * 1024]), None);
        assert!(is_key_frame_wanted(&[6]));
        assert!(!is_key_frame_wanted(&[4, 1, 2]));
    }

    #[tokio::test]
    async fn incoming_answer_audio_video_peer_controls_and_hangup() {
        let media = FakeMediaServer::start(true).await;
        let (session, mgr, transport, dir) = make_manager(media.port);
        let mut ev_rx = session.subscribe_events();

        mgr.on_voip_control(&ring_control(media.port, "321000003", true), "socket", now_ms(), false);
        tokio::time::sleep(Duration::from_millis(20)).await;

        let ev = ev_rx.try_recv().unwrap();
        assert_eq!(ev["type"], "call");
        assert_eq!(ev["call"]["type"], "incoming");
        assert_eq!(ev["call"]["call"]["callerName"], "Bảo");
        assert_eq!(ev["call"]["call"]["video"], true);

        let cur = mgr.answer_call("321000003", None).await.unwrap();
        assert_eq!(cur.state, CallState::Connected);
        assert_eq!(cur.caller_name, "Bảo");
        assert!(mgr.answer_call("321000003", None).await.is_err());

        // Audio & video gated by camera state
        mgr.send_call_audio(b"my-voice".to_vec());
        let frame = hex::decode("00000001658880").unwrap();
        mgr.send_call_video(true, frame.clone());
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(media.got.lock().unwrap().video.len(), 0);

        mgr.set_call_camera(true).unwrap();
        mgr.send_call_video(true, frame);
        mgr.request_call_key_frame();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(media.got.lock().unwrap().video.len(), 1);
        let h = rtp_header(&media.got.lock().unwrap().video[0]).unwrap();
        assert_eq!(h.payload_type, 98);
        assert!(h.marker);
        assert!(media
            .got
            .lock()
            .unwrap()
            .audio
            .iter()
            .any(|a| a == b"my-voice"));
        assert!(media
            .got
            .lock()
            .unwrap()
            .audio
            .iter()
            .any(|a| a == &[0xf8, 0xff, 0xfe]));
        let has_pli = media
            .got
            .lock()
            .unwrap()
            .rtcp_video
            .iter()
            .flat_map(|b| parse_rtcp(b))
            .any(|p| matches!(p, RtcpPacket::Pli { media_ssrc: 0x5151, .. }));
        assert!(has_pli);

        // Peer cam/mic controls emit peer events without ending call
        mgr.on_voip_control(
            &json!({ "act": "onoff_camera", "data": { "callId": "321000003", "params": "{\"type\":0}" }, "raw": {} }),
            "socket",
            now_ms(),
            false,
        );
        mgr.on_voip_control(
            &json!({ "act": "mute_audio", "data": { "callId": "321000003", "params": "{\"status\":1}" }, "raw": {} }),
            "socket",
            now_ms(),
            false,
        );
        // Own answer echo must NOT end the call
        mgr.on_voip_control(
            &json!({ "act": "answer", "data": { "callId": "321000003" }, "raw": {} }),
            "socket",
            now_ms(),
            false,
        );
        assert_eq!(mgr.current_call().unwrap().call_id, "321000003");

        mgr.set_call_muted(true).unwrap();
        mgr.hang_up_call().unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(mgr.current_call().is_none());
        assert_eq!(media.got.lock().unwrap().ends, 1);
        assert!(transport.endpoints().contains(&"endcall".to_string()));
        assert!(transport.endpoints().contains(&"conf".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn outgoing_call_ringing_answer_cancel_and_timeouts() {
        let media = FakeMediaServer::start(true).await;
        let (session, mgr, transport, dir) = make_manager(media.port);
        let mut ev_rx = session.subscribe_events();

        let call = mgr.start_call("465000001", false, None, None).await.unwrap();
        assert_eq!(call.state, CallState::Calling);
        assert!(call.outgoing);
        assert_eq!(media.got.lock().unwrap().caller_joins, 1);
        assert_eq!(transport.endpoints(), vec!["requestcall", "request"]);

        mgr.on_voip_control(
            &json!({ "act": "ring_ring", "data": { "callId": call.call_id }, "raw": {} }),
            "socket",
            now_ms(),
            false,
        );
        assert_eq!(mgr.current_call().unwrap().state, CallState::Ringing);

        mgr.on_voip_control(
            &json!({
                "act": "answer",
                "data": { "callId": call.call_id, "params": json!({ "status": 0 }).to_string() },
                "raw": {},
            }),
            "socket",
            now_ms(),
            false,
        );
        assert_eq!(mgr.current_call().unwrap().state, CallState::Connected);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(transport.endpoints().contains(&"answerack".to_string()));

        mgr.hang_up_call().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(mgr.current_call().is_none());
        assert!(transport.endpoints().contains(&"endcall".to_string()));

        // Cancel before answer sends `cancel`, not `endcall`
        let c2 = mgr.start_call("465000001", true, None, None).await.unwrap();
        mgr.on_voip_control(
            &json!({ "act": "ring_ring", "data": { "callId": c2.call_id }, "raw": {} }),
            "socket",
            now_ms(),
            false,
        );
        mgr.hang_up_call().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(transport.endpoints().contains(&"cancel".to_string()));

        // Drain events and check outgoing -> ringing -> connected -> ended
        let mut seen_types = Vec::new();
        while let Ok(e) = ev_rx.try_recv() {
            if let Some(t) = e["call"]["type"].as_str() {
                seen_types.push(t.to_string());
            }
        }
        assert!(seen_types.contains(&"outgoing".to_string()));
        assert!(seen_types.contains(&"ringing".to_string()));
        assert!(seen_types.contains(&"connected".to_string()));
        assert!(seen_types.contains(&"ended".to_string()));

        // Setup timeout -> unreachable
        let c3 = mgr
            .start_call("465000001", false, Some(40), Some(60))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(90)).await;
        assert!(mgr.current_call().is_none());
        let mut saw_unreachable = false;
        while let Ok(e) = ev_rx.try_recv() {
            if e["call"]["type"] == "ended"
                && e["call"]["callId"] == c3.call_id
                && e["call"]["act"] == "unreachable"
            {
                saw_unreachable = true;
            }
        }
        assert!(saw_unreachable);

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ws_call_audio_replacement_binary_streaming_and_call_end_close() {
        let media = FakeMediaServer::start(true).await;
        let dir = temp_dir("zalo-ws-audio-test");
        let store = ChatStore::open(dir.clone());
        let session = Arc::new(ZaloSession::new(store.clone(), dir.clone(), true));
        let config = Arc::new(Config {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: dir.clone(),
            access_token: None,
            read_receipts: true,
            log_file: None,
            public_dir: None,
        });
        let app_state = AppState {
            config,
            session: Arc::clone(&session),
            store,
        };
        let mgr = CallManager::new(Arc::clone(&session));
        let transport = FakeTransport::new(media.port);
        *mgr.test_transport.lock().unwrap() =
            Some(Arc::clone(&transport) as Arc<dyn VoiceCallTransport>);

        let app = Router::new()
            .route("/ws/call-audio", get(ws_call_audio))
            .layer(Extension(Arc::clone(&mgr)))
            .with_state(app_state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv_handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // 1. No active call -> closes with 1008
        let (mut ws0, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/call-audio"))
            .await
            .unwrap();
        let msg = ws0.next().await.unwrap().unwrap();
        match msg {
            TMsg::Close(Some(f)) => assert_eq!(f.code, CloseCode::Policy),
            other => panic!("expected Close(1008), got {other:?}"),
        }

        // 2. Answer an incoming call, then connect ws1
        mgr.on_voip_control(&ring_control(media.port, "777001", true), "socket", now_ms(), false);
        mgr.answer_call("777001", None).await.unwrap();
        mgr.set_call_camera(true).unwrap();

        let (mut ws1, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/call-audio"))
            .await
            .unwrap();

        // Ending an unrelated call does NOT close ws1
        mgr.on_voip_control(
            &json!({ "act": "cancel", "data": { "callId": "other-call" }, "raw": {} }),
            "socket",
            now_ms(),
            false,
        );
        tokio::time::sleep(Duration::from_millis(30)).await;

        // 3. Connecting ws2 replaces ws1 with 1000 "replaced"
        let (mut ws2, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/call-audio"))
            .await
            .unwrap();
        loop {
            match ws1.next().await.unwrap().unwrap() {
                TMsg::Close(Some(f)) => {
                    assert_eq!(f.code, CloseCode::Normal);
                    assert_eq!(f.reason.as_str(), "replaced");
                    break;
                }
                TMsg::Binary(_) => continue,
                other => panic!("unexpected on ws1: {other:?}"),
            }
        }

        // 4. Send audio (0x02) + video (0x04) + keyframe want (0x06) from ws2 -> server -> fake media
        ws2.send(TMsg::Binary(vec![0x02, 0xaa, 0xbb, 0xcc].into()))
            .await
            .unwrap();
        let mut vid_msg = vec![0x04, 0x01];
        vid_msg.extend_from_slice(&hex::decode("00000001658880").unwrap());
        ws2.send(TMsg::Binary(vid_msg.into())).await.unwrap();
        ws2.send(TMsg::Binary(vec![0x06].into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(media
            .got
            .lock()
            .unwrap()
            .audio
            .iter()
            .any(|a| a == &[0xaa, 0xbb, 0xcc]));
        assert_eq!(media.got.lock().unwrap().video.len(), 1);

        // Push PLI from fake media -> ws2 receives 0x05
        let my_video_ssrc = rtp_header(&media.got.lock().unwrap().video[0]).unwrap().ssrc;
        media.push(zrtc::rtcp(7, &pli(0x5151, my_video_ssrc), true).unwrap());
        let got_kfr = tokio::time::timeout(Duration::from_millis(400), async {
            while let Some(Ok(m)) = ws2.next().await {
                if let TMsg::Binary(b) = m {
                    if b.as_ref() == KEY_FRAME_REQUEST {
                        return true;
                    }
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(got_kfr, "ws2 should receive 0x05 key frame request");

        // 5. Hang up -> ws2 closes with 1000 "call ended"
        mgr.hang_up_call().unwrap();
        loop {
            match ws2.next().await.unwrap().unwrap() {
                TMsg::Close(Some(f)) => {
                    assert_eq!(f.code, CloseCode::Normal);
                    assert_eq!(f.reason.as_str(), "call ended");
                    break;
                }
                TMsg::Binary(_) => continue,
                other => panic!("unexpected on ws2: {other:?}"),
            }
        }

        srv_handle.abort();
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn embedded_audio_replacement_binary_streaming_and_call_end_close() {
        use crate::embedded::{AudioPoll, EmbeddedAudio};

        let media = FakeMediaServer::start(true).await;
        let (_session, mgr, _transport, dir) = make_manager(media.port);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let open = |mgr: &Arc<CallManager>| {
            mgr.open_audio_link()
                .map(|l| EmbeddedAudio::new(l, shutdown_tx.subscribe()))
        };
        let wait = Duration::from_millis(400);

        // 1. No active call -> no link (the WebSocket path closes with 1008)
        assert!(open(&mgr).is_none());
        assert_eq!(NO_ACTIVE_CALL_CLOSE, (1008, "No active call"));

        // 2. Answer an incoming call, then open a1
        mgr.on_voip_control(&ring_control(media.port, "777002", true), "socket", now_ms(), false);
        mgr.answer_call("777002", None).await.unwrap();
        mgr.set_call_camera(true).unwrap();
        let a1 = open(&mgr).expect("link while a call is active");

        // Ending an unrelated call does NOT close a1
        mgr.on_voip_control(
            &json!({ "act": "cancel", "data": { "callId": "other-call" }, "raw": {} }),
            "socket",
            now_ms(),
            false,
        );
        tokio::time::sleep(Duration::from_millis(30)).await;

        // 3. Opening a2 replaces a1 with 1000 "replaced"
        let a2 = open(&mgr).unwrap();
        loop {
            match a1.next(wait).await {
                AudioPoll::Frame(_) => continue,
                AudioPoll::Closed => break,
                AudioPoll::Timeout => panic!("a1 should be closed"),
            }
        }
        assert_eq!(a1.close_info(), Some((1000, "replaced".to_string())));
        assert_eq!(a1.next(wait).await, AudioPoll::Closed);

        // 4. Frames from the app: audio (0x02), video (0x04), key-frame want (0x06)
        a2.send(&[0x02, 0xaa, 0xbb, 0xcc]);
        let mut vid_msg = vec![0x04, 0x01];
        vid_msg.extend_from_slice(&hex::decode("00000001658880").unwrap());
        a2.send(&vid_msg);
        a2.send(&[0x06]);
        a2.send(&[0x09, 1, 2, 3]); // unknown: ignored
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(media
            .got
            .lock()
            .unwrap()
            .audio
            .iter()
            .any(|a| a == &[0xaa, 0xbb, 0xcc]));
        assert_eq!(media.got.lock().unwrap().video.len(), 1);

        // Frames to the app: PLI from the fake media -> 0x05
        let my_video_ssrc = rtp_header(&media.got.lock().unwrap().video[0]).unwrap().ssrc;
        media.push(zrtc::rtcp(7, &pli(0x5151, my_video_ssrc), true).unwrap());
        let got_kfr = tokio::time::timeout(Duration::from_millis(600), async {
            loop {
                if let AudioPoll::Frame(b) = a2.next(Duration::from_millis(100)).await {
                    if b == KEY_FRAME_REQUEST {
                        return true;
                    }
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(got_kfr, "a2 should receive 0x05 key frame request");

        // A quiet link times out instead of closing
        assert_eq!(a2.next(Duration::from_millis(20)).await, AudioPoll::Timeout);
        assert_eq!(a2.close_info(), None);

        // 5. Hang up -> a2 closes with 1000 "call ended"
        mgr.hang_up_call().unwrap();
        loop {
            match a2.next(wait).await {
                AudioPoll::Frame(_) => continue,
                AudioPoll::Closed => break,
                AudioPoll::Timeout => panic!("a2 should be closed"),
            }
        }
        assert_eq!(a2.close_info(), Some((1000, "call ended".to_string())));

        // Closing a handle, and shutting the core down, both end a waiting reader
        mgr.on_voip_control(&ring_control(media.port, "777003", false), "socket", now_ms(), false);
        mgr.answer_call("777003", None).await.unwrap();
        let a3 = open(&mgr).unwrap();
        a3.close();
        assert_eq!(a3.next(wait).await, AudioPoll::Closed);
        assert_eq!(a3.close_info(), Some((1000, "closed".to_string())));
        let a4 = open(&mgr).unwrap();
        shutdown_tx.send_replace(true);
        assert_eq!(a4.next(wait).await, AudioPoll::Closed);
        assert_eq!(a4.close_info(), Some((1001, "shutting down".to_string())));

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn decline_expired_group_and_control_extraction() {
        let media = FakeMediaServer::start(true).await;
        let (session, mgr, transport, dir) = make_manager(media.port);
        let mut ev_rx = session.subscribe_events();

        // 1. Decline a ringing 1:1 call
        mgr.on_voip_control(&ring_control(media.port, "400001", false), "socket", now_ms(), false);
        mgr.decline_call("400001").await.unwrap();
        assert!(transport.endpoints().contains(&"answer".to_string()));
        assert!(mgr.decline_call("400001").await.is_err());

        let mut saw_declined = false;
        while let Ok(e) = ev_rx.try_recv() {
            if e["call"]["type"] == "ended"
                && e["call"]["callId"] == "400001"
                && e["call"]["act"] == "declined"
            {
                saw_declined = true;
            }
        }
        assert!(saw_declined);

        // 2. Expired ringing call rejected with 404
        let old_now = now_ms() - 3 * 60_000;
        mgr.on_voip_control(&ring_control(media.port, "400002", false), "socket", old_now, false);
        let err = mgr.answer_call("400002", None).await.unwrap_err();
        assert_eq!(err.status, 404);

        // 3. Group call rejected with 422 on both answer and decline
        mgr.on_voip_control(
            &json!({
                "act": "group_request",
                "data": { "uidFrom": "465000001", "callId": "400003", "groupId": "g99" },
                "raw": {},
            }),
            "socket",
            now_ms(),
            false,
        );
        assert_eq!(mgr.answer_call("400003", None).await.unwrap_err().status, 422);
        assert_eq!(mgr.decline_call("400003").await.unwrap_err().status, 422);

        // 4. ControlPoller raw control extraction filters only act_type == "voip" and keeps bigints
        let extracted = extract_voip_controls(&[
            json!({ "content": { "act_type": "syncmsgmb", "act": "user_confirm", "data": "{}" } }),
            json!({
                "controlId": "c1",
                "content": {
                    "act_type": "voip",
                    "act": "request",
                    "data": r#"{"callId":12345,"uidN":7234567890123456789}"#
                }
            }),
        ]);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0]["act"], "request");
        assert_eq!(extracted[0]["data"]["uidN"], "7234567890123456789");

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rest_endpoints_lifecycle_over_http() {
        let media = FakeMediaServer::start(true).await;
        let dir = temp_dir("zalo-calls-http-test");
        let store = ChatStore::open(dir.clone());
        let session = Arc::new(ZaloSession::new(store.clone(), dir.clone(), true));
        let config = Arc::new(Config {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: dir.clone(),
            access_token: None,
            read_receipts: true,
            log_file: None,
            public_dir: None,
        });
        let app_state = AppState {
            config,
            session: Arc::clone(&session),
            store,
        };
        let mgr = CallManager::new(Arc::clone(&session));
        let transport = FakeTransport::new(media.port);
        *mgr.test_transport.lock().unwrap() =
            Some(Arc::clone(&transport) as Arc<dyn VoiceCallTransport>);

        let app = Router::new()
            .route("/api/debug/call-log", get(get_call_log).delete(clear_call_log))
            .route("/api/calls/current", get(current_call))
            .route("/api/calls/{callId}/decline", post(post_decline_call))
            .route("/api/calls/{callId}/answer", post(post_answer_call))
            .route("/api/calls/{callId}/answer-test", post(post_answer_test))
            .route("/api/calls/start", post(post_start_call))
            .route("/api/calls/hangup", post(post_hangup_call))
            .route("/api/calls/mute", post(post_mute_call))
            .route("/api/calls/camera", post(post_camera_call))
            .layer(Extension(Arc::clone(&mgr)))
            .with_state(app_state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv_handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let http = reqwest::Client::new();
        let base = format!("http://{addr}");

        // GET /api/calls/current -> 404
        let r = http.get(format!("{base}/api/calls/current")).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);

        // POST /api/calls/start with bad userId -> 400
        let r = http
            .post(format!("{base}/api/calls/start"))
            .json(&json!({ "userId": "not-a-number" }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);

        // POST /api/calls/start -> 200 CurrentCall
        let r = http
            .post(format!("{base}/api/calls/start"))
            .json(&json!({ "userId": "465000001", "video": false }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let started: CurrentCall = r.json().await.unwrap();
        assert_eq!(started.state, CallState::Calling);
        assert!(started.outgoing);

        // GET /api/calls/current -> 200
        let r = http.get(format!("{base}/api/calls/current")).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);

        // POST /api/calls/mute & /camera & /hangup -> 204
        assert_eq!(
            http.post(format!("{base}/api/calls/mute"))
                .json(&json!({ "muted": true }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            http.post(format!("{base}/api/calls/camera"))
                .json(&json!({ "on": true }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            http.post(format!("{base}/api/calls/hangup"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            http.get(format!("{base}/api/calls/current"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );

        // Incoming call answered via POST /api/calls/{id}/answer -> 204
        mgr.on_voip_control(&ring_control(media.port, "555001", false), "socket", now_ms(), false);
        assert_eq!(
            http.post(format!("{base}/api/calls/555001/answer"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            http.post(format!("{base}/api/calls/hangup"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );

        // Call log populated & clearable
        let log_body = http
            .get(format!("{base}/api/debug/call-log"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!log_body.is_empty());
        assert_eq!(
            http.delete(format!("{base}/api/debug/call-log"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );

        srv_handle.abort();
        let _ = fs::remove_dir_all(&dir);
    }
}
