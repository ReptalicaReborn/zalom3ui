//! Zalo's 1:1 video packetization and frame reassembler matching `src/video.ts`.
//! One H.264 Annex-B access unit per frame, payload type 98 (key) / 97 (delta),
//! frames > 1068 bytes split into 1068-byte chunks prefixed `[0x1c, flag]`
//! (`0x80` first, `0x00` middle, `0x40` last), RTP marker bit on the final packet.

use std::collections::{BTreeMap, HashSet};

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

/// Extracts NAL units (without start codes) from an Annex-B buffer.
pub fn nal_units(annex_b: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 2 < annex_b.len() {
        if annex_b[i] == 0 && annex_b[i + 1] == 0 && annex_b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &start) in starts.iter().enumerate() {
        let mut end = if k + 1 < starts.len() {
            starts[k + 1] - 3
        } else {
            annex_b.len()
        };
        while end > start && annex_b[end - 1] == 0 {
            end -= 1;
        }
        if end > start {
            out.push(&annex_b[start..end]);
        }
    }
    out
}

fn is_fragment(p: &[u8]) -> bool {
    p.len() >= 2 && p[0] == 0x1c && (p[1] & 0x3f) == 0
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
        }
    }

    /// True until a key frame has been delivered (and again after a gap).
    pub fn waiting_for_key(&self) -> bool {
        !self.started || self.need_key
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
    pub fn push<F, K>(&mut self, pkt: VideoPacket<'_>, now_ms: i64, mut on_frame: F, mut on_key_needed: K)
    where
        F: FnMut(VideoFrame),
        K: FnMut(KeyNeededReason),
    {
        if (pkt.payload_type != VIDEO_KEY_PT && pkt.payload_type != VIDEO_DELTA_PT) || pkt.payload.is_empty() {
            return;
        }
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
                let to_drop: Vec<i64> = f.parts.range((idx + 1)..).map(|(&k, _)| k).collect();
                for k in to_drop {
                    if let Some(removed) = f.parts.remove(&k) {
                        f.bytes -= removed.len();
                    }
                }
                if let Some(m) = f.max_idx {
                    if m > idx {
                        f.max_idx = Some(idx);
                    }
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
    pub fn tick<F, K>(&mut self, now_ms: i64, mut on_frame: F, mut on_key_needed: K)
    where
        F: FnMut(VideoFrame),
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
        if is_fragment(first_buf) && first_buf[1] != 0x80 {
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
        let mut data = Vec::with_capacity(f.bytes);
        for part in f.parts.values() {
            if is_fragment(part) {
                data.extend_from_slice(&part[2..]);
            } else {
                data.extend_from_slice(part);
            }
        }

        let mut types = HashSet::new();
        for u in nal_units(&data) {
            if u.is_empty() {
                continue;
            }
            let t = u[0] & 0x1f;
            types.insert(t);
            if t == 7 {
                self.sps = Some(u.to_vec());
            } else if t == 8 {
                self.pps = Some(u.to_vec());
            }
        }

        let key = f.key || types.contains(&5) || types.contains(&7);
        if key {
            if let Some(ref sps) = self.sps {
                let missing_sps = !types.contains(&7);
                let missing_pps = !types.contains(&8) && self.pps.is_some();
                if missing_sps || missing_pps {
                    let mut pre = Vec::new();
                    if missing_sps {
                        pre.extend_from_slice(START_CODE);
                        pre.extend_from_slice(sps);
                    }
                    if missing_pps {
                        if let Some(ref pps) = self.pps {
                            pre.extend_from_slice(START_CODE);
                            pre.extend_from_slice(pps);
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
                last: end_idx,
                since: now_ms,
            },
        );
    }

    fn has_earlier_pending(&self, before_idx: i64) -> bool {
        self.pending
            .iter()
            .any(|(_, p)| !p.parts.is_empty() && p.max_idx.is_some_and(|m| m < before_idx))
    }

    fn flush<F, K>(&mut self, now_ms: i64, on_frame: &mut F, on_key_needed: &mut K)
    where
        F: FnMut(VideoFrame),
        K: FnMut(KeyNeededReason),
    {
        loop {
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
        F: FnMut(VideoFrame),
        K: FnMut(KeyNeededReason),
    {
        self.last_idx = Some(fr.last);
        if self.need_key && !fr.key {
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
            on_frame(VideoFrame {
                key: fr.key,
                timestamp: fr.timestamp,
                data: fr.data,
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
}
