import crypto from "node:crypto";
import { parseJsonKeepingBigInts } from "./calls.js";

/**
 * Zalo's control channel, polled the way Zalo PC does when its control socket is off. Zalo PC keeps two realtime
 * channels: chat and control. zca-js connects one WebSocket, which carries messages and some controls (friend and group
 * events, uploads) but no call events: in a test call between two accounts that socket received nothing. Zalo PC reads
 * controls (and with them incoming calls, `act_type: "voip"`) from the control servers of the login answer
 * (`zpw_ctrs_new`), by long polling: `GET <server>/?zpw_ver=…&zpw_type=…&params=<cursor>&ts=<unix>`, answered with
 * `{error_code, data: AES(json)}` (see docs/calls.md).
 */

/** The poll cursor: `lastActionId,lastActionIdOther,` + URL-encoded base64 of the per-queue ids + `,1` (Zalo PC's format). */
export function pollCursor(lastActionId: string, lastActionIdOther: string, queueIds: Record<string, unknown> = {}): string {
    const ids = Buffer.from(JSON.stringify(queueIds)).toString("base64");
    return `${lastActionId},${lastActionIdOther},` + encodeURIComponent(`${ids},1`);
}

/** zca-js's `decodeAES`: URL-decoded base64, AES-CBC with the session key (`zpw_enk`, base64), zero IV, PKCS#7. */
export function decodeAesCbc(secretKey: string, data: string): string | null {
    try {
        const key = Buffer.from(secretKey, "base64");
        const algorithm = key.length === 32 ? "aes-256-cbc" : key.length === 24 ? "aes-192-cbc" : "aes-128-cbc";
        const decipher = crypto.createDecipheriv(algorithm, key, Buffer.alloc(16));
        return Buffer.concat([decipher.update(Buffer.from(decodeURIComponent(data), "base64")), decipher.final()]).toString("utf8");
    } catch {
        return null;
    }
}

export type PollAnswer =
    /** Nothing new before the server's timeout. */
    | { kind: "empty" }
    | { kind: "data"; data: Record<string, unknown> }
    /** Zalo refused: error -69 or a kick-out means this session must stop polling. */
    | { kind: "error"; code: number; message: string };

/** Reads one poll answer (the HTTP body). */
export function readPollAnswer(body: string, secretKey: string): PollAnswer {
    if (body.trim() === "") return { kind: "empty" };
    const outer = parseJsonKeepingBigInts(body) as { error_code?: unknown; error_message?: unknown; data?: unknown };
    const code = Number(outer.error_code ?? 0);
    if (code !== 0) return { kind: "error", code, message: String(outer.error_message ?? "") };
    if (outer.data === "" || outer.data === undefined || outer.data === null) return { kind: "empty" };
    let inner: unknown = outer.data;
    if (typeof inner === "string") {
        const text = inner.trim().startsWith("{") ? inner : decodeAesCbc(secretKey, inner);
        if (!text) return { kind: "error", code: -1, message: "Could not decrypt the poll answer" };
        inner = parseJsonKeepingBigInts(text);
    }
    const wrapped = inner as { data?: unknown; error_code?: unknown; error_message?: unknown };
    if (wrapped && typeof wrapped === "object" && "error_code" in wrapped && Number(wrapped.error_code) !== 0) {
        return { kind: "error", code: Number(wrapped.error_code), message: String(wrapped.error_message ?? "") };
    }
    const data = (wrapped && typeof wrapped === "object" && wrapped.data && typeof wrapped.data === "object" ? wrapped.data : inner) as Record<string, unknown>;
    return { kind: "data", data };
}

/**
 * Every control event in a poll answer, wherever it sits: objects with `content.act_type` (the socket's shape) or with
 * `act_type` at the top (wrapped as `{content}`). Looks a few levels deep, since the key holding them isn't known yet.
 */
export function controlsIn(value: unknown, depth = 0): { content: Record<string, unknown> }[] {
    if (!value || typeof value !== "object" || depth > 4) return [];
    if (Array.isArray(value)) return value.flatMap((item) => {
        const it = item as { content?: Record<string, unknown>; act_type?: unknown } | null;
        if (it && typeof it === "object" && it.content && typeof it.content === "object" && "act_type" in it.content) return [it as { content: Record<string, unknown> }];
        if (it && typeof it === "object" && "act_type" in it) return [{ content: it as Record<string, unknown> }];
        return controlsIn(item, depth + 1);
    });
    return Object.values(value as Record<string, unknown>).flatMap((v) => controlsIn(v, depth + 1));
}

export type PollerDeps = {
    /** Control servers (`zpw_ctrs_new` of the login answer). */
    servers: string[];
    secretKey: string;
    apiVersion: number;
    apiType: number;
    /** Long-poll hold time the server uses, in seconds. */
    holdSeconds: number;
    /** A GET with the session's cookies and user agent; returns the HTTP status and body. */
    get: (url: string, signal: AbortSignal) => Promise<{ status: number; body: string }>;
    onControls: (controls: { content: Record<string, unknown> }[]) => void;
    onEvent: (entry: Record<string, unknown>) => void;
    sleep?: (ms: number) => Promise<void>;
};

const MIN_GAP_MS = 1_000;
/** Zalo PC polls again at once after a held answer, but waits when an empty one came back early (under 9 s). */
const EARLY_EMPTY_MS = 9_000;
const EARLY_EMPTY_WAIT_MS = 5_000;
const BACKOFF_MS = [5_000, 15_000, 30_000, 60_000];
/** When an answer only repeats controls already seen (the cursor didn't move), wait this long before the next poll. */
const REPEAT_WAIT_MS = 30_000;

/** The control servers of a login answer: a list of URLs, or one URL. */
export function serverList(value: unknown): string[] {
    const list = Array.isArray(value) ? value : typeof value === "string" ? [value] : [];
    return list.filter((v): v is string => typeof v === "string" && /^https?:\/\//.test(v));
}

/** One session's control poll loop. Stops on `stop()` or when Zalo refuses (it never logs the session out itself). */
export class ControlPoller {
    private running = false;
    private abort: AbortController | null = null;
    private lastActionId = "0";
    private lastActionIdOther = "1";
    private server = 0;
    private failures = 0;
    /** Ids of controls already handed over (most recent last), to notice answers that only repeat themselves. */
    private seen = new Set<string>();

    constructor(private readonly deps: PollerDeps) {}

    get cursor(): string {
        return pollCursor(this.lastActionId, this.lastActionIdOther);
    }

    start(): void {
        if (this.running || this.deps.servers.length === 0) {
            if (this.deps.servers.length === 0) this.deps.onEvent({ kind: "ctrl_poll_unavailable", reason: "no control servers in the login answer" });
            return;
        }
        this.running = true;
        this.deps.onEvent({ kind: "ctrl_poll_start", servers: this.deps.servers.length, holdSeconds: this.deps.holdSeconds });
        void this.loop();
    }

    stop(): void {
        this.running = false;
        this.abort?.abort();
    }

    /** One poll, exposed for tests; returns whether polling should go on. */
    async pollOnce(): Promise<boolean> {
        const started = Date.now();
        const { deps } = this;
        const base = deps.servers[this.server % deps.servers.length].replace(/\/+$/, "");
        const url = `${base}/?zpw_ver=${deps.apiVersion}&zpw_type=${deps.apiType}&params=${this.cursor}&ts=${Math.floor(Date.now() / 1000)}`;
        this.abort = new AbortController();
        const timer = setTimeout(() => this.abort?.abort(), (deps.holdSeconds + 20) * 1000);
        let answer: PollAnswer;
        try {
            const res = await deps.get(url, this.abort.signal);
            if (res.status !== 200) throw new Error(`HTTP ${res.status}`);
            answer = readPollAnswer(res.body, deps.secretKey);
        } catch (err) {
            if (!this.running) return false;
            this.failures++;
            this.server++;
            deps.onEvent({ kind: "ctrl_poll_error", error: err instanceof Error ? err.message : String(err), failures: this.failures });
            await this.sleep(BACKOFF_MS[Math.min(this.failures - 1, BACKOFF_MS.length - 1)]);
            return this.running;
        } finally {
            clearTimeout(timer);
        }
        if (answer.kind === "error") {
            deps.onEvent({ kind: "ctrl_poll_refused", code: answer.code, message: answer.message });
            return false;
        }
        this.failures = 0;
        if (answer.kind === "empty" && Date.now() - started < EARLY_EMPTY_MS) await this.sleep(EARLY_EMPTY_WAIT_MS);
        if (answer.kind === "data") {
            const d = answer.data;
            if (d.resetLastActionId !== undefined && d.resetLastActionId !== null && d.resetLastActionId !== "") {
                this.lastActionId = String(d.resetLastActionId);
                this.lastActionIdOther = "1";
            } else if (d.lastActionId !== undefined && d.lastActionId !== null && d.lastActionId !== "") {
                this.lastActionId = String(d.lastActionId);
            }
            const controls = controlsIn(d);
            const fresh = controls.filter((c) => {
                const id = String((c as { controlId?: unknown; actionId?: unknown }).controlId ?? (c as { actionId?: unknown }).actionId ?? "");
                if (!id) return true;
                if (this.seen.has(id)) return false;
                this.seen.add(id);
                if (this.seen.size > 500) this.seen.delete(this.seen.values().next().value as string);
                return true;
            });
            deps.onEvent({
                kind: "ctrl_poll",
                keys: Object.keys(d).slice(0, 30),
                controls: fresh.map((c) => `${String(c.content.act_type ?? "?")}/${String(c.content.act ?? "?")}`),
                ...(fresh.length < controls.length ? { repeated: controls.length - fresh.length } : {}),
            });
            if (fresh.length) deps.onControls(fresh);
            else if (controls.length) await this.sleep(REPEAT_WAIT_MS);
        }
        return true;
    }

    private async loop(): Promise<void> {
        while (this.running) {
            const started = Date.now();
            const goOn = await this.pollOnce();
            if (!goOn) {
                this.running = false;
                break;
            }
            const gap = MIN_GAP_MS - (Date.now() - started);
            if (gap > 0) await this.sleep(gap);
        }
    }

    private sleep(ms: number): Promise<void> {
        return (this.deps.sleep ?? ((t: number) => new Promise((r) => setTimeout(r, t))))(ms);
    }
}
