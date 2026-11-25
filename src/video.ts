/**
 * Zalo's 1:1 video packetization (docs/video-calls.md), not RFC 6184: one H.264 access unit in Annex B form per frame,
 * payload type 98 for a key frame and 97 for the others, frames up to 1068 bytes in one RTP packet, larger ones cut
 * into 1068-byte chunks each prefixed `1c` + flag (`80` first, `40` last, `00` middle), the RTP marker bit on the
 * last packet, one timestamp per frame. The format and the assembler's behaviour (reorder window, drop until the
 * first key frame, SPS/PPS caching) follow the Zalos project's engine (BSD-3-Clause, © doandat943 and Việt Nam Linux
 * Family); this is our own code.
 */

export const VIDEO_KEY_PT = 98;
export const VIDEO_DELTA_PT = 97;
export const MAX_CHUNK = 1068;

const START_CODE = Buffer.from([0, 0, 0, 1]);
/** How many finished frames may wait for an earlier one, and how long, before the earlier one counts as lost. */
const REORDER_FRAMES = 3;
const REORDER_MS = 200;
/** Frames being assembled at once; the oldest is given up beyond this. */
const MAX_PENDING = 30;

export type VideoFrame = { key: boolean; timestamp: number; data: Buffer };
export type VideoPacket = { seq: number; timestamp: number; payloadType: number; marker: boolean; payload: Buffer };

/** Splits a frame into the payloads of its RTP packets. */
export function packetizeFrame(annexB: Buffer, maxChunk = MAX_CHUNK): Buffer[] {
    if (annexB.length <= maxChunk) return [annexB];
    const out: Buffer[] = [];
    for (let at = 0; at < annexB.length; at += maxChunk) {
        const end = Math.min(at + maxChunk, annexB.length);
        const flag = at === 0 ? 0x80 : end === annexB.length ? 0x40 : 0x00;
        out.push(Buffer.concat([Buffer.from([0x1c, flag]), annexB.subarray(at, end)]));
    }
    return out;
}

/** The NAL units (without start codes) of an Annex B buffer. */
export function nalUnits(annexB: Buffer): Buffer[] {
    const starts: number[] = [];
    for (let i = 0; i + 2 < annexB.length; i++) {
        if (annexB[i] === 0 && annexB[i + 1] === 0 && annexB[i + 2] === 1) {
            starts.push(i + 3);
            i += 2;
        }
    }
    const out: Buffer[] = [];
    starts.forEach((start, k) => {
        let end = k + 1 < starts.length ? starts[k + 1] - 3 : annexB.length;
        // Trailing zeros belong to the next start code (or are padding).
        while (end > start && annexB[end - 1] === 0) end--;
        if (end > start) out.push(annexB.subarray(start, end));
    });
    return out;
}

function isFragment(p: Buffer): boolean {
    return p.length >= 2 && p[0] === 0x1c && (p[1] & 0x3f) === 0;
}

type Pending = { parts: Map<number, Buffer>; key: boolean; end: number | null };
type Ready = { key: boolean; data: Buffer; timestamp: number; last: number; since: number };

export type AssemblerStats = { frames: number; keys: number; gaps: number; dropped: number };

export class VideoAssembler {
    readonly stats: AssemblerStats = { frames: 0, keys: 0, gaps: 0, dropped: 0 };
    private pending = new Map<number, Pending>();
    /** Complete frames waiting their turn, by the extended sequence number of their first packet. */
    private ready = new Map<number, Ready>();
    private lastIdx = -1;
    private maxIdx = -1;
    private started = false;
    private needKey = false;
    private sps: Buffer | null = null;
    private pps: Buffer | null = null;

    constructor(
        private readonly onFrame: (frame: VideoFrame) => void,
        private readonly now: () => number = Date.now,
        /** A frame was lost, or frames are being dropped until a key frame: ask for one. */
        private readonly onKeyNeeded: (why: "gap" | "no_key") => void = () => {},
    ) {}

    /** True until a key frame has been delivered (and again after a gap). */
    get waitingForKey(): boolean {
        return !this.started || this.needKey;
    }

    /** The 16-bit sequence number as a running index, picking the rollover count nearest the newest packet. */
    private extend(seq: number): number {
        if (this.maxIdx < 0) return seq;
        const cycle = Math.floor(this.maxIdx / 0x10000);
        let best = cycle * 0x10000 + seq;
        for (const c of [cycle - 1, cycle + 1]) {
            const cand = c * 0x10000 + seq;
            if (cand >= 0 && Math.abs(cand - this.maxIdx) < Math.abs(best - this.maxIdx)) best = cand;
        }
        return best;
    }

    push(pkt: VideoPacket): void {
        if ((pkt.payloadType !== VIDEO_KEY_PT && pkt.payloadType !== VIDEO_DELTA_PT) || pkt.payload.length === 0) return;
        const idx = this.extend(pkt.seq);
        this.maxIdx = Math.max(this.maxIdx, idx);
        if (idx <= this.lastIdx) return;
        let f = this.pending.get(pkt.timestamp);
        if (!f) {
            this.pending.set(pkt.timestamp, (f = { parts: new Map(), key: false, end: null }));
            if (this.pending.size > MAX_PENDING) {
                this.pending.delete(this.pending.keys().next().value as number);
                this.stats.dropped++;
            }
        }
        f.parts.set(idx, pkt.payload);
        if (pkt.payloadType === VIDEO_KEY_PT) f.key = true;
        if (pkt.marker) f.end = idx;
        if (f.end !== null) this.complete(pkt.timestamp, f);
        this.flush();
    }

    /** Lets the reorder window expire when no packet arrives to trigger it. */
    tick(): void {
        this.flush();
    }

    private complete(timestamp: number, f: Pending): void {
        const idxs = [...f.parts.keys()].sort((a, b) => a - b);
        const first = f.parts.get(idxs[0])!;
        if (isFragment(first) && first[1] !== 0x80) return;
        for (let i = 1; i < idxs.length; i++) if (idxs[i] !== idxs[i - 1] + 1) return;
        if (idxs[idxs.length - 1] !== f.end) return;
        let data = Buffer.concat(idxs.map((i) => (isFragment(f.parts.get(i)!) ? f.parts.get(i)!.subarray(2) : f.parts.get(i)!)));
        this.pending.delete(timestamp);
        const units = nalUnits(data);
        const types = new Set(units.map((u) => u[0] & 0x1f));
        for (const u of units) {
            const t = u[0] & 0x1f;
            if (t === 7) this.sps = u;
            else if (t === 8) this.pps = u;
        }
        const key = f.key || types.has(5) || types.has(7);
        // A key frame the decoder can start from carries its parameter sets; some senders only send them once.
        if (key && this.sps && (!types.has(7) || (!types.has(8) && this.pps))) {
            const pre: Buffer[] = [];
            if (!types.has(7)) pre.push(START_CODE, this.sps);
            if (!types.has(8) && this.pps) pre.push(START_CODE, this.pps);
            data = Buffer.concat([...pre, data]);
        }
        this.ready.set(idxs[0], { key, data, timestamp, last: f.end!, since: this.now() });
    }

    private flush(): void {
        for (;;) {
            if (this.ready.size === 0) return;
            const next = this.ready.get(this.lastIdx + 1);
            if (next && this.lastIdx >= 0) {
                this.emit(this.lastIdx + 1, next);
                continue;
            }
            const firstIdx = Math.min(...this.ready.keys());
            const oldest = this.ready.get(firstIdx)!;
            // Something before it may still arrive: wait a little.
            if (this.lastIdx >= 0 && this.ready.size <= REORDER_FRAMES && this.now() - oldest.since < REORDER_MS) return;
            if (!this.started && !oldest.key) {
                this.ready.delete(firstIdx);
                this.stats.dropped++;
                this.onKeyNeeded("no_key");
                continue;
            }
            if (this.lastIdx >= 0) {
                this.stats.gaps++;
                this.needKey = true;
                this.onKeyNeeded("gap");
            }
            this.emit(firstIdx, oldest);
        }
    }

    private emit(firstIdx: number, fr: Ready): void {
        this.ready.delete(firstIdx);
        this.lastIdx = fr.last;
        // Delta frames after a loss would show garbage: skip them until the key frame we asked for.
        if (this.needKey && !fr.key) {
            this.stats.dropped++;
            this.onKeyNeeded("no_key");
        } else {
            if (fr.key) this.needKey = false;
            this.started ||= fr.key;
            this.stats.frames++;
            if (fr.key) this.stats.keys++;
            this.onFrame({ key: fr.key, timestamp: fr.timestamp, data: fr.data });
        }
        for (const [ts, p] of this.pending) {
            if (p.parts.size > 0 && Math.max(...p.parts.keys()) <= this.lastIdx) {
                this.pending.delete(ts);
                this.stats.dropped++;
            }
        }
    }
}
