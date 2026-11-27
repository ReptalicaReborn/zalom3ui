//! Zalo's 1:1 video packetization and frame reassembler matching `src/video.ts`.
//! One H.264 Annex-B access unit per frame, payload type 98 (key) / 97 (delta),
//! frames > 1068 bytes split into 1068-byte chunks prefixed `[0x1c, flag]`
//! (`0x80` first, `0x00` middle, `0x40` last), RTP marker bit on the final packet.
//!
//! The receiver is more forgiving than the sender: besides Zalo's chunks it also accepts standard RFC 6184
//! payloads (FU-A with a real NAL type in the FU header, STAP-A, single raw NAL units), detected per packet
//! from the first bytes, and it drops SVC-only NAL units (types 14/15/20) that hardware decoders choke on.

use std::collections::{BTreeMap, BTreeSet};

pub const VIDEO_KEY_PT: u8 = 98;
pub const VIDEO_DELTA_PT: u8 = 97;
pub const MAX_CHUNK: usize = 1068;

const START_CODE: &[u8] = &[0, 0, 0, 1];
const REORDER_FRAMES: usize = 3;
const REORDER_MS: i64 = 200;
const MAX_PENDING: usize = 30;
const MAX_FRAME_PARTS: usize = 512;
const MAX_FRAME_BYTES: usize = 512 * 1024;

/// Reassembled H.264 Annex-B access unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub key: bool,
    pub timestamp: u32,
    pub data: Vec<u8>,
}

/// Reassembled frame paired with the 32-bit bitmask of H.264 NAL unit types present in `frame.data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledFrame {
    pub frame: VideoFrame,
    pub nal_types_mask: u32,
}

/// Incoming RTP video packet fed into [`VideoAssembler`].
#[derive(Debug, Clone, Copy)]
pub struct VideoPacket<'a> {
    pub seq: u16,
    pub timestamp: u32,
    pub payload_type: u8,
    pub marker: bool,
    pub payload: &'a [u8],
}

/// Counters maintained by [`VideoAssembler`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssemblerStats {
    pub frames: u64,
    pub keys: u64,
    pub gaps: u64,
    pub dropped: u64,
}

/// Why [`VideoAssembler`] requested a key frame from the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyNeededReason {
    Gap,
    NoKey,
}

impl KeyNeededReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gap => "gap",
            Self::NoKey => "no_key",
        }
    }
}

/// Splits an Annex-B access unit into Zalo RTP payloads.
pub fn packetize_frame(annex_b: &[u8], max_chunk: usize) -> Result<Vec<Vec<u8>>, String> {
    if max_chunk == 0 {
        return Err(format!("maxChunk must be a positive integer (got {max_chunk})"));
    }
    if annex_b.len() <= max_chunk {
        return Ok(vec![annex_b.to_vec()]);
    }
    let mut out = Vec::with_capacity(annex_b.len().div_ceil(max_chunk));
    let mut at = 0usize;
    while at < annex_b.len() {
        let end = (at + max_chunk).min(annex_b.len());
        let flag = if at == 0 {
            0x80
        } else if end == annex_b.len() {
            0x40
        } else {
            0x00
        };
        let mut chunk = Vec::with_capacity(2 + (end - at));
        chunk.push(0x1c);
        chunk.push(flag);
        chunk.extend_from_slice(&annex_b[at..end]);
        out.push(chunk);
        at = end;
    }
    Ok(out)
}

/// Calls `f` for each NAL unit (without start codes) in an Annex-B buffer without allocating.
fn for_each_nal_unit<'a>(annex_b: &'a [u8], mut f: impl FnMut(&'a [u8])) {
    let len = annex_b.len();
    let mut i = 0usize;
    let mut cur_start: Option<usize> = None;

    while i + 2 < len {
        if annex_b[i + 2] > 1 {
            i += 3;
            continue;
        }
        if annex_b[i] == 0 && annex_b[i + 1] == 0 && annex_b[i + 2] == 1 {
            if let Some(start) = cur_start {
                let mut end = i;
                while end > start && annex_b[end - 1] == 0 {
                    end -= 1;
                }
                if end > start {
                    f(&annex_b[start..end]);
                }
            }
            cur_start = Some(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }

    if let Some(start) = cur_start {
        let mut end = len;
        while end > start && annex_b[end - 1] == 0 {
            end -= 1;
        }
        if end > start {
            f(&annex_b[start..end]);
        }
    }
}

/// Extracts NAL units (without start codes) from an Annex-B buffer.
pub fn nal_units(annex_b: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    for_each_nal_unit(annex_b, |u| out.push(u));
    out
}

/// How one RTP payload is packetized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartKind {
    /// Starts with an Annex-B start code: a whole access unit.
    AnnexB,
    /// Zalo chunk `[0x1c, flag]` (an FU-A indicator whose FU header carries NAL type 0).
    ZaloChunk,
    /// RFC 6184 FU-A (indicator type 28, FU header with the real NAL type).
    FuA,
    /// RFC 6184 STAP-A (type 24).
    StapA,
    /// RFC 6184 single NAL unit without start code (types 1..=23).
    Single,
    Other,
}

fn classify(p: &[u8]) -> PartKind {
    if p.len() >= 4 && p[0] == 0 && p[1] == 0 && (p[2] == 1 || (p[2] == 0 && p[3] == 1)) {
        return PartKind::AnnexB;
    }
    match p.first().map(|b| b & 0x1f) {
        Some(28) if p.len() >= 2 => {
            if p[1] & 0x1f == 0 {
                PartKind::ZaloChunk
            } else {
                PartKind::FuA
            }
        }
        Some(24) => PartKind::StapA,
        Some(1..=23) => PartKind::Single,
        _ => PartKind::Other,
    }
}

/// True when `p` cannot start an access unit (a continuation of a fragmented one).
fn is_continuation(p: &[u8]) -> bool {
    match classify(p) {
        PartKind::ZaloChunk => p[1] != 0x80,
        PartKind::FuA => p[1] & 0x80 == 0,
        _ => false,
    }
}

/// Appends the Annex-B form of one RTP payload to `out`.
fn append_part(out: &mut Vec<u8>, p: &[u8]) {
    match classify(p) {
        PartKind::ZaloChunk => out.extend_from_slice(&p[2..]),
        PartKind::FuA => {
            if p[1] & 0x80 != 0 {
                out.extend_from_slice(START_CODE);
                out.push((p[0] & 0xe0) | (p[1] & 0x1f));
            }
            out.extend_from_slice(&p[2..]);
        }
        PartKind::StapA => {
            let mut at = 1usize;
            while at + 2 <= p.len() {
                let n = u16::from_be_bytes([p[at], p[at + 1]]) as usize;
                at += 2;
                if n == 0 || at + n > p.len() {
                    break;
                }
                out.extend_from_slice(START_CODE);
                out.extend_from_slice(&p[at..at + n]);
                at += n;
            }
        }
        PartKind::Single => {
            out.extend_from_slice(START_CODE);
            out.extend_from_slice(p);
        }
        PartKind::AnnexB | PartKind::Other => out.extend_from_slice(p),
    }
}

/// SVC extension NAL units (prefix, subset SPS, slice extension): not decodable as plain AVC.
fn is_svc_nal(t: u8) -> bool {
    matches!(t, 14 | 15 | 20)
}

struct Pending {
    parts: BTreeMap<i64, Vec<u8>>,
    bytes: usize,
    max_idx: Option<i64>,
    key: bool,
    end: Option<i64>,
}

struct Ready {
    key: bool,
    data: Vec<u8>,
    timestamp: u32,
    nal_types_mask: u32,
    last: i64,
    since: i64,
}

/// Reassembles fragmented/out-of-order H.264 RTP packets into complete Annex-B frames.
pub struct VideoAssembler {
    pub stats: AssemblerStats,
    /// Insertion-ordered pending frames by RTP timestamp (bounded at `MAX_PENDING = 30`).
    pending: Vec<(u32, Pending)>,
    /// Complete frames awaiting ordered emission, keyed by extended seq of first packet.
    ready: BTreeMap<i64, Ready>,
    last_idx: Option<i64>,
    max_idx: Option<i64>,
    started: bool,
    need_key: bool,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    /// Sequence numbers of packets that carry no video (RTP padding probes, other payload types): they use up a
    /// sequence number between two frames, which must not be mistaken for a lost packet.
    fillers: BTreeSet<i64>,
}

impl Default for VideoAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoAssembler {
    pub fn new() -> Self {
        Self {
            stats: AssemblerStats::default(),
            pending: Vec::new(),
            ready: BTreeMap::new(),
            last_idx: None,
            max_idx: None,
            started: false,
            need_key: false,
            sps: None,
            pps: None,
            fillers: BTreeSet::new(),
        }
    }

    /// True until a key frame has been delivered (and again after a gap).
    pub fn waiting_for_key(&self) -> bool {
        !self.started || self.need_key
    }

    /// True when at least one partial or reordered frame is currently buffered.
    pub fn has_buffered_frames(&self) -> bool {
        !self.ready.is_empty() || !self.pending.is_empty()
    }

    /// Maps a 16-bit sequence number to a monotonic extended index nearest `max_idx`.
    fn extend(&self, seq: u16) -> i64 {
        let seq_i = seq as i64;
        let Some(max_idx) = self.max_idx else {
            return seq_i;
        };
        let cycle = max_idx.div_euclid(0x1_0000);
        let mut best = cycle * 0x1_0000 + seq_i;
        for c in [cycle - 1, cycle + 1] {
            let cand = c * 0x1_0000 + seq_i;
            if (cand - max_idx).abs() < (best - max_idx).abs() {
                best = cand;
            }
        }
        best
    }

    /// Feeds one RTP video packet, invoking `on_frame` and `on_key_needed` as frames become ready.
    pub fn push<F, K>(&mut self, pkt: VideoPacket<'_>, now_ms: i64, mut on_frame: F, on_key_needed: K)
    where
        F: FnMut(VideoFrame),
        K: FnMut(KeyNeededReason),
    {
        self.push_assembled(pkt, now_ms, |af| on_frame(af.frame), on_key_needed);
    }

    /// Like [`push`], but yields [`AssembledFrame`] including `nal_types_mask` to avoid rescanning NALs.
    pub fn push_assembled<F, K>(
        &mut self,
        pkt: VideoPacket<'_>,
        now_ms: i64,
        mut on_frame: F,
        mut on_key_needed: K,
    ) where
        F: FnMut(AssembledFrame),
        K: FnMut(KeyNeededReason),
    {
        let idx = self.extend(pkt.seq);
        self.max_idx = Some(match self.max_idx {
            None => idx,
            Some(m) => m.max(idx),
        });
        if let Some(last) = self.last_idx {
            if idx <= last {
                return;
            }
        }
        if (pkt.payload_type != VIDEO_KEY_PT && pkt.payload_type != VIDEO_DELTA_PT) || pkt.payload.is_empty() {
            if self.fillers.len() < 256 {
                self.fillers.insert(idx);
            }
            self.flush(now_ms, &mut on_frame, &mut on_key_needed);
            return;
        }

        let pos = match self.pending.iter().position(|(ts, _)| *ts == pkt.timestamp) {
            Some(i) => i,
            None => {
                self.pending.push((
                    pkt.timestamp,
                    Pending {
                        parts: BTreeMap::new(),
                        bytes: 0,
                        max_idx: None,
                        key: false,
                        end: None,
                    },
                ));
                if self.pending.len() > MAX_PENDING {
                    self.pending.remove(0);
                    self.stats.dropped += 1;
                }
                self.pending.len() - 1
            }
        };

        let should_complete = {
            let f = &mut self.pending[pos].1;
            if let Some(end) = f.end {
                if idx > end {
                    return;
                }
            }
            if !f.parts.contains_key(&idx) {
                if f.parts.len() + 1 > MAX_FRAME_PARTS || f.bytes + pkt.payload.len() > MAX_FRAME_BYTES {
                    self.pending.remove(pos);
                    self.stats.dropped += 1;
                    return;
                }
                f.parts.insert(idx, pkt.payload.to_vec());
                f.bytes += pkt.payload.len();
                f.max_idx = Some(match f.max_idx {
                    None => idx,
                    Some(m) => m.max(idx),
                });
            }
            if pkt.payload_type == VIDEO_KEY_PT {
                f.key = true;
            }
            if pkt.marker {
                f.end = Some(idx);
                if f.max_idx.is_some_and(|m| m > idx) {
                    let to_drop: Vec<i64> = f.parts.range((idx + 1)..).map(|(&k, _)| k).collect();
                    for k in to_drop {
                        if let Some(removed) = f.parts.remove(&k) {
                            f.bytes -= removed.len();
                        }
                    }
                    f.max_idx = Some(idx);
                }
            }
            f.end.is_some()
        };

        if should_complete {
            self.complete(pos, now_ms);
        }
        self.flush(now_ms, &mut on_frame, &mut on_key_needed);
    }

    /// Lets the reorder window expire when no packet arrives to trigger it.
    pub fn tick<F, K>(&mut self, now_ms: i64, mut on_frame: F, on_key_needed: K)
    where
        F: FnMut(VideoFrame),
        K: FnMut(KeyNeededReason),
    {
        self.tick_assembled(now_ms, |af| on_frame(af.frame), on_key_needed);
    }

    /// Like [`tick`], yielding [`AssembledFrame`] with `nal_types_mask`.
    pub fn tick_assembled<F, K>(&mut self, now_ms: i64, mut on_frame: F, mut on_key_needed: K)
    where
        F: FnMut(AssembledFrame),
        K: FnMut(KeyNeededReason),
    {
        self.flush(now_ms, &mut on_frame, &mut on_key_needed);
    }

    fn complete(&mut self, pos: usize, now_ms: i64) {
        let (timestamp, f) = &self.pending[pos];
        let timestamp = *timestamp;
        let Some(end_idx) = f.end else {
            return;
        };
        if f.parts.is_empty() {
            return;
        }
        let (&first_idx, first_buf) = f.parts.first_key_value().unwrap();
        if is_continuation(first_buf) {
            return;
        }
        // Zalo chunks, FU-A starts and whole Annex-B units say that they begin a frame. A bare RFC 6184 NAL or STAP-A
        // does not, so when it is the first packet we hold, make sure the packet before it belongs to another frame
        // (otherwise a reordered SPS/PPS packet would be lost and the key frame emitted without them).
        if matches!(classify(first_buf), PartKind::Single | PartKind::StapA | PartKind::Other)
            && !self.previous_frame_ends_at(first_idx - 1)
        {
            return;
        }
        let (&last_idx, _) = f.parts.last_key_value().unwrap();
        if last_idx != end_idx {
            return;
        }
        if (last_idx - first_idx + 1) as usize != f.parts.len() {
            return;
        }

        let (_, f) = self.pending.remove(pos);
        let mut data = Vec::with_capacity(f.bytes + 16);
        for part in f.parts.values() {
            append_part(&mut data, part);
        }

        let mut types_mask: u32 = 0;
        let mut has_svc = false;
        let sps_slot = &mut self.sps;
        let pps_slot = &mut self.pps;
        for_each_nal_unit(&data, |u| {
            let t = u[0] & 0x1f;
            types_mask |= 1u32 << t;
            if t == 7 {
                *sps_slot = Some(u.to_vec());
            } else if t == 8 {
                *pps_slot = Some(u.to_vec());
            } else if is_svc_nal(t) {
                has_svc = true;
            }
        });
        // The reported mask keeps the SVC types so the call log shows what the sender really used.
        let reported_mask = types_mask;
        if has_svc {
            let mut kept = Vec::with_capacity(data.len());
            types_mask = 0;
            for_each_nal_unit(&data, |u| {
                let t = u[0] & 0x1f;
                if !is_svc_nal(t) {
                    kept.extend_from_slice(START_CODE);
                    kept.extend_from_slice(u);
                    types_mask |= 1u32 << t;
                }
            });
            data = kept;
        }
        let svc_mask = reported_mask & ((1 << 14) | (1 << 15) | (1 << 20));

        let has_type = |mask: u32, t: u8| (mask & (1u32 << t)) != 0;
        let key = !data.is_empty() && (f.key || has_type(types_mask, 5) || has_type(types_mask, 7));
        if key {
            if let Some(ref sps) = self.sps {
                let missing_sps = !has_type(types_mask, 7);
                let missing_pps = !has_type(types_mask, 8) && self.pps.is_some();
                if missing_sps || missing_pps {
                    let extra = (if missing_sps {
                        START_CODE.len() + sps.len()
                    } else {
                        0
                    }) + (if missing_pps {
                        self.pps
                            .as_ref()
                            .map_or(0, |p| START_CODE.len() + p.len())
                    } else {
                        0
                    });
                    let mut pre = Vec::with_capacity(extra + data.len());
                    if missing_sps {
                        pre.extend_from_slice(START_CODE);
                        pre.extend_from_slice(sps);
                        types_mask |= 1u32 << 7;
                    }
                    if missing_pps {
                        if let Some(ref pps) = self.pps {
                            pre.extend_from_slice(START_CODE);
                            pre.extend_from_slice(pps);
                            types_mask |= 1u32 << 8;
                        }
                    }
                    pre.extend_from_slice(&data);
                    data = pre;
                }
            }
        }

        self.ready.insert(
            first_idx,
            Ready {
                key,
                data,
                timestamp,
                nal_types_mask: types_mask | svc_mask,
                last: end_idx,
                since: now_ms,
            },
        );
    }

    /// True when the packet `idx` is known to be the last of an earlier frame (or nothing earlier is known yet).
    fn previous_frame_ends_at(&self, idx: i64) -> bool {
        match self.last_idx {
            None => true,
            Some(last) => {
                last >= idx
                    || self.ready.values().any(|r| r.last == idx)
                    || self.pending.iter().any(|(_, p)| p.end == Some(idx))
            }
        }
    }

    fn has_earlier_pending(&self, before_idx: i64) -> bool {
        self.pending
            .iter()
            .any(|(_, p)| !p.parts.is_empty() && p.max_idx.is_some_and(|m| m < before_idx))
    }

    fn flush<F, K>(&mut self, now_ms: i64, on_frame: &mut F, on_key_needed: &mut K)
    where
        F: FnMut(AssembledFrame),
        K: FnMut(KeyNeededReason),
    {
        loop {
            if let Some(mut last) = self.last_idx {
                while self.fillers.remove(&(last + 1)) {
                    last += 1;
                }
                self.last_idx = Some(last);
                self.fillers = self.fillers.split_off(&(last + 1));
            }
            if self.ready.is_empty() {
                return;
            }
            if let Some(last) = self.last_idx {
                let next_idx = last + 1;
                if let Some(next) = self.ready.remove(&next_idx) {
                    self.emit(next, on_frame, on_key_needed);
                    continue;
                }
            }
            let (&first_idx, oldest) = self.ready.first_key_value().unwrap();
            let should_wait =
                self.last_idx.is_some() || (!oldest.key && self.has_earlier_pending(first_idx));
            if should_wait && self.ready.len() <= REORDER_FRAMES && (now_ms - oldest.since) < REORDER_MS {
                return;
            }
            if !self.started && !oldest.key {
                self.ready.remove(&first_idx);
                self.stats.dropped += 1;
                on_key_needed(KeyNeededReason::NoKey);
                continue;
            }
            if self.last_idx.is_some() {
                self.stats.gaps += 1;
                self.need_key = true;
                on_key_needed(KeyNeededReason::Gap);
            }
            let oldest = self.ready.remove(&first_idx).unwrap();
            self.emit(oldest, on_frame, on_key_needed);
        }
    }

    fn emit<F, K>(&mut self, fr: Ready, on_frame: &mut F, on_key_needed: &mut K)
    where
        F: FnMut(AssembledFrame),
        K: FnMut(KeyNeededReason),
    {
        self.last_idx = Some(fr.last);
        if fr.data.is_empty() {
            // Only SVC enhancement NAL units: nothing to decode, but the sequence numbers are consumed.
        } else if self.need_key && !fr.key {
            self.stats.dropped += 1;
            on_key_needed(KeyNeededReason::NoKey);
        } else {
            if fr.key {
                self.need_key = false;
                self.started = true;
            }
            self.stats.frames += 1;
            if fr.key {
                self.stats.keys += 1;
            }
            on_frame(AssembledFrame {
                frame: VideoFrame {
                    key: fr.key,
                    timestamp: fr.timestamp,
                    data: fr.data,
                },
                nal_types_mask: fr.nal_types_mask,
            });
        }

        let last_idx = fr.last;
        let before_len = self.pending.len();
        self.pending
            .retain(|(_, p)| p.parts.is_empty() || p.max_idx.is_none_or(|m| m > last_idx));
        self.stats.dropped += (before_len - self.pending.len()) as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SC: &[u8] = &[0, 0, 0, 1];
    const SPS: &[u8] = &[0x67, 0x42, 0x00, 0x1f];
    const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];

    fn idr(n: usize) -> Vec<u8> {
        let mut out = SC.to_vec();
        out.extend_from_slice(&[0x65, 0x88]);
        out.extend(std::iter::repeat_n(0x84, n));
        out
    }

    fn slice(n: usize) -> Vec<u8> {
        let mut out = SC.to_vec();
        out.extend_from_slice(&[0x41, 0x9a]);
        out.extend(std::iter::repeat_n(0x22, n));
        out
    }

    fn key_frame() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(SC);
        out.extend_from_slice(SPS);
        out.extend_from_slice(SC);
        out.extend_from_slice(PPS);
        out.extend_from_slice(&idr(6));
        out
    }

    struct OwnedPkt {
        seq: u16,
        timestamp: u32,
        payload_type: u8,
        marker: bool,
        payload: Vec<u8>,
    }

    fn packets(frame: &[u8], key: bool, seq: u16, timestamp: u32) -> Vec<OwnedPkt> {
        let chunks = packetize_frame(frame, MAX_CHUNK).unwrap();
        let len = chunks.len();
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, payload)| OwnedPkt {
                seq: seq.wrapping_add(i as u16),
                timestamp,
                payload_type: if key { VIDEO_KEY_PT } else { VIDEO_DELTA_PT },
                marker: i == len - 1,
                payload,
            })
            .collect()
    }

    fn push_all(
        a: &mut VideoAssembler,
        pkts: &[OwnedPkt],
        now: i64,
        out: &mut Vec<VideoFrame>,
        asks: &mut Vec<KeyNeededReason>,
    ) {
        for p in pkts {
            a.push(
                VideoPacket {
                    seq: p.seq,
                    timestamp: p.timestamp,
                    payload_type: p.payload_type,
                    marker: p.marker,
                    payload: &p.payload,
                },
                now,
                |f| out.push(f),
                |w| asks.push(w),
            );
        }
    }

    #[test]
    fn packetizer_small_and_large_frames() {
        let small = hex::decode("0000000165888000").unwrap();
        assert_eq!(packetize_frame(&small, MAX_CHUNK).unwrap(), vec![small]);
        assert_eq!(packetize_frame(&vec![1u8; MAX_CHUNK], MAX_CHUNK).unwrap().len(), 1);

        let au: Vec<u8> = (0..2500).map(|i| (i & 0xff) as u8).collect();
        let chunks = packetize_frame(&au, MAX_CHUNK).unwrap();
        assert_eq!(chunks.iter().map(|c| c.len()).collect::<Vec<_>>(), vec![1070, 1070, 366]);
        assert_eq!(&chunks[0][..2], &[0x1c, 0x80]);
        assert_eq!(&chunks[1][..2], &[0x1c, 0x00]);
        assert_eq!(&chunks[2][..2], &[0x1c, 0x40]);
        let joined: Vec<u8> = chunks.iter().flat_map(|c| &c[2..]).copied().collect();
        assert_eq!(joined, au);
        assert!(packetize_frame(&key_frame(), 0).is_err());
    }

    #[test]
    fn assembler_single_key_and_chunked_delta() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        let mut asks = Vec::new();
        let mut big = slice(0);
        big.extend(std::iter::repeat_n(0x33, 2500));

        push_all(&mut a, &packets(&key_frame(), true, 100, 9000), 1000, &mut out, &mut asks);
        push_all(&mut a, &packets(&big, false, 101, 12000), 1000, &mut out, &mut asks);
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0],
            VideoFrame {
                key: true,
                timestamp: 9000,
                data: key_frame()
            }
        );
        assert!(!out[1].key);
        assert_eq!(out[1].timestamp, 12000);
        assert_eq!(out[1].data, big);
        assert!(asks.is_empty());
        assert_eq!(
            a.stats,
            AssemblerStats {
                frames: 2,
                keys: 1,
                gaps: 0,
                dropped: 0
            }
        );
    }

    #[test]
    fn assembler_wraps_16bit_sequence_numbers() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        let mut asks = Vec::new();
        push_all(&mut a, &packets(&key_frame(), true, 65535, 1000), 1000, &mut out, &mut asks);
        push_all(&mut a, &packets(&slice(6), false, 0, 4000), 1000, &mut out, &mut asks);
        let mut big = slice(0);
        big.extend(std::iter::repeat_n(7, 2300));
        push_all(&mut a, &packets(&big, false, 1, 7000), 1000, &mut out, &mut asks);
        assert_eq!(
            out.iter().map(|f| f.timestamp).collect::<Vec<_>>(),
            vec![1000, 4000, 7000]
        );
        assert_eq!(a.stats.gaps, 0);

        // Straddling the wrap at startup
        let mut wrap = VideoAssembler::new();
        let mut out2 = Vec::new();
        let mut kf_big = key_frame();
        kf_big.extend(std::iter::repeat_n(5, 2200));
        let first = packets(&kf_big, true, 65534, 500);
        assert_eq!(first.iter().map(|p| p.seq).collect::<Vec<_>>(), vec![65534, 65535, 0]);
        push_all(&mut wrap, &first, 1000, &mut out2, &mut asks);
        assert_eq!(out2.len(), 1);
    }

    #[test]
    fn assembler_reorders_out_of_order_packets_and_frames() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        let mut asks = Vec::new();
        push_all(&mut a, &packets(&key_frame(), true, 9, 500), 1000, &mut out, &mut asks);
        let mut big = slice(0);
        big.extend(std::iter::repeat_n(5, 2200));
        let f1 = packets(&big, false, 10, 1000);
        let f2 = packets(&slice(6), false, 13, 4000);
        push_all(&mut a, &[OwnedPkt { seq: f1[2].seq, timestamp: f1[2].timestamp, payload_type: f1[2].payload_type, marker: f1[2].marker, payload: f1[2].payload.clone() }], 1000, &mut out, &mut asks);
        push_all(&mut a, &[OwnedPkt { seq: f1[0].seq, timestamp: f1[0].timestamp, payload_type: f1[0].payload_type, marker: f1[0].marker, payload: f1[0].payload.clone() }], 1000, &mut out, &mut asks);
        push_all(&mut a, &f2, 1000, &mut out, &mut asks);
        assert_eq!(out.len(), 1);
        push_all(&mut a, &[OwnedPkt { seq: f1[1].seq, timestamp: f1[1].timestamp, payload_type: f1[1].payload_type, marker: f1[1].marker, payload: f1[1].payload.clone() }], 1000, &mut out, &mut asks);
        assert_eq!(
            out.iter().map(|f| f.timestamp).collect::<Vec<_>>(),
            vec![500, 1000, 4000]
        );
        assert_eq!(a.stats.gaps, 0);
    }

    #[test]
    fn assembler_gap_timeout_and_recovery_with_keyframe() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        let mut asks = Vec::new();
        push_all(&mut a, &packets(&key_frame(), true, 0, 1000), 1000, &mut out, &mut asks);
        push_all(&mut a, &packets(&slice(6), false, 2, 7000), 1000, &mut out, &mut asks);
        assert_eq!(out.len(), 1);
        a.tick(1250, |f| out.push(f), |w| asks.push(w));
        assert_eq!(a.stats.gaps, 1);
        assert_eq!(asks[0], KeyNeededReason::Gap);
        assert_eq!(out.len(), 1);
        assert!(a.waiting_for_key());
        push_all(&mut a, &packets(&slice(6), false, 3, 10000), 1250, &mut out, &mut asks);
        assert_eq!(out.len(), 1);
        push_all(&mut a, &packets(&key_frame(), true, 4, 13000), 1250, &mut out, &mut asks);
        assert_eq!(out.iter().map(|f| f.key).collect::<Vec<_>>(), vec![true, true]);
        assert!(!a.waiting_for_key());
        assert_eq!(a.stats.dropped, 2);
    }

    #[test]
    fn assembler_gap_after_three_waiting_frames() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        let mut asks = Vec::new();
        push_all(&mut a, &packets(&key_frame(), true, 0, 1000), 1000, &mut out, &mut asks);
        for (i, seq) in [2u16, 3, 4, 5].into_iter().enumerate() {
            push_all(&mut a, &packets(&slice(6), false, seq, 2000 + i as u32 * 1000), 1000, &mut out, &mut asks);
        }
        assert_eq!(a.stats.gaps, 1);
        assert!(asks.contains(&KeyNeededReason::Gap));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn assembler_drops_before_first_key_and_prepends_cached_sps_pps() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        let mut asks = Vec::new();
        push_all(&mut a, &packets(&slice(6), false, 50, 1000), 1000, &mut out, &mut asks);
        push_all(&mut a, &packets(&slice(6), false, 51, 2000), 1000, &mut out, &mut asks);
        assert_eq!(out.len(), 0);
        assert_eq!(a.stats.dropped, 2);
        assert!(asks.contains(&KeyNeededReason::NoKey));

        push_all(&mut a, &packets(&key_frame(), true, 52, 3000), 1000, &mut out, &mut asks);
        push_all(&mut a, &packets(&idr(6), true, 53, 4000), 1000, &mut out, &mut asks);
        assert_eq!(out[1].data, key_frame());

        let mut sps_idr = Vec::new();
        sps_idr.extend_from_slice(SC);
        sps_idr.extend_from_slice(SPS);
        sps_idr.extend_from_slice(&idr(6));
        push_all(&mut a, &packets(&sps_idr, true, 54, 5000), 1000, &mut out, &mut asks);
        let types: Vec<u8> = nal_units(&out[2].data).iter().map(|u| u[0] & 0x1f).collect();
        assert_eq!(types, vec![8, 7, 5]);
    }

    #[test]
    fn assembler_caps_parts_bytes_handles_startup_wrap_and_in_flight_delta() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        let mut asks = Vec::new();
        for i in 0u16..513 {
            let payload = vec![0x1c, if i == 0 { 0x80 } else { 0x00 }, 1, 2];
            a.push(
                VideoPacket {
                    seq: i,
                    timestamp: 1000,
                    payload_type: VIDEO_KEY_PT,
                    marker: false,
                    payload: &payload,
                },
                1000,
                |f| out.push(f),
                |w| asks.push(w),
            );
        }
        assert_eq!(out.len(), 0);
        assert_eq!(a.stats.dropped, 1);

        let mut a2 = VideoAssembler::new();
        let big = vec![0xaa; 64 * 1024];
        for i in 0u16..9 {
            a2.push(
                VideoPacket {
                    seq: i,
                    timestamp: 2000,
                    payload_type: VIDEO_KEY_PT,
                    marker: false,
                    payload: &big,
                },
                1000,
                |f| out.push(f),
                |w| asks.push(w),
            );
        }
        assert_eq!(a2.stats.dropped, 1);

        // Startup delta waits for in-flight keyframe
        let mut a3 = VideoAssembler::new();
        let mut out3 = Vec::new();
        let mut asks3 = Vec::new();
        let mut kf_big = key_frame();
        kf_big.extend(std::iter::repeat_n(4, 1500));
        let kf = packets(&kf_big, true, 10, 1000);
        assert_eq!(kf.len(), 2);
        push_all(&mut a3, &kf[0..1], 1000, &mut out3, &mut asks3);
        push_all(&mut a3, &packets(&slice(6), false, 12, 4000), 1000, &mut out3, &mut asks3);
        assert_eq!(a3.stats.dropped, 0);
        assert_eq!(out3.len(), 0);
        push_all(&mut a3, &kf[1..2], 1050, &mut out3, &mut asks3);
        assert_eq!(
            out3.iter().map(|f| (f.key, f.timestamp)).collect::<Vec<_>>(),
            vec![(true, 1000), (false, 4000)]
        );
    }

    // ---- RFC 6184 and SVC input ----

    fn raw(seq: u16, ts: u32, pt: u8, marker: bool, payload: Vec<u8>) -> OwnedPkt {
        OwnedPkt { seq, timestamp: ts, payload_type: pt, marker, payload }
    }

    /// RFC 6184 FU-A fragments of one NAL (header byte + body), `size` body bytes per packet.
    fn fu_a(nal: &[u8], size: usize) -> Vec<Vec<u8>> {
        let (hdr, body) = (nal[0], &nal[1..]);
        let n = body.len().div_ceil(size);
        (0..n)
            .map(|i| {
                let mut fh = hdr & 0x1f;
                if i == 0 {
                    fh |= 0x80;
                }
                if i == n - 1 {
                    fh |= 0x40;
                }
                let mut p = vec![(hdr & 0xe0) | 28, fh];
                p.extend_from_slice(&body[i * size..((i + 1) * size).min(body.len())]);
                p
            })
            .collect()
    }

    fn feed(a: &mut VideoAssembler, pkts: &[&OwnedPkt], now: i64, out: &mut Vec<AssembledFrame>) {
        for p in pkts {
            a.push_assembled(
                VideoPacket {
                    seq: p.seq,
                    timestamp: p.timestamp,
                    payload_type: p.payload_type,
                    marker: p.marker,
                    payload: &p.payload,
                },
                now,
                |f| out.push(f),
                |_| {},
            );
        }
    }

    fn big_idr_nal() -> Vec<u8> {
        let mut n = vec![0x65, 0x88];
        n.extend((0..3000).map(|i| (i % 251) as u8 | 1));
        n
    }

    #[test]
    fn rfc6184_fu_a_stap_a_and_single_nal_key_frame_in_shuffled_order_with_duplicates() {
        let idr_nal = big_idr_nal();
        let frags = fu_a(&idr_nal, 1000);
        assert_eq!(frags.len(), 4);
        // seq 10: STAP-A(SPS, PPS), 11..14: FU-A IDR, all one timestamp, marker last.
        let mut stap = vec![0x78];
        for nal in [SPS, PPS] {
            stap.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            stap.extend_from_slice(nal);
        }
        let mut pk = vec![raw(1, 777, 98, false, stap)];
        for (i, f) in frags.iter().enumerate() {
            pk.push(raw(2 + i as u16, 777, 98, i == 3, f.clone()));
        }
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        push_all_assembled(&mut a, &packets(&key_frame(), true, 0, 500), 1000, &mut out);
        out.clear();
        // Shuffled plus duplicates (NACK retransmits).
        feed(&mut a, &[&pk[3], &pk[1], &pk[1], &pk[0], &pk[4], &pk[2], &pk[3], &pk[0]], 1000, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0].frame;
        assert!(f.key);
        assert_eq!(f.timestamp, 777);
        let units = nal_units(&f.data);
        assert_eq!(units.len(), 3);
        assert_eq!(units[0], SPS);
        assert_eq!(units[1], PPS);
        assert_eq!(units[2], &idr_nal[..]);
        assert_eq!(a.stats.frames, 2);
        assert_eq!(a.stats.dropped, 0);
        // A late duplicate of the finished frame is ignored.
        feed(&mut a, &[&pk[2]], 1001, &mut out);
        assert_eq!(out.len(), 1);

        // Separate single-NAL packets (SPS, PPS) before a one-packet IDR.
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        push_all_assembled(&mut a, &packets(&key_frame(), true, 0, 1), 0, &mut out);
        out.clear();
        let p = [
            raw(1, 5, 98, false, SPS.to_vec()),
            raw(2, 5, 98, false, PPS.to_vec()),
            raw(3, 5, 98, true, vec![0x65, 1, 2, 3]),
        ];
        feed(&mut a, &[&p[2], &p[0], &p[1]], 0, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(nal_units(&out[0].frame.data), vec![SPS, PPS, &[0x65, 1, 2, 3][..]]);
    }

    #[test]
    fn rfc6184_fu_a_delta_frame_after_zalo_key_frame() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        push_all_assembled(&mut a, &packets(&key_frame(), true, 0, 1000), 1000, &mut out);
        let nal: Vec<u8> = std::iter::once(0x41).chain((0..2500).map(|i| (i % 7) as u8 + 1)).collect();
        let frags = fu_a(&nal, 1100);
        let pk: Vec<OwnedPkt> = frags
            .iter()
            .enumerate()
            .map(|(i, f)| raw(1 + i as u16, 4000, 97, i == frags.len() - 1, f.clone()))
            .collect();
        feed(&mut a, &pk.iter().rev().collect::<Vec<_>>(), 1000, &mut out);
        assert_eq!(out.len(), 2);
        assert!(!out[1].frame.key);
        assert_eq!(nal_units(&out[1].frame.data), vec![&nal[..]]);
        assert_eq!(a.stats.gaps, 0);
    }

    fn push_all_assembled(a: &mut VideoAssembler, pkts: &[OwnedPkt], now: i64, out: &mut Vec<AssembledFrame>) {
        feed(a, &pkts.iter().collect::<Vec<_>>(), now, out);
    }

    #[test]
    fn zalo_multi_packet_frame_tolerates_reorder_and_duplicates() {
        let mut au = SC.to_vec();
        au.extend_from_slice(&[0x41, 0x9a]);
        au.extend((0..4000).map(|i| (i % 200) as u8 + 1));
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        push_all_assembled(&mut a, &packets(&key_frame(), true, 0, 1000), 1000, &mut out);
        let pk = packets(&au, false, 1, 4000);
        assert_eq!(pk.len(), 4);
        feed(&mut a, &[&pk[2], &pk[2], &pk[0], &pk[3], &pk[1], &pk[0]], 1000, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].frame.data, au);
        assert_eq!(a.stats.dropped, 0);
    }

    #[test]
    fn svc_nal_units_are_stripped_but_reported() {
        let mut au = Vec::new();
        for nal in [&[0x67u8, 0x42, 0, 0x1f][..], &[0x68, 0xce][..], &[0x0e, 1, 2][..], &[0x65, 9, 9][..], &[0x14, 5, 5][..], &[0x6f, 4, 4][..]] {
            au.extend_from_slice(SC);
            au.extend_from_slice(nal);
        }
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        feed(&mut a, &[&raw(1, 1, 98, true, au)], 0, &mut out);
        assert_eq!(out.len(), 1);
        let types: Vec<u8> = nal_units(&out[0].frame.data).iter().map(|u| u[0] & 0x1f).collect();
        assert_eq!(types, vec![7, 8, 5]);
        assert!(out[0].frame.key);
        for t in [14, 15, 20, 5] {
            assert!(out[0].nal_types_mask & (1 << t) != 0, "type {t}");
        }

        // An access unit holding only enhancement layers is swallowed without a gap.
        let mut only = SC.to_vec();
        only.extend_from_slice(&[0x14, 1, 1]);
        feed(&mut a, &[&raw(2, 2, 97, true, only)], 0, &mut out);
        feed(&mut a, &[&raw(3, 3, 97, true, slice(4)[4..].to_vec().into_iter().fold(SC.to_vec(), |mut v, b| { v.push(b); v }))], 0, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(a.stats.gaps, 0);
    }

    #[test]
    fn resolution_change_new_sps_flows_through_and_caches() {
        let sps_a: &[u8] = &[0x67, 0x42, 0x00, 0x0d, 0xab];
        let sps_b: &[u8] = &[0x67, 0x42, 0x00, 0x1e, 0xcd, 0xee];
        let kf = |sps: &[u8]| {
            let mut v = SC.to_vec();
            v.extend_from_slice(sps);
            v.extend_from_slice(SC);
            v.extend_from_slice(PPS);
            v.extend_from_slice(&idr(300));
            v
        };
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        push_all_assembled(&mut a, &packets(&kf(sps_a), true, 0, 100), 0, &mut out);
        let n1 = packets(&kf(sps_a), true, 0, 100).len() as u16;
        push_all_assembled(&mut a, &packets(&slice(500), false, n1, 200), 0, &mut out);
        let n2 = n1 + packets(&slice(500), false, 0, 0).len() as u16;
        let k2 = packets(&kf(sps_b), true, n2, 300);
        let n3 = n2 + k2.len() as u16;
        push_all_assembled(&mut a, &k2, 0, &mut out);
        // Later IDR without parameter sets gets the NEW SPS/PPS prepended.
        let mut bare = idr(50);
        bare.drain(..0);
        push_all_assembled(&mut a, &packets(&bare, true, n3, 400), 0, &mut out);
        assert_eq!(out.len(), 4);
        assert_eq!(nal_units(&out[2].frame.data)[0], sps_b);
        assert_eq!(nal_units(&out[3].frame.data)[0], sps_b);
        assert_eq!(a.stats.gaps, 0);
    }

    #[test]
    fn padding_probe_packets_between_frames_are_not_gaps() {
        let mut a = VideoAssembler::new();
        let mut out = Vec::new();
        push_all_assembled(&mut a, &packets(&key_frame(), true, 0, 100), 0, &mut out);
        // seq 1: empty (padding-only) packet, seq 2: other payload type; seq 3: next frame arrives first.
        let f = raw(3, 400, 97, true, slice(4));
        feed(&mut a, &[&f], 0, &mut out);
        assert_eq!(out.len(), 1, "frame waits for the missing seq 1..2");
        feed(&mut a, &[&raw(2, 300, 99, false, vec![1, 2, 3])], 0, &mut out);
        assert_eq!(out.len(), 1);
        feed(&mut a, &[&raw(1, 200, 97, false, vec![])], 0, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(a.stats.gaps, 0);
        assert!(!a.waiting_for_key());
        // In-order filler followed by a frame.
        feed(&mut a, &[&raw(4, 500, 97, false, vec![]), &raw(5, 600, 97, true, slice(4))], 0, &mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(a.stats.gaps, 0);
    }
}
