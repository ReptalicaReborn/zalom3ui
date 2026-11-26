//! RTCP (RFC 3550 / 4585 / 5104) encoder, decoder, and receive statistics matching `src/rtcp.ts`.
//! Zalo calls run unencrypted RTCP (`srtcp: 0`), wrapped in zrtc kind 5 (audio) or 15 (video).

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
}
