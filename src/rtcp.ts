/**
 * RTCP (RFC 3550) for a call's audio: sender and receiver reports so the other side's engine (WebRTC inside zrtc) sees
 * a healthy stream, and the reports it sends us. Zalo calls don't encrypt RTCP (`srtcp: 0`); zrtc wraps it in
 * kind 5 (audio) or 15 (video) packets with the stream token.
 */

export const RtcpType = { SR: 200, RR: 201, SDES: 202, BYE: 203, APP: 204, RTPFB: 205, PSFB: 206 } as const;

/** Seconds between 1900 (NTP) and 1970 (Unix). */
const NTP_OFFSET = 2_208_988_800;

/** NTP timestamp of [ms] (Unix milliseconds): seconds and fraction. */
export function ntp(ms: number): { sec: number; frac: number } {
    const sec = Math.floor(ms / 1000) + NTP_OFFSET;
    const frac = Math.floor(((ms % 1000) / 1000) * 2 ** 32) >>> 0;
    return { sec: sec >>> 0, frac };
}

/** The middle 32 bits of an NTP timestamp, as reports quote it (LSR). */
export function ntpMiddle(sec: number, frac: number): number {
    return (((sec & 0xffff) << 16) | (frac >>> 16)) >>> 0;
}

export type ReportBlock = {
    ssrc: number;
    fractionLost: number;
    cumulativeLost: number;
    highestSeq: number;
    jitter: number;
    lsr: number;
    dlsr: number;
};

function header(count: number, type: number, words: number): Buffer {
    const h = Buffer.alloc(4);
    h[0] = 0x80 | (count & 0x1f);
    h[1] = type;
    h.writeUInt16BE(words - 1, 2);
    return h;
}

function block(b: ReportBlock): Buffer {
    const out = Buffer.alloc(24);
    out.writeUInt32BE(b.ssrc >>> 0, 0);
    out[4] = b.fractionLost & 0xff;
    const lost = Math.max(-0x800000, Math.min(0x7fffff, b.cumulativeLost));
    out.writeIntBE(lost, 5, 3);
    out.writeUInt32BE(b.highestSeq >>> 0, 8);
    out.writeUInt32BE(b.jitter >>> 0, 12);
    out.writeUInt32BE(b.lsr >>> 0, 16);
    out.writeUInt32BE(b.dlsr >>> 0, 20);
    return out;
}

/** Sender report (with optional report blocks about what we receive). */
export function senderReport(p: { ssrc: number; nowMs: number; rtpTimestamp: number; packets: number; octets: number; blocks?: ReportBlock[] }): Buffer {
    const blocks = p.blocks ?? [];
    const body = Buffer.alloc(24);
    body.writeUInt32BE(p.ssrc >>> 0, 0);
    const t = ntp(p.nowMs);
    body.writeUInt32BE(t.sec, 4);
    body.writeUInt32BE(t.frac, 8);
    body.writeUInt32BE(p.rtpTimestamp >>> 0, 12);
    body.writeUInt32BE(p.packets >>> 0, 16);
    body.writeUInt32BE(p.octets >>> 0, 20);
    return Buffer.concat([header(blocks.length, RtcpType.SR, 1 + 6 + 6 * blocks.length), body, ...blocks.map(block)]);
}

/** Receiver report (when we haven't sent anything yet). */
export function receiverReport(p: { ssrc: number; blocks: ReportBlock[] }): Buffer {
    const s = Buffer.alloc(4);
    s.writeUInt32BE(p.ssrc >>> 0);
    return Buffer.concat([header(p.blocks.length, RtcpType.RR, 2 + 6 * p.blocks.length), s, ...p.blocks.map(block)]);
}

/** SDES with a CNAME, which a compound RTCP packet should carry. */
export function sdesCname(ssrc: number, cname: string): Buffer {
    const name = Buffer.from(cname, "utf8").subarray(0, 255);
    const items = Buffer.concat([Buffer.from([1, name.length]), name, Buffer.from([0])]);
    const padded = Buffer.concat([items, Buffer.alloc((4 - ((4 + items.length) % 4)) % 4)]);
    const s = Buffer.alloc(4);
    s.writeUInt32BE(ssrc >>> 0);
    return Buffer.concat([header(1, RtcpType.SDES, 1 + (4 + padded.length) / 4), s, padded]);
}

/** Picture loss indication (RFC 4585, PSFB format 1): [from] asks the sender of [media] for a key frame. */
export function pli(from: number, media: number): Buffer {
    const b = Buffer.alloc(12);
    b[0] = 0x81;
    b[1] = RtcpType.PSFB;
    b.writeUInt16BE(2, 2);
    b.writeUInt32BE(from >>> 0, 4);
    b.writeUInt32BE(media >>> 0, 8);
    return b;
}

/** Full intra request (RFC 5104, PSFB format 4): same ask as a PLI, with a request counter. */
export function fir(from: number, media: number, seqNo: number): Buffer {
    const b = Buffer.alloc(20);
    b[0] = 0x84;
    b[1] = RtcpType.PSFB;
    b.writeUInt16BE(4, 2);
    b.writeUInt32BE(from >>> 0, 4);
    // Media SSRC of the header is 0 for FIR; the target is in the FCI entry.
    b.writeUInt32BE(media >>> 0, 12);
    b[16] = seqNo & 0xff;
    return b;
}

export type RtcpPacket =
    | { type: "SR"; ssrc: number; ntpSec: number; ntpFrac: number; rtpTimestamp: number; packets: number; octets: number; blocks: ReportBlock[] }
    | { type: "RR"; ssrc: number; blocks: ReportBlock[] }
    | { type: "PLI" | "FIR"; ssrc: number; mediaSsrc: number }
    | { type: "other"; packetType: number; ssrc: number | null; bytes: number };

function readBlocks(buf: Buffer, at: number, count: number): ReportBlock[] {
    const out: ReportBlock[] = [];
    for (let i = 0; i < count && at + 24 <= buf.length; i++, at += 24) {
        out.push({
            ssrc: buf.readUInt32BE(at),
            fractionLost: buf[at + 4],
            cumulativeLost: buf.readIntBE(at + 5, 3),
            highestSeq: buf.readUInt32BE(at + 8),
            jitter: buf.readUInt32BE(at + 12),
            lsr: buf.readUInt32BE(at + 16),
            dlsr: buf.readUInt32BE(at + 20),
        });
    }
    return out;
}

/** Splits a compound RTCP packet. Stops at the first malformed part. */
export function parseRtcp(buf: Buffer): RtcpPacket[] {
    const out: RtcpPacket[] = [];
    let at = 0;
    while (at + 4 <= buf.length) {
        if (buf[at] >> 6 !== 2) break;
        const padded = (buf[at] & 0x20) !== 0;
        const count = buf[at] & 0x1f;
        const type = buf[at + 1];
        const len = (buf.readUInt16BE(at + 2) + 1) * 4;
        if (at + len > buf.length) break;
        const p = buf.subarray(at, at + len);
        let body = p;
        if (padded) {
            const padLen = p[len - 1];
            if (padLen < 1 || padLen > len - 4) break;
            body = p.subarray(0, len - padLen);
        }
        const bodyLen = body.length;
        if (type === RtcpType.SR && bodyLen >= 28) {
            out.push({
                type: "SR",
                ssrc: body.readUInt32BE(4),
                ntpSec: body.readUInt32BE(8),
                ntpFrac: body.readUInt32BE(12),
                rtpTimestamp: body.readUInt32BE(16),
                packets: body.readUInt32BE(20),
                octets: body.readUInt32BE(24),
                blocks: readBlocks(body, 28, count),
            });
        } else if (type === RtcpType.RR && bodyLen >= 8) {
            out.push({ type: "RR", ssrc: body.readUInt32BE(4), blocks: readBlocks(body, 8, count) });
        } else if (type === RtcpType.PSFB && count === 1 && bodyLen >= 12) {
            out.push({ type: "PLI", ssrc: body.readUInt32BE(4), mediaSsrc: body.readUInt32BE(8) });
        } else if (type === RtcpType.PSFB && count === 4 && bodyLen >= 20) {
            const senderSsrc = body.readUInt32BE(4);
            for (let off = 12; off + 8 <= bodyLen; off += 8) {
                out.push({ type: "FIR", ssrc: senderSsrc, mediaSsrc: body.readUInt32BE(off) });
            }
        } else {
            out.push({ type: "other", packetType: type, ssrc: bodyLen >= 8 ? body.readUInt32BE(4) : null, bytes: len });
        }
        at += len;
    }
    return out;
}

/** What we know about one incoming RTP stream, for its report block (RFC 3550 appendix A.3 and A.8). */
export class ReceiveStats {
    private baseSeq = -1;
    private maxSeq = 0;
    private cycles = 0;
    private received = 0;
    private expectedPrior = 0;
    private receivedPrior = 0;
    private jitter = 0;
    private lastTransit: number | null = null;
    private lastSr: { middle: number; at: number } | null = null;

    constructor(readonly ssrc: number, private readonly clockRate: number) {}

    /** One packet arrived at [arrivalMs]. */
    onPacket(seq: number, timestamp: number, arrivalMs: number): void {
        if (this.baseSeq < 0) {
            this.baseSeq = seq;
            this.maxSeq = seq;
        } else {
            const delta = (seq - this.maxSeq) & 0xffff;
            if (delta > 0 && delta < 0x8000) {
                if (seq < this.maxSeq) this.cycles += 0x10000;
                this.maxSeq = seq;
            }
        }
        this.received++;
        const arrival = (arrivalMs / 1000) * this.clockRate;
        const transit = arrival - timestamp;
        if (this.lastTransit !== null) {
            let d = Math.abs(transit - this.lastTransit);
            if (d > 2 ** 31) d = Math.abs(d - 2 ** 32);
            this.jitter += (d - this.jitter) / 16;
        }
        this.lastTransit = transit;
    }

    /** A sender report about this stream arrived: remembered for LSR / DLSR. */
    onSenderReport(ntpSec: number, ntpFrac: number, arrivalMs: number): void {
        this.lastSr = { middle: ntpMiddle(ntpSec, ntpFrac), at: arrivalMs };
    }

    /** The report block to send now; null before the first packet. */
    block(nowMs: number): ReportBlock | null {
        if (this.baseSeq < 0) return null;
        const extendedMax = this.cycles + this.maxSeq;
        const expected = extendedMax - this.baseSeq + 1;
        const lost = expected - this.received;
        const expectedInterval = expected - this.expectedPrior;
        const receivedInterval = this.received - this.receivedPrior;
        this.expectedPrior = expected;
        this.receivedPrior = this.received;
        const lostInterval = expectedInterval - receivedInterval;
        const fraction =
            expectedInterval <= 0 || lostInterval <= 0
                ? 0
                : Math.min(255, Math.max(0, Math.floor((lostInterval * 256) / expectedInterval)));
        const delayMs = this.lastSr ? Math.max(0, nowMs - this.lastSr.at) : 0;
        return {
            ssrc: this.ssrc,
            fractionLost: fraction,
            cumulativeLost: lost,
            highestSeq: extendedMax >>> 0,
            jitter: Math.floor(this.jitter) >>> 0,
            lsr: this.lastSr?.middle ?? 0,
            // Delay since that SR, in 1/65536 s.
            dlsr: this.lastSr ? Math.min(0xffffffff, Math.floor((delayMs / 1000) * 65536)) >>> 0 : 0,
        };
    }
}
