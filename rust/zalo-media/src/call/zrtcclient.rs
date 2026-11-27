//! Async Tokio UDP `MediaCall` engine matching `src/zrtcclient.ts`:
//! joins Zalo's media server (`InitCallCaller`/`InitCallCallee`), maintains separate audio & video
//! SRTP contexts, keepalive pings, audio+video RTCP reports, 50ms video reorder tick + throttled
//! PLI/FIR handling, and 20ms Opus silence filler when app microphone audio stalls.

use std::collections::{BTreeMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

use crate::call::rtcp::{
    parse_rtcp, pli, receiver_report, sdes_cname, sender_report, ReceiveStats, RtcpPacket,
};
use crate::call::srtp::{rtp_header, SrtpContext, AUTH_TAG_LEN};
use crate::call::video::{
    packetize_frame, AssembledFrame, AssemblerStats, VideoAssembler, VideoFrame, VideoPacket,
    MAX_CHUNK, VIDEO_DELTA_PT, VIDEO_KEY_PT,
};
use crate::call::zrtc::{self, cmd, kind, InitCallParams, Packet};

/// Opus payload type and RTP clock offered by Zalo (`opus/16000/1`, PT 112, 20 ms frames).
pub const OPUS_PT: u8 = 112;
pub const OPUS_CLOCK: u32 = 16_000;
pub const FRAME_MS: u64 = 20;

/// Video in 1:1 calls: H.264 PT 97 (98 for key frames), 90 kHz clock.
pub const H264_PT: u8 = VIDEO_DELTA_PT;
pub const VIDEO_CLOCK: u32 = 90_000;

/// 20 ms Opus silence frame (CELT silence).
pub const OPUS_SILENCE: &[u8] = &[0xf8, 0xff, 0xfe];

const PLI_MIN_GAP_MS: i64 = 500;
const KEYREQ_MIN_GAP_MS: i64 = 250;
const PLI_RETRY_MS: i64 = 1000;
const VIDEO_TICK_MS: u64 = 50;
const DEFAULT_PING_MS: u64 = 2000;
const DEFAULT_RTCP_MS: u64 = 2000;
const MAX_SSRCS: usize = 64;
const MAX_RECEIVED_KEYS: usize = 64;

/// Parsed media server host + port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaServer {
    pub host: String,
    pub port: u16,
}

impl MediaServer {
    pub fn socket_addr(&self) -> Option<SocketAddr> {
        let ip: IpAddr = self.host.parse().ok()?;
        Some(SocketAddr::new(ip, self.port))
    }
}

/// Parses `"ip:port"` (IPv4) or `"[ipv6]:port"` / `"ipv6:port"`, matching `src/zrtcclient.ts:parseServer`.
pub fn parse_server(address: &str) -> Option<MediaServer> {
    let trimmed = address.trim();
    let (host, port_str) = if let Some(rest) = trimmed.strip_prefix('[') {
        let (h, p) = rest.split_once("]:")?;
        (h, p)
    } else {
        let idx = trimmed.rfind(':')?;
        let (h, p) = (&trimmed[..idx], &trimmed[idx + 1..]);
        if h.ends_with(':') {
            return None;
        }
        (h, p)
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
    {
        return None;
    }
    if port_str.is_empty() || port_str.len() > 5 || !port_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if host.parse::<IpAddr>().is_err() {
        return None;
    }
    let port: u32 = port_str.parse().ok()?;
    if port == 0 || port >= 65536 {
        return None;
    }
    Some(MediaServer {
        host: host.to_string(),
        port: port as u16,
    })
}

/// Options to create a [`MediaCall`].
#[derive(Debug, Clone)]
pub struct MediaCallOptions {
    pub server: MediaServer,
    pub user_id: u32,
    pub partner_id: u32,
    pub call_id: u32,
    pub session: String,
    pub is_caller: bool,
    pub ping_ms: u64,
    pub rtcp_ms: u64,
}

impl MediaCallOptions {
    pub fn new(
        server: MediaServer,
        user_id: u32,
        partner_id: u32,
        call_id: u32,
        session: String,
        is_caller: bool,
    ) -> Self {
        Self {
            server,
            user_id,
            partner_id,
            call_id,
            session,
            is_caller,
            ping_ms: DEFAULT_PING_MS,
            rtcp_ms: DEFAULT_RTCP_MS,
        }
    }
}

/// Summary of one control packet logged inside [`MediaStats`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSummary {
    pub kind: u8,
    pub cmd: u16,
    pub sub: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<u32>,
}

/// Inbound audio statistics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStats {
    pub packets: u64,
    pub decrypted: u64,
    pub failed: u64,
    pub payload_types: BTreeMap<u8, u64>,
    pub ssrcs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seq: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seq: Option<u16>,
    pub timestamp_steps: Vec<u32>,
    pub opus_toc: Vec<String>,
    pub bytes: u64,
}

/// Video RX/TX & feedback statistics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStats {
    pub packets: u64,
    pub decrypted: u64,
    pub failed: u64,
    pub ssrcs: usize,
    pub payload_types: BTreeMap<u8, u64>,
    pub nal_types: BTreeMap<u8, u64>,
    pub bytes: u64,
    pub frames: u64,
    pub keys: u64,
    pub gaps: u64,
    pub dropped: u64,
    pub sent_frames: u64,
    pub sent_packets: u64,
    pub sent_bytes: u64,
    pub plis_sent: u64,
    pub key_requests: u64,
}

/// Last loss/jitter block reported by the peer about our audio stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TheirView {
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
    pub jitter: u32,
}

/// RTCP packet statistics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RtcpStats {
    pub packets: u64,
    pub types: BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub their_view: Option<TheirView>,
}

/// Complete call media telemetry snapshot recorded in `call-trace.jsonl`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStats {
    pub sent: BTreeMap<String, u64>,
    pub received: BTreeMap<String, u64>,
    pub audio: AudioStats,
    pub controls: Vec<ControlSummary>,
    pub video: VideoStats,
    pub rtcp: RtcpStats,
}

/// Events emitted by [`MediaCall`] to the server / `/ws/call-audio`.
#[derive(Debug, Clone)]
pub enum MediaEvent {
    Audio {
        seq: u16,
        timestamp: u32,
        opus: Vec<u8>,
    },
    Video(VideoFrame),
    KeyFrameRequest,
    Log(Value),
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn ip_matches(from: IpAddr, target: IpAddr) -> bool {
    if from == target {
        return true;
    }
    match (from, target) {
        (IpAddr::V6(v6), IpAddr::V4(v4)) => v6.to_ipv4_mapped() == Some(v4),
        (IpAddr::V4(v4), IpAddr::V6(v6)) => v6.to_ipv4_mapped() == Some(v4),
        _ => false,
    }
}

fn media_kind_name(k: u8) -> Option<&'static str> {
    match k {
        kind::AUDIO => Some("media_3"),
        kind::AUDIO_IN => Some("media_4"),
        kind::AUDIO_RTCP => Some("media_5"),
        kind::AUDIO_FEC => Some("media_7"),
        kind::VIDEO => Some("media_13"),
        kind::VIDEO_IN => Some("media_14"),
        kind::VIDEO_RTCP => Some("media_15"),
        _ => None,
    }
}

/// Pure protocol state machine driving one call's media session.
pub(crate) struct MediaEngine {
    opts: MediaCallOptions,
    server_addr: SocketAddr,
    pub stats: MediaStats,
    pub token: Option<u32>,
    pub nat_address: Option<String>,
    srtp: SrtpContext,
    video_srtp: SrtpContext,
    rx_scratch: Vec<u8>,
    ssrc: u32,
    video_ssrc: u32,
    seq: u16,
    ts: u32,
    first_audio: bool,
    sent_packets: u32,
    sent_octets: u32,
    last_frame_at: i64,
    ssrcs: HashSet<u32>,
    last_ts: Option<u32>,
    incoming: Option<ReceiveStats>,
    v_seq: u16,
    v_ts_base: u32,
    v_clock0: Instant,
    v_sent_packets: u32,
    v_sent_octets: u32,
    video_ssrcs: HashSet<u32>,
    assembler: VideoAssembler,
    retired_frames: AssemblerStats,
    incoming_video: Option<ReceiveStats>,
    video_started_at: i64,
    last_pli_at: i64,
    last_key_req_at: i64,
    logged_unknown: usize,
    received_key_count: usize,
    closed: bool,
    event_tx: mpsc::UnboundedSender<MediaEvent>,
}

impl MediaEngine {
    pub fn new(
        opts: MediaCallOptions,
        event_tx: mpsc::UnboundedSender<MediaEvent>,
    ) -> Result<Self, String> {
        let server_addr = opts
            .server
            .socket_addr()
            .ok_or_else(|| format!("Invalid media server address: {}:{}", opts.server.host, opts.server.port))?;
        let srtp = SrtpContext::from_session(&opts.session).map_err(|e| e.to_string())?;
        let video_srtp = SrtpContext::from_session(&opts.session).map_err(|e| e.to_string())?;
        let mut rng = rand::thread_rng();
        let video_ssrc = if opts.user_id != 0 {
            opts.user_id
        } else {
            rng.gen_range(0x8000_0000u32..=u32::MAX)
        };
        let partner_ssrc = opts.partner_id;
        let mut audio_ssrc = rng.gen_range(1u32..0x7fff_ffff);
        while audio_ssrc == video_ssrc || audio_ssrc == partner_ssrc {
            audio_ssrc = rng.gen_range(1u32..0x7fff_ffff);
        }
        Ok(Self {
            opts,
            server_addr,
            stats: MediaStats::default(),
            token: None,
            nat_address: None,
            srtp,
            video_srtp,
            rx_scratch: Vec::with_capacity(1500),
            ssrc: audio_ssrc,
            video_ssrc,
            seq: rng.gen_range(0..0x8000),
            ts: rng.gen_range(0..0x7fff_ffff),
            first_audio: true,
            sent_packets: 0,
            sent_octets: 0,
            last_frame_at: 0,
            ssrcs: HashSet::new(),
            last_ts: None,
            incoming: None,
            v_seq: rng.gen_range(0..0x8000),
            v_ts_base: rng.gen_range(0..0x7fff_ffff),
            v_clock0: Instant::now(),
            v_sent_packets: 0,
            v_sent_octets: 0,
            video_ssrcs: HashSet::new(),
            assembler: VideoAssembler::new(),
            retired_frames: AssemblerStats::default(),
            incoming_video: None,
            video_started_at: 0,
            last_pli_at: 0,
            last_key_req_at: 0,
            logged_unknown: 0,
            received_key_count: 0,
            closed: false,
            event_tx,
        })
    }

    /// True when the 50 ms video reorder/PLI timer needs to run (false on audio-only calls).
    pub fn needs_video_tick(&self) -> bool {
        self.incoming_video.is_some()
            && (self.assembler.has_buffered_frames() || self.assembler.waiting_for_key())
    }

    fn emit_log(&self, entry: Value) {
        let _ = self.event_tx.send(MediaEvent::Log(entry));
    }

    fn count_sent(&mut self, what: &str) {
        if let Some(c) = self.stats.sent.get_mut(what) {
            *c += 1;
        } else {
            self.stats.sent.insert(what.to_string(), 1);
        }
    }

    fn count_received(&mut self, what: &str) {
        if let Some(c) = self.stats.received.get_mut(what) {
            *c += 1;
            return;
        }
        if self.received_key_count >= MAX_RECEIVED_KEYS {
            if let Some(c) = self.stats.received.get_mut("other_cmd") {
                *c += 1;
            } else {
                self.stats.received.insert("other_cmd".to_string(), 1);
            }
            return;
        }
        self.received_key_count += 1;
        self.stats.received.insert(what.to_string(), 1);
    }

    fn count_rtcp_type(&mut self, what: &str) {
        if let Some(c) = self.stats.rtcp.types.get_mut(what) {
            *c += 1;
        } else {
            self.stats.rtcp.types.insert(what.to_string(), 1);
        }
    }

    fn queue_send(&mut self, pkt: Vec<u8>, what: &str, out: &mut Vec<Vec<u8>>) {
        if self.closed {
            return;
        }
        self.count_sent(what);
        out.push(pkt);
    }

    pub fn build_init_packet(&self) -> Result<Vec<u8>, zrtc::ZrtcError> {
        zrtc::init_call(InitCallParams {
            is_caller: self.opts.is_caller,
            user_id: self.opts.user_id,
            partner_id: self.opts.partner_id,
            call_id: self.opts.call_id,
            session: &self.opts.session,
            attempt: 1,
            extra: "",
        })
    }

    pub fn send_init(&mut self, init_pkt: &[u8], out: &mut Vec<Vec<u8>>) {
        self.queue_send(init_pkt.to_vec(), "init", out);
    }

    pub fn send_ping(&mut self, out: &mut Vec<Vec<u8>>) {
        if let Ok(pkt) = zrtc::ping(self.opts.user_id, self.token.unwrap_or(0), &self.opts.session) {
            self.queue_send(pkt, "ping", out);
        }
    }

    pub fn send_end_call(&mut self, out: &mut Vec<Vec<u8>>) {
        if let Some(tok) = self.token {
            let pkt = zrtc::end_call(self.opts.user_id, self.opts.call_id, tok, false);
            self.queue_send(pkt, "endcall", out);
        }
    }

    pub fn send_silence_if_idle(&mut self, now: i64, out: &mut Vec<Vec<u8>>) {
        if now - self.last_frame_at > (3 * FRAME_MS) as i64 {
            self.send_frame(OPUS_SILENCE, out);
        }
    }

    pub fn send_audio_frame(&mut self, opus: &[u8], now: i64, out: &mut Vec<Vec<u8>>) {
        self.last_frame_at = now;
        self.send_frame(opus, out);
    }

    fn send_frame(&mut self, opus: &[u8], out: &mut Vec<Vec<u8>>) {
        let Some(token) = self.token else {
            return;
        };
        if self.closed {
            return;
        }
        let srtp_len = 12 + opus.len() + AUTH_TAG_LEN;
        if srtp_len > zrtc::MAX_PAYLOAD {
            return;
        }
        let mut pkt = Vec::with_capacity(5 + srtp_len);
        pkt.push(kind::AUDIO);
        pkt.extend_from_slice(&token.to_le_bytes());
        pkt.push(0x80);
        pkt.push(OPUS_PT | if self.first_audio { 0x80 } else { 0 });
        pkt.extend_from_slice(&self.seq.to_be_bytes());
        pkt.extend_from_slice(&self.ts.to_be_bytes());
        pkt.extend_from_slice(&self.ssrc.to_be_bytes());
        pkt.extend_from_slice(opus);

        self.first_audio = false;
        self.seq = self.seq.wrapping_add(1);
        self.ts = self
            .ts
            .wrapping_add((OPUS_CLOCK * FRAME_MS as u32) / 1000);
        self.sent_packets = self.sent_packets.wrapping_add(1);
        self.sent_octets = self.sent_octets.wrapping_add(opus.len() as u32);

        if self.srtp.protect_in_place(&mut pkt, 5).is_ok() {
            self.queue_send(pkt, "audio", out);
        }
    }

    fn video_timestamp(&self) -> u32 {
        let ms = self.v_clock0.elapsed().as_millis() as u64;
        self.v_ts_base.wrapping_add((ms.wrapping_mul(90)) as u32)
    }

    pub fn send_video_frame(&mut self, key: bool, data: &[u8], out: &mut Vec<Vec<u8>>) {
        let Some(token) = self.token else {
            return;
        };
        if self.closed || data.is_empty() {
            return;
        }
        let Ok(chunks) = packetize_frame(data, MAX_CHUNK) else {
            return;
        };
        let ts = self.video_timestamp();
        let total = chunks.len();
        let pt = if key { VIDEO_KEY_PT } else { VIDEO_DELTA_PT };
        for (i, chunk) in chunks.iter().enumerate() {
            let srtp_len = 12 + chunk.len() + AUTH_TAG_LEN;
            let mut pkt = Vec::with_capacity(5 + srtp_len);
            pkt.push(kind::VIDEO);
            pkt.extend_from_slice(&token.to_le_bytes());
            pkt.push(0x80);
            pkt.push(pt | if i + 1 == total { 0x80 } else { 0 });
            pkt.extend_from_slice(&self.v_seq.to_be_bytes());
            pkt.extend_from_slice(&ts.to_be_bytes());
            pkt.extend_from_slice(&self.video_ssrc.to_be_bytes());
            pkt.extend_from_slice(chunk);

            self.v_seq = self.v_seq.wrapping_add(1);
            self.v_sent_packets = self.v_sent_packets.wrapping_add(1);
            self.v_sent_octets = self.v_sent_octets.wrapping_add(chunk.len() as u32);

            if srtp_len <= zrtc::MAX_PAYLOAD && self.video_srtp.protect_in_place(&mut pkt, 5).is_ok()
            {
                self.queue_send(pkt, "video", out);
            }
        }
        self.stats.video.sent_frames += 1;
        self.stats.video.sent_packets += total as u64;
        self.stats.video.sent_bytes += data.len() as u64;
    }

    pub fn send_rtcp(&mut self, now: i64, out: &mut Vec<Vec<u8>>) {
        let Some(token) = self.token else {
            return;
        };
        let block = self.incoming.as_mut().and_then(|inc| inc.block(now));
        let blocks: Vec<_> = block.into_iter().collect();
        let mut compound = if self.sent_packets > 0 {
            sender_report(
                self.ssrc,
                now,
                self.ts,
                self.sent_packets,
                self.sent_octets,
                &blocks,
            )
        } else {
            receiver_report(self.ssrc, &blocks)
        };
        let cname = format!("zaloo-{:x}", self.ssrc);
        compound.extend_from_slice(&sdes_cname(self.ssrc, &cname));
        if let Ok(pkt) = zrtc::rtcp(token, &compound, false) {
            self.queue_send(pkt, "rtcp", out);
        }
        self.send_video_rtcp(now, &cname, out);
    }

    fn send_video_rtcp(&mut self, now: i64, cname: &str, out: &mut Vec<Vec<u8>>) {
        let Some(token) = self.token else {
            return;
        };
        if self.v_sent_packets == 0 && self.incoming_video.is_none() {
            return;
        }
        let vblock = self.incoming_video.as_mut().and_then(|inc| inc.block(now));
        let blocks: Vec<_> = vblock.into_iter().collect();
        let mut compound = if self.v_sent_packets > 0 {
            sender_report(
                self.video_ssrc,
                now,
                self.video_timestamp(),
                self.v_sent_packets,
                self.v_sent_octets,
                &blocks,
            )
        } else {
            receiver_report(self.video_ssrc, &blocks)
        };
        compound.extend_from_slice(&sdes_cname(self.video_ssrc, cname));
        if let Ok(pkt) = zrtc::rtcp(token, &compound, true) {
            self.queue_send(pkt, "rtcp_video", out);
        }
    }

    pub fn request_key_frame(&mut self, why: &str, min_gap_ms: i64, now: i64, out: &mut Vec<Vec<u8>>) {
        let Some(token) = self.token else {
            return;
        };
        let Some(target) = self.incoming_video.as_ref().map(|v| v.ssrc) else {
            return;
        };
        if now - self.last_pli_at < min_gap_ms {
            return;
        }
        self.last_pli_at = now;
        self.stats.video.plis_sent += 1;
        let pli_pkt = pli(self.video_ssrc, target);
        if let Ok(pkt) = zrtc::rtcp(token, &pli_pkt, true) {
            self.queue_send(pkt, "pli", out);
        }
        if self.stats.video.plis_sent <= 10 {
            self.emit_log(json!({ "kind": "media_pli", "why": why }));
        }
    }

    pub fn video_tick(&mut self, now: i64, out: &mut Vec<Vec<u8>>) {
        if self.incoming_video.is_none() {
            return;
        }
        let mut frames = Vec::new();
        let mut asks = Vec::new();
        self.assembler
            .tick_assembled(now, |f| frames.push(f), |w| asks.push(w));
        self.sync_assembler_stats();
        for f in frames {
            self.on_video_frame(f);
        }
        for w in asks {
            self.request_key_frame(w.as_str(), PLI_MIN_GAP_MS, now, out);
        }
        if self.assembler.waiting_for_key() && now - self.video_started_at >= 300 {
            self.request_key_frame("waiting", PLI_RETRY_MS, now, out);
        }
    }

    fn sync_assembler_stats(&mut self) {
        let a = self.assembler.stats;
        let r = self.retired_frames;
        let v = &mut self.stats.video;
        v.frames = a.frames + r.frames;
        v.keys = a.keys + r.keys;
        v.gaps = a.gaps + r.gaps;
        v.dropped = a.dropped + r.dropped;
    }

    fn on_video_frame(&mut self, assembled: AssembledFrame) {
        let mut mask = assembled.nal_types_mask;
        while mask != 0 {
            let t = mask.trailing_zeros() as u8;
            *self.stats.video.nal_types.entry(t).or_insert(0) += 1;
            mask &= mask - 1;
        }
        self.sync_assembler_stats();
        let _ = self.event_tx.send(MediaEvent::Video(assembled.frame));
    }

    /// Processes one incoming UDP datagram. Returns `Some(true)` on InitCall success or `Some(false)` on InitCall error.
    pub fn handle_datagram(
        &mut self,
        msg: &[u8],
        from: SocketAddr,
        now: i64,
        out: &mut Vec<Vec<u8>>,
    ) -> Option<bool> {
        if from.port() != self.server_addr.port() || !ip_matches(from.ip(), self.server_addr.ip()) {
            self.count_received("other_source");
            return None;
        }

        let pkt = match zrtc::parse(msg) {
            Ok(p) => p,
            Err(err) => {
                self.count_received("unparsable");
                if self.logged_unknown < 5 {
                    self.logged_unknown += 1;
                    self.emit_log(json!({
                        "kind": "media_unparsable",
                        "bytes": msg.len(),
                        "head": hex::encode(&msg[..msg.len().min(24)]),
                        "error": err.to_string(),
                    }));
                }
                return None;
            }
        };

        match pkt {
            Packet::Control {
                header,
                error,
                value38,
                value18,
                text,
                ..
            } => {
                self.count_received(&format!("control_{}_{}", header.kind, header.cmd));
                if self.stats.controls.len() < 40 {
                    self.stats.controls.push(ControlSummary {
                        kind: header.kind,
                        cmd: header.cmd,
                        sub: header.sub,
                        error,
                    });
                }
                if header.kind == kind::RESPONSE
                    && matches!(header.cmd, cmd::INIT_CALL_CALLEE | cmd::INIT_CALL_CALLER)
                {
                    if header.user_id != self.opts.user_id {
                        self.count_received("init_wrong_user");
                        return None;
                    }
                    self.emit_log(json!({
                        "kind": "media_init_answer",
                        "error": error,
                        "value38": value38,
                        "value18": value18,
                        "address": text,
                    }));
                    if error == Some(0) && self.token.is_none() {
                        self.token = value18;
                        self.nat_address = text;
                        return Some(true);
                    } else if error != Some(0) {
                        return Some(false);
                    }
                } else {
                    let mut entry = json!({
                        "kind": "media_control",
                        "packetKind": header.kind,
                        "cmd": header.cmd,
                        "sub": header.sub,
                        "error": error,
                    });
                    if let Some(ref t) = text {
                        entry["textBytes"] = json!(t.len());
                    }
                    self.emit_log(entry);
                }
                None
            }
            Packet::Media {
                kind: k,
                token,
                payload,
            } => {
                if let Some(name) = media_kind_name(k) {
                    self.count_received(name);
                } else {
                    self.count_received(&format!("media_{k}"));
                }
                match k {
                    kind::AUDIO_IN => self.on_audio(&payload, now),
                    kind::AUDIO_RTCP => {
                        if let Some(expected) = self.token {
                            if token != Some(expected) {
                                self.count_received("rtcp_wrong_token");
                                return None;
                            }
                        }
                        self.on_rtcp(&payload, false, now);
                    }
                    kind::VIDEO_RTCP => {
                        if let Some(expected) = self.token {
                            if token != Some(expected) {
                                self.count_received("rtcp_wrong_token");
                                return None;
                            }
                        }
                        self.on_rtcp(&payload, true, now);
                    }
                    kind::VIDEO_IN => self.on_video(&payload, now, out),
                    _ => {}
                }
                None
            }
            Packet::P2p { cmd, .. } => {
                self.count_received(&format!("p2p_{cmd}"));
                None
            }
            Packet::Unknown { kind: k, .. } => {
                self.count_received(&format!("kind_{k}"));
                if self.logged_unknown < 5 {
                    self.logged_unknown += 1;
                    self.emit_log(json!({
                        "kind": "media_unknown",
                        "head": hex::encode(&msg[..msg.len().min(24)]),
                        "bytes": msg.len(),
                    }));
                }
                None
            }
        }
    }

    fn on_rtcp(&mut self, payload: &[u8], video: bool, now: i64) {
        self.stats.rtcp.packets += 1;
        for p in parse_rtcp(payload) {
            match p {
                RtcpPacket::Pli { media_ssrc, .. } => {
                    self.count_rtcp_type("PLI");
                    if media_ssrc == self.video_ssrc {
                        self.on_key_request("PLI", now);
                    }
                }
                RtcpPacket::Fir { media_ssrc, .. } => {
                    self.count_rtcp_type("FIR");
                    if media_ssrc == self.video_ssrc {
                        self.on_key_request("FIR", now);
                    }
                }
                RtcpPacket::Sr {
                    ssrc,
                    ntp_sec,
                    ntp_frac,
                    blocks,
                    ..
                } => {
                    self.count_rtcp_type("SR");
                    if video {
                        if let Some(inc) = self.incoming_video.as_mut() {
                            if inc.ssrc == ssrc {
                                inc.on_sender_report(ntp_sec, ntp_frac, now);
                            }
                        }
                    } else {
                        if let Some(inc) = self.incoming.as_mut() {
                            if inc.ssrc == ssrc {
                                inc.on_sender_report(ntp_sec, ntp_frac, now);
                            }
                        }
                        if let Some(about) = blocks.iter().find(|b| b.ssrc == self.ssrc) {
                            self.stats.rtcp.their_view = Some(TheirView {
                                fraction_lost: about.fraction_lost,
                                cumulative_lost: about.cumulative_lost,
                                jitter: about.jitter,
                            });
                        }
                    }
                }
                RtcpPacket::Rr { blocks, .. } => {
                    self.count_rtcp_type("RR");
                    if !video {
                        if let Some(about) = blocks.iter().find(|b| b.ssrc == self.ssrc) {
                            self.stats.rtcp.their_view = Some(TheirView {
                                fraction_lost: about.fraction_lost,
                                cumulative_lost: about.cumulative_lost,
                                jitter: about.jitter,
                            });
                        }
                    }
                }
                RtcpPacket::Other { packet_type, .. } => {
                    let s = packet_type.to_string();
                    self.count_rtcp_type(&s);
                }
            }
        }
    }

    fn on_key_request(&mut self, kind_name: &str, now: i64) {
        self.stats.video.key_requests += 1;
        if now - self.last_key_req_at < KEYREQ_MIN_GAP_MS {
            return;
        }
        self.last_key_req_at = now;
        self.emit_log(json!({ "kind": "media_key_request", "by": kind_name }));
        let _ = self.event_tx.send(MediaEvent::KeyFrameRequest);
    }

    fn on_video(&mut self, payload: &[u8], now: i64, out: &mut Vec<Vec<u8>>) {
        self.stats.video.packets += 1;
        self.stats.video.bytes += payload.len() as u64;
        let Some(h) = self.video_srtp.unprotect_into(payload, &mut self.rx_scratch) else {
            self.stats.video.failed += 1;
            return;
        };
        self.stats.video.decrypted += 1;
        *self
            .stats
            .video
            .payload_types
            .entry(h.payload_type)
            .or_insert(0) += 1;
        if self.video_ssrcs.len() < MAX_SSRCS {
            self.video_ssrcs.insert(h.ssrc);
        }
        self.stats.video.ssrcs = self.video_ssrcs.len();

        let need_new = self
            .incoming_video
            .as_ref()
            .is_none_or(|inc| inc.ssrc != h.ssrc);
        if need_new {
            if self.incoming_video.is_some() {
                self.sync_assembler_stats();
                let a = self.assembler.stats;
                self.retired_frames.frames += a.frames;
                self.retired_frames.keys += a.keys;
                self.retired_frames.gaps += a.gaps;
                self.retired_frames.dropped += a.dropped;
                self.assembler = VideoAssembler::new();
            }
            self.incoming_video = Some(ReceiveStats::new(h.ssrc, VIDEO_CLOCK));
            self.video_started_at = now;
        }
        if let Some(inc) = self.incoming_video.as_mut() {
            inc.on_packet(h.seq, h.timestamp, now);
        }

        let mut frames = Vec::new();
        let mut asks = Vec::new();
        self.assembler.push_assembled(
            VideoPacket {
                seq: h.seq,
                timestamp: h.timestamp,
                payload_type: h.payload_type,
                marker: h.marker,
                payload: &self.rx_scratch[h.header_length..],
            },
            now,
            |f| frames.push(f),
            |w| asks.push(w),
        );
        self.sync_assembler_stats();
        for f in frames {
            self.on_video_frame(f);
        }
        for w in asks {
            self.request_key_frame(w.as_str(), PLI_MIN_GAP_MS, now, out);
        }
    }

    fn on_audio(&mut self, payload: &[u8], now: i64) {
        self.stats.audio.packets += 1;
        self.stats.audio.bytes += payload.len() as u64;
        let Some(h) = self.srtp.unprotect_into(payload, &mut self.rx_scratch) else {
            self.stats.audio.failed += 1;
            if self.logged_unknown < 5 && rtp_header(payload).is_none() {
                self.logged_unknown += 1;
                self.emit_log(json!({
                    "kind": "media_not_rtp",
                    "head": hex::encode(&payload[..payload.len().min(16)]),
                    "bytes": payload.len(),
                }));
            }
            return;
        };
        self.stats.audio.decrypted += 1;
        *self
            .stats
            .audio
            .payload_types
            .entry(h.payload_type)
            .or_insert(0) += 1;
        if self.ssrcs.len() < MAX_SSRCS {
            self.ssrcs.insert(h.ssrc);
        }
        if self.incoming.as_ref().is_none_or(|inc| inc.ssrc != h.ssrc) {
            self.incoming = Some(ReceiveStats::new(h.ssrc, OPUS_CLOCK));
        }
        if let Some(inc) = self.incoming.as_mut() {
            inc.on_packet(h.seq, h.timestamp, now);
        }
        self.stats.audio.ssrcs = self.ssrcs.len();
        if self.stats.audio.first_seq.is_none() {
            self.stats.audio.first_seq = Some(h.seq);
        }
        self.stats.audio.last_seq = Some(h.seq);
        if let Some(prev) = self.last_ts {
            if self.stats.audio.timestamp_steps.len() < 8 {
                self.stats
                    .audio
                    .timestamp_steps
                    .push(h.timestamp.wrapping_sub(prev));
            }
        }
        self.last_ts = Some(h.timestamp);
        if self.rx_scratch.len() > h.header_length {
            if self.stats.audio.opus_toc.len() < 5 {
                self.stats
                    .audio
                    .opus_toc
                    .push(format!("{:02x}", self.rx_scratch[h.header_length]));
            }
            let _ = self.event_tx.send(MediaEvent::Audio {
                seq: h.seq,
                timestamp: h.timestamp,
                opus: self.rx_scratch[h.header_length..].to_vec(),
            });
        }
    }
}

enum MediaCmd {
    Join {
        timeout_ms: u64,
        retry_ms: u64,
        reply: oneshot::Sender<Option<u32>>,
    },
    StartSilence,
    SendAudio(Vec<u8>),
    SendVideo {
        key: bool,
        data: Vec<u8>,
    },
    RequestKeyFrame {
        why: String,
        min_gap_ms: i64,
    },
    GetStats(oneshot::Sender<MediaStats>),
    Close {
        send_end_call: bool,
    },
}

struct SharedState {
    stats: MediaStats,
    token: Option<u32>,
    nat_address: Option<String>,
}

/// Async handle to one running zrtc UDP media connection.
#[derive(Clone)]
pub struct MediaCall {
    cmd_tx: mpsc::UnboundedSender<MediaCmd>,
    shared: Arc<RwLock<SharedState>>,
}

impl MediaCall {
    /// Binds a UDP socket and spawns the media actor loop.
    pub async fn new(
        opts: MediaCallOptions,
    ) -> Result<(Self, mpsc::UnboundedReceiver<MediaEvent>), String> {
        let server_addr = opts
            .server
            .socket_addr()
            .ok_or_else(|| format!("Invalid media server: {}:{}", opts.server.host, opts.server.port))?;
        let bind_addr = if server_addr.is_ipv6() {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        };
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|e| format!("Failed to bind UDP socket: {e}"))?;

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let engine = MediaEngine::new(opts, event_tx)?;

        let shared = Arc::new(RwLock::new(SharedState {
            stats: engine.stats.clone(),
            token: None,
            nat_address: None,
        }));
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let shared_bg = Arc::clone(&shared);
        tokio::spawn(async move {
            run_media_loop(socket, engine, cmd_rx, shared_bg).await;
        });

        Ok((Self { cmd_tx, shared }, event_rx))
    }

    /// Joins the media server with default 5s timeout and 400ms retries.
    pub async fn join_default(&self) -> Option<u32> {
        self.join(5000, 400).await
    }

    /// Joins the media server, returning the assigned 32-bit stream token on success.
    pub async fn join(&self, timeout_ms: u64, retry_ms: u64) -> Option<u32> {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(MediaCmd::Join {
                timeout_ms,
                retry_ms,
                reply: tx,
            })
            .is_err()
        {
            return None;
        }
        rx.await.ok().flatten()
    }

    /// Starts the 20 ms Opus silence filler whenever app mic audio stalls (> 60 ms).
    pub fn start_silence(&self) {
        let _ = self.cmd_tx.send(MediaCmd::StartSilence);
    }

    /// Sends one 20 ms Opus frame from the app's microphone.
    pub fn send_audio_frame(&self, opus: Vec<u8>) {
        let _ = self.cmd_tx.send(MediaCmd::SendAudio(opus));
    }

    /// Sends one Annex-B H.264 access unit from the app's camera.
    pub fn send_video_frame(&self, key: bool, data: Vec<u8>) {
        let _ = self.cmd_tx.send(MediaCmd::SendVideo { key, data });
    }

    /// Asks the peer for a video key frame via RTCP PLI (rate-limited to 500 ms).
    pub fn request_key_frame(&self, why: impl Into<String>) {
        let _ = self.cmd_tx.send(MediaCmd::RequestKeyFrame {
            why: why.into(),
            min_gap_ms: PLI_MIN_GAP_MS,
        });
    }

    /// Closes the media session (optionally sending zrtc `EndCall`).
    pub fn close(&self, send_end_call: bool) {
        let _ = self.cmd_tx.send(MediaCmd::Close { send_end_call });
    }

    /// Returns the latest media telemetry snapshot.
    pub fn stats(&self) -> MediaStats {
        self.shared
            .read()
            .map(|s| s.stats.clone())
            .unwrap_or_default()
    }

    /// Queries an exact up-to-the-packet telemetry snapshot directly from the media actor,
    /// falling back to the last synced snapshot if the loop has already exited.
    pub async fn stats_async(&self) -> MediaStats {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(MediaCmd::GetStats(tx)).is_ok() {
            if let Ok(st) = rx.await {
                return st;
            }
        }
        self.stats()
    }

    /// Returns the server-assigned stream token once joined.
    pub fn token(&self) -> Option<u32> {
        self.shared.read().ok().and_then(|s| s.token)
    }

    /// Returns the server-observed NAT address once joined, if reported.
    pub fn nat_address(&self) -> Option<String> {
        self.shared.read().ok().and_then(|s| s.nat_address.clone())
    }
}

async fn flush_outbound(socket: &UdpSocket, dest: SocketAddr, out: &mut Vec<Vec<u8>>, engine: &MediaEngine) {
    for pkt in out.drain(..) {
        if let Err(err) = socket.send_to(&pkt, dest).await {
            engine.emit_log(json!({ "kind": "media_socket_error", "error": err.to_string() }));
        }
    }
}

fn sync_join_shared(shared: &Arc<RwLock<SharedState>>, engine: &MediaEngine) {
    if let Ok(mut s) = shared.write() {
        s.token = engine.token;
        s.nat_address = engine.nat_address.clone();
        s.stats = engine.stats.clone();
    }
}

fn sync_stats_only(shared: &Arc<RwLock<SharedState>>, engine: &MediaEngine) {
    if let Ok(mut s) = shared.write() {
        s.stats = engine.stats.clone();
    }
}

fn next_silence_deadline(last_audio_at: Option<tokio::time::Instant>) -> tokio::time::Instant {
    let now = tokio::time::Instant::now();
    let stall_window = Duration::from_millis(3 * FRAME_MS + 1);
    match last_audio_at {
        Some(t) => {
            let stall_at = t + stall_window;
            if stall_at > now {
                stall_at
            } else {
                now + Duration::from_millis(FRAME_MS)
            }
        }
        None => now + Duration::from_millis(FRAME_MS),
    }
}

async fn run_media_loop(
    socket: UdpSocket,
    mut engine: MediaEngine,
    mut cmd_rx: mpsc::UnboundedReceiver<MediaCmd>,
    shared: Arc<RwLock<SharedState>>,
) {
    let dest = engine.server_addr;
    let mut recv_buf = vec![0u8; 4096];
    let mut outbound: Vec<Vec<u8>> = Vec::new();

    let mut joined = false;
    let mut silence_active = false;
    let mut stats_dirty = false;
    let mut video_tick_armed = false;
    let mut last_audio_at: Option<tokio::time::Instant> = None;
    let mut join_reply: Option<oneshot::Sender<Option<u32>>> = None;
    let mut init_pkt: Option<Vec<u8>> = None;

    let mut init_retry_tick = tokio::time::interval(Duration::from_secs(3600));
    init_retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut join_deadline_active = false;
    let join_sleep = tokio::time::sleep(Duration::from_secs(3600));
    tokio::pin!(join_sleep);

    let mut ping_tick = tokio::time::interval(Duration::from_millis(engine.opts.ping_ms.max(10)));
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut rtcp_tick = tokio::time::interval(Duration::from_millis(engine.opts.rtcp_ms.max(10)));
    rtcp_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut video_tick = tokio::time::interval(Duration::from_millis(VIDEO_TICK_MS));
    video_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut stats_sync_tick = tokio::time::interval(Duration::from_secs(1));
    stats_sync_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let silence_sleep = tokio::time::sleep(Duration::from_secs(3600));
    tokio::pin!(silence_sleep);

    loop {
        let need_video = joined && engine.needs_video_tick();
        if need_video && !video_tick_armed {
            video_tick.reset();
        }
        video_tick_armed = need_video;

        tokio::select! {
            res = socket.recv_from(&mut recv_buf) => {
                match res {
                    Ok((n, from)) => {
                        let now = now_ms();
                        let join_res = engine.handle_datagram(&recv_buf[..n], from, now, &mut outbound);
                        stats_dirty = true;
                        flush_outbound(&socket, dest, &mut outbound, &engine).await;
                        if let Some(ok) = join_res {
                            if ok && engine.token.is_some() {
                                sync_join_shared(&shared, &engine);
                                stats_dirty = false;
                            }
                            if let Some(reply) = join_reply.take() {
                                join_deadline_active = false;
                                init_pkt = None;
                                if ok && engine.token.is_some() {
                                    joined = true;
                                    ping_tick.reset();
                                    rtcp_tick.reset();
                                    if silence_active {
                                        silence_sleep.as_mut().reset(next_silence_deadline(last_audio_at));
                                    }
                                    let _ = reply.send(engine.token);
                                } else {
                                    sync_stats_only(&shared, &engine);
                                    stats_dirty = false;
                                    let _ = reply.send(None);
                                }
                            }
                        }
                    }
                    Err(err) => {
                        engine.emit_log(json!({ "kind": "media_socket_error", "error": err.to_string() }));
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(MediaCmd::Join { timeout_ms, retry_ms, reply }) => {
                        if engine.closed {
                            let _ = reply.send(None);
                            continue;
                        }
                        match engine.build_init_packet() {
                            Ok(pkt) => {
                                engine.send_init(&pkt, &mut outbound);
                                flush_outbound(&socket, dest, &mut outbound, &engine).await;
                                stats_dirty = true;
                                init_pkt = Some(pkt);
                                join_reply = Some(reply);
                                init_retry_tick = tokio::time::interval(Duration::from_millis(retry_ms.max(10)));
                                init_retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                                init_retry_tick.reset();
                                join_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(timeout_ms));
                                join_deadline_active = true;
                            }
                            Err(_) => {
                                let _ = reply.send(None);
                            }
                        }
                    }
                    Some(MediaCmd::StartSilence) => {
                        if !engine.closed {
                            silence_active = true;
                            if joined {
                                silence_sleep.as_mut().reset(next_silence_deadline(last_audio_at));
                            }
                        }
                    }
                    Some(MediaCmd::SendAudio(opus)) => {
                        let now_inst = tokio::time::Instant::now();
                        last_audio_at = Some(now_inst);
                        if silence_active && joined {
                            silence_sleep.as_mut().reset(now_inst + Duration::from_millis(3 * FRAME_MS + 1));
                        }
                        engine.send_audio_frame(&opus, now_ms(), &mut outbound);
                        if !outbound.is_empty() {
                            flush_outbound(&socket, dest, &mut outbound, &engine).await;
                            stats_dirty = true;
                        }
                    }
                    Some(MediaCmd::SendVideo { key, data }) => {
                        engine.send_video_frame(key, &data, &mut outbound);
                        if !outbound.is_empty() {
                            flush_outbound(&socket, dest, &mut outbound, &engine).await;
                            stats_dirty = true;
                        }
                    }
                    Some(MediaCmd::RequestKeyFrame { why, min_gap_ms }) => {
                        engine.request_key_frame(&why, min_gap_ms, now_ms(), &mut outbound);
                        if !outbound.is_empty() {
                            flush_outbound(&socket, dest, &mut outbound, &engine).await;
                            stats_dirty = true;
                        }
                    }
                    Some(MediaCmd::GetStats(reply)) => {
                        let st = engine.stats.clone();
                        let _ = reply.send(st);
                    }
                    Some(MediaCmd::Close { send_end_call }) => {
                        if send_end_call {
                            engine.send_end_call(&mut outbound);
                            flush_outbound(&socket, dest, &mut outbound, &engine).await;
                        }
                        engine.closed = true;
                        if let Some(reply) = join_reply.take() {
                            let _ = reply.send(None);
                        }
                        sync_join_shared(&shared, &engine);
                        break;
                    }
                    None => {
                        engine.closed = true;
                        if let Some(reply) = join_reply.take() {
                            let _ = reply.send(None);
                        }
                        sync_join_shared(&shared, &engine);
                        break;
                    }
                }
            }
            _ = init_retry_tick.tick(), if join_reply.is_some() => {
                if let Some(ref pkt) = init_pkt {
                    let clone = pkt.clone();
                    engine.send_init(&clone, &mut outbound);
                    flush_outbound(&socket, dest, &mut outbound, &engine).await;
                    stats_dirty = true;
                }
            }
            _ = &mut join_sleep, if join_deadline_active => {
                join_deadline_active = false;
                init_pkt = None;
                if let Some(reply) = join_reply.take() {
                    let _ = reply.send(None);
                }
            }
            _ = ping_tick.tick(), if joined => {
                engine.send_ping(&mut outbound);
                flush_outbound(&socket, dest, &mut outbound, &engine).await;
                stats_dirty = true;
            }
            _ = rtcp_tick.tick(), if joined => {
                engine.send_rtcp(now_ms(), &mut outbound);
                flush_outbound(&socket, dest, &mut outbound, &engine).await;
                stats_dirty = true;
            }
            _ = video_tick.tick(), if video_tick_armed => {
                engine.video_tick(now_ms(), &mut outbound);
                if !outbound.is_empty() {
                    flush_outbound(&socket, dest, &mut outbound, &engine).await;
                }
                stats_dirty = true;
            }
            _ = &mut silence_sleep, if joined && silence_active => {
                engine.send_silence_if_idle(now_ms(), &mut outbound);
                if !outbound.is_empty() {
                    flush_outbound(&socket, dest, &mut outbound, &engine).await;
                    stats_dirty = true;
                }
                silence_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(FRAME_MS));
            }
            _ = stats_sync_tick.tick(), if stats_dirty => {
                sync_stats_only(&shared, &engine);
                stats_dirty = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call::rtcp::{fir, pli};

    const SESSION: &str = "TEST-made-up-session-0123456789abcdefghijklmnop";

    #[test]
    fn parse_server_ipv4_ipv6_and_rejects_malformed() {
        assert_eq!(
            parse_server("192.0.2.10:4200"),
            Some(MediaServer {
                host: "192.0.2.10".into(),
                port: 4200
            })
        );
        assert_eq!(
            parse_server("2001:db8::7:4200"),
            Some(MediaServer {
                host: "2001:db8::7".into(),
                port: 4200
            })
        );
        assert_eq!(
            parse_server("[2001:db8::7]:4200"),
            Some(MediaServer {
                host: "2001:db8::7".into(),
                port: 4200
            })
        );
        assert_eq!(
            parse_server("::1:4200"),
            Some(MediaServer {
                host: "::1".into(),
                port: 4200
            })
        );
        for bad in [
            "nope",
            "1.2.3.4.5:4200",
            ":::4200",
            "::::4200",
            "[192.0.2.10:4200",
            "192.0.2.10]:4200",
            "192.0.2.10:0",
            "192.0.2.10:65536",
        ] {
            assert_eq!(parse_server(bad), None, "should reject {bad}");
        }
    }

    #[test]
    fn engine_security_source_filtering_userid_rtcp_token_and_received_cap() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let opts = MediaCallOptions::new(
            MediaServer {
                host: "192.0.2.10".into(),
                port: 4200,
            },
            1001,
            2002,
            3003,
            SESSION.into(),
            false,
        );
        let mut engine = MediaEngine::new(opts, tx).unwrap();
        assert_ne!(engine.ssrc, engine.video_ssrc);
        assert_ne!(engine.ssrc, 2002);

        let make_init_reply = |user_id: u32, token: u32| -> Vec<u8> {
            let req = zrtc::init_call(InitCallParams {
                is_caller: false,
                user_id,
                partner_id: 2002,
                call_id: 3003,
                session: SESSION,
                attempt: 1,
                extra: "",
            })
            .unwrap();
            let mut reply = req[..21].to_vec();
            reply[0] = kind::RESPONSE;
            reply.extend_from_slice(&0u32.to_le_bytes());
            reply.extend_from_slice(&1u32.to_le_bytes());
            reply.extend_from_slice(&token.to_le_bytes());
            reply
        };

        let mut out = Vec::new();
        // Wrong source IP / port rejected
        assert_eq!(
            engine.handle_datagram(&make_init_reply(1001, 111), "10.0.0.1:4200".parse().unwrap(), 1000, &mut out),
            None
        );
        assert_eq!(
            engine.handle_datagram(&make_init_reply(1001, 111), "192.0.2.10:9999".parse().unwrap(), 1000, &mut out),
            None
        );
        assert_eq!(engine.token, None);
        assert_eq!(engine.stats.received.get("other_source"), Some(&2));

        // Right source, wrong userId rejected
        assert_eq!(
            engine.handle_datagram(&make_init_reply(9999, 222), "192.0.2.10:4200".parse().unwrap(), 1000, &mut out),
            None
        );
        assert_eq!(engine.token, None);
        assert_eq!(engine.stats.received.get("init_wrong_user"), Some(&1));

        // IPv4-mapped IPv6 + exact port + matching userId succeeds
        assert_eq!(
            engine.handle_datagram(
                &make_init_reply(1001, 777),
                "[::ffff:192.0.2.10]:4200".parse().unwrap(),
                1000,
                &mut out
            ),
            Some(true)
        );
        assert_eq!(engine.token, Some(777));

        // VideoRtcp wrong token rejected; right token accepted
        let v_ssrc = engine.video_ssrc;
        let bad_pli = zrtc::rtcp(888, &pli(0x5555, v_ssrc), true).unwrap();
        engine.handle_datagram(&bad_pli, "192.0.2.10:4200".parse().unwrap(), 1000, &mut out);
        assert_eq!(engine.stats.received.get("rtcp_wrong_token"), Some(&1));
        assert_eq!(engine.stats.video.key_requests, 0);

        let good_pli = zrtc::rtcp(777, &pli(0x5555, v_ssrc), true).unwrap();
        engine.handle_datagram(&good_pli, "192.0.2.10:4200".parse().unwrap(), 1000, &mut out);
        assert_eq!(engine.stats.video.key_requests, 1);

        // Drain events and check KeyFrameRequest arrived
        let mut saw_kfr = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, MediaEvent::KeyFrameRequest) {
                saw_kfr = true;
            }
        }
        assert!(saw_kfr);

        // Unauthenticated AudioIn/VideoIn must not mutate SSRCs or payloadTypes
        let mut bogus_rtp = vec![0u8; 30];
        bogus_rtp[0] = 0x80;
        bogus_rtp[1] = OPUS_PT;
        bogus_rtp[2..4].copy_from_slice(&55u16.to_be_bytes());
        bogus_rtp[4..8].copy_from_slice(&12345u32.to_be_bytes());
        bogus_rtp[8..12].copy_from_slice(&0xdead_beefu32.to_be_bytes());
        let mut aud = vec![kind::AUDIO_IN];
        aud.extend_from_slice(&bogus_rtp);
        let mut vid = vec![kind::VIDEO_IN];
        vid.extend_from_slice(&bogus_rtp);
        engine.handle_datagram(&aud, "192.0.2.10:4200".parse().unwrap(), 1000, &mut out);
        engine.handle_datagram(&vid, "192.0.2.10:4200".parse().unwrap(), 1000, &mut out);
        assert_eq!(engine.stats.audio.failed, 1);
        assert_eq!(engine.stats.audio.ssrcs, 0);
        assert!(engine.stats.audio.payload_types.is_empty());
        assert_eq!(engine.stats.video.failed, 1);
        assert_eq!(engine.stats.video.ssrcs, 0);

        // Cap received keys at 64 + other_cmd overflow
        for k in 100u8..220 {
            engine.handle_datagram(&[k, 0, 1], "192.0.2.10:4200".parse().unwrap(), 1000, &mut out);
        }
        assert!(engine.stats.received.len() <= 65);
        assert!(engine.stats.received.get("other_cmd").copied().unwrap_or(0) > 0);
    }

    #[tokio::test]
    async fn media_call_audio_video_rtcp_pli_end_to_end() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        let server = Arc::new(server);

        let srv = Arc::clone(&server);
        let (done_tx, mut done_rx) = oneshot::channel::<()>();
        let srv_stats = Arc::new(RwLock::new((0usize, 0usize, 0usize, 0usize, 0usize)));
        let srv_stats_bg = Arc::clone(&srv_stats);
        let srv_task = tokio::spawn(async move {
            let mut caller_audio = SrtpContext::from_session(SESSION).unwrap();
            let mut caller_video = SrtpContext::from_session(SESSION).unwrap();
            let mut reader_audio = SrtpContext::from_session(SESSION).unwrap();
            let mut reader_video = SrtpContext::from_session(SESSION).unwrap();
            let mut buf = vec![0u8; 4096];

            loop {
                tokio::select! {
                    _ = &mut done_rx => break,
                    res = srv.recv_from(&mut buf) => {
                        let Ok((n, from)) = res else { break };
                        let msg = &buf[..n];
                        if msg.is_empty() { continue; }
                        match msg[0] {
                            kind::REQUEST => {
                                if let Ok(Packet::Control { header, .. }) = zrtc::parse(msg) {
                                    if header.cmd == cmd::INIT_CALL_CALLEE {
                                        let mut reply = msg[..21].to_vec();
                                        reply[0] = kind::RESPONSE;
                                        reply.extend_from_slice(&0u32.to_le_bytes());
                                        reply.extend_from_slice(&1u32.to_le_bytes());
                                        reply.extend_from_slice(&12345u32.to_le_bytes());
                                        reply.extend_from_slice(&9u16.to_le_bytes());
                                        reply.extend_from_slice(b"127.0.0.1");
                                        let _ = srv.send_to(&reply, from).await;

                                        // Send 1 video key frame + 3 audio packets under SAME SSRC (separate SRTP contexts)
                                        let ssrc = 465_000_001u32;
                                        let mut vrtp = vec![0x80, 0x80 | 98, 0, 10, 0, 0, 0x23, 0x28];
                                        vrtp.extend_from_slice(&ssrc.to_be_bytes());
                                        vrtp.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88, 0x80]);
                                        let mut vp = vec![kind::VIDEO_IN];
                                        vp.extend_from_slice(&caller_video.protect(&vrtp).unwrap());
                                        let _ = srv.send_to(&vp, from).await;

                                        for i in 0u16..3 {
                                            let mut artp = vec![0x80, OPUS_PT];
                                            artp.extend_from_slice(&(100 + i).to_be_bytes());
                                            artp.extend_from_slice(&(1000 + 320 * i as u32).to_be_bytes());
                                            artp.extend_from_slice(&ssrc.to_be_bytes());
                                            artp.extend_from_slice(&[0x78, 1, 2, 3]);
                                            let mut ap = vec![kind::AUDIO_IN];
                                            ap.extend_from_slice(&caller_audio.protect(&artp).unwrap());
                                            let _ = srv.send_to(&ap, from).await;
                                        }
                                    } else if header.cmd == cmd::PING {
                                        srv_stats_bg.write().unwrap().2 += 1;
                                    } else if header.cmd == cmd::END_CALL {
                                        srv_stats_bg.write().unwrap().3 += 1;
                                    }
                                }
                            }
                            kind::AUDIO => {
                                if msg.len() > 5 && reader_audio.unprotect(&msg[5..]).is_some() {
                                    srv_stats_bg.write().unwrap().0 += 1;
                                }
                            }
                            kind::VIDEO => {
                                if msg.len() > 5 {
                                    if let Some(plain) = reader_video.unprotect(&msg[5..]) {
                                        srv_stats_bg.write().unwrap().1 += 1;
                                        if let Some(h) = rtp_header(&plain) {
                                            // Send FIR requesting a keyframe from client
                                            let fir_pkt = zrtc::rtcp(12345, &fir(465_000_001, h.ssrc, 1), true).unwrap();
                                            let _ = srv.send_to(&fir_pkt, from).await;
                                        }
                                    }
                                }
                            }
                            kind::AUDIO_RTCP | kind::VIDEO_RTCP => {
                                srv_stats_bg.write().unwrap().4 += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        let mut opts = MediaCallOptions::new(
            MediaServer {
                host: "127.0.0.1".into(),
                port,
            },
            438_000_002,
            465_000_001,
            321_000_003,
            SESSION.into(),
            false,
        );
        opts.ping_ms = 30;
        opts.rtcp_ms = 40;

        let (call, mut ev_rx) = MediaCall::new(opts).await.unwrap();
        assert_eq!(call.join(2000, 100).await, Some(12345));
        assert_eq!(call.nat_address().as_deref(), Some("127.0.0.1"));

        call.start_silence();
        call.send_video_frame(true, vec![0, 0, 0, 1, 0x65, 0x88, 0x99]);
        call.request_key_frame("app");
        tokio::time::sleep(Duration::from_millis(160)).await;
        let live_st = call.stats_async().await;
        assert_eq!(live_st.audio.decrypted, 3);
        assert_eq!(live_st.video.decrypted, 1);
        assert_eq!(live_st.video.sent_frames, 1);
        call.close(true);
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = done_tx.send(());
        let _ = srv_task.await;

        let mut audio_cnt = 0;
        let mut video_cnt = 0;
        let mut key_req_cnt = 0;
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                MediaEvent::Audio { .. } => audio_cnt += 1,
                MediaEvent::Video(f) => {
                    assert!(f.key);
                    video_cnt += 1;
                }
                MediaEvent::KeyFrameRequest => key_req_cnt += 1,
                MediaEvent::Log(_) => {}
            }
        }
        assert_eq!(audio_cnt, 3);
        assert_eq!(video_cnt, 1);
        assert!(key_req_cnt >= 1);

        let st = call.stats();
        assert_eq!(st.audio.decrypted, 3);
        assert_eq!(st.audio.timestamp_steps, vec![320, 320]);
        assert_eq!(st.video.decrypted, 1);
        assert_eq!(st.video.keys, 1);
        assert_eq!(st.video.sent_frames, 1);

        let (aud_ok, vid_ok, pings, ends, rtcps) = *srv_stats.read().unwrap();
        assert!(aud_ok >= 3, "sent silence/audio frames: {aud_ok}");
        assert_eq!(vid_ok, 1, "sent video frame");
        assert!(pings >= 1, "sent pings");
        assert_eq!(ends, 1, "sent EndCall");
        assert!(rtcps >= 1, "sent RTCP");
    }

    /// Continuous 20 ms audio + ~17 fps video in both directions for ~2 s.
    #[tokio::test]
    async fn media_call_continuous_bidirectional_stream() {
        let server = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let port = server.local_addr().unwrap().port();
        let counts = Arc::new(RwLock::new((0usize, 0usize)));
        let counts_bg = Arc::clone(&counts);
        let srv = Arc::clone(&server);
        let (done_tx, mut done_rx) = oneshot::channel::<()>();
        let srv_task = tokio::spawn(async move {
            let mut tx_a = SrtpContext::from_session(SESSION).unwrap();
            let mut tx_v = SrtpContext::from_session(SESSION).unwrap();
            let mut rx_a = SrtpContext::from_session(SESSION).unwrap();
            let mut rx_v = SrtpContext::from_session(SESSION).unwrap();
            let mut buf = vec![0u8; 4096];
            let mut peer: Option<SocketAddr> = None;
            let (mut aseq, mut vseq) = (65_530u16, 500u16);
            let mut t = tokio::time::interval(Duration::from_millis(20));
            let mut n = 0u32;
            loop {
                tokio::select! {
                    _ = &mut done_rx => break,
                    _ = t.tick(), if peer.is_some() => {
                        let to = peer.unwrap();
                        let mut artp = vec![0x80, OPUS_PT];
                        artp.extend_from_slice(&aseq.to_be_bytes());
                        artp.extend_from_slice(&(n * 960).to_be_bytes());
                        artp.extend_from_slice(&777u32.to_be_bytes());
                        artp.extend_from_slice(&[0x78, 1, 2, 3, 4, 5]);
                        aseq = aseq.wrapping_add(1);
                        let mut ap = vec![kind::AUDIO_IN];
                        ap.extend_from_slice(&tx_a.protect(&artp).unwrap());
                        let _ = srv.send_to(&ap, to).await;
                        if n % 3 == 0 {
                            let key = n % 30 == 0;
                            let mut vrtp = vec![0x80, 0x80 | if key { 98 } else { 97 }];
                            vrtp.extend_from_slice(&vseq.to_be_bytes());
                            vrtp.extend_from_slice(&(n * 3000).to_be_bytes());
                            vrtp.extend_from_slice(&777u32.to_be_bytes());
                            vrtp.extend_from_slice(&[0, 0, 0, 1, if key { 0x65 } else { 0x41 }, 0x88, 0x80]);
                            vseq = vseq.wrapping_add(1);
                            let mut vp = vec![kind::VIDEO_IN];
                            vp.extend_from_slice(&tx_v.protect(&vrtp).unwrap());
                            let _ = srv.send_to(&vp, to).await;
                        }
                        n += 1;
                    }
                    res = srv.recv_from(&mut buf) => {
                        let Ok((len, from)) = res else { break };
                        let msg = &buf[..len];
                        if msg.is_empty() { continue; }
                        match msg[0] {
                            kind::REQUEST => {
                                if let Ok(Packet::Control { header, .. }) = zrtc::parse(msg) {
                                    if header.cmd == cmd::INIT_CALL_CALLEE {
                                        let mut reply = msg[..21].to_vec();
                                        reply[0] = kind::RESPONSE;
                                        reply.extend_from_slice(&0u32.to_le_bytes());
                                        reply.extend_from_slice(&1u32.to_le_bytes());
                                        reply.extend_from_slice(&12345u32.to_le_bytes());
                                        reply.extend_from_slice(&9u16.to_le_bytes());
                                        reply.extend_from_slice(b"127.0.0.1");
                                        let _ = srv.send_to(&reply, from).await;
                                        peer = Some(from);
                                    }
                                }
                            }
                            kind::AUDIO if len > 5 && rx_a.unprotect(&msg[5..]).is_some() => {
                                counts_bg.write().unwrap().0 += 1;
                            }
                            kind::VIDEO if len > 5 && rx_v.unprotect(&msg[5..]).is_some() => {
                                counts_bg.write().unwrap().1 += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        let opts = MediaCallOptions::new(
            MediaServer { host: "127.0.0.1".into(), port },
            438_000_002,
            465_000_001,
            321_000_003,
            SESSION.into(),
            false,
        );
        let (call, mut ev_rx) = MediaCall::new(opts).await.unwrap();
        assert_eq!(call.join(2000, 100).await, Some(12345));
        assert_eq!(call.token(), Some(12345));
        call.start_silence();

        let mut tick = tokio::time::interval(Duration::from_millis(20));
        let (mut audio_ev, mut video_ev) = (0usize, 0usize);
        for i in 0..100u32 {
            tick.tick().await;
            call.send_audio_frame(vec![0x78, i as u8, 1, 2, 3, 4, 5, 6]);
            if i % 3 == 0 {
                call.send_video_frame(i % 30 == 0, vec![0, 0, 0, 1, if i % 30 == 0 { 0x65 } else { 0x41 }, 1, 2, 3]);
            }
            while let Ok(ev) = ev_rx.try_recv() {
                match ev {
                    MediaEvent::Audio { .. } => audio_ev += 1,
                    MediaEvent::Video(_) => video_ev += 1,
                    _ => {}
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                MediaEvent::Audio { .. } => audio_ev += 1,
                MediaEvent::Video(_) => video_ev += 1,
                _ => {}
            }
        }
        call.close(false);
        let _ = done_tx.send(());
        let _ = srv_task.await;
        let (a_out, v_out) = *counts.read().unwrap();
        assert!(a_out >= 90, "audio out {a_out}");
        assert!(v_out >= 30, "video out {v_out}");
        assert!(audio_ev >= 90, "audio events {audio_ev}");
        assert!(video_ev >= 25, "video events {video_ev}");
    }

    #[tokio::test]
    async fn close_during_join_returns_none_cleanly() {
        let opts = MediaCallOptions::new(
            MediaServer {
                host: "127.0.0.1".into(),
                port: 1,
            },
            1001,
            2002,
            3003,
            SESSION.into(),
            false,
        );
        let (call, _rx) = MediaCall::new(opts).await.unwrap();
        let c2 = call.clone();
        let handle = tokio::spawn(async move { c2.join(5000, 100).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        call.close(false);
        assert_eq!(handle.await.unwrap(), None);
    }
}
