import dgram from "node:dgram";
import crypto from "node:crypto";
import { ReceiveStats, parseRtcp, pli, receiverReport, sdesCname, senderReport } from "./rtcp.js";
import { SrtpContext, rtpHeader } from "./srtp.js";
import * as zrtc from "./zrtc.js";
import { MAX_CHUNK, VIDEO_DELTA_PT, VIDEO_KEY_PT, VideoAssembler, nalUnits, packetizeFrame, type VideoFrame } from "./video.js";

/**
 * One call's connection to a Zalo media server (zrtc over UDP), as the person called: join the call (init call,
 * command 12), keep it alive (ping), receive the other side's audio and send ours (SRTP-protected Opus in RTP, wrapped
 * in zrtc audio packets carrying the token the server handed out), and hang up. Everything it sees is summed up in
 * [stats] for the call log: packet counts, RTP fields and whether audio decrypted, never the audio itself.
 */

export type MediaServer = { host: string; port: number };

/** "ip:port" (IPv4) or "[ipv6]:port" / "ipv6:port". */
export function parseServer(address: string): MediaServer | null {
    const m = /^\[?([0-9a-fA-F:.]+?)\]?:(\d{1,5})$/.exec(address.trim());
    if (!m) return null;
    const port = Number(m[2]);
    return port > 0 && port < 65536 ? { host: m[1], port } : null;
}

/** Opus payload type and RTP clock of the codec Zalỏ offers ("opus/16000/1", payload 112, 20 ms frames). */
export const OPUS_PT = 112;
export const OPUS_CLOCK = 16000;
export const FRAME_MS = 20;
/** Video in 1:1 calls: H.264 payload type 97 (98 for key frames), 90 kHz clock, no RED (src/video.ts). */
export const H264_PT = VIDEO_DELTA_PT;
export const VIDEO_CLOCK = 90_000;
/** Key-frame requests (PLI) go out at most this often. */
const PLI_MIN_GAP_MS = 500;
/** A key frame request from the other side reaches the app at most this often. */
const KEYREQ_MIN_GAP_MS = 250;
/** Without a key frame yet, ask again this often. */
const PLI_RETRY_MS = 1000;
/** How often the video assembler's reorder window is checked. */
const VIDEO_TICK_MS = 50;

/** A 20 ms Opus frame of silence (CELT, any decoder plays it as silence). */
export const OPUS_SILENCE = Buffer.from([0xf8, 0xff, 0xfe]);

export type MediaCallOptions = {
    server: MediaServer;
    userId: number;
    partnerId: number;
    callId: number;
    session: string;
    isCaller?: boolean;
    log: (entry: Record<string, unknown>) => void;
    /** For tests. */
    createSocket?: () => dgram.Socket;
    pingMs?: number;
    rtcpMs?: number;
    /** The other side's audio, decrypted: RTP sequence number, RTP timestamp and the Opus payload. */
    onAudio?: (seq: number, timestamp: number, opus: Buffer) => void;
    /** The other side's video: one complete H.264 access unit (Annex B, SPS/PPS in front of key frames), 90 kHz timestamp. */
    onVideo?: (frame: VideoFrame) => void;
    /** The other side asked us for a key frame (RTCP PLI or FIR about our video stream). */
    onKeyFrameRequest?: () => void;
};

export type MediaStats = {
    sent: Record<string, number>;
    received: Record<string, number>;
    audio: {
        packets: number;
        decrypted: number;
        failed: number;
        payloadTypes: Record<string, number>;
        ssrcs: number;
        firstSeq?: number;
        lastSeq?: number;
        /** RTP timestamp steps between consecutive packets (tells the clock rate), first few. */
        timestampSteps: number[];
        /** First byte of the first few decrypted payloads (the Opus TOC: mode, bandwidth, frame size). */
        opusToc: string[];
        bytes: number;
    };
    controls: { kind: number; cmd: number; sub: number; error?: number }[];
    /**
     * The other side's video: RTP packets and payload types, what the frame assembler made of them (frames, key
     * frames, gaps = lost frames, dropped), H.264 NAL unit types in the finished frames (1 slice, 5 IDR, 7 SPS, 8 PPS),
     * what we sent (frames, packets, payload bytes) and how many key-frame requests went out / came in.
     */
    video: {
        packets: number;
        decrypted: number;
        failed: number;
        ssrcs: number;
        payloadTypes: Record<string, number>;
        nalTypes: Record<string, number>;
        bytes: number;
        frames: number;
        keys: number;
        gaps: number;
        dropped: number;
        sentFrames: number;
        sentPackets: number;
        sentBytes: number;
        plisSent: number;
        keyRequests: number;
    };
    /** RTCP from the other side: packet types seen, and the last loss / jitter it reported about our audio. */
    rtcp: { packets: number; types: Record<string, number>; theirView?: { fractionLost: number; cumulativeLost: number; jitter: number } };
};

/** How often we send RTCP (sender + receiver report): WebRTC's audio interval is ~5 s; a little more often is fine. */
const RTCP_MS = 2000;

export class MediaCall {
    readonly stats: MediaStats = {
        sent: {},
        received: {},
        audio: { packets: 0, decrypted: 0, failed: 0, payloadTypes: {}, ssrcs: 0, timestampSteps: [], opusToc: [], bytes: 0 },
        controls: [],
        rtcp: { packets: 0, types: {} },
        video: {
            packets: 0, decrypted: 0, failed: 0, ssrcs: 0, payloadTypes: {}, nalTypes: {}, bytes: 0,
            frames: 0, keys: 0, gaps: 0, dropped: 0, sentFrames: 0, sentPackets: 0, sentBytes: 0, plisSent: 0, keyRequests: 0,
        },
    };
    private videoSsrcs = new Set<number>();
    /**
     * Our own video stream's SSRC: our 32-bit zrtc user id, as Zalo's clients use (the Zalos project's engine does the
     * same for audio and video). Its own SRTP context ([videoSrtp]) keeps it apart from the audio stream.
     */
    private readonly videoSsrc: number;
    private vSeq = crypto.randomInt(0, 0x8000);
    private readonly vTsBase = crypto.randomInt(0, 0x7fffffff);
    private readonly vClock0 = process.hrtime.bigint();
    private vSentPackets = 0;
    private vSentOctets = 0;
    private assembler: VideoAssembler;
    /** Counts of assemblers replaced when the other side's video SSRC changed. */
    private retiredFrames = { frames: 0, keys: 0, gaps: 0, dropped: 0 };
    /** The other side's video stream: its SSRC (for PLI) and report statistics. */
    private incomingVideo: ReceiveStats | null = null;
    private videoStartedAt = 0;
    private lastPliAt = 0;
    private lastKeyReqAt = 0;
    token: number | null = null;
    natAddress: string | null = null;
    private readonly socket: dgram.Socket;
    private readonly srtp: SrtpContext;
    /**
     * Video has its own SRTP context with the same key, like Zalo PC's call program (two SrtpTransports): the other side
     * may use one SSRC (its user id) for both streams, and one context would mix up their rollover counters.
     */
    private readonly videoSrtp: SrtpContext;
    private readonly ssrcs = new Set<number>();
    private lastTs: number | null = null;
    private joinWaiter: ((ok: boolean) => void) | null = null;
    private timers: NodeJS.Timeout[] = [];
    private closed = false;
    private seq = crypto.randomInt(0, 0x8000);
    private ts = crypto.randomInt(0, 0x7fffffff);
    private readonly ssrc = crypto.randomInt(1, 0x7fffffff);
    private firstAudio = true;
    private loggedUnknown = 0;
    /** When the app last sent a frame: silence fills in only while it doesn't. */
    private lastFrameAt = 0;
    /** The other side's audio stream, for our receiver reports. */
    private incoming: ReceiveStats | null = null;
    private sentPackets = 0;
    private sentOctets = 0;

    constructor(private readonly o: MediaCallOptions) {
        this.srtp = SrtpContext.fromSession(o.session);
        this.videoSrtp = SrtpContext.fromSession(o.session);
        this.videoSsrc = o.userId >>> 0 || crypto.randomInt(0x8000_0000, 0xffff_ffff);
        this.assembler = this.newAssembler();
        this.socket = o.createSocket ? o.createSocket() : dgram.createSocket(o.server.host.includes(":") ? "udp6" : "udp4");
        this.socket.on("message", (msg, from) => {
            if (from.address === o.server.host || from.address.endsWith(o.server.host)) this.onPacket(msg);
            else this.count("received", "other_source");
        });
        this.socket.on("error", (err) => o.log({ kind: "media_socket_error", error: err.message }));
    }

    private count(dir: "sent" | "received", what: string) {
        this.stats[dir][what] = (this.stats[dir][what] ?? 0) + 1;
    }

    private send(packet: Buffer, what: string) {
        if (this.closed) return;
        this.count("sent", what);
        this.socket.send(packet, this.o.server.port, this.o.server.host);
    }

    /** Joins the call on the media server; resolves with the token, or null when the server didn't accept in time. */
    async join(timeoutMs = 5000, retryMs = 400): Promise<number | null> {
        const init = zrtc.initCall({
            isCaller: this.o.isCaller ?? false,
            userId: this.o.userId,
            partnerId: this.o.partnerId,
            callId: this.o.callId,
            session: this.o.session,
            attempt: 1,
        });
        const ok = await new Promise<boolean>((resolve) => {
            this.joinWaiter = resolve;
            this.send(init, "init");
            const again = setInterval(() => this.send(init, "init"), retryMs);
            const giveUp = setTimeout(() => resolve(false), timeoutMs);
            this.timers.push(again, giveUp);
        });
        this.joinWaiter = null;
        this.clearTimers();
        if (!ok) return null;
        const ping = () => this.send(zrtc.ping({ userId: this.o.userId, serverId: this.token ?? 0, session: this.o.session }), "ping");
        this.timers.push(setInterval(ping, this.o.pingMs ?? 2000));
        this.timers.push(setInterval(() => this.sendRtcp(), this.o.rtcpMs ?? RTCP_MS));
        this.timers.push(setInterval(() => this.videoTick(), VIDEO_TICK_MS));
        return this.token;
    }

    /**
     * Keeps the stream alive with 20 ms frames of silence whenever no real audio is being sent (before the app's
     * microphone starts, or if it stalls), so the other side never sees the call go quiet.
     */
    startSilence(): void {
        this.timers.push(
            setInterval(() => {
                if (Date.now() - this.lastFrameAt > 3 * FRAME_MS) this.sendFrame(OPUS_SILENCE);
            }, FRAME_MS),
        );
    }

    /** One encoded 20 ms Opus frame from the app's microphone. */
    sendAudioFrame(opus: Buffer): void {
        this.lastFrameAt = Date.now();
        this.sendFrame(opus);
    }

    /** Wraps one encoded Opus frame in RTP, protects it and sends it. */
    private sendFrame(opus: Buffer): void {
        if (this.token === null) return;
        const rtp = Buffer.alloc(12);
        rtp[0] = 0x80;
        rtp[1] = OPUS_PT | (this.firstAudio ? 0x80 : 0);
        rtp.writeUInt16BE(this.seq, 2);
        rtp.writeUInt32BE(this.ts >>> 0, 4);
        rtp.writeUInt32BE(this.ssrc, 8);
        this.firstAudio = false;
        this.seq = (this.seq + 1) & 0xffff;
        this.ts = (this.ts + (OPUS_CLOCK * FRAME_MS) / 1000) >>> 0;
        this.sentPackets++;
        this.sentOctets += opus.length;
        this.send(zrtc.audio(this.token, this.srtp.protect(Buffer.concat([rtp, opus]))), "audio");
    }

    /** Sender report about our audio (receiver report before we've sent any), with a block about theirs, and a CNAME. */
    private sendRtcp(): void {
        if (this.token === null) return;
        const now = Date.now();
        const block = this.incoming?.block(now);
        const blocks = block ? [block] : [];
        const report =
            this.sentPackets > 0
                ? senderReport({ ssrc: this.ssrc, nowMs: now, rtpTimestamp: this.ts, packets: this.sentPackets, octets: this.sentOctets, blocks })
                : receiverReport({ ssrc: this.ssrc, blocks });
        this.send(zrtc.rtcp(this.token, Buffer.concat([report, sdesCname(this.ssrc, `zaloo-${this.ssrc.toString(16)}`)])), "rtcp");
        this.sendVideoRtcp(now);
    }

    /** Video sender report (receiver report before we've sent video) on kind 15; nothing while no video flows either way. */
    private sendVideoRtcp(now: number): void {
        if (this.token === null || (this.vSentPackets === 0 && !this.incomingVideo)) return;
        const vblock = this.incomingVideo?.block(now);
        const blocks = vblock ? [vblock] : [];
        const report =
            this.vSentPackets > 0
                ? senderReport({ ssrc: this.videoSsrc, nowMs: now, rtpTimestamp: this.videoTimestamp(), packets: this.vSentPackets, octets: this.vSentOctets, blocks })
                : receiverReport({ ssrc: this.videoSsrc, blocks });
        this.send(zrtc.rtcp(this.token, Buffer.concat([report, sdesCname(this.videoSsrc, `zaloo-${this.ssrc.toString(16)}`)]), true), "rtcp_video");
    }

    /** RTP timestamp for "now": a 90 kHz clock that only moves forward. */
    private videoTimestamp(): number {
        const ms = Number((process.hrtime.bigint() - this.vClock0) / 1_000_000n);
        return (this.vTsBase + ms * 90) >>> 0;
    }

    /**
     * One encoded video frame from the app's camera (H.264 access unit, Annex B, SPS/PPS in front of key frames):
     * chunked the way Zalo does, protected and sent as kind 13.
     */
    sendVideoFrame(frame: { key: boolean; data: Buffer }): void {
        if (this.token === null || this.closed || frame.data.length === 0) return;
        const chunks = packetizeFrame(frame.data, MAX_CHUNK);
        const ts = this.videoTimestamp();
        chunks.forEach((chunk, i) => {
            const rtp = Buffer.alloc(12);
            rtp[0] = 0x80;
            rtp[1] = (frame.key ? VIDEO_KEY_PT : VIDEO_DELTA_PT) | (i === chunks.length - 1 ? 0x80 : 0);
            rtp.writeUInt16BE(this.vSeq, 2);
            rtp.writeUInt32BE(ts, 4);
            rtp.writeUInt32BE(this.videoSsrc, 8);
            this.vSeq = (this.vSeq + 1) & 0xffff;
            this.vSentPackets++;
            this.vSentOctets += chunk.length;
            this.send(zrtc.video(this.token!, this.videoSrtp.protect(Buffer.concat([rtp, chunk]))), "video");
        });
        const v = this.stats.video;
        v.sentFrames++;
        v.sentPackets += chunks.length;
        v.sentBytes += frame.data.length;
    }

    /**
     * Asks the other side for a key frame (RTCP PLI on kind 15), at most once per [minGapMs]. Until their video has
     * been seen there is nobody to ask: the first packets (or the next tick) will.
     */
    requestKeyFrame(why: string, minGapMs = PLI_MIN_GAP_MS): void {
        const target = this.incomingVideo?.ssrc;
        const now = Date.now();
        if (this.token === null || target === undefined || now - this.lastPliAt < minGapMs) return;
        this.lastPliAt = now;
        this.stats.video.plisSent++;
        this.send(zrtc.rtcp(this.token, pli(this.videoSsrc, target), true), "pli");
        if (this.stats.video.plisSent <= 10) this.o.log({ kind: "media_pli", why });
    }

    private newAssembler(): VideoAssembler {
        return new VideoAssembler(
            (frame) => this.onVideoFrame(frame),
            Date.now,
            (why) => this.requestKeyFrame(why),
        );
    }

    private onVideoFrame(frame: VideoFrame): void {
        const v = this.stats.video;
        for (const u of nalUnits(frame.data)) v.nalTypes[u[0] & 0x1f] = (v.nalTypes[u[0] & 0x1f] ?? 0) + 1;
        this.syncAssemblerStats();
        this.o.onVideo?.(frame);
    }

    private syncAssemblerStats(): void {
        const a = this.assembler.stats;
        const r = this.retiredFrames;
        const v = this.stats.video;
        v.frames = a.frames + r.frames;
        v.keys = a.keys + r.keys;
        v.gaps = a.gaps + r.gaps;
        v.dropped = a.dropped + r.dropped;
    }

    /** Lets the assembler's reorder window expire, and keeps asking for a key frame while none has come. */
    private videoTick(): void {
        if (!this.incomingVideo) return;
        this.assembler.tick();
        this.syncAssemblerStats();
        if (this.assembler.waitingForKey && Date.now() - this.videoStartedAt >= 300) this.requestKeyFrame("waiting", PLI_RETRY_MS);
    }

    private onRtcp(payload: Buffer, video = false): void {
        const r = this.stats.rtcp;
        r.packets++;
        for (const p of parseRtcp(payload)) {
            if (p.type === "PLI" || p.type === "FIR") {
                // Only requests about our video stream count.
                r.types[p.type] = (r.types[p.type] ?? 0) + 1;
                if (p.mediaSsrc === this.videoSsrc) this.onKeyRequest(p.type);
                continue;
            }
            if (video) {
                const name = p.type === "other" ? String(p.packetType) : p.type;
                r.types[name] = (r.types[name] ?? 0) + 1;
                if (p.type === "SR" && this.incomingVideo && p.ssrc === this.incomingVideo.ssrc) {
                    this.incomingVideo.onSenderReport(p.ntpSec, p.ntpFrac, Date.now());
                }
                continue;
            }
            const name = p.type === "other" ? String(p.packetType) : p.type;
            r.types[name] = (r.types[name] ?? 0) + 1;
            if (p.type === "SR" && this.incoming && p.ssrc === this.incoming.ssrc) this.incoming.onSenderReport(p.ntpSec, p.ntpFrac, Date.now());
            if (p.type === "SR" || p.type === "RR") {
                const about = p.blocks.find((b) => b.ssrc === this.ssrc);
                if (about) r.theirView = { fractionLost: about.fractionLost, cumulativeLost: about.cumulativeLost, jitter: about.jitter };
            }
        }
    }

    private onKeyRequest(kind: string): void {
        const now = Date.now();
        this.stats.video.keyRequests++;
        if (now - this.lastKeyReqAt < KEYREQ_MIN_GAP_MS) return;
        this.lastKeyReqAt = now;
        this.o.log({ kind: "media_key_request", by: kind });
        this.o.onKeyFrameRequest?.();
    }

    /** Hangs up on the media server and closes the socket. */
    close(sendEndCall = true): void {
        if (this.closed) return;
        if (sendEndCall && this.token !== null) this.send(zrtc.endCall({ userId: this.o.userId, callId: this.o.callId, serverId: this.token }), "endcall");
        this.closed = true;
        this.clearTimers();
        this.joinWaiter?.(false);
        setTimeout(() => this.socket.close(), 50).unref();
    }

    private clearTimers() {
        for (const t of this.timers) clearInterval(t);
        this.timers = [];
    }

    private onPacket(msg: Buffer) {
        let p: zrtc.Packet;
        try {
            p = zrtc.parse(msg);
        } catch (err) {
            this.count("received", "unparsable");
            if (this.loggedUnknown++ < 5) this.o.log({ kind: "media_unparsable", bytes: msg.length, head: msg.subarray(0, 24).toString("hex"), error: String(err) });
            return;
        }
        if (p.type === "control") {
            this.count("received", `control_${p.kind}_${p.cmd}`);
            const summary = { kind: p.kind, cmd: p.cmd, sub: p.sub, ...(p.error !== undefined ? { error: p.error } : {}) };
            if (this.stats.controls.length < 40) this.stats.controls.push(summary);
            if (p.kind === zrtc.Kind.Response && (p.cmd === zrtc.Cmd.InitCallCallee || p.cmd === zrtc.Cmd.InitCallCaller)) {
                this.o.log({ kind: "media_init_answer", error: p.error, value38: p.value38, value18: p.value18, address: p.text });
                if (p.error === 0 && this.token === null) {
                    this.token = p.value18 ?? null;
                    this.natAddress = p.text ?? null;
                    this.joinWaiter?.(true);
                } else if (p.error !== 0) {
                    this.joinWaiter?.(false);
                }
            } else {
                this.o.log({ kind: "media_control", packetKind: p.kind, cmd: p.cmd, sub: p.sub, error: p.error, ...(p.text ? { textBytes: p.text.length } : {}) });
            }
            return;
        }
        if (p.type === "media") {
            this.count("received", `media_${p.kind}`);
            if (p.kind === zrtc.Kind.AudioIn) this.onAudio(p.payload);
            else if (p.kind === zrtc.Kind.AudioRtcp) this.onRtcp(p.payload);
            else if (p.kind === zrtc.Kind.VideoRtcp) this.onRtcp(p.payload, true);
            else if (p.kind === zrtc.Kind.VideoIn) this.onVideo(p.payload);
            return;
        }
        this.count("received", p.type === "p2p" ? `p2p_${p.cmd}` : `kind_${p.kind}`);
        if (p.type === "unknown" && this.loggedUnknown++ < 5) this.o.log({ kind: "media_unknown", head: msg.subarray(0, 24).toString("hex"), bytes: msg.length });
    }

    private onVideo(payload: Buffer) {
        const v = this.stats.video;
        v.packets++;
        v.bytes += payload.length;
        const h = rtpHeader(payload);
        if (!h) {
            v.failed++;
            return;
        }
        v.payloadTypes[h.payloadType] = (v.payloadTypes[h.payloadType] ?? 0) + 1;
        this.videoSsrcs.add(h.ssrc);
        v.ssrcs = this.videoSsrcs.size;
        const rtp = this.videoSrtp.unprotect(payload);
        if (!rtp) {
            v.failed++;
            return;
        }
        v.decrypted++;
        if (!this.incomingVideo || this.incomingVideo.ssrc !== h.ssrc) {
            // A new stream (first packet, or the other side restarted its encoder): assemble afresh.
            if (this.incomingVideo) {
                this.syncAssemblerStats();
                const a = this.assembler.stats;
                for (const k of ["frames", "keys", "gaps", "dropped"] as const) this.retiredFrames[k] += a[k];
                this.assembler = this.newAssembler();
            }
            this.incomingVideo = new ReceiveStats(h.ssrc, VIDEO_CLOCK);
            this.videoStartedAt = Date.now();
        }
        this.incomingVideo.onPacket(h.seq, h.timestamp, Date.now());
        this.assembler.push({ seq: h.seq, timestamp: h.timestamp, payloadType: h.payloadType, marker: h.marker, payload: rtp.subarray(h.headerLength) });
        this.syncAssemblerStats();
    }

    private onAudio(payload: Buffer) {
        const a = this.stats.audio;
        a.packets++;
        a.bytes += payload.length;
        const h = rtpHeader(payload);
        if (!h) {
            a.failed++;
            if (this.loggedUnknown++ < 5) this.o.log({ kind: "media_not_rtp", head: payload.subarray(0, 16).toString("hex"), bytes: payload.length });
            return;
        }
        a.payloadTypes[h.payloadType] = (a.payloadTypes[h.payloadType] ?? 0) + 1;
        this.ssrcs.add(h.ssrc);
        if (!this.incoming || this.incoming.ssrc !== h.ssrc) this.incoming = new ReceiveStats(h.ssrc, OPUS_CLOCK);
        this.incoming.onPacket(h.seq, h.timestamp, Date.now());
        a.ssrcs = this.ssrcs.size;
        a.firstSeq ??= h.seq;
        a.lastSeq = h.seq;
        if (this.lastTs !== null && a.timestampSteps.length < 8) a.timestampSteps.push((h.timestamp - this.lastTs) >>> 0);
        this.lastTs = h.timestamp;
        const rtp = this.srtp.unprotect(payload);
        if (!rtp) {
            a.failed++;
            return;
        }
        a.decrypted++;
        if (a.opusToc.length < 5 && rtp.length > h.headerLength) a.opusToc.push(rtp[h.headerLength].toString(16).padStart(2, "0"));
        if (rtp.length > h.headerLength) this.o.onAudio?.(h.seq, h.timestamp, rtp.subarray(h.headerLength));
    }
}
