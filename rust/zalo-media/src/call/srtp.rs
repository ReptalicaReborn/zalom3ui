//! SRTP (RFC 3711) with `AES_CM_128_HMAC_SHA1_80` matching `src/srtp.ts` and libsrtp byte-for-byte.
//! Zalo PC hands the session's first 30 latin1 characters to libsrtp as master key (16B) + master
//! salt (14B), shared across both directions, separated by TX/RX stream maps and SSRC.

use std::collections::HashMap;

use aes::Aes128;
use ctr::cipher::{InnerIvInit, KeyInit, KeyIvInit, StreamCipher};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;
use thiserror::Error;

type Aes128Ctr = ctr::Ctr128BE<Aes128>;
type HmacSha1 = Hmac<Sha1>;

pub const KEY_LEN: usize = 16;
pub const SALT_LEN: usize = 14;
pub const AUTH_KEY_LEN: usize = 20;
pub const AUTH_TAG_LEN: usize = 10;

const MAX_RX_STREAMS: usize = 64;
const REPLAY_WINDOW: u64 = 64;
const MAX_PACKET_INDEX: u64 = (1u64 << 48) - 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SrtpError {
    #[error("SRTP needs a 16-byte key and a 14-byte salt")]
    InvalidKeySalt,
    #[error("Session too short for an SRTP key")]
    SessionTooShort,
    #[error("Not an RTP packet")]
    NotRtpPacket,
    #[error("SRTP packet index reached 2^48")]
    PacketIndexOverflow,
}

/// Parsed RTP v2 fixed header, CSRCs, and optional header extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpHeader {
    pub version: u8,
    pub payload_type: u8,
    pub marker: bool,
    pub seq: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub header_length: usize,
}

/// Reads an RTP header (fixed part, CSRCs, and header extension); `None` when not RTP v2 or truncated.
pub fn rtp_header(packet: &[u8]) -> Option<RtpHeader> {
    if packet.len() < 12 || (packet[0] >> 6) != 2 {
        return None;
    }
    let cc = (packet[0] & 0x0f) as usize;
    let mut header_length = 12 + 4 * cc;
    if (packet[0] & 0x10) != 0 {
        if packet.len() < header_length + 4 {
            return None;
        }
        let ext_words = u16::from_be_bytes([packet[header_length + 2], packet[header_length + 3]]) as usize;
        header_length += 4 + 4 * ext_words;
    }
    if packet.len() < header_length {
        return None;
    }
    Some(RtpHeader {
        version: 2,
        payload_type: packet[1] & 0x7f,
        marker: (packet[1] & 0x80) != 0,
        seq: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
        header_length,
    })
}

/// AES-128 counter mode (`AES_CM_128`) from a 16-byte big-endian counter IV.
fn aes_cm(key: &[u8; KEY_LEN], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut cipher = Aes128Ctr::new(key.into(), iv.into());
    cipher.apply_keystream(&mut out);
    out
}

/// RFC 3711 §4.3 key derivation with `key_derivation_rate = 0`: one key per label.
pub fn derive_key(master_key: &[u8; KEY_LEN], master_salt: &[u8; SALT_LEN], label: u8, length: usize) -> Vec<u8> {
    let mut iv = [0u8; 16];
    iv[..SALT_LEN].copy_from_slice(master_salt);
    iv[7] ^= label;
    let zeros = vec![0u8; length];
    aes_cm(master_key, &iv, &zeros)
}

#[derive(Debug, Clone, Copy)]
struct TxStream {
    roc: u32,
    last_seq: u16,
    started: bool,
}

#[derive(Debug, Clone, Copy)]
struct RxStream {
    roc: u32,
    last_seq: u16,
    started: bool,
    replay_mask: u64,
}

/// RFC 3711 SRTP context for one media stream direction/transport (separate TX/RX maps per SSRC).
pub struct SrtpContext {
    aes_enc: Aes128,
    hmac_init: HmacSha1,
    salt: [u8; SALT_LEN],
    tx_streams: HashMap<u32, TxStream>,
    rx_streams: HashMap<u32, RxStream>,
    last_rx: Option<(u32, RxStream)>,
}

impl SrtpContext {
    /// Creates an SRTP context from a 16-byte master key and 14-byte master salt.
    pub fn new(master_key: &[u8], master_salt: &[u8]) -> Result<Self, SrtpError> {
        let mk: &[u8; KEY_LEN] = master_key.try_into().map_err(|_| SrtpError::InvalidKeySalt)?;
        let ms: &[u8; SALT_LEN] = master_salt.try_into().map_err(|_| SrtpError::InvalidKeySalt)?;
        let enc: [u8; KEY_LEN] = derive_key(mk, ms, 0, KEY_LEN).try_into().unwrap();
        let auth: [u8; AUTH_KEY_LEN] = derive_key(mk, ms, 1, AUTH_KEY_LEN).try_into().unwrap();
        let salt: [u8; SALT_LEN] = derive_key(mk, ms, 2, SALT_LEN).try_into().unwrap();
        let aes_enc = Aes128::new((&enc).into());
        let hmac_init =
            <HmacSha1 as Mac>::new_from_slice(&auth).expect("HMAC-SHA1 accepts 20-byte key");
        Ok(Self {
            aes_enc,
            hmac_init,
            salt,
            tx_streams: HashMap::new(),
            rx_streams: HashMap::new(),
            last_rx: None,
        })
    }

    /// Derives the key Zalo uses for a call: the session's first 30 latin1/ISO-8859-1 bytes.
    pub fn from_session(session: &str) -> Result<Self, SrtpError> {
        let raw: Vec<u8> = session.encode_utf16().map(|u| (u & 0xff) as u8).collect();
        if raw.len() < KEY_LEN + SALT_LEN {
            return Err(SrtpError::SessionTooShort);
        }
        Self::new(&raw[..KEY_LEN], &raw[KEY_LEN..KEY_LEN + SALT_LEN])
    }

    #[inline]
    fn iv(&self, ssrc: u32, index: u64) -> [u8; 16] {
        let mut iv = [0u8; 16];
        iv[..SALT_LEN].copy_from_slice(&self.salt);
        let w = u32::from_be_bytes([iv[4], iv[5], iv[6], iv[7]]) ^ ssrc;
        iv[4..8].copy_from_slice(&w.to_be_bytes());
        // The 48-bit packet index goes into bytes 8..14.
        for i in 0..6 {
            iv[13 - i] ^= ((index >> (8 * i)) & 0xff) as u8;
        }
        iv
    }

    #[inline]
    fn apply_aes_cm_in_place(&self, iv: &[u8; 16], data: &mut [u8]) {
        let core = ctr::CtrCore::<Aes128, ctr::flavors::Ctr128BE>::inner_iv_init(
            self.aes_enc.clone(),
            iv.into(),
        );
        let mut cipher = Aes128Ctr::from_core(core);
        cipher.apply_keystream(data);
    }

    #[inline]
    fn tag(&self, authenticated: &[u8], roc: u32) -> [u8; AUTH_TAG_LEN] {
        let mut mac = self.hmac_init.clone();
        mac.update(authenticated);
        mac.update(&roc.to_be_bytes());
        let full = mac.finalize().into_bytes();
        let mut out = [0u8; AUTH_TAG_LEN];
        out.copy_from_slice(&full[..AUTH_TAG_LEN]);
        out
    }

    /// Estimates the rollover counter for a received sequence number (RFC 3711 §3.3.1).
    fn estimate_roc(s: Option<&RxStream>, seq: u16) -> Option<u32> {
        let Some(s) = s else {
            return Some(0);
        };
        if !s.started {
            return Some(0);
        }
        let seq_i = seq as i32;
        let last_i = s.last_seq as i32;
        if last_i < 0x8000 {
            if seq_i - last_i > 0x8000 {
                if s.roc > 0 {
                    Some(s.roc - 1)
                } else {
                    None
                }
            } else {
                Some(s.roc)
            }
        } else if last_i - 0x8000 > seq_i {
            s.roc.checked_add(1)
        } else {
            Some(s.roc)
        }
    }

    /// Encrypts and authenticates an RTP packet already written at `&mut buf[rtp_offset..]` in place,
    /// appending the 10-byte HMAC-SHA1 authentication tag.
    pub fn protect_in_place(&mut self, buf: &mut Vec<u8>, rtp_offset: usize) -> Result<(), SrtpError> {
        let h = rtp_header(&buf[rtp_offset..]).ok_or(SrtpError::NotRtpPacket)?;
        let s = self.tx_streams.entry(h.ssrc).or_insert(TxStream {
            roc: 0,
            last_seq: 0,
            started: false,
        });
        let mut next_roc = s.roc as u64;
        if s.started && h.seq < s.last_seq && (s.last_seq as u32 - h.seq as u32) > 0x8000 {
            next_roc += 1;
        }
        if next_roc > u32::MAX as u64 {
            return Err(SrtpError::PacketIndexOverflow);
        }
        let index = (next_roc << 16) | (h.seq as u64);
        if index > MAX_PACKET_INDEX {
            return Err(SrtpError::PacketIndexOverflow);
        }
        s.roc = next_roc as u32;
        s.started = true;
        s.last_seq = h.seq;
        let roc = s.roc;

        let iv = self.iv(h.ssrc, index);
        self.apply_aes_cm_in_place(&iv, &mut buf[rtp_offset + h.header_length..]);
        let tag = self.tag(&buf[rtp_offset..], roc);
        buf.extend_from_slice(&tag);
        Ok(())
    }

    /// Encrypts and authenticates `rtp`, appending the SRTP packet into `out`.
    pub fn protect_into(&mut self, rtp: &[u8], out: &mut Vec<u8>) -> Result<(), SrtpError> {
        let start = out.len();
        out.reserve(rtp.len() + AUTH_TAG_LEN);
        out.extend_from_slice(rtp);
        if let Err(e) = self.protect_in_place(out, start) {
            out.truncate(start);
            return Err(e);
        }
        Ok(())
    }

    /// Encrypts and authenticates one RTP packet into an SRTP packet.
    pub fn protect(&mut self, rtp: &[u8]) -> Result<Vec<u8>, SrtpError> {
        let mut out = Vec::with_capacity(rtp.len() + AUTH_TAG_LEN);
        self.protect_into(rtp, &mut out)?;
        Ok(out)
    }

    fn update_rx_stream(ex: Option<RxStream>, roc: u32, seq: u16, index: u64) -> RxStream {
        match ex {
            None => RxStream {
                roc,
                last_seq: seq,
                started: true,
                replay_mask: 1,
            },
            Some(mut st) => {
                let max_idx = ((st.roc as u64) << 16) | (st.last_seq as u64);
                if index > max_idx {
                    let shift = index - max_idx;
                    st.replay_mask = if shift >= REPLAY_WINDOW {
                        1
                    } else {
                        (st.replay_mask << shift) | 1
                    };
                    st.roc = roc;
                    st.last_seq = seq;
                } else {
                    st.replay_mask |= 1u64 << (max_idx - index);
                }
                st
            }
        }
    }

    /// Verifies and decrypts `srtp` into `out` (reusing its capacity), returning the parsed
    /// [`RtpHeader`] on success. Returns `None` when tag fails, replayed, damaged, or invalid.
    pub fn unprotect_into(&mut self, srtp: &[u8], out: &mut Vec<u8>) -> Option<RtpHeader> {
        if srtp.len() < 12 + AUTH_TAG_LEN {
            return None;
        }
        let body = &srtp[..srtp.len() - AUTH_TAG_LEN];
        let tag_in = &srtp[srtp.len() - AUTH_TAG_LEN..];
        let h = rtp_header(body)?;

        let cached_hit = matches!(self.last_rx, Some((s, _)) if s == h.ssrc);
        let existing = if cached_hit {
            Some(self.last_rx.unwrap().1)
        } else {
            self.rx_streams.get(&h.ssrc).copied()
        };
        let total_streams = self.rx_streams.len() + usize::from(self.last_rx.is_some());
        if existing.is_none() && total_streams >= MAX_RX_STREAMS {
            return None;
        }
        let roc = Self::estimate_roc(existing.as_ref(), h.seq)?;
        let index = ((roc as u64) << 16) | (h.seq as u64);
        if index > MAX_PACKET_INDEX {
            return None;
        }
        if let Some(ex) = existing {
            let max_idx = ((ex.roc as u64) << 16) | (ex.last_seq as u64);
            if index <= max_idx {
                let diff = max_idx - index;
                if diff >= REPLAY_WINDOW || ((ex.replay_mask >> diff) & 1) != 0 {
                    return None;
                }
            }
        }

        let expected_tag = self.tag(body, roc);
        if !bool::from(expected_tag.ct_eq(tag_in)) {
            return None;
        }

        out.clear();
        out.extend_from_slice(body);
        let iv = self.iv(h.ssrc, index);
        self.apply_aes_cm_in_place(&iv, &mut out[h.header_length..]);
        let has_padding = (out[0] & 0x20) != 0;
        if has_padding {
            let payload_len = out.len() - h.header_length;
            if payload_len == 0 {
                return None;
            }
            let pad_len = out[out.len() - 1] as usize;
            if pad_len < 1 || pad_len > payload_len {
                return None;
            }
            out.truncate(out.len() - pad_len);
            out[0] &= !0x20;
        }

        let updated = Self::update_rx_stream(existing, roc, h.seq, index);
        if cached_hit {
            self.last_rx = Some((h.ssrc, updated));
        } else {
            if existing.is_some() {
                self.rx_streams.remove(&h.ssrc);
            }
            if let Some((prev_ssrc, prev_st)) = self.last_rx.replace((h.ssrc, updated)) {
                self.rx_streams.insert(prev_ssrc, prev_st);
            }
        }

        Some(h)
    }

    /// Verifies and decrypts an SRTP packet stored in `buf` in place, truncating the auth tag and
    /// any RTP padding, returning the parsed [`RtpHeader`] on success.
    pub fn unprotect_in_place(&mut self, buf: &mut Vec<u8>) -> Option<RtpHeader> {
        if buf.len() < 12 + AUTH_TAG_LEN {
            return None;
        }
        let body_len = buf.len() - AUTH_TAG_LEN;
        let h = rtp_header(&buf[..body_len])?;

        let cached_hit = matches!(self.last_rx, Some((s, _)) if s == h.ssrc);
        let existing = if cached_hit {
            Some(self.last_rx.unwrap().1)
        } else {
            self.rx_streams.get(&h.ssrc).copied()
        };
        let total_streams = self.rx_streams.len() + usize::from(self.last_rx.is_some());
        if existing.is_none() && total_streams >= MAX_RX_STREAMS {
            return None;
        }
        let roc = Self::estimate_roc(existing.as_ref(), h.seq)?;
        let index = ((roc as u64) << 16) | (h.seq as u64);
        if index > MAX_PACKET_INDEX {
            return None;
        }
        if let Some(ex) = existing {
            let max_idx = ((ex.roc as u64) << 16) | (ex.last_seq as u64);
            if index <= max_idx {
                let diff = max_idx - index;
                if diff >= REPLAY_WINDOW || ((ex.replay_mask >> diff) & 1) != 0 {
                    return None;
                }
            }
        }

        let expected_tag = self.tag(&buf[..body_len], roc);
        if !bool::from(expected_tag.ct_eq(&buf[body_len..])) {
            return None;
        }

        buf.truncate(body_len);
        let iv = self.iv(h.ssrc, index);
        self.apply_aes_cm_in_place(&iv, &mut buf[h.header_length..]);
        let has_padding = (buf[0] & 0x20) != 0;
        if has_padding {
            let payload_len = buf.len() - h.header_length;
            if payload_len == 0 {
                return None;
            }
            let pad_len = buf[buf.len() - 1] as usize;
            if pad_len < 1 || pad_len > payload_len {
                return None;
            }
            buf.truncate(buf.len() - pad_len);
            buf[0] &= !0x20;
        }

        let updated = Self::update_rx_stream(existing, roc, h.seq, index);
        if cached_hit {
            self.last_rx = Some((h.ssrc, updated));
        } else {
            if existing.is_some() {
                self.rx_streams.remove(&h.ssrc);
            }
            if let Some((prev_ssrc, prev_st)) = self.last_rx.replace((h.ssrc, updated)) {
                self.rx_streams.insert(prev_ssrc, prev_st);
            }
        }

        Some(h)
    }

    /// Verifies and decrypts one SRTP packet; returns `None` when tag fails, replayed, damaged, or invalid.
    pub fn unprotect(&mut self, srtp: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(srtp.len().saturating_sub(AUTH_TAG_LEN));
        self.unprotect_into(srtp, &mut out)?;
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "TEST-made-up-session-0123456789abcdefghijklmnop";

    fn decode_hex(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    fn make_rtp(ssrc: u32, seq: u16, ts: u32, pt: u8, padding: usize, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 12];
        out[0] = 0x80 | if padding > 0 { 0x20 } else { 0 };
        out[1] = pt;
        out[2..4].copy_from_slice(&seq.to_be_bytes());
        out[4..8].copy_from_slice(&ts.to_be_bytes());
        out[8..12].copy_from_slice(&ssrc.to_be_bytes());
        out.extend_from_slice(payload);
        if padding > 0 {
            let mut pad = vec![0u8; padding];
            pad[padding - 1] = padding as u8;
            out.extend_from_slice(&pad);
        }
        out
    }

    #[test]
    fn key_derivation_matches_rfc3711_appendix_b3() {
        let key: [u8; 16] = decode_hex("E1F97A0D3E018BE0D64FA32C06DE4139").try_into().unwrap();
        let salt: [u8; 14] = decode_hex("0EC675AD498AFEEBB6960B3AABE6").try_into().unwrap();
        assert_eq!(
            hex::encode_upper(derive_key(&key, &salt, 0, 16)),
            "C61E7A93744F39EE10734AFE3FF7A087"
        );
        assert_eq!(
            hex::encode_upper(derive_key(&key, &salt, 2, 14)),
            "30CBBC08863D8C85D49DB34A9AE1"
        );
        assert_eq!(
            &hex::encode_upper(derive_key(&key, &salt, 1, 20))[..32],
            "CEBE321F6FF7716B6FD4AB49AF256A15"
        );
    }

    #[test]
    fn protect_and_unprotect_match_libsrtp_vectors() {
        let vectors: &[(&str, &[(&str, &str)])] = &[
            (
                "simple",
                &[
                    (
                        "807000010000000011223344f8fffe",
                        "807000010000000011223344152e16ffcce4a347bc5d0792c1",
                    ),
                    (
                        "80700002000001401122334468656c6c6f206f707573206672616d65",
                        "8070000200000140112233440c8d5e62574bb7f06b7ee73e2fad856620f25e7396e79b9e4888",
                    ),
                ],
            ),
            (
                "rollover",
                &[
                    ("8070fffe000000001122334461", "8070fffe000000001122334496eb133da2fc1bec62583d"),
                    ("8070ffff000001401122334462", "8070ffff0000014011223344f17ae5005c0e1cff79e805"),
                    ("80700000000002801122334463", "80700000000002801122334454f4d7733f10dbe2c67a5c"),
                    ("80700001000003c01122334464", "80700001000003c01122334428c18f3dfc89391d711c52"),
                ],
            ),
            (
                "csrc",
                &[(
                    "817000070000000011223344aabbccdd776974682063737263",
                    "817000070000000011223344aabbccdd4f02578623c179036154c2dc6b78b04693db11",
                )],
            ),
        ];

        for (name, pkts) in vectors {
            let mut tx = SrtpContext::from_session(SESSION).unwrap();
            let mut rx = SrtpContext::from_session(SESSION).unwrap();
            let mut rx_in_place = SrtpContext::from_session(SESSION).unwrap();
            for (rtp_hex, srtp_hex) in *pkts {
                let got = tx.protect(&decode_hex(rtp_hex)).unwrap();
                assert_eq!(hex::encode(&got), *srtp_hex, "protect {name}");
                let plain = rx.unprotect(&decode_hex(srtp_hex)).expect(name);
                assert_eq!(hex::encode(&plain), *rtp_hex, "unprotect {name}");
                let mut ip = decode_hex(srtp_hex);
                let h = rx_in_place.unprotect_in_place(&mut ip).expect(name);
                assert_eq!(hex::encode(&ip), *rtp_hex, "unprotect_in_place {name}");
                assert_eq!(h, rtp_header(&plain).unwrap());
            }
        }

        // Damaged tag and wrong key
        let mut rx = SrtpContext::from_session(SESSION).unwrap();
        let mut bad = decode_hex("807000010000000011223344152e16ffcce4a347bc5d0792c1");
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(rx.unprotect(&bad).is_none());
        let mut wrong = SrtpContext::from_session("another-session-entirely-xxxxxxxxxx").unwrap();
        assert!(wrong
            .unprotect(&decode_hex("807000010000000011223344152e16ffcce4a347bc5d0792c1"))
            .is_none());
    }

    #[test]
    fn rtp_header_parsing() {
        assert_eq!(
            rtp_header(&decode_hex("807000010000000011223344f8fffe")),
            Some(RtpHeader {
                version: 2,
                payload_type: 112,
                marker: false,
                seq: 1,
                timestamp: 0,
                ssrc: 0x1122_3344,
                header_length: 12,
            })
        );
        assert_eq!(rtp_header(&[0x00]), None);
    }

    #[test]
    fn tx_and_rx_states_are_independent_for_same_ssrc() {
        let mut ctx = SrtpContext::from_session(SESSION).unwrap();
        let mut peer = SrtpContext::from_session(SESSION).unwrap();
        let ssrc = 0x4242_4242;
        ctx.protect(&make_rtp(ssrc, 0xfffe, 0, 112, 0, &[1])).unwrap();
        ctx.protect(&make_rtp(ssrc, 0xffff, 320, 112, 0, &[2])).unwrap();
        ctx.protect(&make_rtp(ssrc, 0x0000, 640, 112, 0, &[3])).unwrap();

        let incoming = peer.protect(&make_rtp(ssrc, 10, 3200, 112, 0, b"hello")).unwrap();
        let plain = ctx.unprotect(&incoming).unwrap();
        assert_eq!(&plain[12..], b"hello");
    }

    #[test]
    fn replay_window_and_forged_packet_isolation() {
        let mut tx = SrtpContext::from_session(SESSION).unwrap();
        let mut rx = SrtpContext::from_session(SESSION).unwrap();
        let mut pkts = HashMap::new();
        for seq in 1u16..=80 {
            pkts.insert(
                seq,
                tx.protect(&make_rtp(0x1122_3344, seq, seq as u32 * 320, 112, 0, &[seq as u8]))
                    .unwrap(),
            );
        }

        assert!(rx.unprotect(&pkts[&1]).is_some());
        assert!(rx.unprotect(&pkts[&1]).is_none());

        assert!(rx.unprotect(&pkts[&70]).is_some());
        let mut forged = pkts[&75].clone();
        let last = forged.len() - 1;
        forged[last] ^= 0xff;
        assert!(rx.unprotect(&forged).is_none());
        assert!(rx.unprotect(&pkts[&75]).is_some());

        // 75 - 64 = 11 outside window; 75 - 63 = 12 inside window
        assert!(rx.unprotect(&pkts[&11]).is_none());
        assert!(rx.unprotect(&pkts[&12]).is_some());
        assert!(rx.unprotect(&pkts[&12]).is_none());
    }

    #[test]
    fn pre_wrap_negative_roc_rejected() {
        let mut tx = SrtpContext::from_session(SESSION).unwrap();
        let mut rx = SrtpContext::from_session(SESSION).unwrap();
        assert!(rx
            .unprotect(&tx.protect(&make_rtp(0x1122_3344, 5, 0, 112, 0, &[1])).unwrap())
            .is_some());
        let mut tx2 = SrtpContext::from_session(SESSION).unwrap();
        let pre_wrap = tx2.protect(&make_rtp(0x1122_3344, 65535, 0, 112, 0, &[1])).unwrap();
        assert!(rx.unprotect(&pre_wrap).is_none());
    }

    #[test]
    fn rx_ssrc_cap_ignores_unauthenticated_and_caps_at_64() {
        let mut tx = SrtpContext::from_session(SESSION).unwrap();
        let mut rx = SrtpContext::from_session(SESSION).unwrap();
        for i in 1u32..=100 {
            let mut bad = tx.protect(&make_rtp(0x1000 + i, 1, 0, 112, 0, &[1])).unwrap();
            let last = bad.len() - 1;
            bad[last] ^= 0x55;
            assert!(rx.unprotect(&bad).is_none());
        }
        let mut tx_auth = SrtpContext::from_session(SESSION).unwrap();
        for i in 1u32..=64 {
            assert!(rx
                .unprotect(&tx_auth.protect(&make_rtp(0x2000 + i, 1, 0, 112, 0, &[1])).unwrap())
                .is_some());
        }
        assert!(rx
            .unprotect(&tx_auth.protect(&make_rtp(0x3000, 1, 0, 112, 0, &[1])).unwrap())
            .is_none());
    }

    #[test]
    fn padding_stripped_and_invalid_padding_rejected() {
        let mut tx = SrtpContext::from_session(SESSION).unwrap();
        let mut rx = SrtpContext::from_session(SESSION).unwrap();
        let padded = make_rtp(0x1122_3344, 1, 320, 112, 4, b"opus-data");
        assert_ne!(padded[0] & 0x20, 0);
        let plain = rx.unprotect(&tx.protect(&padded).unwrap()).unwrap();
        assert_eq!(plain[0] & 0x20, 0);
        assert_eq!(&plain[12..], b"opus-data");

        // padLen = 0
        let mut zero_pad = make_rtp(0x1122_3344, 2, 640, 112, 0, &[1, 2, 0]);
        zero_pad[0] |= 0x20;
        assert!(rx.unprotect(&tx.protect(&zero_pad).unwrap()).is_none());

        // padLen > payload
        let mut over_pad = make_rtp(0x1122_3344, 3, 960, 112, 0, &[1, 2, 10]);
        over_pad[0] |= 0x20;
        assert!(rx.unprotect(&tx.protect(&over_pad).unwrap()).is_none());

        // empty payload with P bit set
        let mut empty_pad = make_rtp(0x1122_3344, 4, 1280, 112, 0, &[]);
        empty_pad[0] |= 0x20;
        assert!(rx.unprotect(&tx.protect(&empty_pad).unwrap()).is_none());
    }

    #[test]
    fn packet_index_overflow_returns_error() {
        let mut tx = SrtpContext::from_session(SESSION).unwrap();
        let ssrc = 0x9988_7766;
        tx.tx_streams.insert(
            ssrc,
            TxStream {
                roc: u32::MAX,
                last_seq: u16::MAX,
                started: true,
            },
        );
        assert_eq!(
            tx.protect(&make_rtp(ssrc, 0, 0, 112, 0, &[1])),
            Err(SrtpError::PacketIndexOverflow)
        );
    }
}
