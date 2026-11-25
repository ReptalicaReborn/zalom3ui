import dgram from "node:dgram";
import crypto from "node:crypto";
import { SrtpContext, rtpHeader } from "./srtp.js";
import * as zrtc from "./zrtc.js";

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
};

export class MediaCall {
    readonly stats: MediaStats = {
        sent: {},
        received: {},
        audio: { packets: 0, decrypted: 0, failed: 0, payloadTypes: {}, ssrcs: 0, timestampSteps: [], opusToc: [], bytes: 0 },
        controls: [],
    };
    token: number | null = null;
    natAddress: string | null = null;
    private readonly socket: dgram.Socket;
    private readonly srtp: SrtpContext;
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

    constructor(private readonly o: MediaCallOptions) {
        this.srtp = SrtpContext.fromSession(o.session);
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
        return this.token;
    }

    /** Sends one 20 ms frame of silence every 20 ms, so the other side gets a live stream. */
    startSilence(): void {
        this.timers.push(setInterval(() => this.sendAudioFrame(OPUS_SILENCE), FRAME_MS));
    }

    /** Wraps one encoded Opus frame in RTP, protects it and sends it. */
    sendAudioFrame(opus: Buffer): void {
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
        this.send(zrtc.audio(this.token, this.srtp.protect(Buffer.concat([rtp, opus]))), "audio");
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
            if (p.kind === zrtc.Kind.AudioIn || p.kind === zrtc.Kind.AudioInToken) this.onAudio(p.payload);
            return;
        }
        this.count("received", p.type === "p2p" ? `p2p_${p.cmd}` : `kind_${p.kind}`);
        if (p.type === "unknown" && this.loggedUnknown++ < 5) this.o.log({ kind: "media_unknown", head: msg.subarray(0, 24).toString("hex"), bytes: msg.length });
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
    }
}
