import { EventEmitter } from "node:events";
import type { API } from "zca-js";
import { decodeEventPayload, frameHeader, jsonObject } from "./calls.js";
import { HttpError } from "./errors.js";
import { importConversation, isOutgoing, readConversation, type BackupConversation, type ImportCounts, type ImportSink } from "./syncimport.js";
import { BackupError, backupKeyText, conversationFiles, generateSyncKeyPair, rsaDecryptPkcs1, unpackBackup, type SyncKeyPair } from "./zdb.js";
import { HISTORY_MESSAGES_PER_THREAD } from "./store.js";

/**
 * "Sync messages from the phone": asks the official Zalo app on the phone to upload an encrypted backup of its chat
 * history, downloads and decrypts it, and imports the messages. The protocol is the one Zalo PC uses, as implemented by
 * the Zalos project (`MobileHistorySync`, MIT); docs/history-sync.md has the full description and what is still unproven
 * against a real phone.
 *
 * Privacy: nothing here logs message content or keys. The log keeps event names, field names, counts and sizes.
 */

export type SyncState = "idle" | "waiting_for_phone" | "downloading" | "importing" | "done" | "failed";

export type HistorySyncStatus = {
    state: SyncState;
    /** 0-100. */
    progress: number;
    error: string | null;
    /** A short sentence for the progress screen. */
    detail: string;
    /** How many messages the phone said it would send. */
    expectedMessages?: number;
    /** Set once the import finished. */
    counts?: ImportCounts & { conversations: number };
};

/** Control events about a phone sync (`content.act_type`). */
export const SYNC_ACT_TYPES = new Set(["syncmsgmb", "transfer_msg", "transfer_sync2", "transfer_after_login"]);

export type SyncControl = { actType: string; act: string; data: Record<string, unknown> };

/** Reads a control's `content` (the shape of a cmd 601 control or a control-poll entry). */
export function syncControlOf(content: unknown): SyncControl | null {
    if (!content || typeof content !== "object") return null;
    const c = content as Record<string, unknown>;
    const actType = typeof c.act_type === "string" ? c.act_type : "";
    if (!SYNC_ACT_TYPES.has(actType)) return null;
    const data = parseJsonObject(c.data);
    return { actType, act: typeof c.act === "string" ? c.act : "", data };
}

/** The phone-sync controls in a list of `{content}` entries. */
export function syncControlsIn(controls: unknown[]): SyncControl[] {
    const out: SyncControl[] = [];
    for (const entry of controls) {
        const control = syncControlOf((entry as { content?: unknown } | null)?.content);
        if (control) out.push(control);
    }
    return out;
}

/** The phone-sync controls in a raw chat-socket frame (cmd 601), or none. */
export function syncControlsOfFrame(frame: Buffer, cipherKey: string | undefined): SyncControl[] {
    const header = frameHeader(frame);
    if (!header || header.version !== 1 || header.cmd !== 601 || header.subCmd !== 0) return [];
    const text = frame.subarray(4).toString("utf8");
    if (!text) return [];
    const decoded = decodeEventPayload(JSON.parse(text), cipherKey) as { data?: { controls?: unknown[] }; controls?: unknown[] } | null;
    return syncControlsIn(decoded?.data?.controls ?? decoded?.controls ?? []);
}

/** The phone's `transfer_error` event: its `status` says whether it is idle (2) or working (1); else an error code. */
export type TransferError = "phone_idle" | "phone_active" | "failed" | "ignore";

export function classifyTransferError(data: Record<string, unknown>): TransferError {
    const status = Number(data.status ?? -1);
    if (status === 2) return "phone_idle";
    if (status === 1) return "phone_active";
    return Number(data.error_code ?? 0) !== 0 ? "failed" : "ignore";
}

const MAX_DOWNLOAD_ATTEMPTS = 5;
const RETRY_BASE_MS = 1000;
const RETRY_MAX_MS = 8000;
const RETRY_AFTER_MAX_MS = 30_000;

/**
 * How long to wait before downloading again, or -1 to give up. The backup file is prepared asynchronously, so the first
 * requests can see 404 or 400; those, rate limits and server errors (and network failures: `status` 0) are retried with
 * a growing delay, honouring `Retry-After` (seconds or a date) for 429 and 503, capped at 30 s.
 */
export function downloadRetryDelayMs(status: number, attempt: number, retryAfter: string | undefined, now = Date.now()): number {
    if (attempt < 1 || attempt >= MAX_DOWNLOAD_ATTEMPTS) return -1;
    const retryable = status === 0 || status === 400 || status === 404 || status === 408 || status === 429 || status >= 500;
    if (!retryable) return -1;
    const backoff = Math.min(RETRY_MAX_MS, RETRY_BASE_MS * 2 ** (attempt - 1));
    const hint = (retryAfter ?? "").trim();
    if ((status !== 429 && status !== 503) || !hint) return backoff;
    let waitMs = -1;
    if (/^\d+$/.test(hint)) waitMs = Math.min(Number(hint), RETRY_AFTER_MAX_MS / 1000) * 1000;
    else {
        const at = Date.parse(hint);
        if (Number.isFinite(at)) waitMs = at - now;
    }
    if (waitMs < 0) return backoff;
    return Math.min(Math.max(waitMs, backoff), RETRY_AFTER_MAX_MS);
}

/** An HTTP answer that is not a success, from the backup download. */
export class DownloadError extends Error {
    constructor(
        message: string,
        /** The HTTP status, or 0 for a network failure. */
        readonly status: number,
        readonly retryAfter?: string,
        /** What the server said, shortened (its error text; never the backup). */
        readonly answer?: string,
    ) {
        super(message);
    }
}

export type HistorySyncDeps = {
    ownId(): string | null;
    /** Zalo's device id, sent with the requests. */
    imei(): string;
    /** The device name the phone's prompt shows. */
    pcName: string;
    /** Calls `/api/message/<endpoint>` on the file service with encrypted parameters; resolves with Zalo's answer. */
    call(endpoint: string, params: Record<string, unknown>): Promise<unknown>;
    /** Downloads the backup. Throws [DownloadError] for an HTTP error. */
    download(url: string, onProgress: (received: number, total: number) => void, signal: AbortSignal): Promise<Buffer>;
    sink: ImportSink & { allowMessages(cap: number): void };
    /** A name for a user id (used for the sender of imported messages), or "". */
    nameOf?(userId: string): string;
    /** Called before a conversation is imported, so [nameOf] can know the names of its senders (best effort). */
    prepare?(conversationId: string, isGroup: boolean): Promise<void>;
    keyPair?: () => SyncKeyPair;
    /** Waits between download attempts; tests pass a fake. */
    sleep?: (ms: number) => Promise<void>;
    timeouts?: { confirmMs?: number; backupMs?: number; downloadStallMs?: number };
    now?: () => number;
};

const CONFIRM_TIMEOUT_MS = 3 * 60_000;
const BACKUP_TIMEOUT_MS = 30 * 60_000;
const DOWNLOAD_STALL_MS = 2 * 60_000;
/** Zalo's answer when the phone's Zalo uses the newer "sync v2" (iPhone), which isn't implemented here. */
const CODE_CLIENT_NOT_SUPPORTED = 211;
const LOG_LINES = 200;

type Events = { change: [HistorySyncStatus] };

const ACTIVE: ReadonlySet<SyncState> = new Set(["waiting_for_phone", "downloading", "importing"]);

export class HistorySync extends EventEmitter<Events> {
    private status: HistorySyncStatus = { state: "idle", progress: 0, error: null, detail: "" };
    private keys: SyncKeyPair | null = null;
    private generation = 0;
    private timer: NodeJS.Timeout | null = null;
    private abort: AbortController | null = null;
    private tempKey = "";
    private readonly lines: string[] = [];

    constructor(private readonly deps: HistorySyncDeps) {
        super();
    }

    getStatus(): HistorySyncStatus {
        return { ...this.status };
    }

    isActive(): boolean {
        return ACTIVE.has(this.status.state);
    }

    /** What happened so far: event names, field names, counts and sizes only. For finding out what a phone sends. */
    getLog(): string[] {
        return [...this.lines];
    }

    private log(line: string): void {
        this.lines.push(`${new Date((this.deps.now ?? Date.now)()).toISOString()} ${line}`);
        if (this.lines.length > LOG_LINES) this.lines.splice(0, this.lines.length - LOG_LINES);
    }

    private set(patch: Partial<HistorySyncStatus>): void {
        this.status = { ...this.status, ...patch };
        this.emit("change", this.getStatus());
    }

    /** Asks the phone to send its history. Resolves once the request was made; progress follows through "change". */
    async start(): Promise<HistorySyncStatus> {
        if (this.isActive()) return this.getStatus();
        const ownId = this.deps.ownId();
        if (!ownId) throw new HttpError(409, "Not logged in");
        this.clearRun();
        const gen = ++this.generation;
        const keys = (this.deps.keyPair ?? generateSyncKeyPair)();
        this.keys = keys;
        this.status = { state: "idle", progress: 0, error: null, detail: "" };
        this.set({ state: "waiting_for_phone", progress: 5, detail: "Request sent. Open Zalo on your phone and tap Allow to transfer your messages." });
        this.arm(this.deps.timeouts?.confirmMs ?? CONFIRM_TIMEOUT_MS, () =>
            this.fail(gen, "The phone did not confirm the request. Open Zalo on the phone and try again."),
        );
        try {
            await this.deps.call("pull_mobile_msg", {
                pc_name: this.deps.pcName,
                public_key: keys.publicKey,
                from_seq_id: 0,
                is_retry: 0,
                min_seq_id: 0,
                temp_key: this.tempKey,
                imei: this.deps.imei(),
            });
            this.log("pull_mobile_msg ok");
        } catch (err) {
            const code = Number((err as { code?: unknown })?.code);
            this.log(`pull_mobile_msg failed code=${Number.isFinite(code) ? code : "?"}`);
            if (code === CODE_CLIENT_NOT_SUPPORTED) {
                this.fail(gen, "This phone uses a newer way of syncing history (sync v2, used by Zalo on iPhone), which isn't supported yet.");
            } else {
                this.fail(gen, `Zalo refused the sync request${Number.isFinite(code) ? ` (code ${code})` : ""}.`);
            }
        }
        return this.getStatus();
    }

    /** Stops a sync in progress and tells Zalo so. */
    cancel(): void {
        const wasActive = this.isActive();
        const keys = this.keys;
        this.clearRun();
        this.generation++;
        if (wasActive && keys) {
            this.deps
                .call("cancel_pull_mobile_msg", { pc_name: this.deps.pcName, public_key: keys.publicKey, imei: this.deps.imei() })
                .catch(() => {});
        }
        this.keys = null;
        if (this.status.state !== "idle") this.set({ state: "idle", progress: 0, error: null, detail: "" });
    }

    /** Frames from the chat socket (the same one the call tap reads). */
    onFrame(frame: Buffer, cipherKey: string | undefined): void {
        let controls: SyncControl[];
        try {
            controls = syncControlsOfFrame(frame, cipherKey);
        } catch {
            return;
        }
        for (const control of controls) this.handleControl(control);
    }

    /** Controls from the control-channel poll. */
    onControls(controls: unknown[]): void {
        for (const control of syncControlsIn(controls)) this.handleControl(control);
    }

    handleControl(control: SyncControl): void {
        const { actType, act, data } = control;
        const ours = Boolean(this.keys) && data.public_key === this.keys?.publicKey;
        // Field names only: what a phone sends is what the next step needs to know, and none of it is message content.
        this.log(`control act_type=${actType} act=${act} state=${this.status.state} ours=${ours} keys=[${Object.keys(data).sort().join(",")}]`);

        if (actType === "transfer_after_login") {
            const key = String(data.tempKey ?? data.temp_key ?? "").trim();
            if (key) this.tempKey = key;
            return;
        }
        if (actType === "transfer_sync2") {
            // The phone answered the old request with the newer protocol: unsupported, say so instead of waiting.
            if (this.status.state === "waiting_for_phone") this.log("phone speaks sync v2 (not supported)");
            return;
        }
        if (actType !== "syncmsgmb" || this.status.state !== "waiting_for_phone" || !ours) return;
        const gen = this.generation;
        const samePc = data.pc_name === undefined || data.pc_name === this.deps.pcName;

        if (act === "user_confirm" && samePc) {
            const action = Number(data.user_action ?? -1);
            if (action === 1 || action === 3) {
                this.set({ progress: 15, detail: "The phone agreed. Waiting for it to pack your messages..." });
                this.arm(this.deps.timeouts?.backupMs ?? BACKUP_TIMEOUT_MS, () => this.fail(gen, "The phone did not send the backup within 30 minutes."));
            } else if (action === 0) {
                this.fail(gen, "The sync request was declined on the phone.");
            } else if (action === 2) {
                this.fail(gen, "The phone is restoring data and can't send messages right now. Try again later.");
            }
        } else if (act === "transfer_error" && samePc) {
            switch (classifyTransferError(data)) {
                case "phone_idle":
                    this.set({ detail: "Keep Zalo open on your phone, with the screen on, so the backup can continue." });
                    break;
                case "phone_active":
                    this.set({ progress: 20, detail: "The phone is backing up your messages..." });
                    this.arm(this.deps.timeouts?.backupMs ?? BACKUP_TIMEOUT_MS, () => this.fail(gen, "The phone did not send the backup within 30 minutes."));
                    break;
                case "failed":
                    this.fail(gen, `The phone could not make the backup (code ${Number(data.error_code)}).`);
                    break;
                case "ignore":
                    break;
            }
        } else if (act === "syncmsg_info") {
            this.disarm();
            void this.receiveBackup(gen, data).catch((err) => this.fail(gen, this.describe(err)));
        }
    }

    private describe(err: unknown): string {
        if (err instanceof BackupError) return `The backup could not be read: ${err.message}.`;
        if (err instanceof HttpError) return err.message;
        return "The history sync failed.";
    }

    private async receiveBackup(gen: number, data: Record<string, unknown>): Promise<void> {
        const info = parseJsonObject(data.db_info);
        const counts = (info.origin_db ?? info.backup_db ?? {}) as Record<string, unknown>;
        const expected = Number(counts.msg_total ?? 0);
        const format = Number(info.db_format ?? 0);
        const url = typeof data.url === "string" ? data.url : "";
        const encryptedKey = typeof data.encrypted_key === "string" ? data.encrypted_key : "";
        this.log(
            `backup info: format=${format} msgs=${expected} hasUrl=${Boolean(url)} host=${hostOf(url)} urlParams=[${paramNamesOf(url)}] hasKey=${Boolean(encryptedKey)} ` +
                `size=${Number(data.file_size ?? 0)} dataKeys=[${Object.keys(data).sort().join(",")}] infoKeys=[${Object.keys(info).sort().join(",")}]`,
        );
        if (Object.keys(info).length === 0) throw new BackupError("the backup description is missing");
        if (expected === 0) {
            this.finish(gen, { imported: 0, duplicates: 0, skipped: 0, recalled: 0, conversations: 0 }, "Your phone had no messages to transfer.");
            return;
        }
        if (format !== 1) throw new BackupError("the backup uses an old format (0) that isn't supported");
        if (!/^https:\/\//i.test(url) || !encryptedKey) throw new BackupError("the backup has no download link or key");
        this.set({ state: "downloading", progress: 20, detail: "Downloading the backup from Zalo...", expectedMessages: expected });

        const bytes = await this.downloadWithRetries(gen, url);
        if (gen !== this.generation) return;
        this.log(`downloaded bytes=${bytes.length} expected=${Number(data.file_size ?? 0)}`);

        this.set({ state: "importing", progress: 55, detail: "Decrypting the backup..." });
        await this.tick();
        const keys = this.keys;
        if (!keys) return;
        const plainKey = rsaDecryptPkcs1(keys.privateKeyPem, encryptedKey);
        if (!plainKey) throw new BackupError("the backup key could not be decrypted");
        const files = unpackBackup(bytes, backupKeyText(plainKey, format));
        const conversations = conversationFiles(files);
        this.log(`unpacked files=${files.length} conversations=${conversations.length}`);
        // The snapshot has served its purpose: let Zalo delete it (best effort).
        this.deps.call("delete_snapshot_mobile_msg", { public_key: keys.publicKey, imei: this.deps.imei() }).catch(() => {});
        if (conversations.length === 0) throw new BackupError("it holds no conversations");

        this.deps.sink.allowMessages(HISTORY_MESSAGES_PER_THREAD);
        const ownId = this.deps.ownId() ?? "";
        const total: ImportCounts = { imported: 0, duplicates: 0, skipped: 0, recalled: 0 };
        let done = 0;
        for (const file of conversations) {
            if (gen !== this.generation) return;
            let conv;
            try {
                conv = readConversation(file.data, file.id, file.isGroup);
            } catch (err) {
                // One damaged conversation shouldn't lose the rest.
                this.log(`conversation unreadable: ${err instanceof Error ? err.constructor.name : "error"}`);
                done++;
                continue;
            }
            if (conv && conv.rows.length) {
                if (done === 0 && conv.columns) this.log(`schema columns=${conv.columns.join(",")}`);
                if (done < 3) this.log(`senders ${describeSenders(conv, ownId)}`);
                await this.deps.prepare?.(conv.id, conv.isGroup).catch(() => {});
                if (gen !== this.generation) return;
                const c = importConversation(conv, ownId, this.deps.sink, this.deps.nameOf);
                total.imported += c.imported;
                total.duplicates += c.duplicates;
                total.skipped += c.skipped + (conv.sourceRows - conv.rows.length);
                total.recalled += c.recalled;
            }
            done++;
            this.set({ state: "importing", progress: 60 + Math.floor((39 * done) / conversations.length), detail: `Importing messages (${done}/${conversations.length} chats)...` });
            await this.tick();
        }
        this.log(`import done imported=${total.imported} duplicates=${total.duplicates} skipped=${total.skipped} recalled=${total.recalled}`);
        this.finish(
            gen,
            { ...total, conversations: conversations.length },
            `Added ${total.imported} messages from ${conversations.length} chats${total.duplicates ? `; ${total.duplicates} were already here` : ""}.`,
        );
    }

    private async downloadWithRetries(gen: number, url: string): Promise<Buffer> {
        const controller = new AbortController();
        this.abort = controller;
        const sleep = this.deps.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
        const stallMs = this.deps.timeouts?.downloadStallMs ?? DOWNLOAD_STALL_MS;
        for (let attempt = 1; ; attempt++) {
            // Aborts the transfer when no bytes arrive for a while.
            let stall = setTimeout(() => controller.abort(), stallMs);
            try {
                return await this.deps.download(
                    url,
                    (got, total) => {
                        clearTimeout(stall);
                        stall = setTimeout(() => controller.abort(), stallMs);
                        if (gen === this.generation && total > 0) this.set({ progress: 20 + Math.floor((30 * got) / total) });
                    },
                    controller.signal,
                );
            } catch (err) {
                if (gen !== this.generation) throw err;
                const status = err instanceof DownloadError ? err.status : 0;
                const wait = controller.signal.aborted ? -1 : downloadRetryDelayMs(status, attempt, err instanceof DownloadError ? err.retryAfter : undefined, (this.deps.now ?? Date.now)());
                const answer = err instanceof DownloadError && err.answer ? ` answer=${JSON.stringify(err.answer)}` : "";
                this.log(`download attempt=${attempt} status=${status} retryIn=${wait}${answer}`);
                if (wait < 0) {
                    throw new BackupError(controller.signal.aborted ? "the download stalled" : status ? `the download failed (HTTP ${status})` : "the download failed (network error)");
                }
                this.set({ progress: 20, detail: "Waiting for Zalo's server to prepare the backup..." });
                await sleep(wait);
                if (gen !== this.generation) throw err;
            } finally {
                clearTimeout(stall);
            }
        }
    }

    private finish(gen: number, counts: ImportCounts & { conversations: number }, detail: string): void {
        if (gen !== this.generation) return;
        this.clearRun();
        this.keys = null;
        this.set({ state: "done", progress: 100, error: null, detail, counts });
    }

    private fail(gen: number, error: string): void {
        if (gen !== this.generation) return;
        const keys = this.keys;
        this.clearRun();
        this.generation++;
        this.keys = null;
        if (keys) {
            this.deps
                .call("cancel_pull_mobile_msg", { pc_name: this.deps.pcName, public_key: keys.publicKey, imei: this.deps.imei() })
                .catch(() => {});
        }
        this.log(`failed: ${error}`);
        this.set({ state: "failed", progress: 0, error, detail: "" });
    }

    private arm(ms: number, onTimeout: () => void): void {
        this.disarm();
        this.timer = setTimeout(onTimeout, ms);
        this.timer.unref();
    }

    private disarm(): void {
        if (this.timer) clearTimeout(this.timer);
        this.timer = null;
    }

    private clearRun(): void {
        this.disarm();
        this.abort?.abort();
        this.abort = null;
    }

    /** Lets the event loop breathe between heavy steps. */
    private tick(): Promise<void> {
        return new Promise((resolve) => setImmediate(resolve));
    }
}

/** An object, or JSON text of one (long ids kept as text), else {}. */
function parseJsonObject(value: unknown): Record<string, unknown> {
    return jsonObject(value) ?? {};
}

function hostOf(url: string): string {
    try {
        return new URL(url).host;
    } catch {
        return "";
    }
}

/** The names of a URL's query parameters (not their values, which can be keys). */
function paramNamesOf(url: string): string {
    try {
        return [...new URL(url).searchParams.keys()].join(",");
    } catch {
        return "";
    }
}

/** Hosts that get the Zalo session cookies, as in Zalo PC and the Zalos project. */
export function isZaloCookieHost(host: string): boolean {
    return /(^|\.)(zalo\.me|zaloapp\.com|zdn\.vn|zadn\.vn|zalo\.vn|dlmd\.me)$/i.test(host);
}

/**
 * The cookie header for a backup download: the session's chat.zalo.me cookies (the backup sits on another Zalo host,
 * whose own cookies would be none), plus zpw_uin when the jar lacks it, the way the Zalos project downloads it.
 */
export function backupCookieHeader(chatCookies: { key: string; value: string }[], hostCookies: { key: string; value: string }[], uid: string): string {
    const parts = new Map<string, string>();
    for (const c of [...chatCookies, ...hostCookies]) {
        if (!c.value && (c.key === "zpw_uin" || c.key === "zpw_sek")) continue;
        if (!parts.has(c.key)) parts.set(c.key, c.value);
    }
    if (!parts.has("zpw_uin") && uid) parts.set("zpw_uin", uid);
    return [...parts].map(([k, v]) => `${k}=${v}`).join("; ");
}

/** A short, single-line excerpt of an error answer for the log. */
export function answerExcerpt(text: string): string {
    return text.replace(/[^\x20-\x7e\u00a0-\uffff]+/g, " ").trim().slice(0, 200);
}

// ---- Real network pieces on a logged-in zca-js session ----

const FILE_SERVICE_FALLBACK = "https://tt-files-wpa.chat.zalo.me";
export const MAX_BACKUP_BYTES = 256 * 1024 * 1024;

/**
 * Calls `/api/message/<endpoint>` on the file service the way Zalo PC does: a GET whose `params` are the AES-encrypted
 * JSON of the arguments (like every other zca-js request).
 */
export function fileServiceCaller(api: API): HistorySyncDeps["call"] {
    const name = "zalooFileGet";
    if (!(name in api)) {
        api.custom<unknown, { endpoint: string; params: Record<string, unknown> }>(name, async ({ utils, props }) => {
            const map = (api as unknown as { zpwServiceMap?: { file?: unknown } }).zpwServiceMap?.file;
            const first = Array.isArray(map) ? map[0] : map;
            const base = typeof first === "string" && /^https:\/\//.test(first) ? first.replace(/\/+$/, "") : FILE_SERVICE_FALLBACK;
            const params = utils.encodeAES(JSON.stringify(props.params));
            if (!params) throw new Error("Could not encrypt the request");
            const response = await utils.request(utils.makeURL(`${base}/api/message/${props.endpoint}`, { params }), { method: "GET" });
            return utils.resolve(response);
        });
    }
    const call = (api as unknown as Record<string, (p: { endpoint: string; params: Record<string, unknown> }) => Promise<unknown>>)[name];
    return (endpoint, params) => call({ endpoint, params });
}

/**
 * Downloads a backup into memory, with a size cap. The request is the one the Zalos project sends: a plain GET with the
 * chat.zalo.me session cookies (only to Zalo hosts), chat.zalo.me as Origin and Referer, no form content type, and
 * redirects followed by hand so cookies never leave Zalo's hosts.
 */
export function backupDownloader(api: API): HistorySyncDeps["download"] {
    const name = "zalooBackupGet";
    if (!(name in api)) {
        api.custom<Buffer, { url: string; signal: AbortSignal; onProgress: (got: number, total: number) => void }>(name, async ({ ctx, props }) => {
            const fetcher: typeof fetch = ctx.options?.polyfill ?? fetch;
            const headersFor = async (target: URL): Promise<Record<string, string>> => {
                const headers: Record<string, string> = {
                    "User-Agent": ctx.userAgent,
                    Accept: "*/*",
                    "Accept-Language": "vi-VN,vi;q=0.9,en-US;q=0.8,en;q=0.7",
                };
                if (isZaloCookieHost(target.hostname)) {
                    const jar = ctx.cookie;
                    const chat = jar ? await jar.getCookies("https://chat.zalo.me/") : [];
                    const own = jar ? await jar.getCookies(target.href) : [];
                    const cookie = backupCookieHeader(chat, own, String(ctx.uid ?? ""));
                    if (cookie) headers.Cookie = cookie;
                    headers.Referer = "https://chat.zalo.me/";
                    headers.Origin = "https://chat.zalo.me";
                }
                return headers;
            };
            let response: Response | null = null;
            let target = new URL(props.url);
            try {
                for (let hop = 0; hop < 5; hop++) {
                    const init = { method: "GET", headers: await headersFor(target), signal: props.signal, redirect: "manual" as const };
                    response = await fetcher(target.href, Object.assign(init, ctx.options?.agent ? { agent: ctx.options.agent } : {}));
                    const location = response.status >= 300 && response.status < 400 ? response.headers.get("location") : null;
                    if (!location) break;
                    await response.body?.cancel().catch(() => undefined);
                    target = new URL(location, target);
                    if (target.protocol !== "https:") throw new DownloadError("redirected off https", 0);
                }
            } catch (err) {
                if (err instanceof DownloadError) throw err;
                throw new DownloadError("network error", 0);
            }
            if (!response) throw new DownloadError("network error", 0);
            if (!response.ok) {
                const answer = await response.text().then(answerExcerpt, () => "");
                throw new DownloadError(`HTTP ${response.status}`, response.status, response.headers.get("retry-after") ?? undefined, answer);
            }
            // fetch undoes a content encoding but leaves the encoded length in content-length.
            const encoded = Boolean(response.headers.get("content-encoding")) && response.headers.get("content-encoding") !== "identity";
            const total = encoded ? 0 : Number(response.headers.get("content-length") ?? 0);
            if (total > MAX_BACKUP_BYTES) throw new DownloadError("too large", 413);
            if (!response.body) return Buffer.from(await response.arrayBuffer());
            const chunks: Buffer[] = [];
            let got = 0;
            const reader = response.body.getReader();
            try {
                for (;;) {
                    const { done, value } = await reader.read();
                    if (done) break;
                    got += value.length;
                    if (got > MAX_BACKUP_BYTES) throw new DownloadError("too large", 413);
                    chunks.push(Buffer.from(value));
                    props.onProgress(got, total);
                }
            } catch (err) {
                if (err instanceof DownloadError) throw err;
                throw new DownloadError("network error", 0);
            }
            if (total > 0 && got !== total) throw new DownloadError("incomplete download", 0);
            return Buffer.concat(chunks);
        });
    }
    const get = (api as unknown as Record<string, (p: { url: string; signal: AbortSignal; onProgress: (got: number, total: number) => void }) => Promise<Buffer>>)[name];
    return (url, onProgress, signal) => get({ url, signal, onProgress });
}

/** Counts of how a conversation's rows were classified, with no ids (for the sync log). */
function describeSenders(conv: BackupConversation, ownId: string): string {
    let zero = 0;
    let empty = 0;
    let peer = 0;
    let own = 0;
    let other = 0;
    let sent = 0;
    for (const r of conv.rows) {
        if (r.senderId === "0") zero++;
        else if (r.senderId === "") empty++;
        else if (r.senderId === conv.id) peer++;
        else if (ownId !== "" && r.senderId === ownId) own++;
        else other++;
        if (isOutgoing(r, conv, ownId)) sent++;
    }
    return `${conv.isGroup ? "group" : "chat"} rows=${conv.rows.length} sender0=${zero} empty=${empty} peer=${peer} own=${own} other=${other} direction=${conv.rows.some((r) => r.outgoing !== undefined)} sent=${sent}`;
}
