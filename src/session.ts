import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";
import {
    CloseReason,
    LoginQRCallbackEventType,
    MuteAction,
    MuteDuration,
    Reactions,
    ThreadType,
    Zalo,
    type API,
    type Credentials,
    type DeliveredMessage,
    type LoginQRCallbackEvent,
    type Message,
    type Reaction,
    type SeenMessage,
    type SendMessageQuote,
    type Typing,
    type Undo,
    ZaloApiLoginQRAborted,
} from "zca-js";
import {
    CALL_ENDING_ACTS,
    CallTrace,
    RINGING_MAX_AGE_MS,
    callControlsIn,
    incomingCallOf,
    inspectFrame,
    jsonObject,
    type CallControl,
    type FrameInfo,
    type IncomingCall,
} from "./calls.js";
import { ControlPoller, serverList } from "./ctrlpoll.js";
import { HistorySync, backupDownloader, fileServiceCaller, type HistorySyncStatus } from "./historysync.js";
import { acceptCall, camMicState, declineCall, hangUp, mediaParamsOf, ringRing, ringingCallOf, voiceCallSender, type RingingCall } from "./voicecall.js";
import { MediaCall, parseServer } from "./zrtcclient.js";
import { HttpError } from "./errors.js";
import {
    ChatStore,
    normalizeMessage,
    STICKER_IMAGE_URL,
    REACTIONS,
    recalledMessageId,
    threadKey,
    type ChatMessage,
    type MentionInfo,
    type RawMessage,
    type StoredMessage,
    type ThreadKind,
} from "./store.js";

export type SessionState = "logged_out" | "awaiting_qr_scan" | "logging_in" | "ready" | "disconnected";

export type SessionStatus = {
    state: SessionState;
    /** PNG data URL of the login QR code while state is "awaiting_qr_scan". */
    qrImage: string | null;
    /** Set once the QR code has been scanned on the phone, before it is confirmed. */
    scannedBy: string | null;
    ownId: string | null;
    ownName: string | null;
    error: string | null;
    /** A saved session couldn't reach Zalo and will be retried; a QR login can replace it. */
    retrying: boolean;
};

export type TypingEvent = { threadKind: ThreadKind; threadId: string; uid: string; name: string };
export type DeletedEvent = { threadKind: ThreadKind; threadId: string; id: string };
export type GroupMember = { id: string; name: string };
export type Person = { id: string; name: string; avatar: string };
export type FriendRequest = Person & { message: string; time: number };
export type FoundUser = Person & { isFriend: boolean; requested: boolean };
export type StickerInfo = { id: number; catId: number; type: number; url: string };
/** A file to send, as the client uploaded it. Photos need their size so Zalo can lay them out. */
export type UploadFile = { data: Buffer; filename: string; width?: number; height?: number };
export type MuteOption = "1h" | "4h" | "8am" | "forever";

export type ReadEvent = { threadKind: ThreadKind; threadId: string };
/** Someone is calling (answer it in the official app), or that call stopped ringing. */
/** A call that was picked up here. */
export type CurrentCall = { callId: string; callerId: string; callerName: string; video: boolean; state: "connecting" | "connected"; connectedAt: number | null };
type ActiveCall = {
    info: CurrentCall;
    media: MediaCall | null;
    finish: (reason: string, hangUp: boolean) => void;
    /** Tells the other side our camera / microphone state. */
    camMic?: (state: { camOn?: boolean; muted?: boolean }) => void;
    /** Our camera, as the app last said: video from the app is only forwarded while it is on. */
    camOn: boolean;
    /** The other side's camera and microphone, from their `onoff_camera` / `mute_audio` controls. */
    peer: { cameraOn: boolean; muted: boolean };
    /** Which peer controls have arrived at least once. */
    peerHeard: Set<string>;
};

export type CallEvent =
    | { type: "incoming"; call: IncomingCall & { callerName: string; groupName?: string } }
    | ({ type: "connected" } & CurrentCall)
    | { type: "ended"; callId: string; act: string }
    | { type: "peer"; callId: string; cameraOn: boolean; muted: boolean };

type SessionEvents = {
    status: [SessionStatus];
    read: [ReadEvent];
    call: [CallEvent];
    /** The other side's audio in a call picked up here: decrypted Opus frames for the app. */
    call_audio: [{ seq: number; timestamp: number; opus: Buffer }];
    /** The other side's video in a call picked up here: one H.264 access unit (Annex B), 90 kHz timestamp. */
    call_video: [{ key: boolean; timestamp: number; data: Buffer }];
    /** The other side asked for a key frame: the app's encoder should produce one. */
    call_key_frame: [];
    message: [ChatMessage];
    message_updated: [ChatMessage];
    message_deleted: [DeletedEvent];
    typing: [TypingEvent];
    threads: [];
    /** Progress of "sync messages from the phone" (src/historysync.ts). */
    history_sync: [HistorySyncStatus];
};

/** Controls in which the other side says its camera or microphone changed (not the end of a call). */
const PEER_ACTS = new Set(["onoff_camera", "mute_audio"]);
const GROUP_INFO_BATCH = 50;
const MEMBER_INFO_BATCH = 100;
const MEMBERS_CACHE_MS = 10 * 60_000;
const SEEN_RECEIPT_LIMIT = 50;
const RESUME_RETRY_MIN_MS = 5_000;
const RESUME_RETRY_MAX_MS = 60_000;
const SESSION_EXPIRED = "Your Zalo session has ended. Log in again with the QR code.";
const PHOTO_EXTENSIONS = new Set(["jpg", "jpeg", "png", "webp"]);
const STICKER_SEARCH_LIMIT = 30;
const MAX_STICKERS_CACHE = 500;
const MAX_GROUP_NAME = 100;
/** A client says which chat it shows at least this often; after that the chat counts as closed again. */
const VIEWING_TTL_MS = 90_000;
/** Older messages (replayed after a reconnect) are history, not news: no notification. */
const NOTIFY_MAX_AGE_MS = 10 * 60_000;

export class ZaloSession extends EventEmitter<SessionEvents> {
    private api: API | null = null;
    private loginInFlight: { generation: number; promise: Promise<void> } | null = null;
    private abortQR: (() => unknown) | null = null;
    /** Bumped by logout and by a QR login that replaces a resume, so stale attempts drop their result. */
    private generation = 0;
    private resumeRetry: { timer: NodeJS.Timeout; wake: () => void } | null = null;
    private resuming = false;
    private members = new Map<string, { at: number; list: GroupMember[] }>();
    private friendList = new Map<string, Person>();
    private stickers = new Map<number, StickerInfo>();
    private stickerLookups = new Map<number, Promise<StickerInfo | null>>();
    private viewing: { key: string; until: number } | null = null;
    private readonly callTrace: CallTrace;
    /** Sockets already tapped for call events (zca-js opens a new one on each reconnect). */
    private tappedSockets = new WeakSet<object>();
    /** The control-channel poll (calls reach Zalo PC there, not on the chat socket), for [ctrlPollApi]. */
    private ctrlPoller: ControlPoller | null = null;
    private ctrlPollApi: API | null = null;
    /** Turned off for this login when Zalo closed the chat socket while the poll ran, in case the two clash. */
    private ctrlPollOff = false;
    /** "act:callId" of call controls already handled, so one that comes both ways is announced once. */
    private handledCallControls = new Map<string, number>();
    /** Calls ringing on this account now, by call id. */
    private ringing = new Map<string, RingingCall>();
    /** The call that was picked up, while it runs. */
    private activeCall: ActiveCall | null = null;
    private status: SessionStatus = {
        state: "logged_out",
        qrImage: null,
        scannedBy: null,
        ownId: null,
        ownName: null,
        error: null,
        retrying: false,
    };

    private readonly credentialsFile: string;
    /** Asks the phone's Zalo app for its chat history (docs/history-sync.md). */
    readonly historySync: HistorySync;

    constructor(
        readonly store: ChatStore,
        dataDir: string,
        private readonly sendReadReceipts = true,
    ) {
        super();
        this.credentialsFile = path.join(dataDir, "credentials.json");
        this.callTrace = new CallTrace(dataDir);
        this.historySync = new HistorySync({
            ownId: () => this.status.ownId,
            imei: () => (this.requireApi().getContext() as unknown as { imei?: string }).imei ?? "",
            pcName: "Zalo-Android",
            // async: a missing login is a rejected promise, not a throw.
            call: async (endpoint, params) => fileServiceCaller(this.requireApi())(endpoint, params),
            download: async (url, onProgress, signal) => backupDownloader(this.requireApi())(url, onProgress, signal),
            sink: this.store,
            nameOf: (id) => {
                const name = this.store.getThread("user", id)?.name ?? "";
                return name === id ? "" : name;
            },
        });
        this.historySync.on("change", (status) => {
            this.emit("history_sync", status);
            // Imported chats and last messages: clients reload their list.
            if (status.state === "done") this.emit("threads");
        });
    }

    getStatus(): SessionStatus {
        return { ...this.status };
    }

    /**
     * Resume a saved session if there is one; otherwise stay logged out until a QR login is requested.
     *
     * A phone often starts the server before it has a network connection. Those failures
     * don't mean the session is gone, so keep retrying instead of asking for a new QR scan.
     */
    async resume(): Promise<void> {
        const credentials = this.readCredentials();
        if (!credentials) return;
        const generation = this.generation;
        this.resuming = true;
        try {
            await this.resumeLoop(credentials, generation);
        } finally {
            if (generation === this.generation) this.resuming = false;
            if (this.status.retrying) this.setStatus({ retrying: false });
        }
    }

    private async resumeLoop(credentials: Credentials, generation: number): Promise<void> {
        let delayMs = RESUME_RETRY_MIN_MS;
        for (;;) {
            let networkError: unknown = null;
            await this.runLogin(async () => {
                try {
                    const api = await this.newZalo().login(credentials);
                    await this.onLoggedIn(api, generation);
                } catch (err) {
                    if (!isTransientError(err)) throw new Error(SESSION_EXPIRED);
                    networkError = err;
                }
            });
            if (!networkError || generation !== this.generation) return;

            this.setStatus({
                state: "logging_in",
                retrying: true,
                error: `Can't reach Zalo (${describeError(networkError)}). Retrying in ${Math.round(delayMs / 1000)} s…`,
            });
            const woken = await this.waitForRetry(delayMs);
            if (generation !== this.generation) return;
            if (!woken) delayMs = Math.min(delayMs * 2, RESUME_RETRY_MAX_MS);
        }
    }

    /** Start a QR login. The QR image is published through status updates. */
    async startQRLogin(): Promise<void> {
        if (this.status.state === "ready" || this.status.state === "disconnected") return;
        // Replace a resume that is waiting to retry: the user chose to scan a new code instead.
        if (this.resuming) {
            this.cancelResume();
            await this.loginInFlight?.promise.catch(() => {});
        }
        const generation = this.generation;
        return this.runLogin(async () => {
            let declined = false;
            const api = await this.newZalo()
                .loginQR({}, (event) => {
                    if (generation !== this.generation) {
                        if (event.actions && "abort" in event.actions) event.actions.abort();
                        return;
                    }
                    if (event.type === LoginQRCallbackEventType.QRCodeDeclined) declined = true;
                    this.onQREvent(event);
                })
                .catch((err: unknown) => {
                    if (declined) throw new Error("Login was declined on the phone. Try again.");
                    if (err instanceof ZaloApiLoginQRAborted) throw new Error("Login was cancelled.");
                    throw err;
                });
            if (!api) throw new Error("Login was cancelled.");
            await this.onLoggedIn(api, generation);
        });
    }

    /** Retry a waiting resume now, e.g. when the device regains its network connection. */
    retryNow(): void {
        this.resumeRetry?.wake();
    }

    async logout(): Promise<void> {
        this.historySync.cancel();
        this.activeCall?.finish("logout", true);
        this.cancelResume();
        this.abortQR?.();
        this.abortQR = null;
        this.stopControlPoll();
        this.ctrlPollOff = false;
        this.api?.listener.stop();
        this.api = null;
        this.members.clear();
        this.friendList.clear();
        this.ringing.clear();
        this.handledCallControls.clear();
        this.viewing = null;
        this.stickers.clear();
        this.stickerLookups.clear();
        this.callTrace.clear();
        this.store.ownId = null;
        fs.rmSync(this.credentialsFile, { force: true });
        this.store.clear();
        this.store.flush();
        this.setStatus({
            state: "logged_out",
            qrImage: null,
            scannedBy: null,
            ownId: null,
            ownName: null,
            error: null,
            retrying: false,
        });
    }

    /** Stops background activity (active call, poller, listener, timers) without deleting saved credentials or messages. */
    shutdown(): void {
        this.activeCall?.finish("shutdown", true);
        this.cancelResume();
        this.abortQR?.();
        this.abortQR = null;
        this.stopControlPoll();
        this.api?.listener.stop();
        this.api = null;
    }

    /** Reconnect the real-time listener, e.g. after Zalo Web was opened elsewhere and kicked us. */
    reconnect(): void {
        if (!this.api) throw new HttpError(409, "Not logged in");
        this.api.listener.start({ retryOnClose: true });
    }

    // ---- Messages ----

    async sendText(
        kind: ThreadKind,
        threadId: string,
        text: string,
        opts: { quoteId?: string; mentions?: MentionInfo[] } = {},
    ): Promise<string | null> {
        const api = this.requireApi();
        let quote: SendMessageQuote | undefined;
        if (opts.quoteId) {
            const q = this.requireMessage(kind, threadId, opts.quoteId);
            if (q.recalled) throw new HttpError(400, "Can't reply to a recalled message");
            quote = {
                content: q.raw.content,
                msgType: q.raw.msgType,
                propertyExt: q.raw.propertyExt,
                uidFrom: q.raw.uidFrom,
                msgId: q.id,
                cliMsgId: q.cliMsgId,
                ts: q.raw.ts,
                ttl: q.raw.ttl,
            } as SendMessageQuote;
        }
        const mentions = kind === "group" ? opts.mentions : undefined;
        const res = await api.sendMessage({ msg: text, quote, mentions }, threadId, toThreadType(kind));
        // The message itself comes back through the listener (selfListen), which stores it.
        return res.message ? String(res.message.msgId) : null;
    }

    /** `reaction` is a key of REACTIONS, or null to remove ours. */
    async react(kind: ThreadKind, threadId: string, msgId: string, reaction: string | null): Promise<void> {
        const api = this.requireApi();
        const msg = this.requireMessage(kind, threadId, msgId);
        const code = reaction === null ? Reactions.NONE : (REACTIONS[reaction]?.code as Reactions | undefined);
        if (code === undefined) throw new HttpError(400, `Unknown reaction: ${reaction}`);

        await api.addReaction(code, {
            data: { msgId: msg.id, cliMsgId: msg.cliMsgId },
            threadId,
            type: toThreadType(kind),
        });
        // Apply right away; the listener echo is idempotent.
        this.updated(this.store.applyReaction(msg.id, this.status.ownId ?? "", code));
    }

    /** Recall ("thu hồi") one of our own messages for everyone. */
    async recall(kind: ThreadKind, threadId: string, msgId: string): Promise<void> {
        const api = this.requireApi();
        const msg = this.requireMessage(kind, threadId, msgId);
        if (!msg.isSelf) throw new HttpError(403, "You can only recall your own messages");
        await api.undo({ msgId: msg.id, cliMsgId: msg.cliMsgId }, threadId, toThreadType(kind));
        this.updated(this.store.recall(msg.id));
    }

    /** Delete a message from this account's view only. */
    async deleteForMe(kind: ThreadKind, threadId: string, msgId: string): Promise<void> {
        const api = this.requireApi();
        const msg = this.requireMessage(kind, threadId, msgId);
        await api.deleteMessage(
            { data: { cliMsgId: msg.cliMsgId, msgId: msg.id, uidFrom: msg.raw.uidFrom }, threadId, type: toThreadType(kind) },
            true,
        );
        if (this.store.removeMessage(msg.id)) this.emit("message_deleted", { threadKind: kind, threadId, id: msg.id });
    }

    async forward(
        kind: ThreadKind,
        threadId: string,
        msgId: string,
        targets: { kind: ThreadKind; id: string }[],
    ): Promise<void> {
        const api = this.requireApi();
        const msg = this.requireMessage(kind, threadId, msgId);
        if (typeof msg.raw.content !== "string" || msg.recalled) {
            throw new HttpError(400, "Only text messages can be forwarded");
        }
        const payload = {
            message: msg.raw.content,
            reference: { id: msg.id, ts: msg.ts, logSrcType: 1, fwLvl: 1 },
        };
        for (const targetKind of ["user", "group"] as const) {
            const ids = targets.filter((t) => t.kind === targetKind).map((t) => t.id);
            if (!ids.length) continue;
            const res = await api.forwardMessage(payload, ids, toThreadType(targetKind));
            if (res.fail?.length) throw new HttpError(502, `Forwarding failed for ${res.fail.length} chat(s)`);
        }
    }

    async sendTyping(kind: ThreadKind, threadId: string): Promise<void> {
        await this.requireApi().sendTypingEvent(threadId, toThreadType(kind));
    }

    /** Pull recent group history from Zalo. 1:1 chats have no such endpoint. */
    async fetchGroupHistory(groupId: string, count = 50): Promise<void> {
        const api = this.requireApi();
        const res = await api.getGroupChatHistory(groupId, count);
        for (const raw of res.groupMsgs) this.ingest(raw);
        if (res.groupMsgs.length > 0) this.emit("threads");
    }

    // ---- Conversations ----

    async markRead(kind: ThreadKind, threadId: string): Promise<void> {
        const unread = this.store.unreadMessages(kind, threadId);
        const wasMarkedUnread = this.store.markRead(kind, threadId);
        // Clears the chat's notification on the phone.
        this.emit("read", { threadKind: kind, threadId });
        if (!this.api) return;

        // Let the sender see "seen", as the official app does. Best effort.
        const withRaw = unread.filter((m): m is StoredMessage & { raw: NonNullable<StoredMessage["raw"]> } => !!m.raw);
        if (this.sendReadReceipts && withRaw.length) {
            const params = withRaw.slice(-SEEN_RECEIPT_LIMIT).map((m) => ({
                msgId: m.id,
                cliMsgId: m.cliMsgId,
                uidFrom: m.raw.uidFrom,
                idTo: m.raw.idTo,
                msgType: m.raw.msgType,
                st: m.raw.st,
                at: m.raw.at,
                cmd: m.raw.cmd,
                ts: m.raw.ts,
            }));
            await this.api.sendSeenEvent(params, toThreadType(kind)).catch((err) => console.warn("sendSeenEvent:", err));
        }
        if (wasMarkedUnread) {
            await this.api.removeUnreadMark(threadId, toThreadType(kind)).catch(() => {});
        }
    }

    async markUnread(kind: ThreadKind, threadId: string): Promise<void> {
        await this.requireApi().addUnreadMark(threadId, toThreadType(kind));
        this.store.markUnread(kind, threadId);
        this.emit("threads");
    }

    async setPinned(kind: ThreadKind, threadId: string, pinned: boolean): Promise<void> {
        await this.requireApi().setPinnedConversations(pinned, threadId, toThreadType(kind));
        this.store.setPinned(kind, threadId, pinned);
        this.emit("threads");
    }

    async setMuted(kind: ThreadKind, threadId: string, option: MuteOption | null): Promise<void> {
        const api = this.requireApi();
        if (option === null) {
            await api.setMute({ action: MuteAction.UNMUTE }, threadId, toThreadType(kind));
            this.store.setMuted(kind, threadId, null);
        } else {
            const duration = {
                "1h": MuteDuration.ONE_HOUR,
                "4h": MuteDuration.FOUR_HOURS,
                "8am": MuteDuration.UNTIL_8AM,
                forever: MuteDuration.FOREVER,
            }[option];
            await api.setMute({ action: MuteAction.MUTE, duration }, threadId, toThreadType(kind));
            this.store.setMuted(kind, threadId, muteUntil(option));
        }
        this.emit("threads");
    }

    /** Delete the whole conversation history (for this account). */
    async deleteConversation(kind: ThreadKind, threadId: string): Promise<void> {
        const api = this.requireApi();
        const last = this.store.lastMessage(kind, threadId);
        if (last?.raw) {
            await api.deleteChat(
                { ownerId: last.raw.uidFrom, cliMsgId: last.cliMsgId, globalMsgId: last.id },
                threadId,
                toThreadType(kind),
            );
        }
        this.store.clearThreadMessages(kind, threadId);
        this.emit("threads");
    }

    async lastOnline(userId: string): Promise<number | null> {
        const res = await this.requireApi().lastOnline(userId);
        return res.settings?.show_online_status && res.lastOnline ? res.lastOnline : null;
    }

    async groupMembers(groupId: string): Promise<GroupMember[]> {
        const cached = this.members.get(groupId);
        if (cached && Date.now() - cached.at < MEMBERS_CACHE_MS) return cached.list;

        const api = this.requireApi();
        const info = (await api.getGroupInfo(groupId)).gridInfoMap[groupId];
        if (!info) throw new HttpError(404, "Group not found");

        let list: GroupMember[] = (info.currentMems ?? []).map((m) => ({ id: String(m.id), name: m.dName || m.zaloName }));
        if (list.length === 0) {
            // memVerList entries look like "<uid>_<version>".
            const ids = (info.memVerList ?? info.memberIds ?? []).map((v) => String(v).split("_")[0]);
            for (let i = 0; i < ids.length; i += MEMBER_INFO_BATCH) {
                const res = await api.getGroupMembersInfo(ids.slice(i, i + MEMBER_INFO_BATCH));
                for (const [id, p] of Object.entries(res.profiles)) list.push({ id, name: p.displayName || p.zaloName });
            }
        }
        const ownId = this.status.ownId;
        list = list.filter((m) => m.id !== ownId).sort((a, b) => a.name.localeCompare(b.name));
        this.members.set(groupId, { at: Date.now(), list });
        return list;
    }

    // ---- Notifications ----

    /** The chat a client has open (null = none), so its messages don't also notify. */
    setViewing(kind: ThreadKind | null, threadId: string | null, now = Date.now()): void {
        this.viewing = kind && threadId ? { key: threadKey(kind, threadId), until: now + VIEWING_TTL_MS } : null;
    }

    /** Whether a new message deserves a phone notification. */
    shouldNotify(msg: ChatMessage, now = Date.now()): boolean {
        if (msg.isSelf || msg.recalled) return false;
        if (now - msg.ts > NOTIFY_MAX_AGE_MS) return false;
        if (this.store.isMuted(msg.threadKind, msg.threadId, now)) return false;
        const key = threadKey(msg.threadKind, msg.threadId);
        return !(this.viewing && this.viewing.key === key && this.viewing.until > now);
    }

    threadName(kind: ThreadKind, threadId: string): string {
        return this.store.getThread(kind, threadId)?.name ?? threadId;
    }

    // ---- Calls (research: docs/calls.md) ----

    /** The call log (JSON lines) for sharing; empty until a call event arrives. */
    callLog(): string {
        return this.callTrace.read();
    }

    clearCallLog(): void {
        this.callTrace.clear();
    }

    /**
     * zca-js drops the realtime "voip" controls that signal calls. Reads them from the same socket: decoded with the
     * session's cipher key, recorded in the call log, and an incoming call is announced.
     */
    private tapCallEvents(listener: unknown, live: () => boolean = () => true): void {
        const l = listener as { ws?: { on?: (event: string, fn: (data: unknown) => void) => void } | null; cipherKey?: string };
        const ws = l.ws;
        if (!ws || typeof ws.on !== "function" || this.tappedSockets.has(ws)) return;
        this.tappedSockets.add(ws);
        // Proves the hook runs: a log with this line and no call events means Zalo sent none to this session.
        this.callTrace.record({ kind: "listening" });
        ws.on("message", (data: unknown) => {
            if (!live()) return;
            if (!Buffer.isBuffer(data)) return;
            this.historySync.onFrame(data, l.cipherKey);
            let info: FrameInfo | null;
            try {
                info = inspectFrame(data, l.cipherKey);
            } catch (err) {
                this.callTrace.record({ kind: "decode_error", error: err instanceof Error ? err.message : String(err) });
                return;
            }
            if (!info) return;
            if (info.controlTypes.length) this.callTrace.record({ kind: "controls", types: info.controlTypes });
            if (info.unknown) this.callTrace.record({ kind: "frame", ...info.unknown });
            for (const control of info.calls) this.onCallControl(control);
        });
    }

    /** Declines a call ringing on this account; the caller sees it declined, as from the official apps. */
    async declineCall(callId: string): Promise<void> {
        const api = this.requireApi();
        const call = this.ringing.get(callId);
        if (!call || Date.now() - call.receivedAt > RINGING_MAX_AGE_MS) {
            this.ringing.delete(callId);
            throw new HttpError(404, "That call isn't ringing any more");
        }
        if (call.group) throw new HttpError(422, "Group calls can't be declined here");
        // Claim before awaiting so answerCall cannot race declineCall.
        this.ringing.delete(callId);
        const imei = (api.getContext() as unknown as { imei?: string }).imei ?? "";
        const send = voiceCallSender(api);
        try {
            const rang = await ringRing(send, call, imei);
            this.callTrace.record({ kind: "ringring", callId, answer: rang });
        } catch (err) {
            this.callTrace.record({ kind: "ringring_failed", callId, error: describeError(err) });
        }
        try {
            const answer = await declineCall(send, call);
            this.callTrace.record({ kind: "declined", callId, answer });
        } catch (err) {
            this.callTrace.record({ kind: "decline_failed", callId, error: describeError(err) });
            throw new HttpError(502, `Zalo refused: ${describeError(err)}`);
        }
        this.emit("call", { type: "ended", callId, act: "declined" });
    }

    /**
     * Picks up a ringing call the way Zalo PC does: ringring, join the call's media server, answer with status 0. The
     * other side's audio then goes to [onCallAudio] listeners (the app's audio link) and the app's frames go out with
     * [sendCallAudio]; silence fills in until they come. "Answer (test)" passes [testSeconds]: no app audio, hang up
     * after that long. The call log gets the media server's replies and, at the end, what the audio looked like (counts,
     * RTP fields, whether it decrypted), never the audio itself.
     */
    async answerCall(callId: string, testSeconds?: number): Promise<CurrentCall> {
        const api = this.requireApi();
        const call = this.ringing.get(callId);
        if (!call || Date.now() - call.receivedAt > RINGING_MAX_AGE_MS) {
            this.ringing.delete(callId);
            throw new HttpError(404, "That call isn't ringing any more");
        }
        if (call.group) throw new HttpError(422, "Group calls can't be answered here");
        if (this.activeCall) throw new HttpError(409, "Already in a call");
        const p = mediaParamsOf(call);
        const server = p && parseServer(p.server);
        if (!p || !server) {
            this.callTrace.record({ kind: "answer_unavailable", callId, reason: "the request has no media server, ids or session" });
            throw new HttpError(422, "This call doesn't carry what's needed to answer it");
        }
        // Claim before awaiting so declineCall or a second answerCall can't race this one.
        this.ringing.delete(callId);
        const imei = (api.getContext() as unknown as { imei?: string }).imei ?? "";
        const send = voiceCallSender(api);
        const callerName =
            this.friendList.get(call.callerId)?.name ?? this.store.getThread("user", call.callerId)?.name ?? call.displayName ?? "Someone";
        const current: CurrentCall = { callId, callerId: call.callerId, callerName, video: call.video, state: "connecting", connectedAt: null };

        let done = false;
        let wantHangUp = false;
        let accepted = false;
        let timer: NodeJS.Timeout | undefined;
        let started = Date.now();
        const active: ActiveCall = {
            info: current,
            media: null,
            finish: (reason: string, sendHangUp: boolean) => {
                if (done) return;
                done = true;
                wantHangUp = sendHangUp;
                clearTimeout(timer);
                const media = active.media;
                if (media) media.close(sendHangUp && accepted);
                if (this.activeCall === active) this.activeCall = null;
                this.callTrace.record({
                    kind: "call_done",
                    callId,
                    reason,
                    seconds: Math.round((Date.now() - started) / 1000),
                    ...(media ? { stats: media.stats } : {}),
                });
                if (sendHangUp && accepted) {
                    hangUp(send, call, imei).then(
                        (answer) => this.callTrace.record({ kind: "hung_up", callId, answer }),
                        (err) => this.callTrace.record({ kind: "hang_up_failed", callId, error: describeError(err) }),
                    );
                }
                this.emit("call", { type: "ended", callId, act: reason });
            },
            camOn: false,
            peer: { cameraOn: true, muted: false },
            peerHeard: new Set(),
        };
        this.activeCall = active;
        try {
            await ringRing(send, call, imei).then(
                (answer) => this.callTrace.record({ kind: "ringring", callId, answer }),
                (err) => this.callTrace.record({ kind: "ringring_failed", callId, error: describeError(err) }),
            );
            if (done) throw new HttpError(409, "Call ended while connecting");
            const media = new MediaCall({
                server,
                userId: p.userId,
                partnerId: p.partnerId,
                callId: p.callId,
                session: p.session,
                log: (e) => this.callTrace.record({ callId, ...e }),
                onAudio: (seq, ts, opus) => this.emit("call_audio", { seq, timestamp: ts, opus }),
                onVideo: (frame) => this.emit("call_video", frame),
                onKeyFrameRequest: () => this.emit("call_key_frame"),
            });
            active.media = media;
            started = Date.now();
            const token = await media.join();
            this.callTrace.record({ kind: "media_join", callId, server: p.server, joined: token !== null, ms: Date.now() - started });
            if (done) {
                media.close(false);
                throw new HttpError(409, "Call ended while connecting");
            }
            if (token === null) {
                media.close(false);
                throw new HttpError(502, "Zalo's media server didn't let us join the call");
            }
            try {
                const answer = await acceptCall(send, call, p.server);
                this.callTrace.record({ kind: "answered", callId, answer, video: call.video, test: testSeconds !== undefined });
            } catch (err) {
                media.close();
                this.callTrace.record({ kind: "answer_failed", callId, error: describeError(err) });
                throw new HttpError(502, `Zalo refused: ${describeError(err)}`);
            }
            if (done) {
                media.close(wantHangUp);
                if (wantHangUp) {
                    hangUp(send, call, imei).then(
                        (answer) => this.callTrace.record({ kind: "hung_up", callId, answer }),
                        (err) => this.callTrace.record({ kind: "hang_up_failed", callId, error: describeError(err) }),
                    );
                }
                throw new HttpError(409, "Call ended while connecting");
            }
            accepted = true;
            media.startSilence();
            active.camMic = (state) => {
                camMicState(send, call, state).then(
                    (answer) => this.callTrace.record({ kind: "cam_mic", callId, ...state, answer }),
                    (err) => this.callTrace.record({ kind: "cam_mic_failed", callId, ...state, error: describeError(err) }),
                );
            };
            // No camera yet (unless already turned on while connecting): say so, or the other side waits for our picture.
            if (call.video) active.camMic({ camOn: active.camOn });
            current.state = "connected";
            current.connectedAt = Date.now();
            timer = testSeconds === undefined ? undefined : setTimeout(() => active.finish("time_up", true), testSeconds * 1000);
            this.emit("call", { type: "connected", ...current, connectedAt: current.connectedAt });
            return { ...current };
        } catch (err) {
            active.media?.close(false);
            if (this.activeCall === active) this.activeCall = null;
            throw err;
        }
    }

    /** "Answer (test)": picks up without app audio and hangs up after [seconds]. */
    answerCallTest(callId: string, seconds = 30): Promise<CurrentCall> {
        return this.answerCall(callId, seconds);
    }

    /** Hangs up the connected call. */
    hangUpCall(): void {
        if (!this.activeCall) throw new HttpError(404, "No call in progress");
        this.activeCall.finish("hangup", true);
    }

    /** Mutes or unmutes us in the call in progress, as the other side's app shows it. (The app stops the audio itself.) */
    setCallMuted(muted: boolean): void {
        if (!this.activeCall) throw new HttpError(404, "No call in progress");
        this.activeCall.camMic?.({ muted });
    }

    /** The app turned its camera on or off: tells the other side, and starts / stops forwarding the app's video. */
    setCallCamera(on: boolean): void {
        if (!this.activeCall) throw new HttpError(404, "No call in progress");
        this.activeCall.camOn = on;
        this.activeCall.camMic?.({ camOn: on });
    }

    /** One encoded video frame from the app's camera (H.264, Annex B), for the call in progress. */
    sendCallVideo(frame: { key: boolean; data: Buffer }): void {
        const call = this.activeCall;
        if (call?.info.state === "connected" && call.camOn) call.media?.sendVideoFrame(frame);
    }

    /** The app's decoder needs a key frame: asks the other side for one. */
    requestCallKeyFrame(): void {
        if (this.activeCall?.info.state === "connected") this.activeCall.media?.requestKeyFrame("app");
    }

    /** The call in progress, if any. */
    currentCall(): CurrentCall | null {
        return this.activeCall ? { ...this.activeCall.info } : null;
    }

    /** One 20 ms Opus frame from the app's microphone, for the call in progress. */
    sendCallAudio(opus: Buffer): void {
        if (this.activeCall?.info.state === "connected") this.activeCall.media?.sendAudioFrame(opus);
    }

    /**
     * Polls Zalo's control servers the way Zalo PC does (src/ctrlpoll.ts), for the call events the chat socket never
     * carries. Notes in the call log what the login offered (field names, server hosts, socket flags), so a log shows
     * why it could not run.
     */
    private startControlPoll(api: API): void {
        if (this.ctrlPollOff || (this.ctrlPoller && this.ctrlPollApi === api)) return;
        this.stopControlPoll();
        const ctx = api.getContext() as unknown as {
            loginInfo?: Record<string, unknown>;
            secretKey?: string | null;
            settings?: { features?: { socket?: Record<string, unknown> } };
            API_TYPE: number;
            API_VERSION: number;
        };
        const info = ctx.loginInfo ?? {};
        const servers = serverList(info.zpw_ctrs_new);
        const socket = ctx.settings?.features?.socket ?? {};
        this.callTrace.record({
            kind: "ctrl_setup",
            loginKeys: Object.keys(info).filter((k) => k.startsWith("zpw_")).sort(),
            servers: servers.map(hostOf),
            socket: { enable_ctrl_socket: socket.enable_ctrl_socket, enable_chat_socket: socket.enable_chat_socket, disable_lp: socket.disable_lp },
        });
        if (!ctx.secretKey) {
            this.callTrace.record({ kind: "ctrl_poll_unavailable", reason: "no session key" });
            return;
        }
        // Zalo PC reads controls from its socket and never long-polls when the account says so; neither does Zalỏ
        // (the chat socket carries the call controls then).
        if (socket.enable_ctrl_socket === true && socket.disable_lp === true) {
            this.callTrace.record({ kind: "ctrl_poll_skipped", reason: "controls come over the socket (disable_lp)" });
            return;
        }
        const GET = "zalooControlGet";
        if (!(GET in api)) {
            api.custom<Response, { url: string; signal: AbortSignal }>(GET, ({ utils, props }) =>
                utils.request(props.url, { method: "GET", signal: props.signal }),
            );
        }
        const get = (api as unknown as Record<string, (p: { url: string; signal: AbortSignal }) => Promise<Response>>)[GET];
        const hold = Number((info.zpw_conf as { lp_ctl_timeout?: unknown } | undefined)?.lp_ctl_timeout);
        const live = () => this.api === api && this.ctrlPollApi === api;
        const poller = new ControlPoller({
            servers,
            secretKey: ctx.secretKey,
            apiVersion: ctx.API_VERSION,
            apiType: ctx.API_TYPE,
            holdSeconds: hold > 0 && hold < 300 ? hold : 30,
            get: async (url, signal) => {
                const res = await get({ url, signal });
                return { status: res.status, body: await res.text() };
            },
            onControls: (controls) => {
                if (!live()) return;
                this.historySync.onControls(controls);
                for (const control of callControlsIn(controls)) this.onCallControl(control, "poll");
            },
            onEvent: (entry) => live() && this.callTrace.record(entry),
        });
        this.ctrlPoller = poller;
        this.ctrlPollApi = api;
        poller.start();
    }

    private stopControlPoll(): void {
        this.ctrlPoller?.stop();
        this.ctrlPoller = null;
        this.ctrlPollApi = null;
    }

    /** Public for tests: one decoded voip control, from the chat socket or the control poll. */
    onCallControl(control: CallControl, source: "socket" | "poll" = "socket", now = Date.now()): void {
        this.callTrace.record({ kind: "control", source, act: control.act, control: control.raw });
        const callId = control.data.callId ?? control.data.call_id;
        if (callId !== undefined && callId !== "") {
            // Camera / microphone toggles repeat legitimately (off, on, off…): they are told apart by state in
            // onPeerControl, not by this once-per-call check.
            if (PEER_ACTS.has(control.act)) {
                this.onPeerControl(control, callId);
                return;
            }
            const key = `${control.act}:${String(callId)}`;
            for (const [k, at] of this.handledCallControls) if (now - at > 10 * 60_000) this.handledCallControls.delete(k);
            if (this.handledCallControls.has(key)) return;
            this.handledCallControls.set(key, now);
        }
        if (PEER_ACTS.has(control.act)) {
            this.onPeerControl(control, undefined);
            return;
        }
        const incoming = incomingCallOf(control, now);
        if (incoming && now - incoming.ts > RINGING_MAX_AGE_MS) {
            this.callTrace.record({ kind: "call_too_old", callId: incoming.callId, ageSeconds: Math.round((now - incoming.ts) / 1000) });
        } else if (incoming) {
            for (const [id, call] of this.ringing) if (now - call.receivedAt > RINGING_MAX_AGE_MS) this.ringing.delete(id);
            this.ringing.set(incoming.callId, ringingCallOf(incoming, control, now));
            const caller =
                this.friendList.get(incoming.callerId)?.name ?? this.store.getThread("user", incoming.callerId)?.name ?? incoming.displayName ?? "";
            const groupName = incoming.groupId ? this.store.getThread("group", incoming.groupId)?.name : undefined;
            this.emit("call", { type: "incoming", call: { ...incoming, callerName: caller || "Someone", ...(groupName ? { groupName } : {}) } });
        } else if (CALL_ENDING_ACTS.has(control.act)) {
            if (callId !== undefined) this.ringing.delete(String(callId));
            // The caller hung up (our own answer comes back as "answer" too: not an end).
            if (this.activeCall && String(callId) === this.activeCall.info.callId) {
                if (control.act === "answer") return;
                this.activeCall.finish(control.act, false);
                return;
            }
            this.emit("call", { type: "ended", callId: callId === undefined ? "" : String(callId), act: control.act });
        }
    }

    /** The other side turned its camera (`onoff_camera`, 0 = off) or microphone (`mute_audio`, 1 = muted) on or off. */
    private onPeerControl(control: CallControl, callId: unknown): void {
        const call = this.activeCall;
        if (!call || (callId !== undefined && callId !== "" && String(callId) !== call.info.callId)) return;
        const params = jsonObject(control.data.params);
        // The Zalos project's engine reads `type`, falling back to `status`.
        const value = Number(params?.type ?? params?.status);
        if (!Number.isFinite(value)) return;
        const before = JSON.stringify(call.peer);
        if (control.act === "onoff_camera") call.peer.cameraOn = value !== 0;
        else call.peer.muted = value === 1;
        // The same state arriving again (a control that comes on both channels) isn't news, once we've heard one.
        if (before === JSON.stringify(call.peer) && call.peerHeard.has(control.act)) return;
        call.peerHeard.add(control.act);
        this.emit("call", { type: "peer", callId: call.info.callId, ...call.peer });
    }

    // ---- Photos, files and stickers ----

    /**
     * Sends one photo or file. Zalo sends jpg/png/webp as photos (they need their pixel size) and anything else as a
     * file; GIFs need a path-based metadata reader zca-js has no hook for here, so they are refused.
     */
    async sendAttachment(kind: ThreadKind, threadId: string, file: UploadFile): Promise<string[]> {
        const api = this.requireApi();
        const ext = path.extname(file.filename).slice(1).toLowerCase();
        if (!ext) throw new HttpError(400, "The file needs a name with an extension");
        if (ext === "gif") throw new HttpError(400, "GIFs can't be sent yet");
        if (file.data.length === 0) throw new HttpError(400, "The file is empty");
        const isPhoto = PHOTO_EXTENSIONS.has(ext);
        if (isPhoto && !(file.width && file.height)) throw new HttpError(400, "Photos need their width and height");
        const res = await api.sendMessage(
            {
                msg: "",
                attachments: [
                    {
                        data: file.data,
                        filename: file.filename as `${string}.${string}`,
                        metadata: isPhoto
                            ? { totalSize: file.data.length, width: file.width, height: file.height }
                            : { totalSize: file.data.length },
                    },
                ],
            },
            threadId,
            toThreadType(kind),
        );
        // The message itself comes back through the listener, which stores it.
        return res.attachment.map((a) => String(a.msgId));
    }

    /** Zalo's sticker search ("hi", "love", "cảm ơn"…), with the image of each. */
    async searchStickers(keyword: string): Promise<StickerInfo[]> {
        const api = this.requireApi();
        const ids = (await api.getStickers(keyword)).slice(0, STICKER_SEARCH_LIMIT);
        const missing = ids.filter((id) => !this.stickers.has(id));
        if (missing.length) {
            for (const d of await api.getStickersDetail(missing)) this.rememberSticker(d);
        }
        return ids.map((id) => this.stickers.get(id)).filter((s): s is StickerInfo => !!s);
    }

    async sendSticker(kind: ThreadKind, threadId: string, stickerId: number): Promise<void> {
        const api = this.requireApi();
        const sticker = this.stickers.get(stickerId) ?? (await this.lookupSticker(stickerId));
        if (!sticker) throw new HttpError(404, "Sticker not found");
        await api.sendSticker({ id: sticker.id, cateId: sticker.catId, type: sticker.type }, threadId, toThreadType(kind));
    }

    // ---- Friends ----

    friends(): Person[] {
        if (!this.api) return [];
        return [...this.friendList.values()].sort((a, b) => a.name.localeCompare(b.name));
    }

    /** Friend requests other people sent me. */
    async friendRequests(): Promise<FriendRequest[]> {
        const res = await this.requireApi().getFriendRecommendations();
        return (res.recommItems ?? [])
            .map((item) => item.dataInfo)
            .filter((d) => d && d.recommType === 2)
            .map((d) => ({
                id: String(d.userId),
                name: d.displayName || d.zaloName,
                avatar: d.avatar ?? "",
                message: d.recommInfo?.message ?? "",
                time: d.recommTime ?? 0,
            }));
    }

    async acceptFriendRequest(userId: string): Promise<void> {
        const api = this.requireApi();
        await api.acceptFriendRequest(userId);
        // Load their name and avatar as a friend, and show the chat.
        await this.loadFriends(api).catch(() => {});
        this.emit("threads");
    }

    async rejectFriendRequest(userId: string): Promise<void> {
        await this.requireApi().rejectFriendRequest(userId);
    }

    /** Looks someone up by phone number, as the official "Add friend" does. */
    async findUser(phone: string): Promise<FoundUser> {
        const api = this.requireApi();
        const user = await api.findUser(phone).catch(() => null);
        if (!user || !user.uid) throw new HttpError(404, "No Zalo account uses this phone number");
        const id = String(user.uid);
        let requested = false;
        try {
            requested = id in (await api.getSentFriendRequest());
        } catch {
            // Zalo answers with an error when there are none.
        }
        return {
            id,
            name: user.display_name || user.zalo_name,
            avatar: user.avatar ?? "",
            isFriend: this.friendList.has(id),
            requested,
        };
    }

    async sendFriendRequest(userId: string, message: string): Promise<void> {
        await this.requireApi().sendFriendRequest(message, userId);
    }

    async undoFriendRequest(userId: string): Promise<void> {
        await this.requireApi().undoFriendRequest(userId);
    }

    // ---- Groups ----

    async createGroup(name: string, memberIds: string[]): Promise<{ groupId: string; failed: string[] }> {
        const api = this.requireApi();
        const members = [...new Set(memberIds)].filter((id) => id !== this.status.ownId);
        if (members.length < 2) throw new HttpError(400, "A group needs at least 2 other people");
        const trimmed = name.trim().slice(0, MAX_GROUP_NAME);
        const res = await api.createGroup({ name: trimmed || undefined, members });
        const groupId = String(res.groupId);
        await this.loadGroups(api, [groupId]).catch(() => {});
        if (!this.store.getThread("group", groupId)) {
            this.store.upsertThread({ id: groupId, kind: "group", name: trimmed || "New group", avatar: "" });
        }
        this.emit("threads");
        return { groupId, failed: (res.errorMembers ?? []).map(String) };
    }

    async addGroupMembers(groupId: string, memberIds: string[]): Promise<{ failed: string[] }> {
        const ids = [...new Set(memberIds)].filter((id) => id !== this.status.ownId);
        if (!ids.length) throw new HttpError(400, "Pick someone to add");
        const res = await this.requireApi().addUserToGroup(ids, groupId);
        this.members.delete(groupId);
        return { failed: (res.errorMembers ?? []).map(String) };
    }

    async removeGroupMember(groupId: string, memberId: string): Promise<void> {
        const res = await this.requireApi().removeUserFromGroup(memberId, groupId);
        this.members.delete(groupId);
        if (res.errorMembers?.length) throw new HttpError(403, "Only the group's owner and deputies can remove members");
    }

    async renameGroup(groupId: string, name: string): Promise<void> {
        const trimmed = name.trim();
        if (!trimmed) throw new HttpError(400, "The group needs a name");
        await this.requireApi().changeGroupName(trimmed.slice(0, MAX_GROUP_NAME), groupId);
        const info = this.store.getThread("group", groupId);
        this.store.upsertThread({ id: groupId, kind: "group", name: trimmed.slice(0, MAX_GROUP_NAME), avatar: info?.avatar ?? "" });
        this.emit("threads");
    }

    /** Leaves a group (silently: only its admins are told). The chat is gone afterwards, as in the official app. */
    async leaveGroup(groupId: string, silent: boolean): Promise<void> {
        await this.requireApi().leaveGroup(groupId, silent);
        this.members.delete(groupId);
        this.store.removeThread("group", groupId);
        this.emit("threads");
    }

    // ---- Internals ----

    private rememberSticker(d: { id: number; cateId: number; type: number; stickerWebpUrl?: string | null; stickerUrl?: string }): StickerInfo | null {
        if (!d.id) return null;
        // The same picture received stickers show, so a sent sticker looks like the one picked.
        const url = STICKER_IMAGE_URL + d.id;
        const info = { id: Number(d.id), catId: Number(d.cateId), type: Number(d.type), url };
        if (this.stickers.has(info.id)) this.stickers.delete(info.id);
        this.stickers.set(info.id, info);
        while (this.stickers.size > MAX_STICKERS_CACHE) {
            const oldest = this.stickers.keys().next().value;
            if (oldest !== undefined) this.stickers.delete(oldest);
        }
        return info;
    }

    private lookupSticker(id: number): Promise<StickerInfo | null> {
        const known = this.stickers.get(id);
        if (known) return Promise.resolve(known);
        let pending = this.stickerLookups.get(id);
        if (!pending) {
            const api = this.api;
            pending = (api ? api.getStickersDetail([id]) : Promise.resolve([]))
                .then((list) => list.map((d) => this.rememberSticker(d)).find((s) => s?.id === id) ?? null)
                .catch(() => null)
                .finally(() => this.stickerLookups.delete(id));
            this.stickerLookups.set(id, pending);
        }
        return pending;
    }

    private async loadFriends(api: API, generation = this.generation): Promise<void> {
        const friends = await api.getAllFriends();
        if (generation !== this.generation || this.api !== api) return;
        this.friendList.clear();
        for (const f of friends) {
            const person = { id: String(f.userId), name: f.displayName || f.zaloName, avatar: f.avatar ?? "" };
            this.friendList.set(person.id, person);
            this.store.upsertThread({ id: person.id, kind: "user", name: person.name, avatar: person.avatar });
        }
    }

    private requireApi(): API {
        if (!this.api || (this.status.state !== "ready" && this.status.state !== "disconnected")) {
            throw new HttpError(409, "Not logged in");
        }
        return this.api;
    }

    private requireMessage(
        kind: ThreadKind,
        threadId: string,
        msgId: string,
    ): StoredMessage & { raw: NonNullable<StoredMessage["raw"]> } {
        const msg = this.store.getMessage(msgId);
        if (!msg || msg.threadKind !== kind || msg.threadId !== threadId) throw new HttpError(404, "Message not found");
        if (!msg.raw) throw new HttpError(400, "This message is too old to act on");
        return msg as StoredMessage & { raw: NonNullable<StoredMessage["raw"]> };
    }

    private newZalo(): Zalo {
        return new Zalo({ selfListen: true, logging: false, checkUpdate: false });
    }

    private runLogin(fn: () => Promise<void>): Promise<void> {
        const generation = this.generation;
        if (this.loginInFlight && this.loginInFlight.generation === generation) {
            return this.loginInFlight.promise;
        }
        if (this.status.state === "ready") return Promise.resolve();

        this.setStatus({ state: "logging_in", qrImage: null, scannedBy: null, error: null, retrying: false });
        const promise = fn()
            .catch((err: unknown) => {
                if (generation !== this.generation) return;
                this.stopControlPoll();
                this.api = null;
                this.setStatus({ state: "logged_out", qrImage: null, scannedBy: null, error: describeError(err) });
            })
            .finally(() => {
                if (generation === this.generation) {
                    if (this.loginInFlight?.generation === generation) this.loginInFlight = null;
                    this.abortQR = null;
                }
            });
        this.loginInFlight = { generation, promise };
        return promise;
    }

    /** Stops a resume that is in flight or waiting to retry; its result, if any, is dropped. */
    private cancelResume(): void {
        this.generation++;
        this.resuming = false;
        if (this.resumeRetry) {
            clearTimeout(this.resumeRetry.timer);
            this.resumeRetry.wake();
        }
    }

    /** Resolves after `ms`, or early (with true) when retryNow() or cancelResume() is called. */
    private waitForRetry(ms: number): Promise<boolean> {
        return new Promise((resolve) => {
            const done = (woken: boolean) => {
                if (this.resumeRetry?.timer === timer) this.resumeRetry = null;
                clearTimeout(timer);
                resolve(woken);
            };
            const timer = setTimeout(() => done(false), ms);
            this.resumeRetry = { timer, wake: () => done(true) };
        });
    }

    private onQREvent(event: LoginQRCallbackEvent): void {
        switch (event.type) {
            case LoginQRCallbackEventType.QRCodeGenerated:
                this.abortQR = event.actions.abort;
                this.setStatus({
                    state: "awaiting_qr_scan",
                    qrImage: `data:image/png;base64,${event.data.image}`,
                    scannedBy: null,
                });
                break;
            case LoginQRCallbackEventType.QRCodeScanned:
                this.setStatus({ scannedBy: event.data.display_name });
                break;
            case LoginQRCallbackEventType.QRCodeExpired:
                // Get a fresh code rather than making the user restart.
                event.actions.retry();
                break;
            case LoginQRCallbackEventType.QRCodeDeclined:
                // startQRLogin turns the resulting rejection into a "declined" error.
                event.actions.abort();
                break;
            case LoginQRCallbackEventType.GotLoginInfo:
                this.writeCredentials({
                    imei: event.data.imei,
                    cookie: event.data.cookie,
                    userAgent: event.data.userAgent,
                });
                this.setStatus({ state: "logging_in", qrImage: null });
                break;
        }
    }

    private async onLoggedIn(api: API, generation: number): Promise<void> {
        // Logged out (or replaced by a QR login) while this login was in flight.
        if (generation !== this.generation) return;
        if (this.ctrlPollApi !== api) this.stopControlPoll();
        this.api = api;
        const ownId = api.getOwnId();
        this.store.ownId = ownId;
        let ownName: string | null = null;
        try {
            ownName = (await api.fetchAccountInfo()).profile.displayName;
        } catch {
            // Not essential.
        }
        if (generation !== this.generation || this.api !== api) return;

        const { listener } = api;
        // Ignore stragglers from a session that was logged out.
        const live = () => generation === this.generation && this.api === api;
        listener.on("message", (msg) => live() && this.ingest(msg));
        listener.on("old_messages", (msgs) => live() && msgs.forEach((m) => this.ingest(m)));
        listener.on("undo", (undo) => live() && this.onUndo(undo));
        listener.on("reaction", (r) => live() && this.onReaction(r));
        listener.on("old_reactions", (rs) => live() && rs.forEach((r) => this.onReaction(r)));
        listener.on("typing", (t) => live() && this.onTyping(t));
        listener.on("seen_messages", (seen) => live() && seen.forEach((s) => this.onSeen(s)));
        listener.on("delivered_messages", (ds) => live() && ds.forEach((d) => this.onDelivered(d)));
        listener.on("group_event", (e) => {
            if (!live() || !e.threadId) return;
            this.members.delete(e.threadId);
            this.loadGroups(api, [e.threadId], generation).then(() => live() && this.emit("threads"), () => {});
        });
        listener.on("connected", () => {
            if (!live()) return;
            this.tapCallEvents(listener, live);
            this.startControlPoll(api);
            this.setStatus({ state: "ready", error: null });
            // Ask the server to replay messages and reactions we missed while offline.
            listener.requestOldMessages(ThreadType.User);
            listener.requestOldMessages(ThreadType.Group);
            listener.requestOldReactions(ThreadType.User);
            listener.requestOldReactions(ThreadType.Group);
        });
        listener.on("closed", (code, reason) => {
            if (!live()) return;
            const kicked = code === CloseReason.DuplicateConnection || code === CloseReason.KickConnection;
            if (kicked && this.ctrlPoller) {
                // Maybe Zalo Web elsewhere, maybe the poll: stop it so it can't be the reason this keeps happening.
                this.stopControlPoll();
                this.ctrlPollOff = true;
                this.callTrace.record({ kind: "ctrl_poll_off", reason: `chat socket closed (${code}) while polling` });
            }
            const error = kicked
                ? "Zalo Web was opened somewhere else. Only one web session can listen at a time."
                : `Connection closed (${code}${reason ? `: ${reason}` : ""})`;
            this.setStatus({ state: "disconnected", error });
        });
        listener.on("error", (err) => console.error("Zalo listener error:", err));

        this.setStatus({ state: "ready", ownId, ownName, qrImage: null, scannedBy: null, error: null });
        listener.start({ retryOnClose: true });

        this.loadContacts(api, generation).catch((err) => console.warn("Could not load contacts:", err));
    }

    private ingest(raw: Message): void {
        const rawMsg = raw as unknown as RawMessage;
        // Replayed history includes recall records as ordinary messages.
        const recalledId = recalledMessageId(rawMsg);
        if (recalledId) {
            this.updated(this.store.recall(recalledId));
            return;
        }
        const msg = normalizeMessage(rawMsg);
        if (this.store.addMessage(msg)) this.emit("message", this.store.toPublic(msg));
    }

    private onUndo(undo: Undo): void {
        this.updated(this.store.recall(String(undo.data.content.globalMsgId)));
    }

    private onReaction(r: Reaction): void {
        for (const target of r.data.content?.rMsg ?? []) {
            this.updated(this.store.applyReaction(String(target.gMsgID), String(r.data.uidFrom), r.data.content.rIcon));
        }
    }

    private onTyping(t: Typing): void {
        const kind: ThreadKind = t.type === ThreadType.Group ? "group" : "user";
        const uid = String(t.data.uid);
        const name =
            kind === "user"
                ? (this.store.getThread("user", uid)?.name ?? "")
                : (this.members.get(t.threadId)?.list.find((m) => m.id === uid)?.name ??
                  this.store.getThread("user", uid)?.name ??
                  "");
        this.emit("typing", { threadKind: kind, threadId: String(t.threadId), uid, name });
    }

    private onSeen(s: SeenMessage): void {
        const ownId = this.status.ownId;
        if (s.type === ThreadType.Group) {
            const seenBy = (s.data.seenUids ?? []).map(String).filter((u) => u !== ownId);
            this.receipt("group", s.threadId, s.data.msgId, { seenBy });
            // Read on another device: keep our unread count in sync.
            if (s.isSelf) {
                const msg = this.store.getMessage(String(s.data.msgId));
                if (msg) {
                    this.store.markRead("group", s.threadId, msg.ts);
                    this.emit("read", { threadKind: "group", threadId: String(s.threadId) });
                    this.emit("threads");
                }
            }
        } else {
            // Read on another device in a 1:1 chat: keep our unread count in sync instead of applying a peer seen receipt.
            if (Boolean((s as { isSelf?: unknown }).isSelf)) {
                const msg = this.store.getMessage(String(s.data.msgId));
                this.store.markRead("user", String(s.threadId), msg ? msg.ts : Date.now());
                this.emit("read", { threadKind: "user", threadId: String(s.threadId) });
                this.emit("threads");
            } else {
                this.receipt("user", s.threadId, s.data.msgId, { seenBy: [String(s.threadId)] });
            }
        }
    }

    private onDelivered(d: DeliveredMessage): void {
        const kind: ThreadKind = d.type === ThreadType.Group ? "group" : "user";
        const ownId = this.status.ownId;
        const others = (ids: string[]) => ids.map(String).filter((u) => u !== ownId);
        this.receipt(kind, d.threadId, d.data.msgId, {
            deliveredTo: others(d.data.deliveredUids ?? []),
            seenBy: others(d.data.seenUids ?? []),
        });
    }

    private receipt(kind: ThreadKind, threadId: string, msgId: string, r: { deliveredTo?: string[]; seenBy?: string[] }) {
        for (const m of this.store.applyReceipt(kind, String(threadId), String(msgId), r)) this.updated(m);
    }

    private updated(msg: StoredMessage | undefined): void {
        if (msg) this.emit("message_updated", this.store.toPublic(msg));
    }

    private async loadContacts(api: API, generation = this.generation): Promise<void> {
        const live = () => generation === this.generation && this.api === api;
        await this.loadFriends(api, generation);
        if (!live()) return;
        const allGroups = await api.getAllGroups();
        if (!live()) return;
        await this.loadGroups(api, Object.keys(allGroups.gridVerMap), generation);
        if (!live()) return;

        // Pinned and muted conversations are account settings shared with the phone app.
        try {
            const pins = await api.getPinConversations();
            if (!live()) return;
            this.store.replacePinned(
                pins.conversations.map((c) => threadKey(c.startsWith("g") ? "group" : "user", c.slice(1))),
            );
        } catch (err) {
            console.warn("Could not load pinned chats:", err);
        }
        try {
            const mute = await api.getMute();
            if (!live()) return;
            const entries: [string, number][] = [];
            for (const [kind, list] of [
                ["user", mute.chatEntries],
                ["group", mute.groupChatEntries],
            ] as const) {
                for (const e of list ?? []) {
                    const until = e.duration === -1 ? -1 : (e.startTime + e.duration) * 1000;
                    entries.push([threadKey(kind, String(e.id)), until]);
                }
            }
            this.store.replaceMuted(entries);
        } catch (err) {
            console.warn("Could not load muted chats:", err);
        }
        if (!live()) return;
        this.emit("threads");
    }

    private async loadGroups(api: API, groupIds: string[], generation = this.generation): Promise<void> {
        for (let i = 0; i < groupIds.length; i += GROUP_INFO_BATCH) {
            const info = await api.getGroupInfo(groupIds.slice(i, i + GROUP_INFO_BATCH));
            if (generation !== this.generation || this.api !== api) return;
            for (const [id, g] of Object.entries(info.gridInfoMap)) {
                this.store.upsertThread({ id, kind: "group", name: g.name, avatar: g.avt });
            }
        }
    }

    private setStatus(patch: Partial<SessionStatus>): void {
        this.status = { ...this.status, ...patch };
        this.emit("status", this.getStatus());
    }

    private readCredentials(): Credentials | null {
        try {
            return JSON.parse(fs.readFileSync(this.credentialsFile, "utf8")) as Credentials;
        } catch {
            return null;
        }
    }

    private writeCredentials(credentials: Credentials): void {
        fs.mkdirSync(path.dirname(this.credentialsFile), { recursive: true, mode: 0o700 });
        const tmp = `${this.credentialsFile}.tmp`;
        fs.writeFileSync(tmp, JSON.stringify(credentials), { mode: 0o600 });
        fs.renameSync(tmp, this.credentialsFile);
    }
}

function toThreadType(kind: ThreadKind): ThreadType {
    return kind === "group" ? ThreadType.Group : ThreadType.User;
}

/** When a mute set now with `option` ends, as epoch ms (-1 = until turned off). */
export function muteUntil(option: MuteOption, now = new Date()): number {
    switch (option) {
        case "1h":
            return now.getTime() + 3_600_000;
        case "4h":
            return now.getTime() + 4 * 3_600_000;
        case "forever":
            return -1;
        case "8am": {
            const next = new Date(now);
            next.setHours(8, 0, 0, 0);
            if (next <= now) next.setDate(next.getDate() + 1);
            return next.getTime();
        }
    }
}

const NETWORK_CODES = /^(E[A-Z]+|UND_ERR_[A-Z_]+)$/;

/** Errors that mean "couldn't talk to Zalo right now" rather than "Zalo rejected the session". */
export function isTransientError(err: unknown): boolean {
    for (let e: unknown = err, depth = 0; e && depth < 5; e = (e as { cause?: unknown }).cause, depth++) {
        const { message, code } = e as { message?: unknown; code?: unknown };
        if (typeof code === "string" && NETWORK_CODES.test(code)) return true;
        if (message === "fetch failed" || (typeof message === "string" && message.startsWith("Failed to fetch"))) {
            return true;
        }
        if (e instanceof Error && (e.name === "AbortError" || e.name === "TimeoutError")) return true;
    }
    return false;
}

/** A short, human-readable reason; Node's "fetch failed" alone says nothing. */
/** The host of a server URL, for the call log. */
function hostOf(url: string): string {
    try {
        return new URL(url).host;
    } catch {
        return "?";
    }
}

export function describeError(err: unknown): string {
    if (!(err instanceof Error)) return String(err);
    const cause = (err as { cause?: { code?: unknown; message?: unknown } }).cause;
    if (err.message === "fetch failed") {
        const code = typeof cause?.code === "string" ? cause.code : null;
        if (code === "ENOTFOUND" || code === "EAI_AGAIN") return "no internet connection";
        if (code === "ECONNREFUSED" || code === "ECONNRESET") return "connection refused";
        if (code?.includes("TIMEOUT") || code === "ETIMEDOUT") return "connection timed out";
        return typeof cause?.message === "string" && cause.message ? cause.message : "network error";
    }
    return err.message;
}
