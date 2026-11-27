//! zrtc, Zalo's call media protocol: the packets a call exchanges with Zalo's media server over UDP.
//! Exact byte-for-byte port of `src/zrtc.ts`. Multi-byte numbers are little-endian; strings are
//! length-prefixed (16- or 32-bit length) without a terminator.

use thiserror::Error;

/// First byte of every zrtc UDP packet.
pub mod kind {
    /// Request from a client (init call, ping, end call, forward…), with the 21-byte control header.
    pub const REQUEST: u8 = 1;
    /// Answer from the media server, with the 21-byte control header.
    pub const RESPONSE: u8 = 2;
    /// Audio RTP from a client: kind, 32-bit stream token, then the (SRTP-protected) RTP packet.
    pub const AUDIO: u8 = 3;
    /// Audio RTP from the server, without a token.
    pub const AUDIO_IN: u8 = 4;
    /// Audio RTCP, both ways, with the token (plain RTCP: Zalo calls run with `srtcp: 0`).
    pub const AUDIO_RTCP: u8 = 5;
    /// Audio FEC from a client (sent with a token); from the server it comes without one.
    pub const AUDIO_FEC: u8 = 7;
    /// Video RTP from a client (sent with a token).
    pub const VIDEO: u8 = 13;
    /// Video RTP from the server, without a token.
    pub const VIDEO_IN: u8 = 14;
    /// Video RTCP, both ways, with the token.
    pub const VIDEO_RTCP: u8 = 15;
    /// Peer-to-peer packets (binding, echo, media sent directly between phones).
    pub const P2P: u8 = 0x7f;
}

/// Command of a request/response (the 16-bit LE field of the control header).
pub mod cmd {
    pub const PING: u16 = 2;
    pub const END_CALL: u16 = 3;
    pub const ECHO_ANONYMOUS: u16 = 5;
    pub const INIT_CALL_CALLER: u16 = 11;
    pub const INIT_CALL_CALLEE: u16 = 12;
    pub const CHANGE_ADDRESS: u16 = 14;
    pub const FORWARD: u16 = 32;
    pub const CMD_33: u16 = 33;
    pub const CMD_50: u16 = 50;
    pub const INIT_ZAVI_PING: u16 = 51;
}

/// Size of the control packet header in bytes.
pub const HEADER_BYTES: usize = 21;

/// Largest payload Zalo PC accepts in one packet (`0x578 = 1400`).
pub const MAX_PAYLOAD: usize = 0x578;

/// Errors produced when encoding or decoding zrtc packets.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ZrtcError {
    #[error("Empty zrtc packet")]
    Empty,
    #[error("zrtc packet cut short (need {needed} more bytes)")]
    Truncated { needed: usize },
    #[error("zrtc payload too large ({0} bytes)")]
    PayloadTooLarge(usize),
    #[error("zrtc str16 too long ({0} bytes)")]
    Str16TooLong(usize),
}

/// The 21-byte header of requests and responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlHeader {
    pub kind: u8,
    /// Byte 1: `0x7e` on init call / end call, `1` on ping and most others.
    pub flag: u8,
    pub w2: u32,
    pub w6: u32,
    /// Bytes 10–13: the sender's zrtc user id on everything a client sends.
    pub user_id: u32,
    /// Bytes 14–17: the media server's id on ping / end call.
    pub w14: u32,
    pub cmd: u16,
    /// Byte 20: init call attempt (1 first, 2 again), end call flag, forward sub-type…
    pub sub: u8,
}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn str16(&mut self, s: &[u8]) -> Result<&mut Self, ZrtcError> {
        if s.len() > 0xffff {
            return Err(ZrtcError::Str16TooLong(s.len()));
        }
        self.u16(s.len() as u16);
        self.buf.extend_from_slice(s);
        Ok(self)
    }

    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    fn header(&mut self, h: ControlHeader) -> &mut Self {
        self.u8(h.kind)
            .u8(h.flag)
            .u32(h.w2)
            .u32(h.w6)
            .u32(h.user_id)
            .u32(h.w14)
            .u16(h.cmd)
            .u8(h.sub)
    }

    fn done(self) -> Vec<u8> {
        self.buf
    }
}

/// Parameters for [`init_call`].
#[derive(Debug, Clone, Copy)]
pub struct InitCallParams<'a> {
    pub is_caller: bool,
    pub user_id: u32,
    pub partner_id: u32,
    pub call_id: u32,
    pub session: &'a str,
    /// `1` on the first attempt, `2` when re-joining.
    pub attempt: u8,
    pub extra: &'a str,
}

/// Asks the media server to join a call (`initZRTPPacketRequestInitCall`).
/// The caller sends command 11; the callee sends 12, which adds a second string (`extra`).
pub fn init_call(p: InitCallParams<'_>) -> Result<Vec<u8>, ZrtcError> {
    let mut w = Writer::with_capacity(HEADER_BYTES + 12 + p.session.len() + p.extra.len());
    w.header(ControlHeader {
        kind: kind::REQUEST,
        flag: 0x7e,
        w2: 0,
        w6: 0,
        user_id: p.user_id,
        w14: 0,
        cmd: if p.is_caller {
            cmd::INIT_CALL_CALLER
        } else {
            cmd::INIT_CALL_CALLEE
        },
        sub: p.attempt,
    })
    .u32(p.call_id)
    .u32(p.partner_id);
    w.str16(p.session.as_bytes())?;
    if !p.is_caller {
        w.str16(p.extra.as_bytes())?;
    }
    Ok(w.done())
}

/// Keep-alive to the media server (`_sendRequestZRTPPing`).
pub fn ping(user_id: u32, server_id: u32, session: &str) -> Result<Vec<u8>, ZrtcError> {
    let mut w = Writer::with_capacity(HEADER_BYTES + 2 + session.len());
    w.header(ControlHeader {
        kind: kind::REQUEST,
        flag: 1,
        w2: 0,
        w6: 0,
        user_id,
        w14: server_id,
        cmd: cmd::PING,
        sub: 1,
    });
    w.str16(session.as_bytes())?;
    Ok(w.done())
}

/// Hangs up on the media server (`_sendRequestZRTPEndCall`).
pub fn end_call(user_id: u32, call_id: u32, server_id: u32, flag: bool) -> Vec<u8> {
    let mut w = Writer::with_capacity(HEADER_BYTES + 4);
    w.header(ControlHeader {
        kind: kind::REQUEST,
        flag: 0x7e,
        w2: 0,
        w6: 0,
        user_id,
        w14: server_id,
        cmd: cmd::END_CALL,
        sub: u8::from(flag),
    })
    .u32(call_id);
    w.done()
}

/// Forward request (command 32) to the media server: control header with a forward counter in
/// bytes 6..10 and the stream token in bytes 14..18, then `body` verbatim.
pub fn forward(user_id: u32, token: u32, seq: u32, sub: u8, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(HEADER_BYTES + body.len());
    w.header(ControlHeader {
        kind: kind::REQUEST,
        flag: 1,
        w2: 0,
        w6: seq,
        user_id,
        w14: token,
        cmd: cmd::FORWARD,
        sub,
    })
    .bytes(body);
    w.done()
}

/// Per-second receive status (forward sub 10). The body is a 32-bit-length string holding five
/// u32 LE values `[0, 1, 1, audio_rtp_received_last_second, 0]` (so on the wire six u32s, the
/// first being the length 20).
pub fn forward_status(user_id: u32, token: u32, seq: u32, audio_rx_last_second: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(24);
    for v in [20u32, 0, 1, 1, audio_rx_last_second, 0] {
        body.extend_from_slice(&v.to_le_bytes());
    }
    forward(user_id, token, seq, 10, &body)
}

/// One-off hello after joining (forward sub 8): a 32-bit-length string of four zero bytes.
pub fn forward_hello(user_id: u32, token: u32) -> Vec<u8> {
    forward(user_id, token, 0, 8, &[4, 0, 0, 0, 0, 0, 0, 0])
}

/// RTCP to the media server: kind 5 (audio) or 15 (video), stream token, RTCP packet.
pub fn rtcp(token: u32, packet: &[u8], video: bool) -> Result<Vec<u8>, ZrtcError> {
    if packet.len() > MAX_PAYLOAD {
        return Err(ZrtcError::PayloadTooLarge(packet.len()));
    }
    let mut w = Writer::with_capacity(5 + packet.len());
    w.u8(if video {
        kind::VIDEO_RTCP
    } else {
        kind::AUDIO_RTCP
    })
    .u32(token)
    .bytes(packet);
    Ok(w.done())
}

/// One audio packet to the media server: kind 3, stream token, SRTP-protected RTP packet.
pub fn audio(token: u32, rtp: &[u8]) -> Result<Vec<u8>, ZrtcError> {
    if rtp.len() > MAX_PAYLOAD {
        return Err(ZrtcError::PayloadTooLarge(rtp.len()));
    }
    let mut w = Writer::with_capacity(5 + rtp.len());
    w.u8(kind::AUDIO).u32(token).bytes(rtp);
    Ok(w.done())
}

/// One video packet to the media server: kind 13, stream token, SRTP-protected RTP packet.
pub fn video(token: u32, rtp: &[u8]) -> Result<Vec<u8>, ZrtcError> {
    if rtp.len() > MAX_PAYLOAD {
        return Err(ZrtcError::PayloadTooLarge(rtp.len()));
    }
    let mut w = Writer::with_capacity(5 + rtp.len());
    w.u8(kind::VIDEO).u32(token).bytes(rtp);
    Ok(w.done())
}

/// Decoded zrtc UDP packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Control {
        header: ControlHeader,
        /// Error code of a response (0 = success), when the command has one.
        error: Option<u32>,
        /// Init call response: two numbers assigned by the server (`value38`, `value18` = stream token).
        value38: Option<u32>,
        value18: Option<u32>,
        /// Init call response: observed NAT address (`"ip:port"`); forward/echo: text.
        text: Option<String>,
        payload: Option<Vec<u8>>,
    },
    Media {
        kind: u8,
        token: Option<u32>,
        payload: Vec<u8>,
    },
    P2p {
        cmd: u16,
        from: u32,
        flag1: u8,
        flag2: u8,
        payload: Option<Vec<u8>>,
        rest: Vec<u8>,
    },
    Unknown {
        kind: u8,
        bytes: Vec<u8>,
    },
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }

    fn left(&self) -> usize {
        self.b.len().saturating_sub(self.at)
    }

    fn need(&self, n: usize) -> Result<(), ZrtcError> {
        if self.left() < n {
            return Err(ZrtcError::Truncated { needed: n });
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, ZrtcError> {
        self.need(1)?;
        let v = self.b[self.at];
        self.at += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, ZrtcError> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.b[self.at], self.b[self.at + 1]]);
        self.at += 2;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32, ZrtcError> {
        self.need(4)?;
        let v = u32::from_le_bytes([
            self.b[self.at],
            self.b[self.at + 1],
            self.b[self.at + 2],
            self.b[self.at + 3],
        ]);
        self.at += 4;
        Ok(v)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ZrtcError> {
        self.need(n)?;
        let v = &self.b[self.at..self.at + n];
        self.at += n;
        Ok(v)
    }

    fn str16(&mut self) -> Result<String, ZrtcError> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    fn str32(&mut self) -> Result<String, ZrtcError> {
        let n = self.u32()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    fn rest(&mut self) -> &'a [u8] {
        let v = &self.b[self.at..];
        self.at = self.b.len();
        v
    }
}

fn limit(b: &[u8]) -> Result<Vec<u8>, ZrtcError> {
    if b.len() > MAX_PAYLOAD {
        return Err(ZrtcError::PayloadTooLarge(b.len()));
    }
    Ok(b.to_vec())
}

/// Forward (command 32): the sub-type says whether the text has a 32- or a 16-bit length.
fn read_forward(r: &mut Reader<'_>, sub: u8) -> Result<Option<String>, ZrtcError> {
    if sub > 0x11 {
        return Ok(None);
    }
    let bit = 1u32 << sub;
    if (bit & 0x30b80) != 0 || sub == 10 {
        Ok(Some(r.str32()?))
    } else if (bit & 0x3000) != 0 {
        Ok(Some(r.str16()?))
    } else {
        Ok(None)
    }
}

/// Parses one packet from the media server (or peer for P2P), matching Zalo PC's `_parsePacketInternal`.
pub fn parse(buf: &[u8]) -> Result<Packet, ZrtcError> {
    if buf.is_empty() {
        return Err(ZrtcError::Empty);
    }
    let mut r = Reader::new(buf);
    let kind = r.u8()?;

    if matches!(kind, kind::AUDIO_IN | kind::AUDIO_FEC | kind::VIDEO_IN) {
        return Ok(Packet::Media {
            kind,
            token: None,
            payload: limit(r.rest())?,
        });
    }
    if matches!(kind, kind::AUDIO_RTCP | kind::VIDEO_RTCP) {
        let token = r.u32()?;
        return Ok(Packet::Media {
            kind,
            token: Some(token),
            payload: limit(r.rest())?,
        });
    }
    if matches!(kind, kind::REQUEST | kind::RESPONSE) {
        let header = ControlHeader {
            kind,
            flag: r.u8()?,
            w2: r.u32()?,
            w6: r.u32()?,
            user_id: r.u32()?,
            w14: r.u32()?,
            cmd: r.u16()?,
            sub: r.u8()?,
        };
        let mut error = None;
        let mut value38 = None;
        let mut value18 = None;
        let mut text = None;
        let mut payload = None;

        if kind == kind::RESPONSE {
            match header.cmd {
                cmd::INIT_CALL_CALLER | cmd::INIT_CALL_CALLEE => {
                    let err = r.u32()?;
                    error = Some(err);
                    if err == 0 {
                        value38 = Some(r.u32()?);
                        value18 = Some(r.u32()?);
                        // The address is optional: Zalo's servers often send just 33 bytes.
                        if r.left() >= 2 {
                            text = Some(r.str16()?);
                        }
                    }
                }
                cmd::END_CALL | cmd::CHANGE_ADDRESS => {
                    error = Some(r.u32()?);
                }
                cmd::CMD_33 => {
                    error = Some(r.u32()?);
                    text = Some(r.str16()?);
                }
                cmd::CMD_50 => {
                    let err = r.u32()?;
                    error = Some(err);
                    if err == 0 {
                        payload = Some(limit(r.rest())?);
                    }
                }
                cmd::FORWARD => {
                    text = read_forward(&mut r, header.sub)?;
                }
                _ => {}
            }
        } else if header.cmd == cmd::FORWARD {
            text = read_forward(&mut r, header.sub)?;
        } else if header.cmd == cmd::ECHO_ANONYMOUS {
            text = Some(r.str16()?);
        }

        return Ok(Packet::Control {
            header,
            error,
            value38,
            value18,
            text,
            payload,
        });
    }
    if kind == kind::P2P {
        let flag1 = r.u8()?;
        let flag2 = r.u8()?;
        let from = r.u32()?;
        let cmd = r.u16()?;
        if (6..=10).contains(&cmd) {
            return Ok(Packet::P2p {
                cmd,
                from,
                flag1,
                flag2,
                payload: Some(limit(r.rest())?),
                rest: Vec::new(),
            });
        }
        return Ok(Packet::P2p {
            cmd,
            from,
            flag1,
            flag2,
            payload: None,
            rest: r.rest().to_vec(),
        });
    }
    Ok(Packet::Unknown {
        kind,
        bytes: buf.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(s: &str) -> Vec<u8> {
        let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        hex::decode(clean).unwrap()
    }

    const A: u32 = 0x1111_1111;
    const B: u32 = 0x2222_2222;
    const C: u32 = 0x3333_3333;

    #[test]
    fn forward_status_and_hello_round_trip() {
        let st = forward_status(A, B, 7, 49);
        assert_eq!(st.len(), HEADER_BYTES + 24);
        assert_eq!(&st[..2], &[1, 1]);
        assert_eq!(&st[2..6], &[0; 4]);
        assert_eq!(&st[6..10], &7u32.to_le_bytes());
        assert_eq!(&st[10..14], &A.to_le_bytes());
        assert_eq!(&st[14..18], &B.to_le_bytes());
        assert_eq!(&st[18..20], &32u16.to_le_bytes());
        assert_eq!(st[20], 10);
        let vals: Vec<u32> = st[21..]
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(vals, [20, 0, 1, 1, 49, 0]);
        match parse(&st).unwrap() {
            Packet::Control { header, text, .. } => {
                assert_eq!(
                    (header.kind, header.cmd, header.sub),
                    (kind::REQUEST, cmd::FORWARD, 10)
                );
                assert_eq!((header.w6, header.user_id, header.w14), (7, A, B));
                // str32 of the five remaining u32s.
                assert_eq!(text.map(|t| t.len()), Some(20));
            }
            other => panic!("expected control, got {other:?}"),
        }

        let hello = forward_hello(A, B);
        assert_eq!(hello[18..], [32, 0, 8, 4, 0, 0, 0, 0, 0, 0, 0]);
        match parse(&hello).unwrap() {
            Packet::Control { header, text, .. } => {
                assert_eq!((header.cmd, header.sub, header.w6), (cmd::FORWARD, 8, 0));
                assert_eq!(text.as_deref(), Some("\0\0\0\0"));
            }
            other => panic!("expected control, got {other:?}"),
        }
    }

    #[test]
    fn init_call_as_caller_matches_zalo_pc() {
        let pkt = init_call(InitCallParams {
            is_caller: true,
            user_id: A,
            partner_id: B,
            call_id: C,
            session: "SESS",
            attempt: 0x55,
            extra: "KEY",
        })
        .unwrap();
        assert_eq!(
            pkt,
            decode_hex(
                "01 7e 00000000 00000000 11111111 00000000 0b00 55 33333333 22222222 0400 53455353"
            )
        );
    }

    #[test]
    fn init_call_as_callee_matches_zalo_pc() {
        let pkt = init_call(InitCallParams {
            is_caller: false,
            user_id: A,
            partner_id: B,
            call_id: C,
            session: "SESS",
            attempt: 0x55,
            extra: "KEY",
        })
        .unwrap();
        assert_eq!(
            pkt,
            decode_hex(
                "01 7e 00000000 00000000 11111111 00000000 0c00 55 33333333 22222222 0400 53455353 0300 4b4559"
            )
        );

        let first = init_call(InitCallParams {
            is_caller: false,
            user_id: A,
            partner_id: B,
            call_id: C,
            session: "SESS",
            attempt: 1,
            extra: "",
        })
        .unwrap();
        assert_eq!(&first[first.len() - 8..], &decode_hex("0400 53455353 0000"));
    }

    #[test]
    fn ping_and_end_call_match_zalo_pc() {
        assert_eq!(
            ping(A, B, "PING").unwrap(),
            decode_hex("01 01 00000000 00000000 11111111 22222222 0200 01 0400 50494e47")
        );
        assert_eq!(
            end_call(A, B, C, true),
            decode_hex("01 7e 00000000 00000000 11111111 33333333 0300 01 22222222")
        );
        assert_eq!(
            end_call(A, B, C, false),
            decode_hex("01 7e 00000000 00000000 11111111 33333333 0300 00 22222222")
        );
    }

    #[test]
    fn audio_video_rtcp_and_size_limits() {
        assert_eq!(audio(B, b"RTP").unwrap(), decode_hex("03 22222222 525450"));
        assert_eq!(video(B, b"VID").unwrap(), decode_hex("0d 22222222 564944"));
        assert_eq!(
            rtcp(B, b"RC", false).unwrap(),
            decode_hex("05 22222222 5243")
        );
        assert_eq!(
            rtcp(B, b"RC", true).unwrap(),
            decode_hex("0f 22222222 5243")
        );
        assert!(matches!(
            audio(1, &vec![0u8; MAX_PAYLOAD + 1]),
            Err(ZrtcError::PayloadTooLarge(0x579))
        ));
    }

    fn hdr(kind: u8, cmd: u16, sub: u8) -> Vec<u8> {
        let mut out = vec![kind, 0x7e];
        out.extend_from_slice(&decode_hex("a1a1a1a1 b2b2b2b2 c3c3c3c3 d4d4d4d4"));
        out.extend_from_slice(&cmd.to_le_bytes());
        out.push(sub);
        out
    }

    fn s16(s: &str) -> Vec<u8> {
        let mut out = (s.len() as u16).to_le_bytes().to_vec();
        out.extend_from_slice(s.as_bytes());
        out
    }

    #[test]
    fn parse_init_call_responses() {
        let mut raw = hdr(2, 12, 1);
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&0x1122_3344u32.to_le_bytes());
        raw.extend_from_slice(&0x5566_7788u32.to_le_bytes());
        raw.extend_from_slice(&s16("1.2.3.4:5678"));
        match parse(&raw).unwrap() {
            Packet::Control {
                header,
                error,
                value38,
                value18,
                text,
                ..
            } => {
                assert_eq!(error, Some(0));
                assert_eq!(value38, Some(0x1122_3344));
                assert_eq!(value18, Some(0x5566_7788));
                assert_eq!(text.as_deref(), Some("1.2.3.4:5678"));
                assert_eq!(header.cmd, 12);
                assert_eq!(header.sub, 1);
                assert_eq!(header.user_id, 0xc3c3_c3c3);
            }
            other => panic!("unexpected {other:?}"),
        }

        // 33-byte server response without address
        let mut real = decode_hex("027e00000000000000006a3d201a000000000c0001");
        real.extend_from_slice(&0u32.to_le_bytes());
        real.extend_from_slice(&0x1122_3344u32.to_le_bytes());
        real.extend_from_slice(&0x5566_7788u32.to_le_bytes());
        match parse(&real).unwrap() {
            Packet::Control {
                header,
                error,
                value38,
                value18,
                text,
                ..
            } => {
                assert_eq!(error, Some(0));
                assert_eq!(value38, Some(0x1122_3344));
                assert_eq!(value18, Some(0x5566_7788));
                assert_eq!(text, None);
                assert_eq!(header.user_id, 438_320_490);
                assert_eq!(header.cmd, 12);
            }
            other => panic!("unexpected {other:?}"),
        }

        let mut failed = hdr(2, 11, 1);
        failed.extend_from_slice(&7u32.to_le_bytes());
        match parse(&failed).unwrap() {
            Packet::Control { error, .. } => assert_eq!(error, Some(7)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_other_responses_and_requests() {
        let mut end = hdr(2, 3, 0);
        end.extend_from_slice(&0u32.to_le_bytes());
        assert!(matches!(
            parse(&end).unwrap(),
            Packet::Control { error: Some(0), .. }
        ));

        let mut fwd = hdr(1, 32, 7);
        fwd.extend_from_slice(&8u32.to_le_bytes());
        fwd.extend_from_slice(b"FWD-JSON");
        assert!(matches!(
            parse(&fwd).unwrap(),
            Packet::Control { text: Some(ref t), .. } if t == "FWD-JSON"
        ));

        let mut fwd16 = hdr(1, 32, 12);
        fwd16.extend_from_slice(&s16("S16"));
        assert!(matches!(
            parse(&fwd16).unwrap(),
            Packet::Control { text: Some(ref t), .. } if t == "S16"
        ));

        let mut echo = hdr(1, 5, 0);
        echo.extend_from_slice(&s16("ECHO"));
        assert!(matches!(
            parse(&echo).unwrap(),
            Packet::Control { text: Some(ref t), .. } if t == "ECHO"
        ));

        let mut r50 = hdr(2, 50, 0);
        r50.extend_from_slice(&0u32.to_le_bytes());
        r50.extend_from_slice(b"PAYLOAD50");
        assert!(matches!(
            parse(&r50).unwrap(),
            Packet::Control { payload: Some(ref p), .. } if p == b"PAYLOAD50"
        ));
    }

    #[test]
    fn parse_media_p2p_and_errors() {
        assert_eq!(
            parse(b"\x04RTPDATA4").unwrap(),
            Packet::Media {
                kind: 4,
                token: None,
                payload: b"RTPDATA4".to_vec(),
            }
        );

        let mut rtcp_in = vec![5];
        rtcp_in.extend_from_slice(&0x9988_7766u32.to_le_bytes());
        rtcp_in.extend_from_slice(b"RTPDATA5");
        assert_eq!(
            parse(&rtcp_in).unwrap(),
            Packet::Media {
                kind: 5,
                token: Some(0x9988_7766),
                payload: b"RTPDATA5".to_vec(),
            }
        );

        let mut p2p = vec![0x7f, 0, 1];
        p2p.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        p2p.extend_from_slice(&[7, 0]);
        p2p.extend_from_slice(b"P2PMEDIA");
        assert_eq!(
            parse(&p2p).unwrap(),
            Packet::P2p {
                cmd: 7,
                from: 0x1234_5678,
                flag1: 0,
                flag2: 1,
                payload: Some(b"P2PMEDIA".to_vec()),
                rest: Vec::new(),
            }
        );

        assert!(matches!(
            parse(&[2, 0x7e, 1]),
            Err(ZrtcError::Truncated { .. })
        ));
        assert!(matches!(parse(&[]), Err(ZrtcError::Empty)));
    }

    #[test]
    fn str16_overflow_rejected() {
        let too_long = "a".repeat(0x10000);
        assert!(matches!(
            ping(A, B, &too_long),
            Err(ZrtcError::Str16TooLong(0x10000))
        ));
        let max_ok = "a".repeat(0xffff);
        assert!(ping(A, B, &max_ok).is_ok());
    }
}
