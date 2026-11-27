//! RTCP (RFC 3550 / 4585 / 5104) encoder, decoder, and receive statistics matching `src/rtcp.ts`.
//! Zalo calls run unencrypted RTCP (`srtcp: 0`), wrapped in zrtc kind 5 (audio) or 15 (video).

use std::collections::{BTreeMap, HashMap};

/// RTCP packet types.
pub mod rtcp_type {
    pub const SR: u8 = 200;
    pub const RR: u8 = 201;
    pub const SDES: u8 = 202;
    pub const BYE: u8 = 203;
    pub const APP: u8 = 204;
    pub const RTPFB: u8 = 205;
    pub const PSFB: u8 = 206;
}

/// Seconds between 1900 (NTP epoch) and 1970 (Unix epoch).
const NTP_OFFSET: i64 = 2_208_988_800;

/// NTP timestamp of `ms` (Unix milliseconds): `(sec, frac)`.
pub fn ntp(ms: i64) -> (u32, u32) {
    let sec = ms.div_euclid(1000) + NTP_OFFSET;
    let rem_ms = ms.rem_euclid(1000) as f64;
    let frac = ((rem_ms / 1000.0) * (4_294_967_296.0_f64)).floor() as u32;
    (sec as u32, frac)
}

/// The middle 32 bits of an NTP timestamp, as reports quote it (LSR).
pub fn ntp_middle(sec: u32, frac: u32) -> u32 {
    ((sec & 0xffff) << 16) | (frac >> 16)
}

/// RFC 3550 reception report block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
    pub highest_seq: u32,
    pub jitter: u32,
    pub lsr: u32,
    pub dlsr: u32,
}

fn write_header(out: &mut Vec<u8>, count: u8, pkt_type: u8, words: u16) {
    out.push(0x80 | (count & 0x1f));
    out.push(pkt_type);
    out.extend_from_slice(&(words - 1).to_be_bytes());
}

fn write_block(out: &mut Vec<u8>, b: &ReportBlock) {
    out.extend_from_slice(&b.ssrc.to_be_bytes());
    out.push(b.fraction_lost);
    let lost = b.cumulative_lost.clamp(-0x80_0000, 0x7f_ffff);
    let lost_bytes = lost.to_be_bytes();
    out.extend_from_slice(&lost_bytes[1..4]);
    out.extend_from_slice(&b.highest_seq.to_be_bytes());
    out.extend_from_slice(&b.jitter.to_be_bytes());
    out.extend_from_slice(&b.lsr.to_be_bytes());
    out.extend_from_slice(&b.dlsr.to_be_bytes());
}

/// Encodes an RTCP Sender Report (SR = 200) with optional report blocks.
pub fn sender_report(
    ssrc: u32,
    now_ms: i64,
    rtp_timestamp: u32,
    packets: u32,
    octets: u32,
    blocks: &[ReportBlock],
) -> Vec<u8> {
    let words = 1 + 6 + 6 * (blocks.len() as u16);
    let mut out = Vec::with_capacity((words as usize) * 4);
    write_header(&mut out, blocks.len() as u8, rtcp_type::SR, words);
    out.extend_from_slice(&ssrc.to_be_bytes());
    let (sec, frac) = ntp(now_ms);
    out.extend_from_slice(&sec.to_be_bytes());
    out.extend_from_slice(&frac.to_be_bytes());
    out.extend_from_slice(&rtp_timestamp.to_be_bytes());
    out.extend_from_slice(&packets.to_be_bytes());
    out.extend_from_slice(&octets.to_be_bytes());
    for b in blocks {
        write_block(&mut out, b);
    }
    out
}

/// Encodes an RTCP Receiver Report (RR = 201).
pub fn receiver_report(ssrc: u32, blocks: &[ReportBlock]) -> Vec<u8> {
    let words = 2 + 6 * (blocks.len() as u16);
    let mut out = Vec::with_capacity((words as usize) * 4);
    write_header(&mut out, blocks.len() as u8, rtcp_type::RR, words);
    out.extend_from_slice(&ssrc.to_be_bytes());
    for b in blocks {
        write_block(&mut out, b);
    }
    out
}

/// Encodes an SDES (202) packet carrying a single CNAME item.
pub fn sdes_cname(ssrc: u32, cname: &str) -> Vec<u8> {
    let raw = cname.as_bytes();
    let name = &raw[..raw.len().min(255)];
    let items_len = 2 + name.len() + 1;
    let pad = (4 - ((4 + items_len) % 4)) % 4;
    let chunk_len = 4 + items_len + pad;
    let words = 1 + (chunk_len / 4) as u16;
    let mut out = Vec::with_capacity(4 + chunk_len);
    write_header(&mut out, 1, rtcp_type::SDES, words);
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.push(1); // CNAME
    out.push(name.len() as u8);
    out.extend_from_slice(name);
    out.push(0); // END
    out.resize(out.len() + pad, 0);
    out
}

/// Picture Loss Indication (RFC 4585, PSFB = 206, FMT = 1).
pub fn pli(from: u32, media: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.push(0x81);
    out.push(rtcp_type::PSFB);
    out.extend_from_slice(&2u16.to_be_bytes());
    out.extend_from_slice(&from.to_be_bytes());
    out.extend_from_slice(&media.to_be_bytes());
    out
}

/// Full Intra Request (RFC 5104, PSFB = 206, FMT = 4).
pub fn fir(from: u32, media: u32, seq_no: u8) -> Vec<u8> {
    let mut out = vec![0u8; 20];
    out[0] = 0x84;
    out[1] = rtcp_type::PSFB;
    out[2..4].copy_from_slice(&4u16.to_be_bytes());
    out[4..8].copy_from_slice(&from.to_be_bytes());
    // Media SSRC of the header is 0 for FIR; target SSRC goes in FCI entry.
    out[12..16].copy_from_slice(&media.to_be_bytes());
    out[16] = seq_no;
    out
}

/// Parsed packet inside a compound RTCP datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtcpPacket {
    Sr {
        ssrc: u32,
        ntp_sec: u32,
        ntp_frac: u32,
        rtp_timestamp: u32,
        packets: u32,
        octets: u32,
        blocks: Vec<ReportBlock>,
    },
    Rr {
        ssrc: u32,
        blocks: Vec<ReportBlock>,
    },
    Pli {
        ssrc: u32,
        media_ssrc: u32,
    },
    Fir {
        ssrc: u32,
        media_ssrc: u32,
    },
    Other {
        packet_type: u8,
        ssrc: Option<u32>,
        bytes: usize,
    },
}

fn read_u32_be(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn read_i24_be(b: &[u8], at: usize) -> i32 {
    let sign = if (b[at] & 0x80) != 0 { 0xff } else { 0x00 };
    i32::from_be_bytes([sign, b[at], b[at + 1], b[at + 2]])
}

fn read_blocks(buf: &[u8], mut at: usize, count: u8) -> Vec<ReportBlock> {
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if at + 24 > buf.len() {
            break;
        }
        out.push(ReportBlock {
            ssrc: read_u32_be(buf, at),
            fraction_lost: buf[at + 4],
            cumulative_lost: read_i24_be(buf, at + 5),
            highest_seq: read_u32_be(buf, at + 8),
            jitter: read_u32_be(buf, at + 12),
            lsr: read_u32_be(buf, at + 16),
            dlsr: read_u32_be(buf, at + 20),
        });
        at += 24;
    }
    out
}

/// Splits a compound RTCP packet, stopping at the first malformed part.
pub fn parse_rtcp(buf: &[u8]) -> Vec<RtcpPacket> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 4 <= buf.len() {
        if (buf[at] >> 6) != 2 {
            break;
        }
        let padded = (buf[at] & 0x20) != 0;
        let count = buf[at] & 0x1f;
        let pkt_type = buf[at + 1];
        let words = u16::from_be_bytes([buf[at + 2], buf[at + 3]]) as usize;
        let len = (words + 1) * 4;
        if at + len > buf.len() {
            break;
        }
        let p = &buf[at..at + len];
        let body = if padded {
            let pad_len = p[len - 1] as usize;
            if pad_len < 1 || pad_len > len - 4 {
                break;
            }
            &p[..len - pad_len]
        } else {
            p
        };
        let body_len = body.len();
        if pkt_type == rtcp_type::SR && body_len >= 28 {
            out.push(RtcpPacket::Sr {
                ssrc: read_u32_be(body, 4),
                ntp_sec: read_u32_be(body, 8),
                ntp_frac: read_u32_be(body, 12),
                rtp_timestamp: read_u32_be(body, 16),
                packets: read_u32_be(body, 20),
                octets: read_u32_be(body, 24),
                blocks: read_blocks(body, 28, count),
            });
        } else if pkt_type == rtcp_type::RR && body_len >= 8 {
            out.push(RtcpPacket::Rr {
                ssrc: read_u32_be(body, 4),
                blocks: read_blocks(body, 8, count),
            });
        } else if pkt_type == rtcp_type::PSFB && count == 1 && body_len >= 12 {
            out.push(RtcpPacket::Pli {
                ssrc: read_u32_be(body, 4),
                media_ssrc: read_u32_be(body, 8),
            });
        } else if pkt_type == rtcp_type::PSFB && count == 4 && body_len >= 20 {
            let sender_ssrc = read_u32_be(body, 4);
            let mut off = 12;
            while off + 8 <= body_len {
                out.push(RtcpPacket::Fir {
                    ssrc: sender_ssrc,
                    media_ssrc: read_u32_be(body, off),
                });
                off += 8;
            }
        } else {
            out.push(RtcpPacket::Other {
                packet_type: pkt_type,
                ssrc: if body_len >= 8 {
                    Some(read_u32_be(body, 4))
                } else {
                    None
                },
                bytes: len,
            });
        }
        at += len;
    }
    out
}

/// Tracks statistics for one incoming RTP stream to generate RFC 3550 report blocks.
#[derive(Debug, Clone)]
pub struct ReceiveStats {
    pub ssrc: u32,
    clock_rate: u32,
    base_seq: Option<u16>,
    max_seq: u16,
    cycles: u64,
    received: u64,
    expected_prior: u64,
    received_prior: u64,
    jitter: f64,
    last_transit: Option<f64>,
    last_sr: Option<(u32, i64)>,
}

impl ReceiveStats {
    pub fn new(ssrc: u32, clock_rate: u32) -> Self {
        Self {
            ssrc,
            clock_rate,
            base_seq: None,
            max_seq: 0,
            cycles: 0,
            received: 0,
            expected_prior: 0,
            received_prior: 0,
            jitter: 0.0,
            last_transit: None,
            last_sr: None,
        }
    }

    /// Records one RTP packet arrival at `arrival_ms`.
    pub fn on_packet(&mut self, seq: u16, timestamp: u32, arrival_ms: i64) {
        if self.base_seq.is_none() {
            self.base_seq = Some(seq);
            self.max_seq = seq;
        } else {
            let delta = seq.wrapping_sub(self.max_seq);
            if delta > 0 && delta < 0x8000 {
                if seq < self.max_seq {
                    self.cycles += 0x1_0000;
                }
                self.max_seq = seq;
            }
        }
        self.received += 1;
        let arrival = (arrival_ms as f64 / 1000.0) * (self.clock_rate as f64);
        let transit = arrival - (timestamp as f64);
        if let Some(prev) = self.last_transit {
            let mut d = (transit - prev).abs();
            if d > 2_147_483_648.0 {
                d = (d - 4_294_967_296.0).abs();
            }
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.last_transit = Some(transit);
    }

    /// Records a sender report arrival for this stream (remembered for LSR / DLSR).
    pub fn on_sender_report(&mut self, ntp_sec: u32, ntp_frac: u32, arrival_ms: i64) {
        self.last_sr = Some((ntp_middle(ntp_sec, ntp_frac), arrival_ms));
    }

    /// Builds the report block for `now_ms`; `None` before the first packet.
    pub fn block(&mut self, now_ms: i64) -> Option<ReportBlock> {
        let base_seq = self.base_seq? as u64;
        let extended_max = self.cycles + (self.max_seq as u64);
        let expected = extended_max.saturating_sub(base_seq) + 1;
        let lost = (expected as i64) - (self.received as i64);
        let expected_interval = (expected as i64) - (self.expected_prior as i64);
        let received_interval = (self.received as i64) - (self.received_prior as i64);
        self.expected_prior = expected;
        self.received_prior = self.received;
        let lost_interval = expected_interval - received_interval;
        let fraction = if expected_interval <= 0 || lost_interval <= 0 {
            0u8
        } else {
            let ratio = ((lost_interval as i128 * 256) / (expected_interval as i128)).clamp(0, 255);
            ratio as u8
        };
        let (lsr, dlsr) = match self.last_sr {
            Some((mid, at)) => {
                let delay_ms = (now_ms - at).max(0) as f64;
                let d = ((delay_ms / 1000.0) * 65536.0).floor().min(u32::MAX as f64) as u32;
                (mid, d)
            }
            None => (0, 0),
        };
        Some(ReportBlock {
            ssrc: self.ssrc,
            fraction_lost: fraction,
            cumulative_lost: lost.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            highest_seq: extended_max as u32,
            jitter: self.jitter.floor().max(0.0).min(u32::MAX as f64) as u32,
            lsr,
            dlsr,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Receiver feedback: transport-wide congestion control (TWCC) and generic NACK.
// ---------------------------------------------------------------------------------------------

/// RTP header-extension element id Zalo uses for the 16-bit transport-wide sequence number.
const TWCC_EXT_ID: u8 = 5;
/// Most packets one TWCC feedback covers.
const TWCC_MAX_PACKETS: i64 = 400;
/// Arrivals kept without being reported before the oldest are discarded.
const TWCC_MAX_PENDING: usize = 2048;

/// Reads the transport-wide sequence number (one-byte-header extension, id 5, 2 bytes) from a
/// plaintext RTP packet. `None` when the packet has no such element.
pub fn read_tw_seq(rtp: &[u8]) -> Option<u16> {
    if rtp.len() < 16 || (rtp[0] & 0x10) == 0 {
        return None;
    }
    let mut at = 12 + 4 * (rtp[0] & 0x0f) as usize;
    if at + 4 > rtp.len() || u16::from_be_bytes([rtp[at], rtp[at + 1]]) != 0xbede {
        return None;
    }
    let words = u16::from_be_bytes([rtp[at + 2], rtp[at + 3]]) as usize;
    let end = rtp.len().min(at + 4 + 4 * words);
    at += 4;
    while at < end {
        let b = rtp[at];
        if b == 0 {
            at += 1; // padding
            continue;
        }
        let id = b >> 4;
        let len = (b & 0x0f) as usize + 1;
        if id == 15 {
            return None; // reserved: stop parsing
        }
        if id == TWCC_EXT_ID && len == 2 && at + 3 <= end {
            return Some(u16::from_be_bytes([rtp[at + 1], rtp[at + 2]]));
        }
        at += 1 + len;
    }
    None
}

/// Records transport-wide sequence arrivals and builds TWCC feedback (RTPFB, FMT 15).
#[derive(Debug, Default)]
pub struct TwccRecorder {
    /// Unwrapped transport seq -> arrival time in 250 microsecond units.
    arrivals: BTreeMap<i64, i64>,
    next: Option<i64>,
    top: Option<i64>,
    fb_count: u8,
}

impl TwccRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    fn unwrap(&self, seq: i64) -> i64 {
        let Some(top) = self.top else {
            return seq;
        };
        let mut u = seq + (top - (top & 0xffff));
        if u - top > 0x8000 {
            u -= 0x1_0000;
        } else if top - u > 0x8000 {
            u += 0x1_0000;
        }
        u
    }

    /// Notes the arrival of transport seq `seq` at `now_ms`.
    pub fn add(&mut self, seq: u16, now_ms: i64) {
        let u = self.unwrap(seq as i64);
        let next = *self.next.get_or_insert(u);
        if u < next || self.arrivals.contains_key(&u) {
            return;
        }
        self.arrivals.insert(u, now_ms * 4);
        if self.top.is_none_or(|t| u > t) {
            self.top = Some(u);
        }
        while self.arrivals.len() > TWCC_MAX_PENDING {
            if let Some((&oldest, _)) = self.arrivals.iter().next() {
                self.arrivals.remove(&oldest);
                self.next = Some(oldest + 1);
            }
        }
    }

    /// True when arrivals exist that no feedback has covered yet.
    pub fn pending(&self) -> bool {
        matches!((self.next, self.top), (Some(n), Some(t)) if t >= n) && !self.arrivals.is_empty()
    }

    /// Builds feedback packets for every unreported arrival (usually one).
    pub fn build(&mut self, sender_ssrc: u32, media_ssrc: u32) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while self.pending() {
            let base = self.next.unwrap_or(0);
            let top = self.top.unwrap_or(base);
            let end = top.min(base + TWCC_MAX_PACKETS - 1);
            if self.arrivals.range(base..=end).next().is_none() {
                self.next = Some(end + 1);
                continue;
            }
            out.push(encode_twcc(
                sender_ssrc,
                media_ssrc,
                base,
                end,
                &self.arrivals,
                self.fb_count,
            ));
            self.fb_count = self.fb_count.wrapping_add(1);
            let keep = self.arrivals.split_off(&(end + 1));
            self.arrivals = keep;
            self.next = Some(end + 1);
        }
        out
    }
}

/// Encodes one TWCC feedback covering `base..=end` (unwrapped seqs); at least one must have arrived.
fn encode_twcc(
    sender_ssrc: u32,
    media_ssrc: u32,
    base: i64,
    end: i64,
    arrivals: &BTreeMap<i64, i64>,
    fb_count: u8,
) -> Vec<u8> {
    let count = (end - base + 1) as usize;
    let first_q = arrivals
        .range(base..=end)
        .next()
        .map(|(_, &q)| q)
        .unwrap_or(0);
    let ref_units = first_q.div_euclid(256);
    let ref_q = ref_units * 256;
    let mut symbols: Vec<u8> = Vec::with_capacity(count);
    let mut deltas: Vec<i32> = Vec::new();
    let mut prev_q = ref_q;
    for s in base..=end {
        match arrivals.get(&s) {
            None => symbols.push(0),
            Some(&q) => {
                let d = q - prev_q;
                prev_q = q;
                if (0..=255).contains(&d) {
                    symbols.push(1);
                } else {
                    symbols.push(2);
                }
                deltas.push(d.clamp(-32768, 32767) as i32);
            }
        }
    }
    let mut chunks: Vec<u16> = Vec::new();
    let mut i = 0usize;
    while i < symbols.len() {
        let mut run = 1usize;
        while i + run < symbols.len() && symbols[i + run] == symbols[i] && run < 0x1fff {
            run += 1;
        }
        if run >= 8 {
            chunks.push(((symbols[i] as u16) << 13) | run as u16);
            i += run;
            continue;
        }
        let mut v: u16 = 0xc000;
        for k in 0..7 {
            let sym = symbols.get(i + k).copied().unwrap_or(0) as u16;
            v |= sym << (12 - 2 * k);
        }
        chunks.push(v);
        i += 7;
    }
    let delta_bytes: usize = symbols
        .iter()
        .map(|&s| match s {
            1 => 1,
            2 => 2,
            _ => 0,
        })
        .sum();
    let len = 20 + 2 * chunks.len() + delta_bytes;
    let padded = (len + 3) & !3;
    let mut b = Vec::with_capacity(padded);
    b.push(0x8f);
    b.push(rtcp_type::RTPFB);
    b.extend_from_slice(&((padded / 4 - 1) as u16).to_be_bytes());
    b.extend_from_slice(&sender_ssrc.to_be_bytes());
    b.extend_from_slice(&media_ssrc.to_be_bytes());
    b.extend_from_slice(&((base & 0xffff) as u16).to_be_bytes());
    b.extend_from_slice(&(count as u16).to_be_bytes());
    b.extend_from_slice(&((ref_units & 0xff_ffff) as u32).to_be_bytes()[1..4]);
    b.push(fb_count);
    for c in chunks {
        b.extend_from_slice(&c.to_be_bytes());
    }
    let mut di = 0usize;
    for &s in &symbols {
        match s {
            1 => {
                b.push(deltas[di] as u8);
                di += 1;
            }
            2 => {
                b.extend_from_slice(&(deltas[di] as i16).to_be_bytes());
                di += 1;
            }
            _ => {}
        }
    }
    b.resize(padded, 0);
    b
}

/// Generic NACK (RFC 4585, RTPFB = 205, FMT = 1) asking for `seqs` (in the order given).
pub fn nack(sender_ssrc: u32, media_ssrc: u32, seqs: &[u16]) -> Vec<u8> {
    let mut items: Vec<(u16, u16)> = Vec::new();
    let mut i = 0;
    while i < seqs.len() {
        let pid = seqs[i];
        let mut blp = 0u16;
        let mut j = i + 1;
        while j < seqs.len() {
            let d = seqs[j].wrapping_sub(pid);
            if !(1..=16).contains(&d) {
                break;
            }
            blp |= 1 << (d - 1);
            j += 1;
        }
        items.push((pid, blp));
        i = j;
    }
    let mut b = Vec::with_capacity(12 + 4 * items.len());
    b.push(0x81);
    b.push(rtcp_type::RTPFB);
    b.extend_from_slice(&((2 + items.len()) as u16).to_be_bytes());
    b.extend_from_slice(&sender_ssrc.to_be_bytes());
    b.extend_from_slice(&media_ssrc.to_be_bytes());
    for (pid, blp) in items {
        b.extend_from_slice(&pid.to_be_bytes());
        b.extend_from_slice(&blp.to_be_bytes());
    }
    b
}

const NACK_MAX_HOLE: u16 = 64;
const NACK_RETRY_MS: i64 = 40;
const NACK_MAX_TRIES: u32 = 5;
const NACK_DROP_MS: i64 = 400;
const NACK_KEEP: usize = 128;
const NACK_MAX_STREAMS: usize = 8;

#[derive(Debug)]
struct NackStream {
    highest: u16,
    /// Missing seqs in detection order: (seq, first noticed at ms, requests sent).
    miss: Vec<(u16, i64, u32)>,
}

/// A NACK request produced by [`NackTracker`]: the stream it concerns and the seqs wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NackRequest {
    pub ssrc: u32,
    pub seqs: Vec<u16>,
}

/// Detects holes in incoming RTP sequence numbers and schedules NACK (re)transmissions.
#[derive(Debug, Default)]
pub struct NackTracker {
    streams: HashMap<u32, NackStream>,
}

impl NackTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while some seq is still being chased (a periodic tick is worth running).
    pub fn has_missing(&self) -> bool {
        self.streams.values().any(|s| !s.miss.is_empty())
    }

    /// Notes an arrived packet and returns the requests due now.
    pub fn note(&mut self, ssrc: u32, seq: u16, now: i64) -> Vec<NackRequest> {
        if !self.streams.contains_key(&ssrc) {
            if self.streams.len() >= NACK_MAX_STREAMS {
                return Vec::new();
            }
            self.streams.insert(
                ssrc,
                NackStream {
                    highest: seq,
                    miss: Vec::new(),
                },
            );
            return Vec::new();
        }
        let st = self.streams.get_mut(&ssrc).expect("checked above");
        let ahead = seq.wrapping_sub(st.highest);
        if ahead == 0 {
            return Self::due(st, ssrc, now);
        }
        if ahead >= 0x8000 {
            st.miss.retain(|m| m.0 != seq);
            return Self::due(st, ssrc, now);
        }
        if ahead > NACK_MAX_HOLE {
            st.highest = seq;
            st.miss.clear();
            return Vec::new();
        }
        for d in 1..ahead {
            let missing = st.highest.wrapping_add(d);
            if !st.miss.iter().any(|m| m.0 == missing) {
                st.miss.push((missing, now, 0));
            }
        }
        st.highest = seq;
        st.miss.retain(|m| m.0 != seq);
        while st.miss.len() > NACK_KEEP {
            st.miss.remove(0);
        }
        Self::due(st, ssrc, now)
    }

    /// Requests due at `now` with no new packet (timer driven).
    pub fn tick(&mut self, now: i64) -> Vec<NackRequest> {
        let mut out = Vec::new();
        for (&ssrc, st) in self.streams.iter_mut() {
            out.extend(Self::due(st, ssrc, now));
        }
        out
    }

    fn due(st: &mut NackStream, ssrc: u32, now: i64) -> Vec<NackRequest> {
        let mut seqs = Vec::new();
        st.miss.retain_mut(|m| {
            if now - m.1 > NACK_DROP_MS {
                return false;
            }
            if m.2 < NACK_MAX_TRIES && now >= m.1 + (m.2 as i64) * NACK_RETRY_MS {
                m.2 += 1;
                seqs.push(m.0);
            }
            true
        });
        if seqs.is_empty() {
            return Vec::new();
        }
        let high = st.highest;
        // Oldest (furthest behind the highest seq) first.
        seqs.sort_by_key(|&s| std::cmp::Reverse(high.wrapping_sub(s)));
        vec![NackRequest { ssrc, seqs }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_block() -> ReportBlock {
        ReportBlock {
            ssrc: 0xabcd_ef01,
            fraction_lost: 25,
            cumulative_lost: 3,
            highest_seq: 70000,
            jitter: 42,
            lsr: 0x1234_5678,
            dlsr: 65536,
        }
    }

    #[test]
    fn sr_rr_sdes_match_rfc3550_vectors() {
        let blk = sample_block();
        assert_eq!(
            hex::encode(sender_report(
                0x1122_3344,
                1_795_632_118_951,
                160_000,
                50,
                1234,
                &[blk]
            )),
            "81c8000c11223344eeb1aa76f374bc6a0002710000000032000004d2abcdef0119000003000111700000002a1234567800010000"
        );
        assert_eq!(
            hex::encode(receiver_report(0x1122_3344, &[blk])),
            "81c9000711223344abcdef0119000003000111700000002a1234567800010000"
        );
        assert_eq!(
            hex::encode(sdes_cname(0x1122_3344, "zaloo")),
            "81ca00031122334401057a616c6f6f00"
        );
        assert_eq!(ntp(0).0, 2_208_988_800);
        assert_eq!(ntp_middle(0x0001_aaaa, 0xbbbb_0000), 0xaaaa_bbbb);
    }

    #[test]
    fn compound_packets_parse_back() {
        let blk = sample_block();
        let mut compound = sender_report(1, 1000, 2, 3, 4, &[blk]);
        compound.extend_from_slice(&sdes_cname(1, "x"));
        compound.extend_from_slice(&hex::decode("81cd0002000000010000000a").unwrap());

        let parts = parse_rtcp(&compound);
        assert_eq!(parts.len(), 3);
        match &parts[0] {
            RtcpPacket::Sr { blocks, .. } => assert_eq!(blocks, &[blk]),
            other => panic!("expected SR, got {other:?}"),
        }
        assert_eq!(
            parts[2],
            RtcpPacket::Other {
                packet_type: 205,
                ssrc: Some(1),
                bytes: 12,
            }
        );
        assert!(parse_rtcp(&hex::decode("00c80001").unwrap()).is_empty());
    }

    #[test]
    fn receive_stats_loss_wrap_jitter_and_lsr_dlsr() {
        let mut s = ReceiveStats::new(0xabc, 16000);
        assert!(s.block(0).is_none());
        s.on_packet(65534, 0, 1000);
        s.on_packet(65535, 320, 1020);
        s.on_packet(1, 960, 1060);
        s.on_sender_report(0x0001_aaaa, 0xbbbb_0000, 1000);
        let b = s.block(1500).unwrap();
        assert_eq!(b.highest_seq, 65536 + 1);
        assert_eq!(b.cumulative_lost, 1);
        assert_eq!(b.fraction_lost, 64);
        assert_eq!(b.jitter, 0);
        assert_eq!(b.lsr, 0xaaaa_bbbb);
        assert_eq!(b.dlsr, 32768);
        assert_eq!(s.block(2000).unwrap().fraction_lost, 0);
    }

    #[test]
    fn receive_stats_guards_dlsr_and_large_loss_fraction() {
        let mut s = ReceiveStats::new(0x123, 16000);
        s.on_packet(1, 0, 1000);
        s.on_sender_report(0x0001_aaaa, 0xbbbb_0000, 5000);
        let b1 = s.block(4000).unwrap();
        assert_eq!(b1.dlsr, 0);

        s.cycles = 0x90_0000;
        let b2 = s.block(6000).unwrap();
        assert!((250..=255).contains(&b2.fraction_lost));
    }

    #[test]
    fn pli_fir_encoding_padding_and_multi_fci() {
        assert_eq!(hex::encode(pli(1, 2)), "81ce00020000000100000002");
        assert_eq!(
            hex::encode(fir(1, 2, 7)),
            "84ce000400000001000000000000000207000000"
        );
        assert_eq!(
            parse_rtcp(&pli(0xaabb_ccdd, 5)),
            vec![RtcpPacket::Pli {
                ssrc: 0xaabb_ccdd,
                media_ssrc: 5
            }]
        );
        let mut combo = fir(9, 77, 1);
        combo.extend_from_slice(&pli(9, 5));
        assert_eq!(
            parse_rtcp(&combo),
            vec![
                RtcpPacket::Fir {
                    ssrc: 9,
                    media_ssrc: 77
                },
                RtcpPacket::Pli {
                    ssrc: 9,
                    media_ssrc: 5
                }
            ]
        );

        // Multi-FCI FIR
        let mut multi_fir = vec![0u8; 28];
        multi_fir[0] = 0x84;
        multi_fir[1] = 206;
        multi_fir[2..4].copy_from_slice(&6u16.to_be_bytes());
        multi_fir[4..8].copy_from_slice(&0x1111u32.to_be_bytes());
        multi_fir[12..16].copy_from_slice(&0xaaaau32.to_be_bytes());
        multi_fir[16] = 1;
        multi_fir[20..24].copy_from_slice(&0xbbbbu32.to_be_bytes());
        multi_fir[24] = 2;
        assert_eq!(
            parse_rtcp(&multi_fir),
            vec![
                RtcpPacket::Fir {
                    ssrc: 0x1111,
                    media_ssrc: 0xaaaa
                },
                RtcpPacket::Fir {
                    ssrc: 0x1111,
                    media_ssrc: 0xbbbb
                },
            ]
        );

        // Padded FIR
        let mut padded_fir = vec![0u8; 28];
        padded_fir[0] = 0xa4;
        padded_fir[1] = 206;
        padded_fir[2..4].copy_from_slice(&6u16.to_be_bytes());
        padded_fir[4..8].copy_from_slice(&0x2222u32.to_be_bytes());
        padded_fir[12..16].copy_from_slice(&0xccccu32.to_be_bytes());
        padded_fir[16] = 3;
        padded_fir[20..24].copy_from_slice(&0xddddu32.to_be_bytes());
        padded_fir[27] = 8;
        assert_eq!(
            parse_rtcp(&padded_fir),
            vec![RtcpPacket::Fir {
                ssrc: 0x2222,
                media_ssrc: 0xcccc
            }]
        );

        // Bad padding lengths
        let mut bad_pad = padded_fir.clone();
        bad_pad[27] = 0;
        assert!(parse_rtcp(&bad_pad).is_empty());
        bad_pad[27] = 25;
        assert!(parse_rtcp(&bad_pad).is_empty());
    }

    // The TWCC and NACK vectors below (TWCC_ADDS, TWCC_BUILDS, NACK_STEPS and the NACK hex) come
    // from the test vectors of Zalos' call engine (https://github.com/cpfriend1721994/zalos,
    // engines/zcall-cpp/tests/vectors/rtcp.json; BSD-3-Clause, © doandat943 and Việt Nam Linux
    // Family), an independent implementation of the same wire format; see docs/video-calls.md.
    const TWCC_ADDS: [(u16, i64); 247] = [
        (65501, 1700000000001),
        (65502, 1700000000018),
        (65503, 1700000000047),
        (65504, 1700000000054),
        (65505, 1700000000063),
        (65506, 1700000000092),
        (65507, 1700000000111),
        (65508, 1700000000132),
        (65509, 1700000000157),
        (65510, 1700000000182),
        (65511, 1700000000185),
        (65512, 1700000000214),
        (65513, 1700000000231),
        (65514, 1700000000260),
        (65515, 1700000000289),
        (65516, 1700000000296),
        (65517, 1700000000325),
        (65518, 1700000000328),
        (65519, 1700000000343),
        (65520, 1700000000364),
        (65521, 1700000000391),
        (65522, 1700000000392),
        (65523, 1700000000403),
        (65521, 1700000000420),
        (65525, 1700000000437),
        (65523, 1700000000454),
        (65527, 1700000000457),
        (65528, 1700000000464),
        (65529, 1700000000471),
        (65530, 1700000000494),
        (65531, 1700000000505),
        (65532, 1700000000530),
        (65533, 1700000000549),
        (65534, 1700000000550),
        (65535, 1700000000563),
        (0, 1700000000572),
        (1, 1700000000597),
        (2, 1700000000614),
        (3, 1700000000637),
        (4, 1700000000640),
        (5, 1700000000669),
        (6, 1700000000672),
        (7, 1700000000691),
        (8, 1700000000710),
        (9, 1700000000719),
        (10, 1700000000732),
        (11, 1700000000755),
        (12, 1700000000770),
        (13, 1700000000777),
        (14, 1700000000778),
        (15, 1700000000789),
        (16, 1700000000802),
        (17, 1700000000819),
        (18, 1700000000838),
        (19, 1700000000861),
        (17, 1700000000876),
        (21, 1700000000889),
        (22, 1700000000914),
        (23, 1700000000943),
        (21, 1700000000966),
        (25, 1700000000979),
        (26, 1700000000984),
        (27, 1700000000991),
        (28, 1700000000996),
        (29, 1700000001021),
        (30, 1700000001042),
        (31, 1700000001065),
        (32, 1700000001074),
        (33, 1700000001075),
        (34, 1700000001080),
        (35, 1700000001101),
        (36, 1700000001128),
        (37, 1700000001133),
        (38, 1700000001134),
        (39, 1700000001157),
        (37, 1700000001158),
        (41, 1700000001181),
        (42, 1700000001184),
        (43, 1700000001193),
        (41, 1700000001212),
        (45, 1700000001223),
        (46, 1700000001238),
        (47, 1700000001253),
        (48, 1700000001254),
        (46, 1700000001269),
        (50, 1700000001298),
        (51, 1700000001321),
        (52, 1700000001342),
        (53, 1700000001351),
        (54, 1700000001362),
        (52, 1700000001379),
        (56, 1700000001402),
        (57, 1700000001425),
        (58, 1700000001430),
        (59, 1700000001441),
        (60, 1700000001442),
        (61, 1700000001461),
        (62, 1700000001468),
        (63, 1700000001491),
        (64, 1700000001496),
        (65, 1700000001499),
        (66, 1700000001520),
        (67, 1700000001521),
        (65, 1700000001546),
        (69, 1700000001557),
        (70, 1700000001582),
        (71, 1700000001607),
        (72, 1700000001626),
        (73, 1700000001649),
        (74, 1700000001674),
        (75, 1700000001695),
        (76, 1700000001722),
        (77, 1700000001731),
        (78, 1700000001750),
        (79, 1700000001771),
        (80, 1700000001788),
        (81, 1700000001799),
        (82, 1700000001812),
        (83, 1700000001841),
        (84, 1700000001854),
        (85, 1700000001859),
        (86, 1700000001868),
        (87, 1700000001893),
        (88, 1700000001900),
        (89, 1700000001913),
        (90, 1700000001920),
        (91, 1700000001933),
        (92, 1700000001952),
        (93, 1700000001981),
        (94, 1700000001988),
        (95, 1700000002011),
        (96, 1700000002040),
        (97, 1700000002047),
        (95, 1700000002064),
        (99, 1700000002089),
        (97, 1700000002094),
        (101, 1700000002109),
        (99, 1700000002126),
        (103, 1700000002147),
        (104, 1700000002176),
        (105, 1700000002203),
        (106, 1700000002218),
        (107, 1700000002219),
        (108, 1700000002236),
        (109, 1700000002251),
        (110, 1700000002268),
        (111, 1700000002293),
        (109, 1700000002308),
        (113, 1700000002321),
        (114, 1700000002348),
        (115, 1700000002375),
        (116, 1700000002388),
        (117, 1700000002417),
        (118, 1700000002440),
        (116, 1700000002455),
        (120, 1700000002476),
        (121, 1700000002495),
        (122, 1700000002500),
        (123, 1700000002513),
        (124, 1700000002522),
        (125, 1700000002545),
        (126, 1700000002572),
        (127, 1700000002575),
        (128, 1700000002596),
        (129, 1700000002615),
        (130, 1700000002618),
        (131, 1700000002639),
        (132, 1700000002644),
        (133, 1700000002661),
        (134, 1700000002688),
        (135, 1700000002693),
        (136, 1700000002710),
        (137, 1700000002723),
        (138, 1700000002740),
        (139, 1700000002751),
        (140, 1700000002778),
        (141, 1700000002805),
        (142, 1700000002818),
        (143, 1700000002839),
        (144, 1700000002866),
        (145, 1700000002877),
        (146, 1700000002884),
        (147, 1700000002889),
        (148, 1700000002904),
        (149, 1700000002909),
        (150, 1700000002936),
        (151, 1700000002949),
        (152, 1700000002954),
        (153, 1700000002971),
        (154, 1700000002984),
        (155, 1700000003013),
        (153, 1700000003036),
        (157, 1700000003051),
        (158, 1700000003060),
        (159, 1700000003063),
        (160, 1700000003074),
        (161, 1700000003075),
        (162, 1700000003096),
        (163, 1700000003101),
        (164, 1700000003118),
        (165, 1700000003147),
        (166, 1700000003176),
        (167, 1700000003177),
        (168, 1700000003204),
        (169, 1700000003221),
        (170, 1700000003246),
        (171, 1700000003269),
        (172, 1700000003290),
        (173, 1700000003319),
        (174, 1700000003332),
        (175, 1700000003351),
        (176, 1700000003372),
        (177, 1700000003383),
        (178, 1700000003408),
        (179, 1700000003419),
        (180, 1700000003430),
        (181, 1700000003449),
        (182, 1700000003474),
        (183, 1700000003503),
        (184, 1700000003522),
        (185, 1700000003549),
        (186, 1700000003574),
        (187, 1700000003593),
        (188, 1700000003602),
        (189, 1700000003605),
        (690, 1700000003624),
        (691, 1700000003641),
        (692, 1700000003670),
        (693, 1700000003687),
        (694, 1700000003710),
        (695, 1700000003735),
        (693, 1700000003756),
        (697, 1700000003761),
        (698, 1700000003774),
        (699, 1700000003779),
        (697, 1700000003794),
        (701, 1700000003823),
        (702, 1700000003846),
        (703, 1700000003855),
        (704, 1700000003860),
        (705, 1700000003863),
        (706, 1700000003866),
        (707, 1700000003895),
        (708, 1700000003910),
        (709, 1700000003929),
        (710, 1700000003932),
        (711, 1700000003947),
    ];
    const NACK_STEPS: [(u32, u16, i64, &[&[u16]]); 60] = [
        (2, 65001, 1015, &[]),
        (1, 65002, 1026, &[]),
        (1, 65003, 1027, &[]),
        (1, 65004, 1048, &[]),
        (1, 65005, 1067, &[]),
        (1, 65006, 1088, &[]),
        (1, 65007, 1093, &[]),
        (1, 65008, 1098, &[]),
        (
            2,
            65009,
            1114,
            &[&[65002, 65003, 65004, 65005, 65006, 65007, 65008]],
        ),
        (1, 65008, 1120, &[]),
        (2, 65009, 1124, &[]),
        (2, 65010, 1124, &[]),
        (1, 65011, 1145, &[&[65009, 65010]]),
        (1, 65012, 1162, &[]),
        (1, 65013, 1177, &[]),
        (
            1,
            65020,
            1190,
            &[&[65009, 65010, 65014, 65015, 65016, 65017, 65018, 65019]],
        ),
        (1, 65023, 1203, &[&[65021, 65022]]),
        (1, 65024, 1205, &[]),
        (1, 65025, 1223, &[]),
        (
            1,
            65024,
            1246,
            &[&[
                65009, 65010, 65014, 65015, 65016, 65017, 65018, 65019, 65021, 65022,
            ]],
        ),
        (
            1,
            65025,
            1270,
            &[&[65009, 65010, 65014, 65015, 65016, 65017, 65018, 65019]],
        ),
        (1, 65026, 1291, &[&[65021, 65022]]),
        (1, 65027, 1308, &[&[65009, 65010]]),
        (
            1,
            65028,
            1322,
            &[&[65014, 65015, 65016, 65017, 65018, 65019]],
        ),
        (1, 65029, 1328, &[&[65021, 65022]]),
        (
            1,
            65036,
            1328,
            &[&[65030, 65031, 65032, 65033, 65034, 65035]],
        ),
        (
            1,
            65037,
            1352,
            &[&[65014, 65015, 65016, 65017, 65018, 65019]],
        ),
        (1, 65038, 1353, &[]),
        (
            2,
            65039,
            1356,
            &[&[
                65002, 65003, 65004, 65005, 65006, 65007, 65008, 65011, 65012, 65013, 65014, 65015,
                65016, 65017, 65018, 65019, 65020, 65021, 65022, 65023, 65024, 65025, 65026, 65027,
                65028, 65029, 65030, 65031, 65032, 65033, 65034, 65035, 65036, 65037, 65038,
            ]],
        ),
        (
            1,
            65040,
            1380,
            &[&[
                65021, 65022, 65030, 65031, 65032, 65033, 65034, 65035, 65039,
            ]],
        ),
        (1, 65039, 1380, &[]),
        (1, 65040, 1401, &[]),
        (
            2,
            65041,
            1408,
            &[&[
                65002, 65003, 65004, 65005, 65006, 65007, 65008, 65011, 65012, 65013, 65014, 65015,
                65016, 65017, 65018, 65019, 65020, 65021, 65022, 65023, 65024, 65025, 65026, 65027,
                65028, 65029, 65030, 65031, 65032, 65033, 65034, 65035, 65036, 65037, 65038, 65040,
            ]],
        ),
        (
            1,
            65042,
            1425,
            &[&[65030, 65031, 65032, 65033, 65034, 65035, 65041]],
        ),
        (1, 65043, 1426, &[]),
        (1, 65044, 1447, &[]),
        (
            2,
            65045,
            1456,
            &[&[
                65002, 65003, 65004, 65005, 65006, 65007, 65008, 65011, 65012, 65013, 65014, 65015,
                65016, 65017, 65018, 65019, 65020, 65021, 65022, 65023, 65024, 65025, 65026, 65027,
                65028, 65029, 65030, 65031, 65032, 65033, 65034, 65035, 65036, 65037, 65038, 65040,
                65042, 65043, 65044,
            ]],
        ),
        (
            1,
            65046,
            1479,
            &[&[65030, 65031, 65032, 65033, 65034, 65035, 65041, 65045]],
        ),
        (
            1,
            65045,
            1493,
            &[&[65030, 65031, 65032, 65033, 65034, 65035]],
        ),
        (1, 65046, 1504, &[]),
        (1, 65047, 1528, &[&[65041]]),
        (1, 65048, 1546, &[&[65041]]),
        (1, 65051, 1552, &[&[65049, 65050]]),
        (1, 65052, 1561, &[]),
        (1, 65053, 1583, &[]),
        (1, 65054, 1606, &[&[65041, 65049, 65050]]),
        (1, 65055, 1615, &[]),
        (1, 65056, 1625, &[]),
        (1, 65057, 1646, &[&[65049, 65050]]),
        (1, 65058, 1660, &[]),
        (1, 65059, 1679, &[&[65049, 65050]]),
        (
            2,
            65060,
            1682,
            &[&[
                65011, 65012, 65013, 65014, 65015, 65016, 65017, 65018, 65019, 65020, 65021, 65022,
                65023, 65024, 65025, 65026, 65027, 65028, 65029, 65030, 65031, 65032, 65033, 65034,
                65035, 65036, 65037, 65038, 65040, 65042, 65043, 65044, 65046, 65047, 65048, 65049,
                65050, 65051, 65052, 65053, 65054, 65055, 65056, 65057, 65058, 65059,
            ]],
        ),
        (1, 65061, 1685, &[&[65060]]),
        (1, 65062, 1686, &[]),
        (1, 65063, 1689, &[]),
        (1, 65062, 1692, &[]),
        (1, 65063, 1700, &[]),
        (1, 65064, 1724, &[&[65049, 65050]]),
        (1, 65065, 1734, &[&[65060]]),
        (
            2,
            65066,
            1758,
            &[&[
                65040, 65042, 65043, 65044, 65046, 65047, 65048, 65049, 65050, 65051, 65052, 65053,
                65054, 65055, 65056, 65057, 65058, 65059, 65061, 65062, 65063, 65064, 65065,
            ]],
        ),
    ];

    const TWCC_BUILDS: [(usize, &[&str]); 7] = [
        (1, &[
            "8fcd00050000006f000000deffdd00013f95a000d0000400",
        ]),
        (42, &[
            "8fcd00100000006f000000deffde00293f95a0012016c455200c48741c24744c5464640c744474741c740c3c546c042c88501c1c5c2c644c04342464445c0c740c000000",
        ]),
        (83, &[
            "8fcd00100000006f000000de000700293f95aa02200dc545200dc545d000cc4c24345c3c1c042c34444c5c70647490141c1464545c240414546c14045c600c24783c3c00",
        ]),
        (124, &[
            "8fcd00100000006f000000de003000293f95b303d155c555d554201498b05c54242ca05c142c044c1c5c140c54049064644c5c64546c244c54442c3474341424641c0000",
        ]),
        (165, &[
            "8fcd00100000006f000000de005900293f95bd042009c445d555c555c555d540e41c344c741c5c741ca85098746c3c04443c4464706c6c34745c904c1434245c6c0c544c",
        ]),
        (206, &[
            "8fcd00100000006f000000de008200293f95c805201ac5552008e85414446c144434442c6c6c34546c2c1c143c146c341444347498240c2c045414447474046c44640000",
        ]),
        (247, &[
            "8fcd000a0000006f000000de00ab01903f95d3062013017d145474344c542c642c2c4c64744c6c644c240c00",
            "8fcd000b0000006f000000de023b008d3f95d8070077d554d5152008a04474445c64683414b05c24140c0c743c4c0c3c",
        ]),
    ];

    fn rtp_with_ext(cc: u8, ext: &[u8], words: u16) -> Vec<u8> {
        let mut p = vec![0x90 | cc, 0x70, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3];
        p.extend(std::iter::repeat_n(0xaa, 4 * cc as usize));
        p.extend_from_slice(&[0xbe, 0xde]);
        p.extend_from_slice(&words.to_be_bytes());
        p.extend_from_slice(ext);
        p.extend_from_slice(&[1, 2, 3, 4]);
        p
    }

    #[test]
    fn tw_seq_extension_parsing() {
        // id 5, len field 1 (= 2 bytes), seq 0xfffb, then 3 bytes padding.
        assert_eq!(
            read_tw_seq(&hex::decode("906100650000000000000309bede000151fffb0000000000").unwrap()),
            Some(0xfffb)
        );
        // No X bit.
        let mut plain = vec![0x80, 0x70, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 9, 9, 9, 9];
        assert_eq!(read_tw_seq(&plain), None);
        plain[0] = 0x90; // X bit but no 0xbede profile
        assert_eq!(read_tw_seq(&plain), None);
        // Other ids before ours, leading padding, CSRCs.
        let ext = [0, 0x10, 7, 0x22, 1, 2, 3, 0x51, 0x12, 0x34, 0, 0];
        assert_eq!(read_tw_seq(&rtp_with_ext(2, &ext, 3)), Some(0x1234));
        // Right id with the wrong length is ignored.
        let ext = [0x52, 1, 2, 3, 0, 0, 0, 0];
        assert_eq!(read_tw_seq(&rtp_with_ext(0, &ext, 2)), None);
        // Reserved id 15 stops parsing; element running past the extension is ignored.
        let ext = [0xf0, 0, 0x51, 1, 2, 0, 0, 0];
        assert_eq!(read_tw_seq(&rtp_with_ext(0, &ext, 2)), None);
        let ext = [0x00, 0x00, 0x00, 0x51];
        assert_eq!(read_tw_seq(&rtp_with_ext(0, &ext, 1)), None);
        // Truncated packet.
        assert_eq!(read_tw_seq(&[0x90, 0x70, 0, 1]), None);
    }

    #[test]
    fn twcc_matches_reference_vectors() {
        let mut rec = TwccRecorder::new();
        let mut builds = TWCC_BUILDS.iter().peekable();
        for i in 0..=247usize {
            while let Some((after, want)) = builds.peek() {
                if *after != i {
                    break;
                }
                let got: Vec<String> = rec.build(111, 222).iter().map(hex::encode).collect();
                assert_eq!(got, *want, "feedback after {i} arrivals");
                builds.next();
            }
            if let Some(&(seq, ms)) = TWCC_ADDS.get(i) {
                rec.add(seq, ms);
            }
        }
        assert!(builds.next().is_none());
    }

    #[test]
    fn twcc_chunks_deltas_gaps_and_wrap() {
        let mut rec = TwccRecorder::new();
        // 10 consecutive 1 ms arrivals: run-length chunk (sym 1, run 10), deltas of 4 units.
        for i in 0..10u16 {
            rec.add(100 + i, 1_000_000 + i as i64);
        }
        let p = &rec.build(1, 2)[0];
        assert_eq!(&p[..2], &[0x8f, 205]);
        assert_eq!(u16::from_be_bytes([p[12], p[13]]), 100);
        assert_eq!(u16::from_be_bytes([p[14], p[15]]), 10);
        assert_eq!(u16::from_be_bytes([p[20], p[21]]), (1 << 13) | 10);
        // 4_000_000 quarter-ms / 256 = 15625 exactly: first delta 0.
        assert_eq!(&p[16..19], &[0, 0x3d, 0x09]);
        assert_eq!(&p[22..32], &[0, 4, 4, 4, 4, 4, 4, 4, 4, 4]);
        assert_eq!(p.len() % 4, 0);
        assert_eq!(p[19], 0);

        // Gap + large delta: status vector chunk (sym 1,0,0,2), 1 byte then 2-byte delta.
        let mut rec = TwccRecorder::new();
        rec.add(7, 1000);
        rec.add(10, 1100); // 400 units after the first: > 255 => symbol 2
        let p = &rec.build(1, 2)[0];
        assert_eq!(u16::from_be_bytes([p[14], p[15]]), 4);
        let chunk = u16::from_be_bytes([p[20], p[21]]);
        assert_eq!(chunk, 0xc000 | (1 << 12) | (2 << 6));
        // ref = floor(4000/256)=15 -> 3840; first delta 160, second 400-160... = 4400-4000.
        assert_eq!(p[22], 160);
        assert_eq!(i16::from_be_bytes([p[23], p[24]]), 400);
        assert_eq!(p[19], 0); // first feedback
                              // Next feedback increments the counter; nothing pending => nothing built.
        assert!(rec.build(1, 2).is_empty());
        rec.add(11, 1101);
        assert_eq!(rec.build(1, 2)[0][19], 1);

        // 16-bit wrap: arrivals continue across 65535 -> 0 and base wraps.
        let mut rec = TwccRecorder::new();
        rec.add(65535, 50);
        rec.add(0, 51);
        let p = &rec.build(1, 2)[0];
        assert_eq!(u16::from_be_bytes([p[12], p[13]]), 65535);
        assert_eq!(u16::from_be_bytes([p[14], p[15]]), 2);
        // Late and duplicate packets behind the reported range are ignored.
        rec.add(65535, 60);
        assert!(!rec.pending());
    }

    #[test]
    fn twcc_skips_empty_ranges_and_bounds_memory() {
        let mut rec = TwccRecorder::new();
        rec.add(0, 10);
        assert_eq!(rec.build(1, 2).len(), 1);
        // Jump of 1000: the range 1..=400 and friends hold no arrivals and are skipped.
        rec.add(1000, 20);
        let out = rec.build(1, 2);
        assert_eq!(out.len(), 1);
        assert!(u16::from_be_bytes([out[0][12], out[0][13]]) >= 401);
        // Memory stays bounded when nothing is ever reported.
        let mut rec = TwccRecorder::new();
        for i in 0..10_000u32 {
            rec.add(i as u16, i as i64);
        }
        assert!(rec.arrivals.len() <= TWCC_MAX_PENDING);
    }

    #[test]
    fn nack_encoding_matches_reference() {
        assert_eq!(
            hex::encode(nack(1, 2, &[65534, 65535, 0, 3, 17, 18, 40])),
            "81cd00050000000100000002fffe00130011000100280000"
        );
    }

    #[test]
    fn nack_tracker_matches_reference_steps() {
        let mut t = NackTracker::new();
        for (i, (ssrc, seq, now, want)) in NACK_STEPS.iter().enumerate() {
            let got: Vec<Vec<u16>> = t
                .note(*ssrc, *seq, *now)
                .into_iter()
                .map(|r| r.seqs)
                .collect();
            let want: Vec<Vec<u16>> = want.iter().map(|w| w.to_vec()).collect();
            assert_eq!(got, want, "step {i}");
        }
    }

    #[test]
    fn nack_tracker_retries_drops_and_resets() {
        let mut t = NackTracker::new();
        assert!(t.note(9, 100, 0).is_empty()); // first packet: nothing
        assert!(!t.has_missing());
        let r = t.note(9, 103, 10);
        assert_eq!(
            r,
            vec![NackRequest {
                ssrc: 9,
                seqs: vec![101, 102]
            }]
        );
        assert!(t.has_missing());
        // Retries every 40 ms, five tries in total.
        assert!(t.tick(30).is_empty());
        assert_eq!(t.tick(50).len(), 1);
        assert_eq!(t.tick(90).len(), 1);
        assert_eq!(t.tick(130).len(), 1);
        assert_eq!(t.tick(170).len(), 1);
        assert!(t.tick(250).is_empty()); // tries exhausted
        assert!(t.has_missing());
        // Dropped after 400 ms.
        assert!(t.tick(411).is_empty());
        assert!(!t.has_missing());
        // A recovered seq stops being chased.
        let r = t.note(9, 106, 500);
        assert_eq!(r[0].seqs, vec![104, 105]);
        assert!(t.note(9, 104, 505).is_empty());
        assert_eq!(t.tick(545)[0].seqs, vec![105]);
        // Big jump resets without NACKing.
        assert!(t.note(9, 106 + 65, 550).is_empty());
        assert!(!t.has_missing());
        // Wrap-around holes and the 128 cap.
        assert!(t.note(5, 65534, 600).is_empty());
        assert_eq!(t.note(5, 1, 600)[0].seqs, vec![65535, 0]);
        let mut t = NackTracker::new();
        t.note(1, 0, 0);
        for k in 1..=4u16 {
            t.note(1, k * 60, 0);
        }
        assert!(t.streams[&1].miss.len() <= NACK_KEEP);
    }

    #[test]
    fn twcc_feedback_decodes_with_independent_parser() {
        // Parse our own output back (status vector + run-length + deltas) as a receiver would.
        let mut rec = TwccRecorder::new();
        let mut times = Vec::new();
        for (seq, ms) in TWCC_ADDS.iter().take(41).skip(1) {
            rec.add(*seq, *ms);
            if !times.iter().any(|(s, _)| s == seq) {
                times.push((*seq, *ms));
            }
        }
        let p = &rec.build(5, 6)[0];
        let count = u16::from_be_bytes([p[14], p[15]]) as usize;
        let mut syms = Vec::new();
        let mut o = 20;
        while syms.len() < count {
            let c = u16::from_be_bytes([p[o], p[o + 1]]);
            o += 2;
            if c & 0x8000 == 0 {
                for _ in 0..(c & 0x1fff) {
                    syms.push((c >> 13) & 3);
                }
            } else {
                for k in 0..7 {
                    syms.push((c >> (12 - 2 * k)) & 3);
                }
            }
        }
        syms.truncate(count);
        let refq = ((p[16] as i64) << 16 | (p[17] as i64) << 8 | p[18] as i64) * 256;
        let mut t = refq;
        let mut got = Vec::new();
        for s in syms {
            match s {
                1 => {
                    t += p[o] as i64;
                    o += 1;
                    got.push(t);
                }
                2 => {
                    t += i16::from_be_bytes([p[o], p[o + 1]]) as i64;
                    o += 2;
                    got.push(t);
                }
                _ => {}
            }
        }
        // The 24-bit reference wraps every 2^24 * 64 ms.
        let want: Vec<i64> = times.iter().map(|(_, ms)| (ms * 4) & 0xffff_ffff).collect();
        assert_eq!(got.len(), want.len());
        assert_eq!(
            got.iter().map(|g| g & 0xffff_ffff).collect::<Vec<_>>(),
            want
        );
    }
}
