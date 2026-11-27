use std::collections::HashMap;

use crate::time_util::now_ms;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::api::ZaloClient;
use crate::crypto::parse_json_keeping_bigints;
use crate::error::Result;

/// Default Zalo voice call domain when not present in `zpw_service_map_v3.voice_call`.
pub const DEFAULT_VOICE_CALL_DOMAIN: &str = "https://voicecall-wpa.chat.zalo.me";

/// Codec used when answering/requesting calls: Opus 16 kHz mono, payload type 112, 20 ms frames.
pub const ANSWER_CODEC: &str =
    r#"[{"dynamicFptime":0,"frmPtime":20,"name":"opus/16000/1","payload":112}]"#;

const REQUEST_CALL_CODEC: &str = "[]\n";
const MIN_SRTP_SESSION_BYTES: usize = 30;
pub const ZRTC_PLATFORM_PC: i64 = 2;
pub const RINGING_MAX_AGE_MS: i64 = 2 * 60_000;

/// Acts indicating a call has ended or been handled elsewhere.
pub const CALL_ENDING_ACTS: &[&str] = &[
    "answer",
    "cancel",
    "endcall",
    "end_call",
    "reject",
    "busy",
    "group_answer",
    "group_cancel",
    "group_endcall",
    "group_finish",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AnswerStatus {
    Accept = 0,
    Busy = 1,
    Decline = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingCall {
    pub call_id: String,
    pub caller_id: String,
    pub video: bool,
    pub group: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RingingCall {
    pub incoming: IncomingCall,
    pub raw_call_id: Value,
    pub call_type: i32,
    pub data: Map<String, Value>,
    pub received_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferedServer {
    pub rtp: String,
    pub rtcp: String,
}

#[derive(Debug, Clone)]
pub struct OutgoingMedia {
    pub servers: Vec<OfferedServer>,
    pub session: String,
    pub user_id: u32,
    pub partner_id: u32,
    /// `zrtc_config.srtpKey`, when the answer has a usable one. Zalo PC keys SRTP with it rather
    /// than with the session.
    pub srtp_key: Option<String>,
    /// `zrtc_config.srtpMode` (libsrtp profile: 1 = 80-bit tag, 2 = 32-bit tag, 0 = none).
    pub srtp_mode: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct MediaParams {
    pub server: String,
    pub user_id: u32,
    pub partner_id: u32,
    pub call_id: u32,
    pub session: String,
    pub srtp_key: Option<String>,
    /// `zrtc_config.srtpMode` (libsrtp profile: 1 = 80-bit tag, 2 = 32-bit tag, 0 = none).
    pub srtp_mode: Option<u8>,
}

/// `zrtc_config.srtpKey` when it is long enough to key SRTP (30 characters: key + salt).
fn srtp_key_of(zrtc_cfg: Option<&Map<String, Value>>) -> Option<String> {
    let key = text_of(zrtc_cfg.and_then(|z| z.get("srtpKey")));
    (key.len() >= MIN_SRTP_SESSION_BYTES).then_some(key)
}

/// `zrtc_config.srtpMode` as a number (Zalo sends it as a number or a numeric string).
fn srtp_mode_of(zrtc_cfg: Option<&Map<String, Value>>) -> Option<u8> {
    match zrtc_cfg?.get("srtpMode")? {
        Value::Number(n) => n.as_u64().and_then(|m| u8::try_from(m).ok()),
        Value::String(s) => s.trim().parse::<u8>().ok(),
        _ => None,
    }
}

/// Resolves the voice call service domain from `zpw_service_map_v3`.
pub fn voice_call_domain(service_map: &HashMap<String, Vec<String>>) -> String {
    service_map
        .get("voice_call")
        .and_then(|v| v.first())
        .filter(|s| s.starts_with("https://"))
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_VOICE_CALL_DOMAIN.to_string())
}

/// Generates a fresh 32-bit call ID in `[1_000_000_000, 2_147_483_647]`.
pub fn new_call_id() -> u32 {
    rand::thread_rng().gen_range(1_000_000_000..=2_147_483_647)
}

/// Builds a [`RingingCall`] representation for an outgoing call we initiate.
pub fn outgoing_call_of(peer_id: &str, call_id: u32, video: bool) -> RingingCall {
    let now = now_ms();
    RingingCall {
        incoming: IncomingCall {
            call_id: call_id.to_string(),
            caller_id: peer_id.to_string(),
            video,
            group: false,
            group_id: None,
            ts: now,
            display_name: None,
        },
        raw_call_id: json!(call_id),
        call_type: if video { 3 } else { 1 },
        data: Map::new(),
        received_at: now,
    }
}

/// Parses an [`IncomingCall`] from a VoIP `request` or `group_request` control.
pub fn incoming_call_of(act: &str, data: &Map<String, Value>, now_ms: i64) -> Option<IncomingCall> {
    if act != "request" && act != "group_request" {
        return None;
    }
    let caller_id = first_text(
        data,
        &["callerId", "uidN", "fromUid", "uidFrom", "uid", "hostCall", "srcId"],
    );
    let call_id = first_text(data, &["callId", "call_id"]).trim().to_string();
    if caller_id.is_empty() || call_id.is_empty() {
        return None;
    }
    let raw_ts = first_text(data, &["ts", "time", "timestamp"])
        .parse::<i64>()
        .unwrap_or(0);
    let ts = if raw_ts > 0 {
        if raw_ts < 1_000_000_000_000 {
            raw_ts * 1000
        } else {
            raw_ts
        }
    } else {
        now_ms
    };
    let ctype = first_text(data, &["callType", "type", "calltype"])
        .parse::<i64>()
        .unwrap_or(0);
    let params = data.get("params").and_then(json_object);
    let extend = params
        .as_ref()
        .and_then(|p| p.get("extendData"))
        .and_then(json_object);
    let display_name = params
        .as_ref()
        .and_then(|p| p.get("Dname"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let extend_type = extend
        .as_ref()
        .and_then(|e| e.get("callType"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let video = ctype == 1
        || extend_type == 1
        || first_text(data, &["isVideo"]) == "1"
        || data.get("isVideo").and_then(|v| v.as_bool()) == Some(true);
    let gid = first_text(data, &["groupId", "gid"]);

    Some(IncomingCall {
        call_id,
        caller_id,
        video,
        group: act == "group_request",
        group_id: if gid.is_empty() { None } else { Some(gid) },
        ts,
        display_name,
    })
}

/// Promotes an [`IncomingCall`] + control payload into a [`RingingCall`].
pub fn ringing_call_of(incoming: IncomingCall, data: Map<String, Value>, now_ms: i64) -> RingingCall {
    let raw_id_str = first_text(&data, &["callId", "call_id"]);
    let raw_call_id = raw_id_str
        .parse::<i64>()
        .or_else(|_| incoming.call_id.parse::<i64>())
        .map(|n| json!(n))
        .unwrap_or_else(|_| json!(incoming.call_id));
    let call_type = if incoming.video { 3 } else { 1 };
    RingingCall {
        incoming,
        raw_call_id,
        call_type,
        data,
        received_at: now_ms,
    }
}

/// Extracts zRTC media parameters from a ringing call.
pub fn media_params_of(call: &RingingCall) -> Option<MediaParams> {
    let d = &call.data;
    let params = d.get("params").and_then(json_object);
    let zrtc_cfg = params
        .as_ref()
        .and_then(|p| p.get("zrtc_config"))
        .and_then(json_object)
        .or_else(|| d.get("zrtc_config").and_then(json_object));

    let server = {
        let s = text_of(d.get("rtpAddress"));
        if !s.is_empty() {
            s
        } else {
            text_of(params.as_ref().and_then(|p| p.get("rtpIP")))
        }
    };
    let user_id = parse_u32_pos(
        params
            .as_ref()
            .and_then(|p| p.get("userId"))
            .or_else(|| d.get("uidTo")),
    )?;
    let partner_id = parse_u32_pos(d.get("uidFrom"))?;
    let call_id = parse_u32_pos(Some(&call.raw_call_id))?;

    let srtp_key = srtp_key_of(zrtc_cfg.as_ref());
    let srtp_mode = srtp_mode_of(zrtc_cfg.as_ref());

    let session = {
        let s1 = text_of(d.get("session"));
        let s2 = text_of(params.as_ref().and_then(|p| p.get("sessId")));
        if !s1.is_empty() {
            s1
        } else if !s2.is_empty() {
            s2
        } else {
            srtp_key.clone().unwrap_or_default()
        }
    };
    if server.is_empty() || session.len() < MIN_SRTP_SESSION_BYTES {
        return None;
    }

    Some(MediaParams {
        server,
        user_id,
        partner_id,
        call_id,
        session,
        srtp_key,
        srtp_mode,
    })
}

/// Parses the server reply to `/api/voicecall/requestcall`.
pub fn outgoing_media_of(answer: &Value, call_id: u32) -> std::result::Result<OutgoingMedia, String> {
    let d = json_object(answer).unwrap_or_default();
    if let Some(st) = d.get("status").and_then(|v| v.as_i64()) {
        if st != 0 {
            let msg = text_of(d.get("msg"));
            return Err(format!("Zalo answered status {st} {msg}").trim().to_string());
        }
    }
    if let Some(reply_id) = d.get("id").and_then(|v| v.as_u64()) {
        if reply_id != call_id as u64 {
            return Err("the answer is about another call".into());
        }
    }
    let mut servers = Vec::new();
    if let Some(arr) = d.get("servers").and_then(|v| v.as_array()) {
        for s in arr {
            if let Some(o) = json_object(s) {
                let rtp = {
                    let a = text_of(o.get("rtpaddr"));
                    if !a.is_empty() { a } else { text_of(o.get("rtpAddress")) }
                };
                if !rtp.is_empty() {
                    let rtcp = {
                        let a = text_of(o.get("rtcpaddr"));
                        let b = text_of(o.get("rtcpAddress"));
                        if !a.is_empty() {
                            a
                        } else if !b.is_empty() {
                            b
                        } else {
                            rtp.clone()
                        }
                    };
                    servers.push(OfferedServer { rtp, rtcp });
                }
            }
        }
    }
    if servers.is_empty() {
        return Err("the answer names no media server".into());
    }
    let zrtc_cfg = d.get("zrtc_config").and_then(json_object);
    let session = {
        let s = text_of(d.get("sessId"));
        if !s.is_empty() {
            s
        } else {
            text_of(zrtc_cfg.as_ref().and_then(|z| z.get("srtpKey")))
        }
    };
    if session.len() < MIN_SRTP_SESSION_BYTES {
        return Err("the answer has no usable session".into());
    }
    let user_id = parse_u32_pos(d.get("fromId")).ok_or_else(|| "the answer has no zrtc ids".to_string())?;
    let partner_id = parse_u32_pos(d.get("toId")).ok_or_else(|| "the answer has no zrtc ids".to_string())?;

    Ok(OutgoingMedia {
        servers,
        session,
        user_id,
        partner_id,
        srtp_key: srtp_key_of(zrtc_cfg.as_ref()),
        srtp_mode: srtp_mode_of(zrtc_cfg.as_ref()),
    })
}

/// Builds our `extendData` JSON matching Zalo PC's `CallController::getExtendData`.
pub fn our_extend_data(call: &RingingCall, server_addr_override: Option<Vec<Value>>) -> Value {
    let params = call.data.get("params").and_then(json_object);
    let theirs = params
        .as_ref()
        .and_then(|p| p.get("extendData"))
        .and_then(json_object)
        .unwrap_or_default();
    let num = |k: &str, fallback: i64| -> i64 {
        theirs
            .get(k)
            .and_then(|v| v.as_i64())
            .unwrap_or(fallback)
    };
    let servers = server_addr_override.unwrap_or_else(|| {
        theirs
            .get("serverAddr")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().take(1).cloned().collect())
            .unwrap_or_default()
    });

    json!({
        "p2p": [],
        "callType": if call.incoming.video { 1 } else { 0 },
        "fecTP": 0,
        "gccAudio": num("gccAudio", 1),
        "gccEarlyCall": 0,
        "gccMode": num("gccMode", 1),
        "gccSVLR": num("gccSVLR", 1),
        "maxFT": num("maxFT", 60),
        "newZrtc": 1,
        "numServers": 0,
        "packetMode": num("packetMode", 2),
        "platform": ZRTC_PLATFORM_PC,
        "sP2P": 0,
        "select2side": 1,
        "spTcp": 1,
        "srtcp": 0,
        "srtpMode": 1,
        "tpType": 0,
        "supportCallBusy": 1,
        "supportHevcDecode": 0,
        "serverAddr": servers,
        "video": { "codec": [{ "name": "h264", "payload": 97 }] },
    })
}

/// Helper wrapping `/api/voicecall/<endpoint>` encrypted GET requests on a [`ZaloClient`].
#[derive(Clone)]
pub struct VoiceCallApi {
    client: ZaloClient,
}

impl VoiceCallApi {
    pub fn new(client: ZaloClient) -> Self {
        Self { client }
    }

    /// Sends a generic encrypted GET to `/api/voicecall/<endpoint>`.
    pub async fn send(&self, endpoint: &str, params: Value) -> Result<Value> {
        let domain = voice_call_domain(&self.client.ctx.service_map);
        let url = format!("{domain}/api/voicecall/{}", endpoint.trim_start_matches('/'));
        self.client
            .http
            .get_encrypted(&self.client.ctx, &url, &params, &[])
            .await
    }

    /// Tells the caller this device is ringing (`/api/voicecall/ringring`).
    pub async fn ring_ring(&self, call: &RingingCall) -> Result<Value> {
        self.send(
            "ringring",
            json!({
                "callerId": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "status": 0,
                "imei": self.client.ctx.imei,
            }),
        )
        .await
    }

    /// Declines an incoming call (`/api/voicecall/answer`, status 3).
    pub async fn decline_call(&self, call: &RingingCall) -> Result<Value> {
        let d = &call.data;
        let params = d.get("params").and_then(json_object);
        let server = {
            let s = text_of(d.get("rtpAddress"));
            if !s.is_empty() {
                s
            } else {
                text_of(params.as_ref().and_then(|p| p.get("rtpIP")))
            }
        };
        let rtcp = {
            let a = text_of(d.get("rtcpAddress"));
            let b = text_of(params.as_ref().and_then(|p| p.get("rtcpIP")));
            if !a.is_empty() {
                a
            } else if !b.is_empty() {
                b
            } else {
                server.clone()
            }
        };
        let session = {
            let a = text_of(d.get("session"));
            if !a.is_empty() {
                a
            } else {
                text_of(params.as_ref().and_then(|p| p.get("sessId")))
            }
        };
        self.send(
            "answer",
            json!({
                "callerId": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "status": AnswerStatus::Decline as i32,
                "codec": text_of(d.get("codec")),
                "rtcpAddress": rtcp,
                "rtpAddress": server,
                "session_id": session,
                "extendData": our_extend_data(call, None).to_string(),
                "subCommand": 3,
            }),
        )
        .await
    }

    /// Accepts an incoming call after joining `server` (`/api/voicecall/answer`, status 0).
    pub async fn accept_call(&self, call: &RingingCall, server: &str) -> Result<Value> {
        let d = &call.data;
        let params = d.get("params").and_then(json_object);
        let session = {
            let a = text_of(d.get("session"));
            if !a.is_empty() {
                a
            } else {
                text_of(params.as_ref().and_then(|p| p.get("sessId")))
            }
        };
        self.send(
            "answer",
            json!({
                "callerId": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "status": AnswerStatus::Accept as i32,
                "codec": ANSWER_CODEC,
                "rtcpAddress": server,
                "rtpAddress": server,
                "session_id": session,
                "extendData": our_extend_data(call, None).to_string(),
                "subCommand": 3,
            }),
        )
        .await
    }

    /// Reports camera/microphone state (`/api/voicecall/conf`).
    pub async fn cam_mic_state(
        &self,
        call: &RingingCall,
        cam_on: Option<bool>,
        muted: Option<bool>,
    ) -> Result<Value> {
        self.send(
            "conf",
            json!({
                "calleeId": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "isOnCam": match cam_on { Some(true) => 1, Some(false) => 0, None => -1 },
                "isOnMic": match muted { Some(true) => 1, Some(false) => 0, None => -1 },
                "data": "",
            }),
        )
        .await
    }

    /// Hangs up an active call (`/api/voicecall/endcall`).
    pub async fn hang_up(&self, call: &RingingCall) -> Result<Value> {
        self.send(
            "endcall",
            json!({
                "uidTo": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "status": 3,
                "imei": self.client.ctx.imei,
            }),
        )
        .await
    }

    /// Initiates an outgoing call (`/api/voicecall/requestcall`).
    pub async fn request_call(&self, call: &RingingCall) -> Result<Value> {
        self.send(
            "requestcall",
            json!({
                "calleeId": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "codec": REQUEST_CALL_CODEC,
                "typeRequest": call.call_type,
                "imei": self.client.ctx.imei,
            }),
        )
        .await
    }

    /// Rings the callee after joining the media server (`/api/voicecall/request`).
    pub async fn send_call_request(
        &self,
        call: &RingingCall,
        media: &OutgoingMedia,
        server: &OfferedServer,
    ) -> Result<Value> {
        let ext = our_extend_data(
            call,
            Some(vec![json!({
                "rtcp": server.rtcp,
                "rtp": server.rtp,
                "tpType": 0,
            })]),
        );
        self.send(
            "request",
            json!({
                "calleeId": call.incoming.caller_id,
                "rtcpAddress": server.rtcp,
                "rtpAddress": server.rtp,
                "codec": ANSWER_CODEC,
                "session": media.session,
                "callId": call.raw_call_id,
                "imei": self.client.ctx.imei,
                "extendData": ext.to_string(),
                "subCommand": 3,
            }),
        )
        .await
    }

    /// Cancels an unanswered outgoing call (`/api/voicecall/cancel`).
    pub async fn cancel_call(&self, call: &RingingCall) -> Result<Value> {
        self.send(
            "cancel",
            json!({
                "callerId": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "callType": if call.incoming.video { 1 } else { 0 },
                "status": 0,
                "imei": self.client.ctx.imei,
            }),
        )
        .await
    }

    /// Acknowledges callee's answer (`/api/voicecall/answerack`).
    pub async fn answer_ack(&self, call: &RingingCall) -> Result<Value> {
        self.send(
            "answerack",
            json!({
                "calleeId": call.incoming.caller_id,
                "callId": call.raw_call_id,
                "status": 0,
                "imei": self.client.ctx.imei,
            }),
        )
        .await
    }
}

/// Recursively redacts sensitive call trace fields (session/SRTP keys, tokens, P2P addresses, caller PII).
pub fn redact_for_trace(value: &Value) -> Value {
    const SENSITIVE: &[&str] = &[
        "session",
        "sessid",
        "session_id",
        "srtpkey",
        "srtp_key",
        "token",
        "p2p",
        "nataddress",
        "address",
        "displayname",
        "callername",
        "dname",
        "avatar",
        "calleravatar",
    ];
    fn walk(v: &Value, depth: usize) -> Value {
        if depth > 16 {
            return v.clone();
        }
        match v {
            Value::String(s) => {
                let t = s.trim();
                if (t.starts_with('{') && t.ends_with('}'))
                    || (t.starts_with('[') && t.ends_with(']'))
                {
                    if let Ok(parsed) = parse_json_keeping_bigints(t) {
                        let redacted = walk(&parsed, depth + 1);
                        if redacted != parsed {
                            return Value::String(redacted.to_string());
                        }
                    }
                }
                v.clone()
            }
            Value::Array(arr) => Value::Array(arr.iter().map(|i| walk(i, depth + 1)).collect()),
            Value::Object(map) => {
                let mut out = Map::new();
                for (k, val) in map {
                    if SENSITIVE.contains(&k.to_ascii_lowercase().as_str()) {
                        if val.is_null() {
                            out.insert(k.clone(), Value::Null);
                        } else {
                            out.insert(k.clone(), Value::String("[redacted]".to_string()));
                        }
                    } else {
                        out.insert(k.clone(), walk(val, depth + 1));
                    }
                }
                Value::Object(out)
            }
            _ => v.clone(),
        }
    }
    walk(value, 0)
}

fn json_object(v: &Value) -> Option<Map<String, Value>> {
    match v {
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

fn first_text(map: &Map<String, Value>, keys: &[&str]) -> String {
    for k in keys {
        let s = text_of(map.get(*k));
        if !s.is_empty() {
            return s;
        }
    }
    String::new()
}

fn parse_u32_pos(v: Option<&Value>) -> Option<u32> {
    let n = match v? {
        Value::Number(num) => num.as_u64()?,
        Value::String(s) => s.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    if n > 0 && n <= u32::MAX as u64 {
        Some(n as u32)
    } else {
        None
    }
}

#[cfg(test)]
mod srtp_offer_tests {
    use super::*;

    const SESSION: &str = "SESSIONsessionSESSIONsessionSESSIONsession";
    const KEY: &str = "KEYkeyKEYkeyKEYkeyKEYkeyKEYkeyKEY";

    fn answer(zrtc_config: Value) -> Value {
        json!({
            "status": 0,
            "id": 42,
            "fromId": 465816670u32,
            "toId": 459524852u32,
            "sessId": SESSION,
            "servers": [{ "rtpaddr": "171.244.25.108:4200" }],
            "zrtc_config": zrtc_config,
        })
    }

    #[test]
    fn outgoing_answer_keeps_srtp_key_and_mode_apart_from_the_session() {
        let m = outgoing_media_of(&answer(json!({ "srtpKey": KEY, "srtpMode": 2 })), 42).unwrap();
        assert_eq!(m.session, SESSION);
        assert_eq!(m.srtp_key.as_deref(), Some(KEY));
        assert_eq!(m.srtp_mode, Some(2));
    }

    #[test]
    fn outgoing_answer_reads_zrtc_config_sent_as_a_json_string() {
        let cfg = json!({ "srtpKey": KEY, "srtpMode": "1" }).to_string();
        let m = outgoing_media_of(&answer(Value::String(cfg)), 42).unwrap();
        assert_eq!(m.srtp_key.as_deref(), Some(KEY));
        assert_eq!(m.srtp_mode, Some(1));
    }

    #[test]
    fn outgoing_answer_without_srtp_fields_or_with_a_short_key() {
        let m = outgoing_media_of(&answer(json!({})), 42).unwrap();
        assert_eq!((m.srtp_key, m.srtp_mode), (None, None));
        let m = outgoing_media_of(&answer(json!({ "srtpKey": "short", "srtpMode": 0 })), 42).unwrap();
        assert_eq!((m.srtp_key, m.srtp_mode), (None, Some(0)));
    }
}
