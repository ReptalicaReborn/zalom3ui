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

const MAX_RX_STREAMS = 64;
const REPLAY_WINDOW = 64n;
const REPLAY_MASK_64 = (1n << REPLAY_WINDOW) - 1n;
const MAX_PACKET_INDEX = (1n << 48n) - 1n;

type TxStream = { roc: number; lastSeq: number; started: boolean };
type RxStream = { roc: number; lastSeq: number; started: boolean; replayMask: bigint };

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
    private readonly txStreams = new Map<number, TxStream>();
    private readonly rxStreams = new Map<number, RxStream>();

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

    /** Guesses the rollover counter for a received sequence number (RFC 3711 §3.3.1); null if out of range. */
    private estimateRoc(s: RxStream | undefined, seq: number): number | null {
        if (!s || !s.started) return 0;
        if (s.lastSeq < 0x8000) {
            if (seq - s.lastSeq > 0x8000) return s.roc > 0 ? s.roc - 1 : null;
            return s.roc;
        }
        if (s.lastSeq - 0x8000 > seq) return s.roc < 0xffffffff ? s.roc + 1 : null;
        return s.roc;
    }

    private txStream(ssrc: number): TxStream {
        let s = this.txStreams.get(ssrc);
        if (!s) this.txStreams.set(ssrc, (s = { roc: 0, lastSeq: 0, started: false }));
        return s;
    }

    /** Encrypts and signs one RTP packet. */
    protect(rtp: Buffer): Buffer {
        const h = rtpHeader(rtp);
        if (!h) throw new Error("Not an RTP packet");
        const s = this.txStream(h.ssrc);
        let nextRoc = s.roc;
        if (s.started && h.seq < s.lastSeq && s.lastSeq - h.seq > 0x8000) nextRoc++;
        if (nextRoc > 0xffffffff) throw new RangeError("SRTP packet index reached 2^48");
        const index = (BigInt(nextRoc) << 16n) | BigInt(h.seq);
        if (index > MAX_PACKET_INDEX) throw new RangeError("SRTP packet index reached 2^48");
        s.roc = nextRoc;
        s.started = true;
        s.lastSeq = h.seq;
        const payload = aesCm(this.encKey, this.iv(h.ssrc, index), rtp.subarray(h.headerLength));
        const body = Buffer.concat([rtp.subarray(0, h.headerLength), payload]);
        return Buffer.concat([body, this.tag(body, s.roc)]);
    }

    /** Checks and decrypts one SRTP packet; null when the tag doesn't match, replayed, damaged, or invalid. */
    unprotect(srtp: Buffer): Buffer | null {
        if (srtp.length < 12 + AUTH_TAG_LEN) return null;
        const body = srtp.subarray(0, srtp.length - AUTH_TAG_LEN);
        const h = rtpHeader(body);
        if (!h) return null;
        const existing = this.rxStreams.get(h.ssrc);
        if (!existing && this.rxStreams.size >= MAX_RX_STREAMS) return null;
        const roc = this.estimateRoc(existing, h.seq);
        if (roc === null) return null;
        const index = (BigInt(roc) << 16n) | BigInt(h.seq);
        if (index > MAX_PACKET_INDEX) return null;
        if (existing) {
            const maxIdx = (BigInt(existing.roc) << 16n) | BigInt(existing.lastSeq);
            if (index <= maxIdx) {
                const diff = maxIdx - index;
                if (diff >= REPLAY_WINDOW || ((existing.replayMask >> diff) & 1n) !== 0n) return null;
            }
        }
        if (!crypto.timingSafeEqual(this.tag(body, roc), srtp.subarray(srtp.length - AUTH_TAG_LEN))) return null;
        let payload = aesCm(this.encKey, this.iv(h.ssrc, index), body.subarray(h.headerLength));
        const hasPadding = (body[0] & 0x20) !== 0;
        if (hasPadding) {
            if (payload.length === 0) return null;
            const padLen = payload[payload.length - 1];
            if (padLen < 1 || padLen > payload.length) return null;
            payload = payload.subarray(0, payload.length - padLen);
        }
        if (!existing) {
            this.rxStreams.set(h.ssrc, { roc, lastSeq: h.seq, started: true, replayMask: 1n });
        } else {
            const maxIdx = (BigInt(existing.roc) << 16n) | BigInt(existing.lastSeq);
            if (index > maxIdx) {
                const shift = index - maxIdx;
                existing.replayMask = shift >= REPLAY_WINDOW ? 1n : ((existing.replayMask << shift) | 1n) & REPLAY_MASK_64;
                existing.roc = roc;
                existing.lastSeq = h.seq;
            } else {
                existing.replayMask |= 1n << (maxIdx - index);
            }
        }
        const out = Buffer.concat([body.subarray(0, h.headerLength), payload]);
        if (hasPadding) out[0] &= ~0x20;
        return out;
    }
}
