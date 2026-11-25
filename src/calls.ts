import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";

/**
 * Zalo call events. Calls are signalled with "control" frames on the realtime WebSocket (cmd 601, `act_type: "voip"`:
 * `request`, `answer`, `cancel`, `endcall`, `group_request`…), which zca-js reads but drops. This module decodes them
 * from the raw frames, so Zalỏ can say who is calling and record the events for the call research (docs/calls.md).
 */

export type CallControl = {
    /** "request" (incoming call), "answer", "cancel", "endcall", "group_request"… */
    act: string;
    /** The control's `data`, parsed when it is JSON text. */
    data: Record<string, unknown>;
    /** The whole control as Zalo sent it, for the call log. */
    raw: unknown;
};

/** Incoming call, as far as the control says. */
export type IncomingCall = {
    callId: string;
    callerId: string;
    video: boolean;
    group: boolean;
    groupId?: string;
    ts: number;
    /** The caller's display name as the request carries it (`params.Dname`), for callers who aren't contacts. */
    displayName?: string;
};

/** JSON text parsed (keeping long ids), an object passed through, anything else null. */
export function jsonObject(value: unknown): Record<string, unknown> | null {
    if (value && typeof value === "object" && !Array.isArray(value)) return value as Record<string, unknown>;
    if (typeof value !== "string" || !value.trim().startsWith("{")) return null;
    try {
        return parseJsonKeepingBigInts(value) as Record<string, unknown>;
    } catch {
        return null;
    }
}

/** First 4 bytes of a frame: version, command (little-endian u16), sub-command (same layout zca-js reads). */
export function frameHeader(frame: Buffer): { version: number; cmd: number; subCmd: number } | null {
    if (frame.length < 4) return null;
    return { version: frame[0], cmd: frame.readUInt16LE(1), subCmd: frame[3] };
}

/**
 * JSON.parse that keeps integers too long for a JS number (Zalo's ids) as text. Only numbers outside strings are
 * touched: Zalo nests JSON text inside strings, and that text is parsed on its own later.
 */
export function parseJsonKeepingBigInts(text: string): unknown {
    let out = "";
    let inString = false;
    for (let i = 0; i < text.length; i++) {
        const c = text[i];
        if (inString) {
            out += c;
            if (c === "\\") {
                out += text[i + 1] ?? "";
                i++;
            } else if (c === '"') {
                inString = false;
            }
            continue;
        }
        if (c === '"') {
            inString = true;
            out += c;
            continue;
        }
        if (c === "-" || (c >= "0" && c <= "9")) {
            let j = i + 1;
            while (j < text.length && /[0-9.eE+-]/.test(text[j])) j++;
            const num = text.slice(i, j);
            out += /^-?\d{16,}$/.test(num) ? `"${num}"` : num;
            i = j - 1;
            continue;
        }
        out += c;
    }
    return JSON.parse(out);
}

/**
 * Decodes an event payload `{ data, encrypt }` like zca-js's `decodeEventData`: 0 plain JSON, 1 base64 + zlib,
 * 2 URL-encoded base64 of AES-GCM (16-byte IV, 16-byte additional data, tag at the end) + zlib, 3 the same without zlib.
 */
export function decodeEventPayload(parsed: { data?: unknown; encrypt?: unknown }, cipherKey: string | undefined): unknown {
    if (typeof parsed.data !== "string" || typeof parsed.encrypt !== "number") return null;
    const type = parsed.encrypt;
    if (type === 0) return parseJsonKeepingBigInts(parsed.data);
    const buf = Buffer.from(type === 1 ? parsed.data : decodeURIComponent(parsed.data), "base64");
    let plain = buf;
    if (type !== 1) {
        if (!cipherKey || buf.length < 48) return null;
        const key = Buffer.from(cipherKey, "base64");
        const algorithm = key.length === 32 ? "aes-256-gcm" : key.length === 24 ? "aes-192-gcm" : "aes-128-gcm";
        const decipher = crypto.createDecipheriv(algorithm, key, buf.subarray(0, 16));
        decipher.setAAD(buf.subarray(16, 32));
        decipher.setAuthTag(buf.subarray(buf.length - 16));
        plain = Buffer.concat([decipher.update(buf.subarray(32, buf.length - 16)), decipher.final()]);
    }
    const text = (type === 3 ? plain : inflateAny(plain)).toString("utf8");
    return text ? parseJsonKeepingBigInts(text) : null;
}

/**
 * Decompresses like pako.inflate, which zca-js uses: it detects a zlib or a gzip header (call controls come gzipped);
 * raw deflate as a last resort. A failure names the first bytes, so the call log shows what arrived.
 */
export function inflateAny(buf: Buffer): Buffer {
    try {
        return zlib.unzipSync(buf);
    } catch (first) {
        try {
            return zlib.inflateRawSync(buf);
        } catch {
            const head = buf.subarray(0, 8).toString("hex").replace(/(..)/g, "$1 ").trim();
            throw new Error(`${first instanceof Error ? first.message : String(first)} (${buf.length} bytes, starting ${head})`);
        }
    }
}

/** Frame commands zca-js handles (zca-js 2.2.0 `listen.js`); anything else is worth noting in the call log. */
export const KNOWN_FRAME_CMDS = new Set([1, 2, 3000, 501, 502, 510, 511, 521, 522, 601, 602, 610, 611, 612]);

/** What a frame says, for the call log: never message content, only types and field names. */
export type FrameInfo = {
    /** `voip` controls (calls), in full. */
    calls: CallControl[];
    /** "act_type/act" of every control in a control frame (cmd 601). */
    controlTypes: string[];
    /** A frame zca-js doesn't handle: its command and the names of its fields. */
    unknown?: { cmd: number; subCmd: number; bytes: number; keys: string[] };
};

/** Field names of a decoded payload, one level deep ("data.controls", "data.msgs"…), without values. */
function shapeOf(value: unknown): string[] {
    if (!value || typeof value !== "object") return [];
    const keys: string[] = [];
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
        keys.push(k);
        if (v && typeof v === "object" && !Array.isArray(v)) for (const inner of Object.keys(v)) keys.push(`${k}.${inner}`);
    }
    return keys.slice(0, 40);
}

/** Reads a raw frame for the call log and the call announcements. */
export function inspectFrame(frame: Buffer, cipherKey: string | undefined): FrameInfo | null {
    const header = frameHeader(frame);
    if (!header || header.version !== 1) return null;
    const known = KNOWN_FRAME_CMDS.has(header.cmd);
    if (known && header.cmd !== 601) return null;
    const text = frame.subarray(4).toString("utf8");
    if (!text) return null;
    let parsed: unknown;
    try {
        parsed = JSON.parse(text);
    } catch {
        return known ? null : { calls: [], controlTypes: [], unknown: { ...header, bytes: frame.length, keys: [] } };
    }
    let decoded: unknown = null;
    try {
        decoded = decodeEventPayload(parsed as { data?: unknown; encrypt?: unknown }, cipherKey);
    } catch {
        // Not an event payload (or not ours to read): keep the outer shape only.
    }
    if (!known) {
        return { calls: [], controlTypes: [], unknown: { ...header, bytes: frame.length, keys: shapeOf(decoded ?? parsed) } };
    }
    if (header.subCmd !== 0) return null;
    return { calls: controlsOf(decoded), controlTypes: controlTypesOf(decoded) };
}

function controlList(decoded: unknown): unknown[] {
    const d = decoded as { data?: { controls?: unknown[] }; controls?: unknown[] } | null;
    return d?.data?.controls ?? d?.controls ?? [];
}

function controlTypesOf(decoded: unknown): string[] {
    return controlList(decoded).map((control) => {
        const content = (control as { content?: Record<string, unknown> })?.content ?? {};
        return `${String(content.act_type ?? "?")}/${String(content.act ?? "?")}`;
    });
}

/** The `voip` controls of a raw frame, if it is a control frame (cmd 601). */
export function callControlsOf(frame: Buffer, cipherKey: string | undefined): CallControl[] {
    const header = frameHeader(frame);
    if (!header || header.version !== 1 || header.cmd !== 601 || header.subCmd !== 0) return [];
    const text = frame.subarray(4).toString("utf8");
    if (!text) return [];
    return controlsOf(decodeEventPayload(JSON.parse(text), cipherKey));
}

function controlsOf(decoded: unknown): CallControl[] {
    return callControlsIn(controlList(decoded));
}

/** The `voip` controls of a list of controls (`{content: {act_type, act, data}}`), from the socket or the control poll. */
export function callControlsIn(controls: unknown[]): CallControl[] {
    const out: CallControl[] = [];
    for (const control of controls) {
        const content = (control as { content?: Record<string, unknown> })?.content;
        if (!content || content.act_type !== "voip") continue;
        let data = content.data;
        if (typeof data === "string") {
            try {
                data = parseJsonKeepingBigInts(data);
            } catch {
                data = { text: data };
            }
        }
        out.push({ act: String(content.act ?? ""), data: (data ?? {}) as Record<string, unknown>, raw: control });
    }
    return out;
}

/** Reads an incoming call from a `request` / `group_request` control. Field names vary, so several are tried. */
export function incomingCallOf(control: CallControl, now = Date.now()): IncomingCall | null {
    if (control.act !== "request" && control.act !== "group_request") return null;
    const d = control.data;
    const text = (...keys: string[]) => {
        for (const k of keys) {
            const v = d[k];
            if ((typeof v === "string" && v) || typeof v === "number") return String(v);
        }
        return "";
    };
    const callerId = text("callerId", "uidN", "fromUid", "uidFrom", "uid", "hostCall", "srcId");
    if (!callerId) return null;
    const raw = Number(text("ts", "time", "timestamp"));
    // Seconds or milliseconds.
    const ts = raw ? (raw < 1e12 ? raw * 1000 : raw) : now;
    const type = Number(text("callType", "type", "calltype"));
    // A 1:1 request carries its details in `params`, and the call type (0 voice, 1 video) in `params.extendData`.
    const params = jsonObject(d.params);
    const extend = jsonObject(params?.extendData);
    const displayName = typeof params?.Dname === "string" && params.Dname ? params.Dname : undefined;
    return {
        callId: text("callId", "call_id"),
        callerId,
        video: type === 1 || Number(extend?.callType) === 1 || text("isVideo") === "1" || d.isVideo === true,
        group: control.act === "group_request",
        groupId: text("groupId", "gid") || undefined,
        ts,
        ...(displayName ? { displayName } : {}),
    };
}

/** A call request older than this is history (a replayed control), not a ringing phone. */
export const RINGING_MAX_AGE_MS = 2 * 60_000;

/** Acts after which a ringing call is over (answered elsewhere, cancelled, ended). */
export const CALL_ENDING_ACTS = new Set(["answer", "cancel", "endcall", "group_answer", "group_cancel", "group_endcall", "group_finish"]);

const TRACE_MAX_BYTES = 2 * 1024 * 1024;

/**
 * The call log: one JSON line per call event, kept in the data folder so it can be shared for the call research.
 * It holds call ids, user ids and Zalo server addresses, never message text. Rotates at 2 MB.
 */
export class CallTrace {
    private readonly file: string;

    constructor(dataDir: string) {
        this.file = path.join(dataDir, "call-trace.jsonl");
    }

    record(entry: Record<string, unknown>): void {
        try {
            fs.mkdirSync(path.dirname(this.file), { recursive: true });
            if (fs.existsSync(this.file) && fs.statSync(this.file).size > TRACE_MAX_BYTES) {
                fs.renameSync(this.file, this.file + ".1");
            }
            fs.appendFileSync(this.file, JSON.stringify({ at: new Date().toISOString(), ...entry }) + "\n", { mode: 0o600 });
        } catch (err) {
            console.warn("Call log:", err);
        }
    }

    read(): string {
        const parts = [this.file + ".1", this.file].filter((f) => fs.existsSync(f)).map((f) => fs.readFileSync(f, "utf8"));
        return parts.join("");
    }

    clear(): void {
        for (const f of [this.file, this.file + ".1"]) fs.rmSync(f, { force: true });
    }
}
