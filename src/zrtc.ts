/**
 * zrtc, Zalo's call media protocol: the packets a call exchanges with Zalo's media server over UDP. Worked out from the
 * official Zalo PC call program (docs/calls.md, "zrtc packets"); the test vectors in test/zrtc.test.ts were produced by
 * running that program's own packet functions, so this codec matches it byte for byte. Multi-byte numbers are
 * little-endian; strings are length-prefixed (16- or 32-bit length) without a terminator.
 */

/** First byte of every packet. */
export const Kind = {
    /** Request from a client (init call, ping, end call, forward…), with the control header. */
    Request: 1,
    /** Answer from the media server, with the control header. */
    Response: 2,
    /** Audio RTP from a client: kind, 32-bit stream token, then the (SRTP-protected) RTP packet. */
    Audio: 3,
    /** Audio RTP from the server, without a token. */
    AudioIn: 4,
    /** Audio RTCP, both ways, with the token (plain RTCP: Zalo calls run with `srtcp: 0`). */
    AudioRtcp: 5,
    /** Audio FEC from a client (sent with a token); from the server it comes without one. */
    AudioFec: 7,
    /** Video RTP from a client (sent with a token). */
    Video: 13,
    /** Video RTP from the server, without a token. */
    VideoIn: 14,
    /** Video RTCP, both ways, with the token. */
    VideoRtcp: 15,
    /** Peer-to-peer packets (binding, echo, media sent directly between the phones). */
    P2P: 0x7f,
} as const;

/** Command of a request/response (the 16-bit field of the control header). */
export const Cmd = {
    Ping: 2,
    EndCall: 3,
    EchoAnonymous: 5,
    InitCallCaller: 11,
    InitCallCallee: 12,
    ChangeAddress: 14,
    Forward: 32,
    Cmd33: 33,
    Cmd50: 50,
    InitZaviPing: 51,
} as const;

/** The 21-byte header of requests and responses. */
export type ControlHeader = {
    kind: number;
    /** Byte 1: 0x7e on init call / end call, 1 on ping and most others. */
    flag: number;
    w2: number;
    w6: number;
    /** Bytes 10–13: the sender's zrtc user id on everything a client sends. */
    userId: number;
    /** Bytes 14–17: the media server's id on ping / end call (Zalo PC's `ZRTPServerInfo`). */
    w14: number;
    cmd: number;
    /** Byte 20: init call attempt (1 first, 2 again), end call flag, forward sub-type… */
    sub: number;
};

export const HEADER_BYTES = 21;
/** Largest payload Zalo PC accepts in one packet. */
export const MAX_PAYLOAD = 0x578;

class Writer {
    private parts: Buffer[] = [];
    u8(v: number) {
        this.parts.push(Buffer.from([v & 0xff]));
        return this;
    }
    u16(v: number) {
        const b = Buffer.alloc(2);
        b.writeUInt16LE(v & 0xffff);
        this.parts.push(b);
        return this;
    }
    u32(v: number) {
        const b = Buffer.alloc(4);
        b.writeUInt32LE(v >>> 0);
        this.parts.push(b);
        return this;
    }
    str16(s: string | Buffer) {
        const b = typeof s === "string" ? Buffer.from(s, "utf8") : s;
        return this.u16(b.length).bytes(b);
    }
    bytes(b: Buffer) {
        this.parts.push(b);
        return this;
    }
    done() {
        return Buffer.concat(this.parts);
    }
}

function header(w: Writer, h: ControlHeader) {
    return w.u8(h.kind).u8(h.flag).u32(h.w2).u32(h.w6).u32(h.userId).u32(h.w14).u16(h.cmd).u8(h.sub);
}

/**
 * Asks the media server to join a call (Zalo PC: `initZRTPPacketRequestInitCall`). The caller sends command 11; the
 * person called sends 12, which adds a second string (empty on a first attempt).
 */
export function initCall(p: {
    isCaller: boolean;
    userId: number;
    partnerId: number;
    callId: number;
    session: string;
    /** 1 on the first attempt, 2 when re-joining. */
    attempt?: number;
    extra?: string;
}): Buffer {
    const w = header(new Writer(), {
        kind: Kind.Request,
        flag: 0x7e,
        w2: 0,
        w6: 0,
        userId: p.userId,
        w14: 0,
        cmd: p.isCaller ? Cmd.InitCallCaller : Cmd.InitCallCallee,
        sub: p.attempt ?? 1,
    })
        .u32(p.callId)
        .u32(p.partnerId)
        .str16(p.session);
    if (!p.isCaller) w.str16(p.extra ?? "");
    return w.done();
}

/** Keep-alive to the media server (Zalo PC: `_sendRequestZRTPPing`): user id, the server's id, the session. */
export function ping(p: { userId: number; serverId: number; session: string }): Buffer {
    return header(new Writer(), { kind: Kind.Request, flag: 1, w2: 0, w6: 0, userId: p.userId, w14: p.serverId, cmd: Cmd.Ping, sub: 1 })
        .str16(p.session)
        .done();
}

/** Hangs up (Zalo PC: `_sendRequestZRTPEndCall`): user id, call id, the server's id. */
export function endCall(p: { userId: number; callId: number; serverId: number; flag?: boolean }): Buffer {
    return header(new Writer(), {
        kind: Kind.Request,
        flag: 0x7e,
        w2: 0,
        w6: 0,
        userId: p.userId,
        w14: p.serverId,
        cmd: Cmd.EndCall,
        sub: p.flag ? 1 : 0,
    })
        .u32(p.callId)
        .done();
}

/**
 * RTCP to the media server: kind 5 (audio) or 15 (video), the stream token, the RTCP packet (Zalo PC:
 * `initZRTPPacketAudio(…, rtcp = true)`).
 */
export function rtcp(token: number, packet: Buffer, video = false): Buffer {
    if (packet.length > MAX_PAYLOAD) throw new Error(`RTCP packet too large (${packet.length} bytes)`);
    return new Writer().u8(video ? Kind.VideoRtcp : Kind.AudioRtcp).u32(token).bytes(packet).done();
}

/** One audio packet to the media server: kind 3, the stream token, then the SRTP-protected RTP packet. */
export function audio(token: number, rtp: Buffer): Buffer {
    if (rtp.length > MAX_PAYLOAD) throw new Error(`Audio packet too large (${rtp.length} bytes)`);
    return new Writer().u8(Kind.Audio).u32(token).bytes(rtp).done();
}

/** One video packet to the media server: kind 13, the stream token, then the SRTP-protected RTP packet. */
export function video(token: number, rtp: Buffer): Buffer {
    if (rtp.length > MAX_PAYLOAD) throw new Error(`Video packet too large (${rtp.length} bytes)`);
    return new Writer().u8(Kind.Video).u32(token).bytes(rtp).done();
}

export type Packet =
    | ({ type: "control" } & ControlHeader & {
          /** Error code of a response (0 = success), when the command has one. */
          error?: number;
          /** Init call response: two numbers the server assigns (Zalo PC keeps them in fields 0x38 and 0x18). */
          value38?: number;
          value18?: number;
          /** Init call response: the address the server saw us at ("ip:port"); forward/echo: their text. */
          text?: string;
          payload?: Buffer;
      })
    | { type: "media"; kind: number; token?: number; payload: Buffer }
    | { type: "p2p"; cmd: number; from: number; flag1: number; flag2: number; payload?: Buffer; rest: Buffer }
    | { type: "unknown"; kind: number; bytes: Buffer };

class Reader {
    private at = 0;
    constructor(private readonly b: Buffer) {}
    get left() {
        return this.b.length - this.at;
    }
    need(n: number) {
        if (this.left < n) throw new RangeError(`zrtc packet cut short (need ${n} more bytes)`);
    }
    u8() {
        this.need(1);
        return this.b[this.at++];
    }
    u16() {
        this.need(2);
        const v = this.b.readUInt16LE(this.at);
        this.at += 2;
        return v;
    }
    u32() {
        this.need(4);
        const v = this.b.readUInt32LE(this.at);
        this.at += 4;
        return v;
    }
    take(n: number) {
        this.need(n);
        const v = this.b.subarray(this.at, this.at + n);
        this.at += n;
        return v;
    }
    str16() {
        return this.take(this.u16()).toString("utf8");
    }
    str32() {
        return this.take(this.u32()).toString("utf8");
    }
    rest() {
        return this.take(this.left);
    }
}

/** Reads one packet from the media server (or the other phone, for P2P), the way Zalo PC's `_parsePacketInternal` does. */
export function parse(buf: Buffer): Packet {
    if (buf.length === 0) throw new RangeError("Empty zrtc packet");
    const r = new Reader(buf);
    const kind = r.u8();
    if (kind === Kind.AudioIn || kind === Kind.AudioFec || kind === Kind.VideoIn) {
        return { type: "media", kind, payload: limit(r.rest()) };
    }
    if (kind === Kind.AudioRtcp || kind === Kind.VideoRtcp) {
        const token = r.u32();
        return { type: "media", kind, token, payload: limit(r.rest()) };
    }
    if (kind === Kind.Request || kind === Kind.Response) {
        const h: ControlHeader = { kind, flag: r.u8(), w2: r.u32(), w6: r.u32(), userId: r.u32(), w14: r.u32(), cmd: r.u16(), sub: r.u8() };
        const pkt: Extract<Packet, { type: "control" }> = { type: "control", ...h };
        if (kind === Kind.Response) {
            if (h.cmd === Cmd.InitCallCaller || h.cmd === Cmd.InitCallCallee) {
                pkt.error = r.u32();
                if (pkt.error === 0) {
                    pkt.value38 = r.u32();
                    pkt.value18 = r.u32();
                    // The address is optional: Zalo's servers send just the error and the two numbers (33 bytes).
                    if (r.left >= 2) pkt.text = r.str16();
                }
            } else if (h.cmd === Cmd.EndCall || h.cmd === Cmd.ChangeAddress) {
                pkt.error = r.u32();
            } else if (h.cmd === Cmd.Cmd33) {
                pkt.error = r.u32();
                pkt.text = r.str16();
            } else if (h.cmd === Cmd.Cmd50) {
                pkt.error = r.u32();
                if (pkt.error === 0) pkt.payload = limit(r.rest());
            } else if (h.cmd === Cmd.Forward) {
                readForward(r, pkt);
            }
        } else if (h.cmd === Cmd.Forward) {
            readForward(r, pkt);
        } else if (h.cmd === Cmd.EchoAnonymous) {
            pkt.text = r.str16();
        }
        return pkt;
    }
    if (kind === Kind.P2P) {
        const flag1 = r.u8();
        const flag2 = r.u8();
        const from = r.u32();
        const cmd = r.u16();
        if (cmd >= 6 && cmd <= 10) return { type: "p2p", cmd, from, flag1, flag2, payload: limit(r.rest()), rest: Buffer.alloc(0) };
        return { type: "p2p", cmd, from, flag1, flag2, rest: r.rest() };
    }
    return { type: "unknown", kind, bytes: buf };
}

/** Forward (command 32): the sub-type says whether the text has a 32- or a 16-bit length. */
function readForward(r: Reader, pkt: { sub: number; text?: string }) {
    const bit = 1 << pkt.sub;
    if (pkt.sub > 0x11) return;
    if (bit & 0x30b80 || pkt.sub === 10) pkt.text = r.str32();
    else if (bit & 0x3000) pkt.text = r.str16();
}

function limit(b: Buffer): Buffer {
    if (b.length > MAX_PAYLOAD) throw new RangeError(`zrtc payload too large (${b.length} bytes)`);
    return b;
}
