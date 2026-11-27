use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};
use url::Url;

use crate::crypto::{
    decode_event_payload, decode_event_payload_with_cipher, parse_json_keeping_bigints, EventCipher,
};
use crate::error::{ProtoError, Result};
use crate::framelog::{FrameReport, ItemReport, MAX_ITEMS};
use crate::http::ZaloHttp;
use crate::types::{FileDonePayload, RawWsMessage, ThreadType, WsEvent, ZaloContext};

static RUSTLS_INIT: Once = Once::new();

fn ensure_rustls_provider() {
    RUSTLS_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Standard Zalo WebSocket close codes (`zca-js` `CloseReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    ManualClosure = 1000,
    AbnormalClosure = 1006,
    DuplicateConnection = 3000,
    KickConnection = 3003,
}

const SYNC_ACT_TYPES: &[&str] = &[
    "syncmsgmb",
    "transfer_msg",
    "transfer_sync2",
    "transfer_after_login",
];

const OFFLINE_PUSH_CMDS: &[u16] = &[504, 513, 514, 524, 534, 551, 570];

/// Commands sent from `ListenerHandle` into the WebSocket loop.
#[derive(Debug)]
enum OutboundCommand {
    SendFrame {
        version: u8,
        cmd: u16,
        sub_cmd: u8,
        data: Value,
        require_id: bool,
    },
    Stop,
}

/// Handle for sending requests to and stopping a running [`ZaloListener`].
#[derive(Clone)]
pub struct ListenerHandle {
    tx: mpsc::UnboundedSender<OutboundCommand>,
}

impl ListenerHandle {
    /// Sends a raw Zalo binary WebSocket frame.
    pub fn send_frame(&self, cmd: u16, sub_cmd: u8, data: Value, require_id: bool) -> Result<()> {
        self.tx
            .send(OutboundCommand::SendFrame {
                version: 1,
                cmd,
                sub_cmd,
                data,
                require_id,
            })
            .map_err(|_| ProtoError::WebSocket("listener channel closed".into()))
    }

    /// Requests old messages (`cmd=510` for User, `cmd=511` for Group).
    pub fn request_old_messages(
        &self,
        thread_type: ThreadType,
        last_msg_id: Option<String>,
    ) -> Result<()> {
        let cmd = match thread_type {
            ThreadType::User => 510,
            ThreadType::Group => 511,
        };
        let last_id = match last_msg_id {
            Some(s) => Value::String(s),
            None => Value::Null,
        };
        self.send_frame(
            cmd,
            1,
            json!({
                "first": true,
                "lastId": last_id,
                "preIds": [],
            }),
            true,
        )
    }

    /// Requests old reactions (`cmd=610` for User, `cmd=611` for Group).
    pub fn request_old_reactions(
        &self,
        thread_type: ThreadType,
        last_msg_id: Option<String>,
    ) -> Result<()> {
        let cmd = match thread_type {
            ThreadType::User => 610,
            ThreadType::Group => 611,
        };
        let last_id = match last_msg_id {
            Some(s) => Value::String(s),
            None => Value::Null,
        };
        self.send_frame(
            cmd,
            1,
            json!({
                "first": true,
                "lastId": last_id,
                "preIds": [],
            }),
            true,
        )
    }

    /// Gracefully stops the listener with WebSocket close code 1000.
    pub fn stop(&self) {
        let _ = self.tx.send(OutboundCommand::Stop);
    }
}

#[derive(Debug, Clone)]
struct RetryEntry {
    count: usize,
    max: usize,
    times: Vec<u64>,
}

/// Zalo WebSocket listener with binary frame codec, heartbeat ping, URL rotation, and unified
/// dispatching (including `pageMsgs`, offline push commands, `file_done`, `voip`, and history-sync controls).
pub struct ZaloListener {
    ctx: Arc<ZaloContext>,
    self_listen: bool,
}

impl ZaloListener {
    pub fn new(ctx: Arc<ZaloContext>, self_listen: bool) -> Self {
        Self { ctx, self_listen }
    }

    /// Starts the WebSocket listener background task, returning an event receiver and control handle.
    pub fn start(
        self,
        retry_on_close: bool,
    ) -> Result<(mpsc::UnboundedReceiver<WsEvent>, ListenerHandle)> {
        if self.ctx.ws_urls.is_empty() {
            return Err(ProtoError::WebSocket(
                "No WebSocket URLs available in ZaloContext".into(),
            ));
        }
        ensure_rustls_provider();

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let handle = ListenerHandle { tx: cmd_tx };

        tokio::spawn(async move {
            run_listener_loop(self.ctx, self.self_listen, retry_on_close, event_tx, cmd_rx).await;
        });

        Ok((event_rx, handle))
    }
}

struct SocketPolicy {
    ping_interval_ms: u64,
    close_and_retry_codes: Vec<u16>,
    rotate_error_codes: Vec<u16>,
    retries: HashMap<u16, RetryEntry>,
}

impl SocketPolicy {
    fn from_settings(settings: &Value) -> Self {
        let socket = settings
            .get("features")
            .and_then(|f| f.get("socket"));

        let ping_interval_ms = socket
            .and_then(|s| s.get("ping_interval"))
            .and_then(|v| v.as_u64())
            .filter(|&ms| ms >= 1000)
            .unwrap_or(15_000);

        let close_and_retry_codes = socket
            .and_then(|s| s.get("close_and_retry_codes"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u16))
                    .collect()
            })
            .unwrap_or_default();

        let rotate_error_codes = socket
            .and_then(|s| s.get("rotate_error_codes"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u16))
                    .collect()
            })
            .unwrap_or_default();

        let mut retries = HashMap::new();
        if let Some(obj) = socket.and_then(|s| s.get("retries")).and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Ok(code) = k.parse::<u16>() {
                    let max = v.get("max").and_then(|m| m.as_u64()).unwrap_or(0) as usize;
                    let times = match v.get("times") {
                        Some(Value::Number(n)) => vec![n.as_u64().unwrap_or(1000)],
                        Some(Value::Array(arr)) => arr
                            .iter()
                            .filter_map(|x| x.as_u64())
                            .collect(),
                        _ => vec![1000],
                    };
                    retries.insert(
                        code,
                        RetryEntry {
                            count: 0,
                            max,
                            times: if times.is_empty() { vec![1000] } else { times },
                        },
                    );
                }
            }
        }

        Self {
            ping_interval_ms,
            close_and_retry_codes,
            rotate_error_codes,
            retries,
        }
    }

    fn can_retry(&mut self, code: u16) -> Option<u64> {
        if !self.close_and_retry_codes.contains(&code) {
            return None;
        }
        let entry = self.retries.get_mut(&code)?;
        if entry.count >= entry.max {
            return None;
        }
        entry.count += 1;
        let idx = (entry.count - 1).min(entry.times.len() - 1);
        Some(entry.times[idx])
    }

    fn should_rotate(&self, code: u16, rotate_idx: usize, total_urls: usize) -> bool {
        self.rotate_error_codes.contains(&code) && rotate_idx + 1 < total_urls
    }
}

async fn run_listener_loop(
    ctx: Arc<ZaloContext>,
    self_listen: bool,
    retry_on_close: bool,
    event_tx: mpsc::UnboundedSender<WsEvent>,
    mut cmd_rx: mpsc::UnboundedReceiver<OutboundCommand>,
) {
    let mut policy = SocketPolicy::from_settings(&ctx.settings);
    let mut rotate_idx: usize = 0;
    let req_seq = AtomicU64::new(0);

    loop {
        let base_ws = &ctx.ws_urls[rotate_idx % ctx.ws_urls.len()];
        let ts_str = Utc::now().timestamp_millis().to_string();
        let ws_url = match ZaloHttp::make_ctx_url(&ctx, base_ws, &[("t", &ts_str)], true) {
            Ok(u) => u,
            Err(e) => {
                let _ = event_tx.send(WsEvent::Error(format!("Invalid WS URL: {e}")));
                let _ = event_tx.send(WsEvent::Closed {
                    code: CloseReason::AbnormalClosure as u16,
                    reason: e.to_string(),
                });
                return;
            }
        };

        let mut request = match ws_url.as_str().into_client_request() {
            Ok(r) => r,
            Err(e) => {
                let _ = event_tx.send(WsEvent::Error(format!("WS request error: {e}")));
                let _ = event_tx.send(WsEvent::Closed {
                    code: CloseReason::AbnormalClosure as u16,
                    reason: e.to_string(),
                });
                return;
            }
        };

        let cookie_str = ctx
            .cookies
            .read()
            .map(|j| j.cookie_header_for_str("https://chat.zalo.me"))
            .unwrap_or_default();
        let headers = request.headers_mut();
        headers.insert(
            "origin",
            HeaderValue::from_static("https://chat.zalo.me"),
        );
        headers.insert(
            "accept-language",
            HeaderValue::from_static("en-US,en;q=0.9"),
        );
        headers.insert("cache-control", HeaderValue::from_static("no-cache"));
        headers.insert("pragma", HeaderValue::from_static("no-cache"));
        if let Ok(ua) = HeaderValue::from_str(&ctx.user_agent) {
            headers.insert("user-agent", ua);
        }
        if !cookie_str.is_empty() {
            if let Ok(ck) = HeaderValue::from_str(&cookie_str) {
                headers.insert("cookie", ck);
            }
        }
        if let Ok(parsed_url) = Url::parse(&ws_url) {
            if let Some(host) = parsed_url.host_str() {
                if let Ok(hv) = HeaderValue::from_str(host) {
                    headers.insert("host", hv);
                }
            }
        }

        debug!(url = %ws_url, "Connecting Zalo WebSocket");
        let (ws_stream, _) = match tokio_tungstenite::connect_async(request).await {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "Zalo WebSocket connect failed");
                let _ = event_tx.send(WsEvent::Error(e.to_string()));
                if retry_on_close {
                    if let Some(delay_ms) = policy.can_retry(CloseReason::AbnormalClosure as u16) {
                        if policy.should_rotate(
                            CloseReason::AbnormalClosure as u16,
                            rotate_idx,
                            ctx.ws_urls.len(),
                        ) {
                            rotate_idx += 1;
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => continue,
                            cmd = cmd_rx.recv() => {
                                if matches!(cmd, None | Some(OutboundCommand::Stop)) {
                                    let _ = event_tx.send(WsEvent::Closed {
                                        code: CloseReason::ManualClosure as u16,
                                        reason: "ManualClosure".into(),
                                    });
                                    return;
                                }
                                continue;
                            }
                        }
                    }
                }
                let _ = event_tx.send(WsEvent::Closed {
                    code: CloseReason::AbnormalClosure as u16,
                    reason: e.to_string(),
                });
                return;
            }
        };

        let _ = event_tx.send(WsEvent::Connected);
        let (mut ws_sink, mut ws_read) = ws_stream.split();
        let mut cipher_key: Option<String> = None;
        let mut event_cipher: Option<EventCipher> = None;
        let mut ping_active = false;
        let mut ping_interval =
            tokio::time::interval(Duration::from_millis(policy.ping_interval_ms));
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let (close_code, close_reason, manual_stop) = loop {
            tokio::select! {
                _ = ping_interval.tick(), if ping_active => {
                    let payload = json!({ "eventId": Utc::now().timestamp_millis() });
                    let frame = encode_frame(1, 2, 1, payload.to_string().as_bytes());
                    if let Err(e) = ws_sink.send(Message::Binary(frame.into())).await {
                        warn!(error = %e, "WS ping send failed");
                        break (CloseReason::AbnormalClosure as u16, e.to_string(), false);
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        None | Some(OutboundCommand::Stop) => {
                            let _ = ws_sink.send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Normal,
                                reason: "ManualClosure".into(),
                            }))).await;
                            break (CloseReason::ManualClosure as u16, "ManualClosure".into(), true);
                        }
                        Some(OutboundCommand::SendFrame { version, cmd, sub_cmd, mut data, require_id }) => {
                            if require_id {
                                if let Some(obj) = data.as_object_mut() {
                                    let id = req_seq.fetch_add(1, Ordering::Relaxed);
                                    obj.insert("req_id".to_string(), Value::String(format!("req_{id}")));
                                }
                            }
                            let frame = encode_frame(version, cmd, sub_cmd, data.to_string().as_bytes());
                            if let Err(e) = ws_sink.send(Message::Binary(frame.into())).await {
                                let _ = event_tx.send(WsEvent::Error(e.to_string()));
                            }
                        }
                    }
                }
                msg = ws_read.next() => {
                    match msg {
                        Some(Ok(Message::Binary(bytes))) => {
                            let (events, frame_report) = dispatch_frame_traced_with_cipher(
                                &bytes,
                                &mut cipher_key,
                                &mut event_cipher,
                                &ctx,
                                self_listen,
                            );
                            if let Some(report) = frame_report {
                                let _ = event_tx.send(WsEvent::Frame(report));
                            }
                            let mut duplicate = false;
                            for ev in events {
                                match &ev {
                                    WsEvent::CipherKey(_) => {
                                        ping_active = true;
                                        ping_interval = tokio::time::interval(Duration::from_millis(policy.ping_interval_ms));
                                        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                                        // Consume immediate tick so first ping fires after ping_interval_ms
                                        ping_interval.tick().await;
                                    }
                                    WsEvent::Closed { code, .. } if *code == CloseReason::DuplicateConnection as u16 => {
                                        duplicate = true;
                                    }
                                    _ => {}
                                }
                                if !duplicate {
                                    let _ = event_tx.send(ev);
                                }
                            }
                            if duplicate {
                                error!("Another Zalo connection is opened (cmd=3000), closing this one");
                                let _ = ws_sink.send(Message::Close(Some(CloseFrame {
                                    code: CloseCode::Library(CloseReason::DuplicateConnection as u16),
                                    reason: "DuplicateConnection".into(),
                                }))).await;
                                break (CloseReason::DuplicateConnection as u16, "DuplicateConnection".into(), false);
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = ws_sink.send(Message::Pong(payload)).await;
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let (code, reason) = match frame {
                                Some(f) => (u16::from(f.code), f.reason.to_string()),
                                None => (CloseReason::AbnormalClosure as u16, String::new()),
                            };
                            break (code, reason, false);
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            let _ = event_tx.send(WsEvent::Error(e.to_string()));
                            break (CloseReason::AbnormalClosure as u16, e.to_string(), false);
                        }
                        None => {
                            break (CloseReason::AbnormalClosure as u16, "EOF".into(), false);
                        }
                    }
                }
            }
        };

        if manual_stop {
            let _ = event_tx.send(WsEvent::Closed {
                code: close_code,
                reason: close_reason,
            });
            return;
        }

        if retry_on_close {
            if let Some(delay_ms) = policy.can_retry(close_code) {
                if policy.should_rotate(close_code, rotate_idx, ctx.ws_urls.len()) {
                    rotate_idx += 1;
                    info!(rotate_idx, "Rotating Zalo WebSocket endpoint");
                }
                info!(code = close_code, delay_ms, "Retrying Zalo WebSocket connection");
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => continue,
                    cmd = cmd_rx.recv() => {
                        if matches!(cmd, None | Some(OutboundCommand::Stop)) {
                            let _ = event_tx.send(WsEvent::Closed {
                                code: CloseReason::ManualClosure as u16,
                                reason: "ManualClosure".into(),
                            });
                            return;
                        }
                        continue;
                    }
                }
            }
        }

        let _ = event_tx.send(WsEvent::Closed {
            code: close_code,
            reason: close_reason,
        });
        return;
    }
}

/// Decodes the 4-byte binary header: `[version: u8, cmd: u16LE, sub_cmd: u8]`.
pub fn decode_frame_header(frame: &[u8]) -> Option<(u8, u16, u8)> {
    if frame.len() < 4 {
        return None;
    }
    let version = frame[0];
    let cmd = u16::from_le_bytes([frame[1], frame[2]]);
    let sub_cmd = frame[3];
    Some((version, cmd, sub_cmd))
}

/// Encodes a Zalo binary WebSocket frame: `u8 ver | u16LE cmd | u8 sub_cmd | json_bytes`.
pub fn encode_frame(version: u8, cmd: u16, sub_cmd: u8, json_bytes: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + json_bytes.len());
    buf.push(version);
    buf.extend_from_slice(&cmd.to_le_bytes());
    buf.push(sub_cmd);
    buf.extend_from_slice(json_bytes);
    buf
}

/// Pure frame dispatcher: decodes a binary WS frame, updates `cipher_key` and `ctx.upload_callbacks`,
/// and returns all resulting [`WsEvent`]s.
pub fn dispatch_frame(
    frame: &[u8],
    cipher_key: &mut Option<String>,
    ctx: &ZaloContext,
    self_listen: bool,
) -> Vec<WsEvent> {
    dispatch_frame_traced(frame, cipher_key, ctx, self_listen).0
}

/// Like [`dispatch_frame`], and also returns the structure-only [`FrameReport`] for the socket
/// log (`None` when the frame has no usable header). The report never contains message text.
pub fn dispatch_frame_traced(
    frame: &[u8],
    cipher_key: &mut Option<String>,
    ctx: &ZaloContext,
    self_listen: bool,
) -> (Vec<WsEvent>, Option<FrameReport>) {
    dispatch_frame_traced_with_cipher(frame, cipher_key, &mut None, ctx, self_listen)
}

fn dispatch_frame_traced_with_cipher(
    frame: &[u8],
    cipher_key: &mut Option<String>,
    event_cipher: &mut Option<EventCipher>,
    ctx: &ZaloContext,
    self_listen: bool,
) -> (Vec<WsEvent>, Option<FrameReport>) {
    let Some((version, cmd, sub_cmd)) = decode_frame_header(frame) else {
        return (Vec::new(), None);
    };
    let mut report = FrameReport::new(version, cmd, sub_cmd);
    let events = dispatch_body(
        frame,
        (version, cmd, sub_cmd),
        cipher_key,
        event_cipher,
        ctx,
        self_listen,
        &mut report,
    );
    (events, Some(report))
}

fn dispatch_body(
    frame: &[u8],
    (version, cmd, sub_cmd): (u8, u16, u8),
    cipher_key: &mut Option<String>,
    event_cipher: &mut Option<EventCipher>,
    ctx: &ZaloContext,
    self_listen: bool,
    report: &mut FrameReport,
) -> Vec<WsEvent> {
    let body = &frame[4..];
    if body.is_empty() {
        report.error = Some("empty body");
        return Vec::new();
    }
    let Ok(text) = std::str::from_utf8(body) else {
        report.error = Some("body not utf-8");
        return vec![WsEvent::Error("WebSocket frame body is not valid UTF-8".into())];
    };
    let parsed: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            report.error = Some("body not json");
            return vec![WsEvent::Error(format!("WebSocket JSON parse error: {e}"))];
        }
    };

    // Handshake cipher key (v=1, cmd=1, sub=1)
    if version == 1 && cmd == 1 && sub_cmd == 1 {
        if let Some(key) = parsed.get("key").and_then(|v| v.as_str()) {
            *cipher_key = Some(key.to_string());
            *event_cipher = EventCipher::from_base64_key(key).ok();
            return vec![WsEvent::CipherKey(key.to_string())];
        }
    }

    // Ping reply (v=1, cmd=2): the server echoes our keepalive `{ eventId }` unencrypted. Nothing to
    // decode; describe it so the socket log shows a ping, not a decode failure.
    if version == 1 && cmd == 2 {
        report.describe_payload(&parsed, &parsed);
        return Vec::new();
    }

    // Duplicate connection (v=1, cmd=3000, sub=0)
    if version == 1 && cmd == 3000 && sub_cmd == 0 {
        return vec![WsEvent::Closed {
            code: CloseReason::DuplicateConnection as u16,
            reason: "DuplicateConnection".into(),
        }];
    }

    // Helper to decode `{ encrypt, data }` event payloads
    let decode_inner = || -> Result<Value> {
        let encrypt = parsed
            .get("encrypt")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ProtoError::WebSocket("missing encrypt field".into()))?
            as u8;
        let data = parsed
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ProtoError::WebSocket("missing data string".into()))?;
        if event_cipher.is_some() {
            decode_event_payload_with_cipher(encrypt, data, event_cipher.as_ref())
        } else {
            decode_event_payload(encrypt, data, cipher_key.as_deref())
        }
    };

    let own_id = ctx.uid.as_str();
    let mut events = Vec::new();

    let decoded = decode_inner();
    let inner_ref = decoded.as_ref().ok().map(unwrap_data);
    if let (Ok(d), Some(inner)) = (decoded.as_ref(), inner_ref) {
        report.describe_payload(d, inner);
    } else {
        report.error = Some("payload decode failed");
        let why = decoded.as_ref().err().map(|e| e.to_string()).unwrap_or_default();
        report.describe_envelope(&parsed, &why);
    }
    // `pageMsgs` (Official Account / ZNS / OTP) are read on 501 and 510 whatever the subCmd.
    let has_page_msgs = inner_ref
        .and_then(|i| i.get("pageMsgs"))
        .is_some_and(Value::is_array);
    let ctl = ListCtx { own_id, self_listen };

    match (version, cmd, sub_cmd) {
        // Live 1:1 messages + pageMsgs (OA/ZNS/OTP)
        (1, 501, 0) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                let mut got = Collected::default();
                collect_list(report, inner, "msgs", ThreadType::User, ListMode::WsLive, &ctl, &mut got);
                collect_list(report, inner, "pageMsgs", ThreadType::User, ListMode::Page, &ctl, &mut got);
                got.push_live(&mut events);
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // pageMsgs on 501 with another subCmd: the TS port captured them (any subCmd). `msgs` stay
        // with the (1, 501, 0) arm only, as in TS, so nothing is emitted twice.
        (1, 501, _) if has_page_msgs => {
            let inner = inner_ref.unwrap_or(&parsed);
            let mut got = Collected::default();
            collect_list(report, inner, "pageMsgs", ThreadType::User, ListMode::Page, &ctl, &mut got);
            got.push_live(&mut events);
        }

        // Live Group messages (+ any pageMsgs)
        (1, 521, 0) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                let mut got = Collected::default();
                collect_list(report, inner, "groupMsgs", ThreadType::Group, ListMode::WsLive, &ctl, &mut got);
                collect_list(report, inner, "pageMsgs", ThreadType::User, ListMode::Page, &ctl, &mut got);
                got.push_live(&mut events);
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Old 1:1 messages + pageMsgs
        (_, 510, 1) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                let mut got = Collected::default();
                collect_list(report, inner, "msgs", ThreadType::User, ListMode::WsOld, &ctl, &mut got);
                collect_list(report, inner, "pageMsgs", ThreadType::User, ListMode::Page, &ctl, &mut got);
                events.push(WsEvent::OldMessages {
                    thread_type: ThreadType::User,
                    messages: got.msgs,
                });
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // pageMsgs on 510 with another subCmd: replayed history like 510/1, but not part of the
        // paged replay (no cursor), so it gets its own event.
        (_, 510, _) if has_page_msgs => {
            let inner = inner_ref.unwrap_or(&parsed);
            let mut got = Collected::default();
            collect_list(report, inner, "pageMsgs", ThreadType::User, ListMode::Page, &ctl, &mut got);
            if !got.msgs.is_empty() {
                events.push(WsEvent::OldPageMessages(got.msgs));
            }
        }

        // Old Group messages
        (_, 511, 1) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                let mut got = Collected::default();
                collect_list(report, inner, "groupMsgs", ThreadType::Group, ListMode::WsOld, &ctl, &mut got);
                events.push(WsEvent::OldMessages {
                    thread_type: ThreadType::Group,
                    messages: got.msgs,
                });
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Offline / re-push frames (504, 513, 514, 524, 534, 551, 570)
        (1, c, _) if OFFLINE_PUSH_CMDS.contains(&c) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                let mut got = Collected::default();
                for (key, ttype) in [
                    ("pageMsgs", ThreadType::User),
                    ("msgs", ThreadType::User),
                    ("groupMsgs", ThreadType::Group),
                ] {
                    collect_list(report, inner, key, ttype, ListMode::Page, &ctl, &mut got);
                }
                if !got.msgs.is_empty() {
                    events.push(WsEvent::Messages(got.msgs));
                }
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Delivered & seen (User)
        (_, 502, 0) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                if let Some(deliv) = inner.get("delivereds").and_then(|v| v.as_array()) {
                    if !deliv.is_empty() {
                        events.push(WsEvent::Delivered {
                            thread_type: ThreadType::User,
                            data: Value::Array(deliv.clone()),
                        });
                    }
                }
                if let Some(seens) = inner.get("seens").and_then(|v| v.as_array()) {
                    if !seens.is_empty() {
                        events.push(WsEvent::Seen {
                            thread_type: ThreadType::User,
                            data: Value::Array(seens.clone()),
                        });
                    }
                }
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Delivered & seen (Group)
        (_, 522, 0) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                if let Some(deliv) = inner.get("delivereds").and_then(|v| v.as_array()) {
                    let filtered: Vec<Value> = deliv
                        .iter()
                        .filter(|d| {
                            if self_listen {
                                return true;
                            }
                            let is_self = d
                                .get("deliveredUids")
                                .and_then(|v| v.as_array())
                                .map(|arr| arr.iter().any(|u| json_as_string(u) == own_id))
                                .unwrap_or(false);
                            !is_self
                        })
                        .cloned()
                        .collect();
                    if !filtered.is_empty() {
                        events.push(WsEvent::Delivered {
                            thread_type: ThreadType::Group,
                            data: Value::Array(filtered),
                        });
                    }
                }
                if let Some(seens) = inner.get("groupSeens").and_then(|v| v.as_array()) {
                    let filtered: Vec<Value> = seens
                        .iter()
                        .filter(|s| {
                            if self_listen {
                                return true;
                            }
                            let is_self = s
                                .get("seenUids")
                                .and_then(|v| v.as_array())
                                .map(|arr| arr.iter().any(|u| json_as_string(u) == own_id))
                                .unwrap_or(false);
                            !is_self
                        })
                        .cloned()
                        .collect();
                    if !filtered.is_empty() {
                        events.push(WsEvent::Seen {
                            thread_type: ThreadType::Group,
                            data: Value::Array(filtered),
                        });
                    }
                }
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Controls (file_done, group, fr, voip, history sync)
        (1, 601, 0) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                if let Some(controls) = inner.get("controls").and_then(|v| v.as_array()) {
                    for control in controls {
                        let Some(content) = control.get("content") else {
                            continue;
                        };
                        let act_type = content
                            .get("act_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let act = content.get("act").and_then(|v| v.as_str()).unwrap_or("");

                        match act_type {
                            "file_done" => {
                                let file_id = content
                                    .get("fileId")
                                    .map(json_as_string)
                                    .unwrap_or_default();
                                let file_url = content
                                    .get("data")
                                    .and_then(|d| d.get("url"))
                                    .and_then(|u| u.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if !file_id.is_empty() {
                                    let payload = FileDonePayload {
                                        file_url,
                                        file_id: file_id.clone(),
                                    };
                                    if let Ok(mut map) = ctx.upload_callbacks.lock() {
                                        if let Some(tx) = map.remove(&file_id) {
                                            let _ = tx.send(payload);
                                        }
                                    }
                                }
                            }
                            "group" => {
                                if act == "join_reject" {
                                    continue;
                                }
                                let data_val = parse_maybe_json_value(content.get("data"));
                                let thread_id = data_val
                                    .get("group_id")
                                    .or_else(|| data_val.get("groupId"))
                                    .map(json_as_string)
                                    .unwrap_or_default();
                                let is_self = group_event_is_self(act, &data_val, own_id);
                                if !is_self || self_listen {
                                    events.push(WsEvent::GroupEvent(json!({
                                        "type": act,
                                        "act": act,
                                        "threadId": thread_id,
                                        "isSelf": is_self,
                                        "data": data_val,
                                    })));
                                }
                            }
                            "fr" => {
                                if act == "req" {
                                    continue;
                                }
                                let data_val = parse_maybe_json_value(content.get("data"));
                                events.push(WsEvent::FriendEvent(json!({
                                    "type": act,
                                    "act": act,
                                    "data": data_val,
                                })));
                            }
                            "voip" => {
                                let data_val = parse_maybe_json_value(content.get("data"));
                                events.push(WsEvent::VoipControl {
                                    cmd,
                                    sub_cmd,
                                    control: json!({
                                        "act": act,
                                        "data": data_val,
                                        "raw": control,
                                    }),
                                });
                            }
                            other if SYNC_ACT_TYPES.contains(&other) => {
                                let data_val = parse_maybe_json_value(content.get("data"));
                                events.push(WsEvent::HistorySyncControl {
                                    cmd,
                                    sub_cmd,
                                    control: json!({
                                        "actType": other,
                                        "act": act,
                                        "data": data_val,
                                        "raw": control,
                                    }),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Typing actions
        (_, 602, 0) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                if let Some(actions) = inner.get("actions").and_then(|v| v.as_array()) {
                    for action in actions {
                        let act_type = action
                            .get("act_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if act_type != "typing" {
                            continue;
                        }
                        let act = action.get("act").and_then(|v| v.as_str()).unwrap_or("");
                        let raw_data = action.get("data").and_then(|v| v.as_str()).unwrap_or("");
                        let wrapped = if raw_data.trim_start().starts_with('{') {
                            raw_data.to_string()
                        } else {
                            format!("{{{raw_data}}}")
                        };
                        let Ok(data) = parse_json_keeping_bigints(&wrapped) else {
                            continue;
                        };
                        let uid = data.get("uid").map(json_as_string).unwrap_or_default();
                        let is_pc = data
                            .get("isPC")
                            .and_then(|v| v.as_i64())
                            .map(|n| n != 0)
                            .or_else(|| data.get("isPC").and_then(|v| v.as_bool()))
                            .unwrap_or(false);
                        if act == "typing" && !uid.is_empty() {
                            events.push(WsEvent::Typing {
                                thread_type: ThreadType::User,
                                thread_id: uid.clone(),
                                uid,
                                is_pc,
                            });
                        } else if act == "gtyping" {
                            let gid = data.get("gid").map(json_as_string).unwrap_or_default();
                            if !gid.is_empty() && !uid.is_empty() {
                                events.push(WsEvent::Typing {
                                    thread_type: ThreadType::Group,
                                    thread_id: gid,
                                    uid,
                                    is_pc,
                                });
                            }
                        }
                    }
                }
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Live reactions (612)
        (_, 612, _) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                for (key, ttype) in [
                    ("reacts", ThreadType::User),
                    ("reactGroups", ThreadType::Group),
                ] {
                    if let Some(arr) = inner.get(key).and_then(|v| v.as_array()) {
                        let mut out = Vec::new();
                        for r in arr {
                            if let Some(norm) = normalize_reaction(r, ttype, own_id, true) {
                                let is_self = norm
                                    .get("isSelf")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                if !is_self || self_listen {
                                    out.push(norm);
                                }
                            }
                        }
                        if !out.is_empty() {
                            events.push(WsEvent::Reactions {
                                thread_type: ttype,
                                reactions: out,
                                is_old: false,
                            });
                        }
                    }
                }
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Old reactions (610 = User, 611 = Group)
        (_, c @ (610 | 611), _) => match decoded.as_ref() {
            Ok(decoded) => {
                let inner = unwrap_data(decoded);
                let ttype = if c == 611 {
                    ThreadType::Group
                } else {
                    ThreadType::User
                };
                let key = if c == 611 { "reactGroups" } else { "reacts" };
                let reactions = inner
                    .get(key)
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|r| normalize_reaction(r, ttype, own_id, false))
                            .collect()
                    })
                    .unwrap_or_default();
                events.push(WsEvent::Reactions {
                    thread_type: ttype,
                    reactions,
                    is_old: true,
                });
            }
            Err(e) => events.push(WsEvent::Error(e.to_string())),
        },

        // Any other frame -> RawFrame
        _ => {
            report.consumed = false;
            let payload = decoded.unwrap_or(parsed);
            events.push(WsEvent::RawFrame {
                cmd,
                sub_cmd,
                payload,
            });
        }
    }

    events
}

fn unwrap_data(v: &Value) -> &Value {
    match v.get("data") {
        Some(inner) if inner.is_object() => inner,
        _ => v,
    }
}

/// Per-frame settings shared by the list handlers.
struct ListCtx<'a> {
    own_id: &'a str,
    self_listen: bool,
}

/// How an array of message items is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ListMode {
    /// `msgs` / `groupMsgs` on live frames: undo items become `Undo` events, e2ee is masked.
    WsLive,
    /// `msgs` / `groupMsgs` on history frames: everything is a message, e2ee is masked.
    WsOld,
    /// `pageMsgs` and offline re-push arrays (port of `toRawMessage`): undo items are skipped.
    Page,
}

#[derive(Default)]
struct Collected {
    msgs: Vec<RawWsMessage>,
    undos: Vec<RawWsMessage>,
}

impl Collected {
    /// Pushes live events: undos first, then one `Messages` batch (if any).
    fn push_live(self, events: &mut Vec<WsEvent>) {
        events.extend(self.undos.into_iter().map(WsEvent::Undo));
        if !self.msgs.is_empty() {
            events.push(WsEvent::Messages(self.msgs));
        }
    }
}

/// Reads `inner[name]` (if it is an array) as `mode` says, collects the resulting messages and
/// records one [`ItemReport`] per item in `report` (outcome and drop reason, never content).
fn collect_list(
    report: &mut FrameReport,
    inner: &Value,
    name: &str,
    thread_type: ThreadType,
    mode: ListMode,
    ctl: &ListCtx<'_>,
    out: &mut Collected,
) {
    let Some(list) = inner.get(name).and_then(|v| v.as_array()) else {
        return;
    };
    let mut items = Vec::with_capacity(list.len().min(MAX_ITEMS));
    for item in list {
        let mut ir = (items.len() < MAX_ITEMS).then(|| ItemReport::describe(item, ctl.own_id));
        let undo = is_undo_content(item.get("content"));
        match mode {
            ListMode::WsLive | ListMode::WsOld => {
                if !item.is_object() {
                    if let Some(ir) = ir.as_mut() {
                        ir.dropped("not an object");
                    }
                } else if mode == ListMode::WsLive && undo {
                    match normalize_ws_message(item, thread_type, ctl.own_id, false) {
                        Some(u) if !u.is_self || ctl.self_listen => {
                            out.undos.push(u);
                            if let Some(ir) = ir.as_mut() {
                                ir.undo();
                            }
                        }
                        _ => {
                            if let Some(ir) = ir.as_mut() {
                                ir.dropped("own undo (selfListen off)");
                            }
                        }
                    }
                } else {
                    match normalize_ws_message(item, thread_type, ctl.own_id, true) {
                        Some(m) if mode == ListMode::WsOld || !m.is_self || ctl.self_listen => {
                            if m.thread_id.is_empty() || m.thread_id == "0" {
                                // Kept as before; the store decides. Flag it for the log.
                                if let Some(ir) = ir.as_mut() {
                                    ir.note = Some("thread id empty/0");
                                }
                            }
                            out.msgs.push(m);
                            if let Some(ir) = ir.as_mut() {
                                ir.kept();
                            }
                        }
                        _ => {
                            if let Some(ir) = ir.as_mut() {
                                ir.dropped("own message (selfListen off)");
                            }
                        }
                    }
                }
            }
            ListMode::Page => match try_normalize_page(item, thread_type, ctl.own_id) {
                Ok(m) => {
                    if let Some(ir) = ir.as_mut() {
                        if is_e2ee_type(item.get("msgType").and_then(|v| v.as_str())) {
                            ir.note = Some("e2ee-masked");
                        }
                        ir.kept();
                    }
                    out.msgs.push(m);
                }
                Err(reason) => {
                    if let Some(ir) = ir.as_mut() {
                        ir.dropped(reason);
                    }
                }
            },
        }
        if let Some(ir) = ir {
            items.push(ir);
        }
    }
    report.set_items(name, items);
}

fn is_undo_content(content: Option<&Value>) -> bool {
    content
        .and_then(|c| c.as_object())
        .map(|obj| obj.contains_key("deleteMsg"))
        .unwrap_or(false)
}

fn is_e2ee_type(msg_type: Option<&str>) -> bool {
    let Some(mt) = msg_type else { return false };
    mt.split('.')
        .any(|part| part.eq_ignore_ascii_case("e2ee"))
}

fn json_as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn parse_maybe_json_value(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                parse_json_keeping_bigints(trimmed).unwrap_or_else(|_| Value::String(s.clone()))
            } else {
                Value::String(s.clone())
            }
        }
        Some(other) => other.clone(),
        None => json!({}),
    }
}

fn normalize_ws_message(
    item: &Value,
    thread_type: ThreadType,
    own_id: &str,
    mask_e2ee: bool,
) -> Option<RawWsMessage> {
    let mut obj = item.as_object()?.clone();
    let raw_uid_from = obj.get("uidFrom").map(json_as_string).unwrap_or_default();
    let raw_id_to = obj.get("idTo").map(json_as_string).unwrap_or_default();
    let is_self = raw_uid_from == "0";
    let thread_id = match thread_type {
        ThreadType::Group => raw_id_to.clone(),
        ThreadType::User => {
            if is_self {
                raw_id_to.clone()
            } else {
                raw_uid_from.clone()
            }
        }
    };

    if raw_id_to == "0" {
        obj.insert("idTo".to_string(), Value::String(own_id.to_string()));
    }
    if raw_uid_from == "0" {
        obj.insert("uidFrom".to_string(), Value::String(own_id.to_string()));
    }
    if let Some(quote) = obj.get_mut("quote").and_then(|q| q.as_object_mut()) {
        if let Some(owner) = quote.get("ownerId") {
            quote.insert("ownerId".to_string(), Value::String(json_as_string(owner)));
        }
    }
    if mask_e2ee && is_e2ee_type(obj.get("msgType").and_then(|v| v.as_str())) {
        obj.insert(
            "content".to_string(),
            Value::String("[Encrypted message]".to_string()),
        );
    }

    Some(RawWsMessage {
        thread_type,
        thread_id,
        is_self,
        data: Value::Object(obj),
    })
}

/// Port of `src/oamsg.ts:toRawMessage` for `pageMsgs` and offline re-push commands.
///
/// Like TS, `??` semantics: a JSON `null` id falls through to the next candidate.
#[cfg(test)]
fn normalize_page_or_offline_message(
    item: &Value,
    thread_type: ThreadType,
    own_id: &str,
) -> Option<RawWsMessage> {
    try_normalize_page(item, thread_type, own_id).ok()
}

/// `normalize_page_or_offline_message` with the reason an item was dropped.
fn try_normalize_page(
    item: &Value,
    thread_type: ThreadType,
    own_id: &str,
) -> std::result::Result<RawWsMessage, &'static str> {
    let obj = item.as_object().ok_or("not an object")?;
    let msg_id = ["msgId", "globalMsgId", "cliMsgId"]
        .iter()
        .filter_map(|k| obj.get(*k))
        .find(|v| !v.is_null())
        .map(json_as_string)
        .filter(|s| !s.is_empty())
        .ok_or("no msgId")?;
    if is_undo_content(obj.get("content")) {
        return Err("undo content");
    }

    let raw_uid_from = obj.get("uidFrom").map(json_as_string).unwrap_or_default();
    let raw_id_to = obj.get("idTo").map(json_as_string).unwrap_or_default();
    let is_self = raw_uid_from == "0" || (!own_id.is_empty() && raw_uid_from == own_id);
    let thread_id = match thread_type {
        ThreadType::Group => raw_id_to.clone(),
        ThreadType::User => {
            if is_self {
                raw_id_to.clone()
            } else {
                raw_uid_from.clone()
            }
        }
    };
    if thread_id.is_empty() || thread_id == "0" {
        return Err("thread id empty/0");
    }

    let uid_from = if raw_uid_from == "0" {
        own_id.to_string()
    } else {
        raw_uid_from
    };
    let id_to = if raw_id_to == "0" {
        own_id.to_string()
    } else {
        raw_id_to
    };

    // TS: `String(m.msgType ?? "webchat")`.
    let msg_type = match obj.get("msgType") {
        Some(Value::String(s)) => s.clone(),
        Some(v @ (Value::Number(_) | Value::Bool(_))) => v.to_string(),
        _ => "webchat".to_string(),
    };
    let content = if is_e2ee_type(Some(&msg_type)) {
        Value::String("[Encrypted message]".to_string())
    } else {
        obj.get("content").cloned().unwrap_or(Value::Null)
    };

    let d_name = obj
        .get("dName")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("oaName").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let cli_msg_id = obj
        .get("cliMsgId")
        .map(json_as_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| msg_id.clone());
    // TS: only a string or a number is a usable `ts`, anything else becomes "now".
    let ts = obj
        .get("ts")
        .filter(|v| v.is_string() || v.is_number())
        .cloned()
        .unwrap_or_else(|| Value::Number(Utc::now().timestamp_millis().into()));

    let mut data_map = serde_json::Map::new();
    data_map.insert("msgId".to_string(), Value::String(msg_id));
    data_map.insert("cliMsgId".to_string(), Value::String(cli_msg_id));
    data_map.insert("msgType".to_string(), Value::String(msg_type));
    data_map.insert("uidFrom".to_string(), Value::String(uid_from));
    data_map.insert("idTo".to_string(), Value::String(id_to));
    data_map.insert("dName".to_string(), Value::String(d_name));
    data_map.insert("ts".to_string(), ts);
    data_map.insert("content".to_string(), content);

    for opt_key in ["propertyExt", "ttl", "st", "at", "cmd", "mentions"] {
        if let Some(v) = obj.get(opt_key) {
            data_map.insert(opt_key.to_string(), v.clone());
        }
    }
    if let Some(q) = obj.get("quote").and_then(|v| v.as_object()) {
        let mut qm = serde_json::Map::new();
        qm.insert(
            "ownerId".to_string(),
            Value::String(q.get("ownerId").map(json_as_string).unwrap_or_default()),
        );
        qm.insert(
            "globalMsgId".to_string(),
            Value::String(q.get("globalMsgId").map(json_as_string).unwrap_or_default()),
        );
        for k in ["fromD", "msg", "attach"] {
            if let Some(s) = q.get(k).and_then(|v| v.as_str()) {
                qm.insert(k.to_string(), Value::String(s.to_string()));
            }
        }
        data_map.insert("quote".to_string(), Value::Object(qm));
    }

    Ok(RawWsMessage {
        thread_type,
        thread_id,
        is_self,
        data: Value::Object(data_map),
    })
}

fn normalize_reaction(
    item: &Value,
    thread_type: ThreadType,
    own_id: &str,
    parse_content_str: bool,
) -> Option<Value> {
    let mut obj = item.as_object()?.clone();
    if parse_content_str {
        if let Some(Value::String(s)) = obj.get("content") {
            if let Ok(parsed) = parse_json_keeping_bigints(s) {
                obj.insert("content".to_string(), parsed);
            }
        }
    }
    let raw_uid_from = obj.get("uidFrom").map(json_as_string).unwrap_or_default();
    let raw_id_to = obj.get("idTo").map(json_as_string).unwrap_or_default();
    let is_self = raw_uid_from == "0";
    let is_group = thread_type.is_group();
    let thread_id = if is_group || is_self {
        raw_id_to.clone()
    } else {
        raw_uid_from.clone()
    };
    if raw_id_to == "0" {
        obj.insert("idTo".to_string(), Value::String(own_id.to_string()));
    }
    if raw_uid_from == "0" {
        obj.insert("uidFrom".to_string(), Value::String(own_id.to_string()));
    }
    Some(json!({
        "threadId": thread_id,
        "isSelf": is_self,
        "isGroup": is_group,
        "data": Value::Object(obj),
    }))
}

fn group_event_is_self(act: &str, data: &Value, own_id: &str) -> bool {
    match act {
        "join_request" => false,
        "new_pin_topic" | "unpin_topic" | "update_pin_topic" | "reorder_pin_topic" => {
            data.get("actorId").map(json_as_string).as_deref() == Some(own_id)
        }
        "update_board" | "remove_board" => {
            data.get("sourceId").map(json_as_string).as_deref() == Some(own_id)
        }
        "accept_remind" | "reject_remind" => data
            .get("updateMembers")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().any(|m| json_as_string(m) == own_id))
            .unwrap_or(false),
        "remind_topic" => data.get("creatorId").map(json_as_string).as_deref() == Some(own_id),
        _ => {
            let in_members = data
                .get("updateMembers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|m| m.get("id").map(json_as_string).as_deref() == Some(own_id))
                })
                .unwrap_or(false);
            let is_source = data.get("sourceId").map(json_as_string).as_deref() == Some(own_id);
            in_members || is_source
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::oneshot;
    use crate::cookies::ZaloCookieJar;

    fn test_ctx() -> ZaloContext {
        ZaloContext {
            uid: "10001".to_string(),
            imei: "imei-test".to_string(),
            user_agent: "UA".to_string(),
            language: "vi".to_string(),
            api_type: 30,
            api_version: 685,
            secret_key: "MDEyMzQ1Njc4OWFiY2RlZg==".to_string(),
            service_map: HashMap::new(),
            ws_urls: vec![],
            ctrs_urls: vec![],
            settings: json!({}),
            extra_ver: json!({}),
            login_info: json!({}),
            cookies: Arc::new(std::sync::RwLock::new(ZaloCookieJar::new())),
            upload_callbacks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn plain_frame(cmd: u16, sub_cmd: u8, inner: Value) -> Vec<u8> {
        let outer = json!({
            "encrypt": 0,
            "data": inner.to_string(),
        });
        encode_frame(1, cmd, sub_cmd, outer.to_string().as_bytes())
    }

    #[test]
    fn frame_501_handles_msgs_undo_e2ee_and_page_msgs() {
        let ctx = test_ctx();
        let mut ck = None;
        let frame = plain_frame(
            501,
            0,
            json!({
                "data": {
                    "msgs": [
                        {
                            "msgId": "111",
                            "cliMsgId": "111",
                            "msgType": "webchat",
                            "uidFrom": "20002",
                            "idTo": "0",
                            "content": "hello",
                            "ts": 1700000000000i64
                        },
                        {
                            "msgId": "112",
                            "cliMsgId": "112",
                            "msgType": "chat.e2ee.text",
                            "uidFrom": "20002",
                            "idTo": "0",
                            "content": "cipherblob",
                            "ts": 1700000000001i64
                        },
                        {
                            "msgId": "113",
                            "cliMsgId": "113",
                            "msgType": "webchat",
                            "uidFrom": "20002",
                            "idTo": "0",
                            "content": { "deleteMsg": 1, "globalMsgId": "111" },
                            "ts": 1700000000002i64
                        }
                    ],
                    "pageMsgs": [
                        {
                            "msgId": "222",
                            "msgType": "webchat",
                            "uidFrom": "99999",
                            "idTo": "0",
                            "oaName": "Zalo OTP",
                            "content": "Your code is 123456",
                            "ts": 1700000000003i64
                        }
                    ]
                }
            }),
        );

        let events = dispatch_frame(&frame, &mut ck, &ctx, true);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], WsEvent::Undo(_)));
        match &events[1] {
            WsEvent::Messages(list) => {
                assert_eq!(list.len(), 3);
                assert_eq!(list[0].thread_id, "20002");
                assert_eq!(list[0].data["idTo"], "10001");
                assert_eq!(list[1].data["content"], "[Encrypted message]");
                assert_eq!(list[2].thread_id, "99999");
                assert_eq!(list[2].data["dName"], "Zalo OTP");
            }
            other => panic!("expected Messages, got {other:?}"),
        }
    }

    fn oa_item(id: &str, text: &str) -> Value {
        json!({
            "msgId": id,
            "msgType": "chat.webcontent",
            "uidFrom": "99999",
            "idTo": "0",
            "oaName": "Some OA",
            "content": { "title": text, "href": "https://example.invalid/x" },
            "ts": 1700000000003i64
        })
    }

    #[test]
    fn page_msgs_on_501_with_other_sub_cmd_are_captured_once() {
        let ctx = test_ctx();
        let mut ck = None;
        // `msgs` next to `pageMsgs` on a non-zero subCmd must NOT be re-emitted (TS did not either).
        let frame = plain_frame(
            501,
            3,
            json!({ "data": {
                "msgs": [{ "msgId": "1", "uidFrom": "20002", "idTo": "0", "content": "hi", "ts": 1 }],
                "pageMsgs": [oa_item("700", "OTP 123456")]
            }}),
        );
        let events = dispatch_frame(&frame, &mut ck, &ctx, true);
        assert_eq!(events.len(), 1);
        match &events[0] {
            WsEvent::Messages(list) => {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].thread_id, "99999");
                assert_eq!(list[0].data["msgType"], "chat.webcontent");
            }
            other => panic!("expected Messages, got {other:?}"),
        }
        // Without pageMsgs, an unknown 501 subCmd is still a RawFrame.
        let frame = plain_frame(501, 3, json!({ "data": { "msgs": [] } }));
        let events = dispatch_frame(&frame, &mut ck, &ctx, true);
        assert!(matches!(events[0], WsEvent::RawFrame { cmd: 501, sub_cmd: 3, .. }));
    }

    #[test]
    fn page_msgs_on_510_other_sub_cmd_are_history_not_live() {
        let ctx = test_ctx();
        let mut ck = None;
        let frame = plain_frame(510, 2, json!({ "data": { "pageMsgs": [oa_item("701", "x")] } }));
        let events = dispatch_frame(&frame, &mut ck, &ctx, true);
        assert_eq!(events.len(), 1);
        match &events[0] {
            WsEvent::OldPageMessages(list) => assert_eq!(list[0].thread_id, "99999"),
            other => panic!("expected OldPageMessages, got {other:?}"),
        }
        // 510/1 keeps its OldMessages semantics (paged replay).
        let frame = plain_frame(510, 1, json!({ "data": { "pageMsgs": [oa_item("702", "x")] } }));
        let events = dispatch_frame(&frame, &mut ck, &ctx, true);
        match &events[0] {
            WsEvent::OldMessages { messages, .. } => assert_eq!(messages.len(), 1),
            other => panic!("expected OldMessages, got {other:?}"),
        }
        // No pageMsgs: not consumed.
        let frame = plain_frame(510, 2, json!({ "data": { "msgs": [] } }));
        assert!(matches!(
            dispatch_frame(&frame, &mut ck, &ctx, true)[0],
            WsEvent::RawFrame { cmd: 510, .. }
        ));
    }

    #[test]
    fn page_message_normalizer_matches_ts_to_raw_message() {
        // `??`: a null msgId falls through to globalMsgId.
        let m = normalize_page_or_offline_message(
            &json!({ "msgId": null, "globalMsgId": 555, "uidFrom": 99999, "idTo": "0", "content": "x" }),
            ThreadType::User,
            "10001",
        )
        .expect("globalMsgId fallback");
        assert_eq!(m.data["msgId"], "555");
        assert_eq!(m.thread_id, "99999");
        // Numeric msgType is stringified, a junk ts becomes "now", own uid (not just 0) is self.
        let m = normalize_page_or_offline_message(
            &json!({ "msgId": "9", "msgType": 7, "uidFrom": "10001", "idTo": "20002", "ts": {"a": 1}, "content": "x" }),
            ThreadType::User,
            "10001",
        )
        .expect("kept");
        assert_eq!(m.data["msgType"], "7");
        assert!(m.is_self);
        assert_eq!(m.thread_id, "20002");
        assert!(m.data["ts"].is_number());
        // Non-numeric OA ids are fine.
        let m = normalize_page_or_offline_message(
            &json!({ "msgId": "9", "uidFrom": "oa.zalo", "idTo": "0", "content": "x" }),
            ThreadType::User,
            "10001",
        )
        .expect("non numeric uid");
        assert_eq!(m.thread_id, "oa.zalo");
        // Real drops.
        for bad in [
            json!({ "uidFrom": "1", "content": "x" }),
            json!({ "msgId": "1", "uidFrom": "0", "idTo": "0", "content": "x" }),
            json!({ "msgId": "1", "uidFrom": "", "content": "x" }),
            json!({ "msgId": "1", "uidFrom": "5", "content": { "deleteMsg": 1 } }),
            json!("str"),
        ] {
            assert!(normalize_page_or_offline_message(&bad, ThreadType::User, "10001").is_none(), "{bad}");
        }
    }

    #[test]
    fn traced_report_unknown_cmd_is_not_consumed() {
        let ctx = test_ctx();
        let mut ck = None;
        let frame = plain_frame(9999, 4, json!({ "data": { "foo": 1, "list": [1, 2, 3] } }));
        let (events, report) = dispatch_frame_traced(&frame, &mut ck, &ctx, true);
        assert!(matches!(events[0], WsEvent::RawFrame { cmd: 9999, .. }));
        let r = report.expect("report");
        assert_eq!((r.version, r.cmd, r.sub_cmd), (1, 9999, 4));
        assert!(!r.consumed);
        assert_eq!(r.payload_keys, vec!["data"]);
        assert_eq!(r.data_keys, vec!["foo", "list"]);
        assert_eq!(r.arrays.len(), 1);
        assert_eq!((r.arrays[0].name.as_str(), r.arrays[0].len, r.arrays[0].read), ("list", 3, false));
    }

    #[test]
    fn traced_report_handled_501_with_page_msgs_and_drop_reasons() {
        let ctx = test_ctx();
        let mut ck = None;
        let frame = plain_frame(
            501,
            0,
            json!({ "data": {
                "pageMsgs": [
                    oa_item("800", "ok"),
                    { "uidFrom": "99999", "idTo": "0", "content": "no id" },
                    { "msgId": "801", "uidFrom": "0", "idTo": "0", "content": "no thread" },
                    { "msgId": "802", "uidFrom": "99999", "content": { "deleteMsg": 1 } },
                    "junk"
                ],
                "other": [1]
            }}),
        );
        let (events, report) = dispatch_frame_traced(&frame, &mut ck, &ctx, true);
        assert_eq!(events.len(), 1);
        let r = report.expect("report");
        assert!(r.consumed);
        let page = r.arrays.iter().find(|a| a.name == "pageMsgs").expect("pageMsgs");
        assert!(page.read);
        assert_eq!(page.len, 5);
        let outcomes: Vec<(&str, Option<&str>)> = page.items.iter().map(|i| (i.outcome, i.reason)).collect();
        assert_eq!(
            outcomes,
            vec![
                ("message", None),
                ("dropped", Some("no msgId")),
                ("dropped", Some("thread id empty/0")),
                ("dropped", Some("undo content")),
                ("dropped", Some("not an object")),
            ]
        );
        assert_eq!(page.items[0].msg_type.as_deref(), Some("chat.webcontent"));
        assert_eq!(page.items[0].content, "{href,title}");
        assert_eq!(page.items[0].uid_from, "other");
        assert_eq!(page.items[2].uid_from, "zero");
        assert!(!r.arrays.iter().find(|a| a.name == "other").unwrap().read);
    }

    #[test]
    fn traced_report_shows_scalar_values_but_never_string_contents() {
        let ctx = test_ctx();
        let mut ck = None;
        let secret = "SECRET-OTP-482910 Nguyen Van A";
        // A 501 like the one seen when an OTP arrived on the phone: every list empty.
        let frame = plain_frame(
            501,
            0,
            json!({
                "data": {
                    "msgs": [], "pageMsgs": [],
                    "more": 1, "lastActionId": 123456789,
                    "queueStatus": { "pending": 2 },
                    "note": secret,
                },
                "error_code": 0,
                "error_message": secret,
            }),
        );
        let (_events, report) = dispatch_frame_traced(&frame, &mut ck, &ctx, true);
        let r = report.expect("report");
        let n = secret.chars().count();
        for want in [
            "error_code=0".to_string(),
            format!("error_message=str({n})"),
            "more=1".to_string(),
            "lastActionId=123456789".to_string(),
            "queueStatus={pending}".to_string(),
            format!("note=str({n})"),
        ] {
            assert!(r.scalars.contains(&want), "missing {want}: {:?}", r.scalars);
        }
        let rendered = serde_json::to_string(&r).unwrap();
        for needle in ["SECRET", "482910", "Nguyen"] {
            assert!(!rendered.contains(needle), "leaked {needle}: {rendered}");
        }
    }

    #[test]
    fn traced_report_describes_the_wrapper_of_an_undecodable_frame() {
        let ctx = test_ctx();
        let mut ck = Some("MDEyMzQ1Njc4OWFiY2RlZg==".to_string());
        let body = json!({ "encrypt": 2, "data": "SECRET!!not-base64" }).to_string();
        let frame = encode_frame(1, 501, 0, body.as_bytes());
        let (_events, report) = dispatch_frame_traced(&frame, &mut ck, &ctx, true);
        let r = report.expect("report");
        assert_eq!(r.error, Some("payload decode failed"));
        assert!(r.envelope.contains(&"encrypt=2".to_string()), "{:?}", r.envelope);
        assert!(r.envelope.contains(&"data=str(18)".to_string()), "{:?}", r.envelope);
        assert!(r.error_detail.as_deref().is_some_and(|d| !d.is_empty()));
        let rendered = serde_json::to_string(&r).unwrap();
        assert!(!rendered.contains("SECRET"), "leaked: {rendered}");
    }

    #[test]
    fn traced_report_labels_the_ping_reply() {
        let ctx = test_ctx();
        let mut ck = None;
        let frame = encode_frame(1, 2, 1, json!({ "eventId": 1_700_000_000_000_i64 }).to_string().as_bytes());
        let (events, report) = dispatch_frame_traced(&frame, &mut ck, &ctx, true);
        assert!(events.is_empty());
        let r = report.expect("report");
        assert_eq!(r.error, None);
        assert!(r.consumed);
        assert_eq!(r.scalars, vec!["eventId=1700000000000"]);
    }

    #[test]
    fn traced_report_lists_control_acts() {
        let ctx = test_ctx();
        let mut ck = None;
        let frame = plain_frame(
            601,
            0,
            json!({ "data": { "controls": [
                { "content": { "act_type": "group", "act": "join_reject" } },
                { "content": { "act_type": "fr", "act": "req" } }
            ]}}),
        );
        let (_events, report) = dispatch_frame_traced(&frame, &mut ck, &ctx, true);
        let r = report.expect("report");
        assert_eq!(r.acts, vec!["group/join_reject", "fr/req"]);
    }

    #[test]
    fn traced_report_never_contains_text_or_values() {
        let ctx = test_ctx();
        let mut ck = None;
        let secret = "SECRET-OTP-482910 Nguyen Van A 0901234567";
        let frame = plain_frame(
            501,
            0,
            json!({ "data": {
                "msgs": [
                    { "msgId": "1", "uidFrom": "20002", "idTo": "0", "dName": secret, "content": secret, "ts": 1 },
                    { "msgId": "2", "uidFrom": "20002", "idTo": "0", "content": { "title": secret, "description": secret }, "ts": 1 },
                    { "msgId": "3", "uidFrom": "20002", "idTo": "0", "content": format!("{{\"params\":\"{secret}\"}}"), "ts": 1 }
                ],
                "pageMsgs": [{
                    "msgId": "4", "msgType": "chat.webcontent", "uidFrom": "99999", "idTo": "0",
                    "oaName": secret, "content": { "href": secret }, "ts": 1
                }]
            }}),
        );
        let (events, report) = dispatch_frame_traced(&frame, &mut ck, &ctx, true);
        assert!(!events.is_empty());
        let r = report.expect("report");
        let rendered = serde_json::to_string(&r).unwrap();
        for needle in ["SECRET", "482910", "Nguyen", "0901234567", "example"] {
            assert!(!rendered.contains(needle), "leaked {needle}: {rendered}");
        }
        let msgs = r.arrays.iter().find(|a| a.name == "msgs").unwrap();
        assert_eq!(msgs.items[0].content, "string");
        assert_eq!(msgs.items[1].content, "{description,title}");
        assert_eq!(msgs.items[2].content, "string>{params}");
    }

    #[test]
    fn frame_521_and_offline_504_dispatch() {
        let ctx = test_ctx();
        let mut ck = None;

        let f521 = plain_frame(
            521,
            0,
            json!({
                "data": {
                    "groupMsgs": [
                        {
                            "msgId": "301",
                            "uidFrom": "0",
                            "idTo": "55555",
                            "msgType": "webchat",
                            "content": "hi group"
                        }
                    ]
                }
            }),
        );
        // self_listen = false filters out uidFrom == "0"
        assert!(dispatch_frame(&f521, &mut ck, &ctx, false).is_empty());
        let evs = dispatch_frame(&f521, &mut ck, &ctx, true);
        assert_eq!(evs.len(), 1);
        if let WsEvent::Messages(list) = &evs[0] {
            assert_eq!(list[0].thread_type, ThreadType::Group);
            assert_eq!(list[0].thread_id, "55555");
            assert_eq!(list[0].data["uidFrom"], "10001");
        } else {
            panic!("expected Messages");
        }

        // Offline 504 skips undo and masks E2EE
        let f504 = plain_frame(
            504,
            0,
            json!({
                "data": {
                    "msgs": [
                        { "msgId": "401", "uidFrom": "30003", "idTo": "0", "msgType": "e2ee.msg", "content": "enc" },
                        { "msgId": "402", "uidFrom": "30003", "idTo": "0", "msgType": "webchat", "content": { "deleteMsg": 1 } }
                    ],
                    "groupMsgs": [
                        { "msgId": "403", "uidFrom": "30003", "idTo": "77777", "msgType": "webchat", "content": "offline grp" }
                    ]
                }
            }),
        );
        let evs504 = dispatch_frame(&f504, &mut ck, &ctx, true);
        assert_eq!(evs504.len(), 1);
        if let WsEvent::Messages(list) = &evs504[0] {
            assert_eq!(list.len(), 2);
            assert_eq!(list[0].data["content"], "[Encrypted message]");
            assert_eq!(list[1].thread_type, ThreadType::Group);
            assert_eq!(list[1].thread_id, "77777");
        } else {
            panic!("expected Messages");
        }
    }

    #[test]
    fn frame_601_routes_file_done_voip_group_and_sync() {
        let ctx = test_ctx();
        let (tx, mut rx) = oneshot::channel();
        ctx.upload_callbacks
            .lock()
            .unwrap()
            .insert("98765".to_string(), tx);

        let mut ck = None;
        let f601 = plain_frame(
            601,
            0,
            json!({
                "data": {
                    "controls": [
                        {
                            "content": {
                                "act_type": "file_done",
                                "fileId": "98765",
                                "data": { "url": "https://files.zdn.vn/abc.mp4" }
                            }
                        },
                        {
                            "content": {
                                "act_type": "voip",
                                "act": "request",
                                "data": "{\"callId\":123456789,\"callerId\":7234567890123456789}"
                            }
                        },
                        {
                            "content": {
                                "act_type": "syncmsgmb",
                                "act": "user_confirm",
                                "data": "{\"user_action\":1}"
                            }
                        },
                        {
                            "content": {
                                "act_type": "group",
                                "act": "update_board",
                                "data": "{\"group_id\":\"888\",\"sourceId\":\"999\",\"groupTopic\":{\"type\":3,\"params\":\"{\\\"pollId\\\":42}\"}}"
                            }
                        }
                    ]
                }
            }),
        );

        let events = dispatch_frame(&f601, &mut ck, &ctx, true);
        let done = rx.try_recv().unwrap();
        assert_eq!(done.file_id, "98765");
        assert_eq!(done.file_url, "https://files.zdn.vn/abc.mp4");

        assert_eq!(events.len(), 3);
        match &events[0] {
            WsEvent::VoipControl { cmd, control, .. } => {
                assert_eq!(*cmd, 601);
                assert_eq!(control["act"], "request");
                assert_eq!(control["data"]["callerId"], "7234567890123456789");
            }
            other => panic!("expected VoipControl, got {other:?}"),
        }
        match &events[1] {
            WsEvent::HistorySyncControl { control, .. } => {
                assert_eq!(control["actType"], "syncmsgmb");
                assert_eq!(control["act"], "user_confirm");
            }
            other => panic!("expected HistorySyncControl, got {other:?}"),
        }
        match &events[2] {
            WsEvent::GroupEvent(ge) => {
                assert_eq!(ge["threadId"], "888");
                assert_eq!(ge["act"], "update_board");
            }
            other => panic!("expected GroupEvent, got {other:?}"),
        }
    }

    #[test]
    fn frame_602_typing_and_612_reactions() {
        let ctx = test_ctx();
        let mut ck = None;

        let f602 = plain_frame(
            602,
            0,
            json!({
                "data": {
                    "actions": [
                        { "act_type": "typing", "act": "typing", "data": "\"uid\":\"222\",\"isPC\":1" },
                        { "act_type": "typing", "act": "gtyping", "data": "\"gid\":\"777\",\"uid\":\"333\",\"isPC\":0" }
                    ]
                }
            }),
        );
        let evs = dispatch_frame(&f602, &mut ck, &ctx, true);
        assert_eq!(evs.len(), 2);
        assert!(matches!(
            &evs[0],
            WsEvent::Typing { thread_type: ThreadType::User, thread_id, uid, is_pc: true }
            if thread_id == "222" && uid == "222"
        ));
        assert!(matches!(
            &evs[1],
            WsEvent::Typing { thread_type: ThreadType::Group, thread_id, uid, is_pc: false }
            if thread_id == "777" && uid == "333"
        ));

        let f612 = plain_frame(
            612,
            0,
            json!({
                "data": {
                    "reacts": [
                        { "uidFrom": "222", "idTo": "0", "content": "{\"rIcon\":\"/-heart\"}" }
                    ],
                    "reactGroups": []
                }
            }),
        );
        let evs612 = dispatch_frame(&f612, &mut ck, &ctx, true);
        assert_eq!(evs612.len(), 1);
        match &evs612[0] {
            WsEvent::Reactions {
                thread_type: ThreadType::User,
                reactions,
                is_old: false,
            } => {
                assert_eq!(reactions.len(), 1);
                assert_eq!(reactions[0]["data"]["content"]["rIcon"], "/-heart");
            }
            other => panic!("expected Reactions, got {other:?}"),
        }
    }

    #[test]
    fn frame_handshake_delivered_seen_old_and_duplicate() {
        let ctx = test_ctx();
        let mut ck = None;

        // Handshake cmd=1, sub=1
        let f1 = encode_frame(1, 1, 1, br#"{"key":"MDEyMzQ1Njc4OWFiY2RlZg=="}"#);
        let evs1 = dispatch_frame(&f1, &mut ck, &ctx, true);
        assert_eq!(ck.as_deref(), Some("MDEyMzQ1Njc4OWFiY2RlZg=="));
        assert!(matches!(&evs1[0], WsEvent::CipherKey(k) if k == "MDEyMzQ1Njc4OWFiY2RlZg=="));

        // Delivered & seen cmd=502
        let f502 = plain_frame(
            502,
            0,
            json!({
                "data": {
                    "delivereds": [{ "deliveredUids": ["222"] }],
                    "seens": [{ "idTo": "222" }]
                }
            }),
        );
        let evs502 = dispatch_frame(&f502, &mut ck, &ctx, true);
        assert_eq!(evs502.len(), 2);
        assert!(matches!(evs502[0], WsEvent::Delivered { thread_type: ThreadType::User, .. }));
        assert!(matches!(evs502[1], WsEvent::Seen { thread_type: ThreadType::User, .. }));

        // Duplicate connection cmd=3000
        let f3000 = encode_frame(1, 3000, 0, b"{}");
        let evs3000 = dispatch_frame(&f3000, &mut ck, &ctx, true);
        assert!(matches!(&evs3000[0], WsEvent::Closed { code: 3000, .. }));
    }
}

