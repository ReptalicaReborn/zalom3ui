import crypto from "node:crypto";

/**
 * SRTP (RFC 3711) with AES_CM_128_HMAC_SHA1_80, the suite Zalo calls use (`srtpMode` 1). Zalo PC's call program hands
 * the session's first 30 characters to libsrtp as master key (16) + salt (14), the same key for both directions; each
 * stream is told apart by its SSRC. RTCP isn't protected (`srtcp: 0`). Checked against libsrtp in test/srtp.test.ts.
 */

const KEY_LEN = 16;
const SALT_LEN = 14;
const AUTH_KEY_LEN = 20;
export const AUTH_TAG_LEN = 10;

/** AES-128 in counter mode from a 16-byte IV, as SRTP's key derivation and encryption use it. */
function aesCm(key: Buffer, iv: Buffer, data: Buffer): Buffer {
    const c = crypto.createCipheriv("aes-128-ctr", key, iv);
    return Buffer.concat([c.update(data), c.final()]);
}

/** RFC 3711 §4.3 key derivation with key_derivation_rate 0: one key per label. */
export function deriveKey(masterKey: Buffer, masterSalt: Buffer, label: number, length: number): Buffer {
    const iv = Buffer.alloc(16);
    masterSalt.copy(iv, 0, 0, SALT_LEN);
    iv[7] ^= label;
    return aesCm(masterKey, iv, Buffer.alloc(length));
}

type Stream = { roc: number; lastSeq: number; started: boolean };

export type RtpHeader = { version: number; payloadType: number; marker: boolean; seq: number; timestamp: number; ssrc: number; headerLength: number };

/** Reads an RTP header (fixed part, CSRCs and header extension); null when it isn't RTP version 2. */
export function rtpHeader(packet: Buffer): RtpHeader | null {
    if (packet.length < 12 || packet[0] >> 6 !== 2) return null;
    const cc = packet[0] & 0x0f;
    let headerLength = 12 + 4 * cc;
    if (packet[0] & 0x10) {
        if (packet.length < headerLength + 4) return null;
        headerLength += 4 + 4 * packet.readUInt16BE(headerLength + 2);
    }
    if (packet.length < headerLength) return null;
    return {
        version: 2,
        payloadType: packet[1] & 0x7f,
        marker: (packet[1] & 0x80) !== 0,
        seq: packet.readUInt16BE(2),
        timestamp: packet.readUInt32BE(4),
        ssrc: packet.readUInt32BE(8),
        headerLength,
    };
}

export class SrtpContext {
    private readonly encKey: Buffer;
    private readonly authKey: Buffer;
    private readonly salt: Buffer;
    private readonly streams = new Map<number, Stream>();

    constructor(masterKey: Buffer, masterSalt: Buffer) {
        if (masterKey.length !== KEY_LEN || masterSalt.length !== SALT_LEN) throw new Error("SRTP needs a 16-byte key and a 14-byte salt");
        this.encKey = deriveKey(masterKey, masterSalt, 0, KEY_LEN);
        this.authKey = deriveKey(masterKey, masterSalt, 1, AUTH_KEY_LEN);
        this.salt = deriveKey(masterKey, masterSalt, 2, SALT_LEN);
    }

    /** The key Zalo uses for a call: the session's first 30 characters, as bytes. */
    static fromSession(session: string): SrtpContext {
        const raw = Buffer.from(session, "latin1");
        if (raw.length < KEY_LEN + SALT_LEN) throw new Error("Session too short for an SRTP key");
        return new SrtpContext(raw.subarray(0, KEY_LEN), raw.subarray(KEY_LEN, KEY_LEN + SALT_LEN));
    }

    private iv(ssrc: number, index: bigint): Buffer {
        const iv = Buffer.alloc(16);
        this.salt.copy(iv, 0);
        iv.writeUInt32BE((iv.readUInt32BE(4) ^ ssrc) >>> 0, 4);
        // The 48-bit packet index goes into bytes 8..13.
        for (let i = 0; i < 6; i++) iv[13 - i] ^= Number((index >> BigInt(8 * i)) & 0xffn);
        return iv;
    }

    private tag(authenticated: Buffer, roc: number): Buffer {
        const r = Buffer.alloc(4);
        r.writeUInt32BE(roc >>> 0);
        return crypto.createHmac("sha1", this.authKey).update(authenticated).update(r).digest().subarray(0, AUTH_TAG_LEN);
    }

    /** Guesses the rollover counter for a received sequence number (RFC 3711 §3.3.1). */
    private estimateRoc(s: Stream, seq: number): number {
        if (!s.started) return s.roc;
        if (s.lastSeq < 0x8000) return seq - s.lastSeq > 0x8000 ? s.roc - 1 : s.roc;
        return s.lastSeq - 0x8000 > seq ? s.roc + 1 : s.roc;
    }

    private stream(ssrc: number): Stream {
        let s = this.streams.get(ssrc);
        if (!s) this.streams.set(ssrc, (s = { roc: 0, lastSeq: 0, started: false }));
        return s;
    }

    /** Encrypts and signs one RTP packet. */
    protect(rtp: Buffer): Buffer {
        const h = rtpHeader(rtp);
        if (!h) throw new Error("Not an RTP packet");
        const s = this.stream(h.ssrc);
        if (s.started && h.seq < s.lastSeq && s.lastSeq - h.seq > 0x8000) s.roc++;
        s.started = true;
        s.lastSeq = h.seq;
        const index = (BigInt(s.roc >>> 0) << 16n) | BigInt(h.seq);
        const payload = aesCm(this.encKey, this.iv(h.ssrc, index), rtp.subarray(h.headerLength));
        const body = Buffer.concat([rtp.subarray(0, h.headerLength), payload]);
        return Buffer.concat([body, this.tag(body, s.roc)]);
    }

    /** Checks and decrypts one SRTP packet; null when the tag doesn't match (wrong key, damaged, not ours). */
    unprotect(srtp: Buffer): Buffer | null {
        if (srtp.length < 12 + AUTH_TAG_LEN) return null;
        const body = srtp.subarray(0, srtp.length - AUTH_TAG_LEN);
        const h = rtpHeader(body);
        if (!h) return null;
        const s = this.stream(h.ssrc);
        const roc = this.estimateRoc(s, h.seq);
        if (!crypto.timingSafeEqual(this.tag(body, roc), srtp.subarray(srtp.length - AUTH_TAG_LEN))) return null;
        if (!s.started || roc > s.roc || (roc === s.roc && h.seq > s.lastSeq)) {
            s.roc = roc;
            s.lastSeq = h.seq;
            s.started = true;
        }
        const index = (BigInt(roc >>> 0) << 16n) | BigInt(h.seq);
        const payload = aesCm(this.encKey, this.iv(h.ssrc, index), body.subarray(h.headerLength));
        return Buffer.concat([body.subarray(0, h.headerLength), payload]);
    }
}
