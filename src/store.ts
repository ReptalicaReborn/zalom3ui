import fs from "node:fs";
import path from "node:path";

export type ThreadKind = "user" | "group";

export type QuoteInfo = {
    msgId: string;
    fromId: string;
    fromName: string;
    text: string;
};

export type MentionInfo = {
    uid: string;
    pos: number;
    len: number;
};

/**
 * The raw zca-js fields needed to quote, react to, recall, delete or mark a
 * message as seen. Kept server-side only.
 */
export type RawRef = {
    msgType: string;
    content: unknown;
    propertyExt: unknown;
    uidFrom: string;
    idTo: string;
    ts: string;
    ttl: number;
    st: number;
    at: number;
    cmd: number;
};

export type StoredMessage = {
    id: string;
    cliMsgId: string;
    threadId: string;
    threadKind: ThreadKind;
    fromId: string;
    fromName: string;
    isSelf: boolean;
    ts: number;
    msgType: string;
    /** Plain text, or a readable placeholder for non-text messages. */
    text: string;
    attachment?: Attachment;
    quote?: QuoteInfo;
    mentions?: MentionInfo[];
    recalled?: boolean;
    /** Latest reaction code per user id. */
    reactions?: Record<string, string>;
    deliveredTo?: string[];
    seenBy?: string[];
    /** Missing for messages cached before this field existed. */
    raw?: RawRef;
};

export type AttachmentKind =
    | "photo"
    | "sticker"
    | "gif"
    | "video"
    | "voice"
    | "file"
    | "link"
    | "contact"
    | "location"
    | "call"
    | "poll"
    | "other";

/** What a non-text message shows. Older cached messages only have title/href/thumb. */
export type Attachment = {
    kind?: AttachmentKind;
    title?: string;
    description?: string;
    /** The photo, video, voice clip or file itself (Zalo CDN), or the shared link. */
    href?: string;
    thumb?: string;
    width?: number;
    height?: number;
    /** Video and voice length in ms. */
    duration?: number;
    /** File size in bytes. */
    size?: number;
    fileExt?: string;
    sticker?: { id: number; catId: number; type: number };
    /** Contact cards: the user id the card is for. */
    userId?: string;
    /** Call log entries. */
    call?: { video: boolean; missed: boolean; outgoing: boolean };
    /** Poll messages: details (options, votes) come from GET /api/polls/:pollId. */
    poll?: { pollId: number; question: string; /** Set when the message reports a change (vote, close) rather than creation. */ event?: string };
    /** Call-to-action button label for OA/ZNS/ecard link cards (e.g. "ĐẶT LẠI MÃ PIN"). */
    actionLabel?: string;
    /** Detected one-time verification / OTP code (4–8 digits) for quick copy. */
    otp?: string;
};

/** Where the official client loads a sticker's picture from (its default `stickerUrl` config) by sticker id. */
export const STICKER_IMAGE_URL = "https://api.zaloapp.com/api/emoticon/sticker/webpc?size=130&eid=";

/** Call log actions the official client draws as call cards. */
const CALL_ACTION = /^recommened\.(misscall|calling|receivecall|calltime)$/;

export type ReactionSummary = {
    key: string;
    emoji: string;
    count: number;
    mine: boolean;
};

export type MessageStatus = "sent" | "delivered" | "seen";

/** What clients see. */
export type ChatMessage = Omit<StoredMessage, "raw" | "reactions" | "deliveredTo" | "seenBy"> & {
    reactions: ReactionSummary[];
    mentionsMe: boolean;
    /** Only for messages sent by this account. */
    status?: MessageStatus;
    seenCount?: number;
};

export type ThreadInfo = {
    id: string;
    kind: ThreadKind;
    name: string;
    avatar: string;
};

export type ThreadSummary = ThreadInfo & {
    lastMessage: ChatMessage | null;
    unread: number;
    pinned: boolean;
    /** Epoch ms the mute ends, -1 for "until turned off", absent when not muted. */
    mutedUntil?: number;
};

export type MessageSearchHit = {
    threadId: string;
    threadKind: ThreadKind;
    threadName: string;
    threadAvatar: string;
    message: ChatMessage;
};

/** Shape of a raw zca-js message (UserMessage/GroupMessage), kept loose on purpose. */
export type RawMessage = {
    type: number;
    threadId: string;
    isSelf: boolean;
    data: {
        msgId: string;
        cliMsgId: string;
        msgType: string;
        uidFrom: string;
        idTo?: string;
        dName: string;
        ts: string | number;
        content: unknown;
        propertyExt?: unknown;
        ttl?: number;
        st?: number;
        at?: number;
        cmd?: number;
        quote?: {
            ownerId: string | number;
            globalMsgId: string | number;
            fromD?: string;
            msg?: string;
            attach?: string;
        };
        mentions?: { uid: string; pos: number; len: number; type?: number }[];
    };
};

/**
 * Reactions offered by the API, keyed by a stable name, with Zalo's code and an emoji to display.
 * The first six are the quick reactions Zalo shows on every message.
 */
export const REACTIONS: Record<string, { code: string; emoji: string }> = {
    heart: { code: "/-heart", emoji: "❤️" },
    like: { code: "/-strong", emoji: "👍" },
    haha: { code: ":>", emoji: "😆" },
    wow: { code: ":o", emoji: "😮" },
    cry: { code: ":-((", emoji: "😭" },
    angry: { code: ":-h", emoji: "😡" },
    kiss: { code: ":-*", emoji: "😘" },
    tears_of_joy: { code: ":')", emoji: "😂" },
    dislike: { code: "/-weak", emoji: "👎" },
    rose: { code: "/-rose", emoji: "🌹" },
    broken_heart: { code: "/-break", emoji: "💔" },
    ok: { code: "/-ok", emoji: "👌" },
    pray: { code: "_()_", emoji: "🙏" },
    thanks: { code: "/-thanks", emoji: "🙏" },
    handclap: { code: ":handclap", emoji: "👏" },
};

const REACTION_BY_CODE = new Map(Object.entries(REACTIONS).map(([key, r]) => [r.code, { key, emoji: r.emoji }]));

const MAX_MESSAGES_PER_THREAD = 500;
/** Per-chat limit once a history import has run: the phone's older messages would otherwise be evicted at once. */
export const HISTORY_MESSAGES_PER_THREAD = 2000;

const KIND_BY_MSG_TYPE: Record<string, AttachmentKind> = {
    "chat.photo": "photo",
    "chat.sticker": "sticker",
    "chat.gif": "gif",
    "chat.video.msg": "video",
    "chat.voice": "voice",
    "share.file": "file",
    "chat.link": "link",
    "chat.location.new": "location",
};

const MSG_TYPE_LABELS: Record<string, string> = {
    "chat.photo": "[Photo]",
    "chat.video.msg": "[Video]",
    "chat.voice": "[Voice message]",
    "chat.sticker": "[Sticker]",
    "chat.gif": "[GIF]",
    "share.file": "[File]",
    "chat.recommended": "[Link]",
    "chat.link": "[Link]",
    "chat.location.new": "[Location]",
    "chat.doodle": "[Drawing]",
    "chat.ecard": "[Card]",
    "chat.list.action": "[Card]",
    "chat.webcontent": "[Card]",
    "chat.webcontent.v2": "[Card]",
    "chat.todo": "[Task]",
    "chat.poll": "[Poll]",
    "group.poll": "[Poll]",
};

const OTP_KEYWORD =
    /\b(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich|pin)|verification\s*code|security\s*code|auth(?:entication)?\s*code|confirm(?:ation)?\s*code|passcode|one[- ]time(?:\s*(?:code|password|pin))?)\b/i;

function foldDiacritics(s: string): string {
    return s.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[đĐ]/g, "d");
}

/**
 * Extracts a 4–8 digit OTP / verification code when accompanied by an OTP keyword (Vietnamese or English).
 * Ignores 9+ digit phone numbers / account IDs (e.g. `0378795621`) and currency amounts.
 */
export function extractOtpCode(text: string): string | undefined {
    const folded = foldDiacritics(text);
    if (!OTP_KEYWORD.test(folded)) return undefined;
    const afterKeyword =
        /\b(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich|pin)|verification\s*code|security\s*code|auth(?:entication)?\s*code|confirm(?:ation)?\s*code|passcode|one[- ]time(?:\s*(?:code|password|pin))?)\b[^\d\n]{0,40}?(?<!\d)(\d{4,8})(?!\d)/i;
    const direct = afterKeyword.exec(folded);
    if (direct?.[1]) return direct[1];
    const beforeKeyword =
        /(?<!\d)(\d{4,8})(?!\d)[^\d\n]{0,24}\b(?:la\s+)?(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich)|verification\s*code|security\s*code|passcode)\b/i;
    const reverse = beforeKeyword.exec(folded);
    if (reverse?.[1]) return reverse[1];
    return undefined;
}

export function threadKey(kind: ThreadKind, id: string): string {
    return `${kind}:${id}`;
}

/** The message id a raw recall ("undo") record points at, if it is one. */
export function recalledMessageId(raw: RawMessage): string | null {
    const content = raw.data.content as { deleteMsg?: unknown; globalMsgId?: unknown } | null;
    if (content && typeof content === "object" && "deleteMsg" in content) return String(content.globalMsgId);
    return null;
}

export function normalizeMessage(raw: RawMessage): StoredMessage {
    const { data } = raw;
    const threadKind: ThreadKind = raw.type === 1 ? "group" : "user";
    const msg: StoredMessage = {
        id: String(data.msgId),
        cliMsgId: String(data.cliMsgId),
        threadId: String(raw.threadId),
        threadKind,
        fromId: String(data.uidFrom),
        fromName: data.dName ?? "",
        isSelf: raw.isSelf,
        ts: Number(data.ts),
        msgType: data.msgType,
        text: "",
        raw: {
            msgType: data.msgType,
            content: data.content,
            propertyExt: data.propertyExt,
            uidFrom: String(data.uidFrom),
            idTo: String(data.idTo ?? ""),
            ts: String(data.ts),
            ttl: data.ttl ?? 0,
            st: data.st ?? 0,
            at: data.at ?? 0,
            cmd: data.cmd ?? 0,
        },
    };

    if (data.quote) {
        msg.quote = {
            msgId: String(data.quote.globalMsgId),
            fromId: String(data.quote.ownerId),
            fromName: data.quote.fromD ?? "",
            text: data.quote.msg || "[Attachment]",
        };
    }
    if (data.mentions?.length) {
        msg.mentions = data.mentions.map(({ uid, pos, len }) => ({ uid: String(uid), pos, len }));
    }

    if (typeof data.content === "string") {
        msg.text = data.content;
        return msg;
    }

    const content: Record<string, unknown> = Array.isArray(data.content)
        ? { items: data.content }
        : ((data.content ?? {}) as Record<string, unknown>);
    const attachment = parseAttachment(data.msgType, content);
    if (attachment) msg.attachment = attachment;
    if (attachment?.kind === "call") {
        msg.text = `[${attachment.title}]`;
        return msg;
    }
    if (attachment?.kind === "poll") {
        msg.text = `[Poll] ${attachment.title}`;
        return msg;
    }
    if (data.msgType === "chat.webcontent" || data.msgType === "chat.webcontent.v2" || data.msgType === "chat.list.action") {
        const preview = attachment?.title || attachment?.description;
        if (preview) {
            msg.text = preview;
            return msg;
        }
    }
    const known = attachment?.kind === "contact" ? "[Contact]" : MSG_TYPE_LABELS[data.msgType];
    // Notices and cards of a type we don't model (login alerts, group events): show their own text, not a placeholder.
    if (!known && attachment?.title) {
        msg.text = attachment.title;
        return msg;
    }
    const label = known ?? "[Unsupported message]";
    // Photos and stickers have no useful title; files, links and cards are named by theirs.
    const named = attachment?.title && attachment.kind !== "photo" && attachment.kind !== "gif";
    msg.text = named ? `${label} ${attachment.title}` : label;
    return msg;
}

/**
 * Reads what a photo, sticker, video, voice clip, file, link, card or location message shows. Zalo puts most details in
 * `content.params`, a JSON string whose keys differ by type (and over time), so every field is optional.
 */
export function parseAttachment(msgType: string, content: Record<string, unknown>): Attachment | undefined {
    const str = (v: unknown) => (typeof v === "string" && v.length > 0 ? v : undefined);
    const num = (...vs: unknown[]) => {
        for (const v of vs) {
            const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : NaN;
            if (Number.isFinite(n) && n > 0) return n;
        }
        return undefined;
    };
    let params: Record<string, unknown> = {};
    if (typeof content.params === "string" && content.params.startsWith("{")) {
        try {
            const parsed = JSON.parse(content.params);
            if (parsed && typeof parsed === "object") params = parsed as Record<string, unknown>;
        } catch {
            // Not JSON: a contact card keeps the user id here.
        }
    }

    if (msgType === "chat.sticker") {
        const id = num(content.id);
        const catId = num(content.catId, content.cateId);
        if (id === undefined) return undefined;
        return { kind: "sticker", thumb: STICKER_IMAGE_URL + id, sticker: { id, catId: catId ?? 0, type: num(content.type) ?? 0 } };
    }

    // Poll: the id and question sit at the top level or in params, depending on the Zalo version.
    if (msgType === "group.poll" || msgType === "chat.poll" || content.poll_id !== undefined || params.pollId !== undefined) {
        const pollId = num(content.poll_id, content.pollId, params.pollId, params.poll_id);
        if (pollId === undefined) return undefined;
        const question = str(content.question) ?? str(params.question) ?? str(content.title) ?? str(params.title) ?? "Poll";
        const act = str(content.action);
        const event = act && act !== "create" && act !== "group.poll" && act !== "chat.poll" ? act : undefined;
        return { kind: "poll", title: question, poll: { pollId, question, ...(event ? { event } : {}) } };
    }

    // Call log: params {calltype (1 = video), isCaller, reason (1 busy, 2 no answer, 3 declined, 4/5 cancelled), duration (s)}.
    const action = str(content.action) ?? "";
    if (CALL_ACTION.test(action)) {
        const missed = action.endsWith("misscall");
        const video = Number(params.calltype) === 1;
        const outgoing = params.isCaller !== undefined && params.isCaller !== null ? Boolean(params.isCaller) : action.endsWith("calling");
        const reason = Number(params.reason) || 0;
        const what = video ? "video call" : "voice call";
        const title = outgoing
            ? !missed
                ? `Outgoing ${what}`
                : reason === 4 || reason === 5
                  ? "You cancelled the call"
                  : reason === 3
                    ? "Call declined"
                    : reason === 1
                      ? "Busy"
                      : `Outgoing ${what}, no answer`
            : !missed
              ? `Incoming ${what}`
              : reason === 3
                ? "You declined the call"
                : `Missed ${what}`;
        const seconds = num(params.duration, content.duration);
        const call: Attachment = { kind: "call", title, call: { video, missed, outgoing } };
        if (!missed && seconds) call.duration = Math.round(seconds * 1000);
        return call;
    }

    const parseJsonObj = (v: unknown): Record<string, unknown> => {
        if (v && typeof v === "object" && !Array.isArray(v)) return v as Record<string, unknown>;
        if (typeof v === "string" && v.trim().startsWith("{")) {
            try {
                const p = JSON.parse(v);
                if (p && typeof p === "object" && !Array.isArray(p)) return p as Record<string, unknown>;
            } catch {
                // ignore
            }
        }
        return {};
    };
    const webUrl = (v: unknown): string | undefined => {
        const s = str(v)?.trim();
        return s && /^https?:\/\/[^\s/]/i.test(s) ? s : undefined;
    };

    // OA list / interactive card messages (`chat.list.action`): array of header + child parts, or single object.
    if (msgType === "chat.list.action") {
        const items = Array.isArray(content.items)
            ? content.items.filter((x): x is Record<string, unknown> => Boolean(x && typeof x === "object"))
            : [content];
        const lead = items.find((x) => typeof x.type === "string" && /^l\.a\.header\./.test(x.type)) ?? items[0];
        if (lead) {
            const title = str(lead.title) ?? items.map((x) => str(x.title)).find(Boolean);
            const description = str(lead.description) ?? items.map((x) => str(x.description)).find((d) => d && d !== title);
            const thumb = webUrl(lead.thumb) ?? items.map((x) => webUrl(x.thumb)).find(Boolean);
            const actionItem = items.find((x) => x !== lead && webUrl(x.href)) ?? items.find((x) => webUrl(x.href));
            const href = webUrl(lead.href) ?? (actionItem ? webUrl(actionItem.href) : undefined);
            const actionLabel = actionItem && actionItem !== lead ? str(actionItem.title) : undefined;
            const otp = extractOtpCode([title, description].filter(Boolean).join("\n"));
            if (title || description || thumb || href) {
                const card: Attachment = {
                    kind: "link",
                    ...(title ? { title } : {}),
                    ...(description && description !== title ? { description } : {}),
                    ...(thumb ? { thumb } : {}),
                    ...(href ? { href } : {}),
                    ...(actionLabel ? { actionLabel } : {}),
                    ...(otp ? { otp } : {}),
                };
                return card;
            }
        }
    }

    // ZInstant / ZNS cards (`chat.webcontent`, `chat.webcontent.v2`): OA notifications, PIN resets, OTPs, order updates.
    if (msgType === "chat.webcontent" || msgType === "chat.webcontent.v2") {
        const zMsg = parseJsonObj(content.zinstant_msg);
        const meta = Object.keys(zMsg).length ? zMsg : params;
        const customObj = parseJsonObj(meta.customMsg ?? params.customMsg);
        const msgMap = parseJsonObj(customObj.msg);
        let localized = str(msgMap.vi) ?? str(msgMap.en) ?? "";
        if (localized && Array.isArray(customObj.highLightsV2)) {
            customObj.highLightsV2.forEach((h, idx) => {
                const ho = parseJsonObj(h);
                const replacement = str(ho.text) ?? str(ho.value) ?? str(ho.name) ?? "";
                if (replacement) localized = localized.replace(`$${idx}$`, replacement);
            });
        }
        const rawActions = Array.isArray(meta.actions)
            ? meta.actions
            : Array.isArray(params.actions)
              ? params.actions
              : Array.isArray(meta.buttons)
                ? meta.buttons
                : [];
        let actionLabel = str(meta.actionTitle) ?? str(params.actionTitle);
        let href = webUrl(content.href) ?? webUrl(meta.href) ?? webUrl(meta.url) ?? webUrl(params.href) ?? webUrl(params.url);
        let otpFromAction: string | undefined;
        for (const entry of rawActions) {
            const act = parseJsonObj(entry);
            const actData = parseJsonObj(act.actionData ?? act.data);
            const label = str(act.actionTitle) ?? str(act.title) ?? str(act.name);
            const url = webUrl(act.actionUrl) ?? webUrl(act.href) ?? webUrl(act.url) ?? webUrl(actData.url) ?? webUrl(actData.href) ?? webUrl(actData.link);
            if (!actionLabel && label) actionLabel = label;
            if (!href && url) href = url;
            const candidateCode = str(actData.otp) ?? str(actData.code) ?? str(actData.copyText) ?? str(actData.text);
            if (!otpFromAction && candidateCode && /^\d{4,8}$/.test(candidateCode.trim())) {
                otpFromAction = candidateCode.trim();
            }
        }
        const lines = localized
            .split("\n")
            .map((l) => l.trim())
            .filter(Boolean);
        const title = str(content.title) ?? str(meta.title) ?? lines[0];
        const bodyFromLocalized = lines.length > 1 ? lines.slice(1).join("\n") : lines[0];
        const rawDesc = str(content.description) ?? str(meta.description) ?? bodyFromLocalized;
        const description = rawDesc && rawDesc !== title ? rawDesc : undefined;
        const thumb = webUrl(content.thumb) ?? webUrl(meta.thumb) ?? webUrl(meta.logo) ?? webUrl(params.thumb);
        const otp = otpFromAction ?? extractOtpCode([title, description, localized].filter(Boolean).join("\n"));
        if (title || description || thumb || href || actionLabel || otp) {
            return {
                kind: "link",
                ...(title ? { title } : {}),
                ...(description ? { description } : {}),
                ...(thumb ? { thumb } : {}),
                ...(href ? { href } : {}),
                ...(actionLabel ? { actionLabel } : {}),
                ...(otp ? { otp } : {}),
            };
        }
    }

    // E-card (`chat.ecard`): greeting/event cards with optional CTA buttons.
    if (msgType === "chat.ecard") {
        const title = str(content.title);
        const description = str(content.description);
        const thumb = webUrl(content.thumb) ?? webUrl(content.href);
        const href = webUrl(content.href);
        const actions = Array.isArray(params.actions) ? params.actions : [];
        const firstAction = actions.length ? parseJsonObj(actions[0]) : {};
        const actionLabel = str(firstAction.name) ?? str(firstAction.title);
        if (title || description || thumb) {
            return {
                kind: "link",
                ...(title ? { title } : {}),
                ...(description && description !== title ? { description } : {}),
                ...(thumb ? { thumb } : {}),
                ...(href ? { href } : {}),
                ...(actionLabel ? { actionLabel } : {}),
            };
        }
    }

    let kind: AttachmentKind = KIND_BY_MSG_TYPE[msgType] ?? "other";
    if (msgType === "chat.recommended") {
        const action = str(content.action) ?? "";
        kind = action === "recommened.user" ? "contact" : action === "recommened.link" ? "link" : "other";
    }
    const isLocalUri = (s: string) => /^(content:\/\/|file:\/\/|\/(?:storage|sdcard|data|mnt)\/)/i.test(s.trim());
    const isMedia = kind === "photo" || kind === "gif" || kind === "video";
    const remoteStr = (v: unknown) => {
        const s = str(v);
        return s && !isLocalUri(s) ? s : undefined;
    };
    const rawTitle = str(content.title);
    const attachment: Attachment = {
        kind,
        title: isMedia && rawTitle && isLocalUri(rawTitle) ? undefined : rawTitle,
        description: str(content.description),
        // A photo's "hd" copy is the full-size one when Zalo sends it; ignore local phone URIs from history backups.
        href: isMedia
            ? (kind === "photo" ? remoteStr(params.hd) : undefined) ??
              remoteStr(content.href) ??
              remoteStr(params.oriUrl) ??
              remoteStr(params.origUrl) ??
              remoteStr(params.normalUrl) ??
              remoteStr(params.url)
            : str(content.href),
        thumb: isMedia ? remoteStr(content.thumb) ?? remoteStr(params.thumbUrl) ?? remoteStr(params.thumb) : str(content.thumb),
        width: num(params.width, params.video_width, params.w),
        height: num(params.height, params.video_height, params.h),
        duration: num(params.duration),
        size: num(params.fileSize, params.size),
        fileExt: str(params.fileExt)?.toLowerCase(),
    };
    if (kind === "contact" && typeof content.params === "string" && /^\d+$/.test(content.params)) {
        attachment.userId = content.params;
    }
    if (kind === "file" && !attachment.fileExt && attachment.title?.includes(".")) {
        attachment.fileExt = attachment.title.split(".").pop()?.toLowerCase();
    }
    for (const key of Object.keys(attachment) as (keyof Attachment)[]) {
        if (attachment[key] === undefined) delete attachment[key];
    }
    const shows = attachment.title || attachment.href || attachment.thumb || attachment.description;
    return shows || kind !== "other" ? attachment : undefined;
}

/** `meta.json`: everything but the messages. */
type Meta = {
    version?: number;
    threads: Record<string, ThreadInfo>;
    readUpTo: Record<string, number>;
    pinned?: string[];
    mutedUntil?: Record<string, number>;
    markedUnread?: string[];
    /** Per-chat message limit, raised by a history import. */
    messageCap?: number;
};

/** The old single-file cache (`messages.json`), read once to move it to the per-thread layout. */
type Snapshot = Meta & { messages: Record<string, StoredMessage[]> };

/** `threads/<hex of the thread key>.json`: one chat's messages. */
type ThreadFile = { key: string; messages: StoredMessage[] };

const META_FILE = "meta.json";
const THREADS_DIR = "threads";
const THREAD_FILE = /^[0-9a-f]+\.json$/;

/** A message's text, title and description folded for search, with the strings they were folded from. */
type SearchText = {
    text: string;
    title: string | undefined;
    description: string | undefined;
    foldedText: string;
    foldedTitle: string | undefined;
    foldedDescription: string | undefined;
};

/**
 * In-memory cache of threads and messages, saved to disk.
 *
 * Zalo Web has no history endpoint for 1:1 chats, so this cache is the
 * client's only long-term history for direct messages.
 *
 * Given `data/messages.json`, it saves to `data/messages/`: `meta.json` (threads, read markers, pins, mutes, the
 * message limit) and one `threads/<hex of the thread key>.json` per chat. A save writes only the files whose contents
 * changed, so a new message rewrites its own chat and not the whole cache. An old single-file `messages.json` is
 * moved to this layout on first load and kept as `messages.json.migrated`.
 */
export class ChatStore {
    private threads = new Map<string, ThreadInfo>();
    private messages = new Map<string, StoredMessage[]>();
    private readUpTo = new Map<string, number>();
    private pinned = new Set<string>();
    private mutedUntil = new Map<string, number>();
    private markedUnread = new Set<string>();
    /** message id -> thread key */
    private index = new Map<string, string>();
    private saveTimer: NodeJS.Timeout | null = null;
    ownId: string | null = null;
    private messageCap = MAX_MESSAGES_PER_THREAD;
    /** Unread incoming messages per thread at the read marker they were counted at; dropped when the thread's messages change. */
    private unreadCounts = new Map<string, { readTs: number; count: number }>();
    /** Folded search text per message, so a search does not fold every message again. Never saved. */
    private searchText = new WeakMap<StoredMessage, SearchText>();
    /** The per-thread layout, in a directory beside the old single file; null for a cache kept in memory only. */
    private readonly paths: { meta: string; threads: string } | null;
    /** Threads whose file must be rewritten (or removed when they have no messages left). */
    private dirtyThreads = new Set<string>();
    private metaDirty = false;
    /** After [clear]: remove every thread file, including ones this process never loaded. */
    private wipe = false;
    /** What an async save is writing now. A sync flush writes these again, as that save may not finish before exit. */
    private writingThreads = new Set<string>();
    private writingMeta = false;
    private saving = false;
    /** The latest write (or removal) of each file. An async write overtaken by a newer one is dropped, not renamed in. */
    private fileSeq = new Map<string, number>();
    private seq = 0;
    /** While moving an old single-file cache: threads not written yet. meta.json, which marks the move done, waits for them. */
    private pendingMigration: Set<string> | null = null;

    constructor(private readonly file: string | null = null) {
        const dir = file === null ? null : file.endsWith(".json") ? file.slice(0, -".json".length) : `${file}.d`;
        this.paths = dir === null ? null : { meta: path.join(dir, META_FILE), threads: path.join(dir, THREADS_DIR) };
        this.load();
    }

    upsertThread(info: ThreadInfo): void {
        const key = threadKey(info.kind, info.id);
        const prev = this.threads.get(key);
        this.threads.set(key, {
            ...info,
            name: info.name || prev?.name || info.id,
            avatar: info.avatar || prev?.avatar || "",
        });
        this.metaChanged();
    }

    getThread(kind: ThreadKind, id: string): ThreadInfo | undefined {
        return this.threads.get(threadKey(kind, id));
    }

    /** Threads still named by their raw id (e.g. created by a history import for someone who is not a friend). */
    unnamedThreads(): ThreadInfo[] {
        return [...this.threads.values()].filter((t) => !t.name || t.name === t.id);
    }

    /** True when the thread has no live messages (every message came from a history import, or it is empty). */
    hasOnlyHistory(kind: ThreadKind, id: string): boolean {
        const list = this.messages.get(threadKey(kind, id));
        return !list || list.every((m) => !m.raw);
    }

    /**
     * Removes an unnamed thread that holds only imported history (e.g. from an earlier sync before plain ids were
     * mapped to noised ids), freeing its message ids in the dedupe index so a fresh import into the real thread wins.
     */
    dropPlainThread(kind: ThreadKind, plainId: string): boolean {
        const key = threadKey(kind, plainId);
        const info = this.threads.get(key);
        if (!info || (info.name && info.name !== info.id)) return false;
        if (!this.hasOnlyHistory(kind, plainId)) return false;
        const list = this.messages.get(key) ?? [];
        for (const m of list) {
            if (this.index.get(m.id) === key) this.index.delete(m.id);
        }
        this.messages.delete(key);
        this.threadChanged(key);
        this.threads.delete(key);
        this.readUpTo.delete(key);
        this.markedUnread.delete(key);
        this.pinned.delete(key);
        this.mutedUntil.delete(key);
        this.metaChanged();
        return true;
    }

    /**
     * Moves messages from an unnamed plain-id history thread into its real noised-id thread and removes the old thread.
     * Preserves read state so migrated history never becomes unread.
     */
    migrateThread(kind: ThreadKind, fromId: string, toId: string): boolean {
        if (!fromId || !toId || fromId === toId) return false;
        const fromKey = threadKey(kind, fromId);
        const toKey = threadKey(kind, toId);
        const oldInfo = this.threads.get(fromKey);
        if (!oldInfo || (oldInfo.name && oldInfo.name !== oldInfo.id)) return false;
        if (!this.hasOnlyHistory(kind, fromId)) return false;

        const oldList = this.messages.get(fromKey) ?? [];
        const targetInfo = this.threads.get(toKey);
        const targetList = this.messages.get(toKey) ?? [];
        const existingIds = new Set(targetList.map((m) => m.id));

        for (const m of oldList) {
            if (this.index.get(m.id) === fromKey) this.index.delete(m.id);
            if (existingIds.has(m.id) || this.index.has(m.id)) continue;
            const moved: StoredMessage = {
                ...m,
                threadId: toId,
                ...(kind === "user" && !m.isSelf
                    ? { fromId: toId, fromName: m.fromName || (targetInfo && targetInfo.name !== toId ? targetInfo.name : "") }
                    : {}),
            };
            targetList.push(moved);
            existingIds.add(moved.id);
            this.index.set(moved.id, toKey);
        }
        targetList.sort((a, b) => a.ts - b.ts);
        if (targetList.length > this.messageCap) {
            for (const dropped of targetList.splice(0, targetList.length - this.messageCap)) {
                if (this.index.get(dropped.id) === toKey) this.index.delete(dropped.id);
            }
        }
        if (targetList.length > 0) this.messages.set(toKey, targetList);
        this.threadChanged(toKey);
        if (!targetInfo) {
            this.threads.set(toKey, { id: toId, kind, name: toId, avatar: "" });
        }

        const mergedRead = Math.max(this.readUpTo.get(toKey) ?? 0, this.readUpTo.get(fromKey) ?? 0);
        if (mergedRead > 0) this.readUpTo.set(toKey, mergedRead);

        this.messages.delete(fromKey);
        this.threadChanged(fromKey);
        this.threads.delete(fromKey);
        this.readUpTo.delete(fromKey);
        this.markedUnread.delete(fromKey);
        this.pinned.delete(fromKey);
        this.mutedUntil.delete(fromKey);
        this.metaChanged();
        return true;
    }

    /** Lets chats keep up to `cap` messages (saved with the cache), for imported history. Never lowers the limit. */
    allowMessages(cap: number): void {
        if (cap > this.messageCap) {
            this.messageCap = cap;
            this.metaChanged();
        }
    }

    /**
     * Adds a message; returns false if it was already stored. `history`: an imported old message, which changes no
     * read state (see [settleHistory]).
     */
    addMessage(msg: StoredMessage, opts: { history?: boolean } = {}): boolean {
        if (this.index.has(msg.id)) return false;
        const key = threadKey(msg.threadKind, msg.threadId);
        const list = this.messages.get(key) ?? [];

        // Messages mostly arrive in order; insert from the end to keep the list sorted by ts.
        let i = list.length;
        while (i > 0 && list[i - 1].ts > msg.ts) i--;
        list.splice(i, 0, msg);
        this.index.set(msg.id, key);
        let evictedSelf = false;
        let evictedOthers = false;
        if (list.length > this.messageCap) {
            for (const dropped of list.splice(0, list.length - this.messageCap)) {
                this.index.delete(dropped.id);
                if (dropped.id === msg.id) evictedSelf = true;
                else evictedOthers = true;
            }
        }
        this.messages.set(key, list);
        // Only this message falling off at once leaves the list as it was.
        if (!evictedSelf || evictedOthers) this.threadChanged(key);
        if (evictedSelf) return false;

        const existingThread = this.threads.get(key);
        if (!existingThread) {
            this.threads.set(key, {
                id: msg.threadId,
                kind: msg.threadKind,
                // For a 1:1/OA chat not in the friend list, their display name is the best we have.
                name: msg.threadKind === "user" && !msg.isSelf ? msg.fromName || msg.threadId : msg.threadId,
                avatar: "",
            });
            this.metaChanged();
        } else if (msg.threadKind === "user" && !msg.isSelf && msg.fromName && existingThread.name === existingThread.id) {
            existingThread.name = msg.fromName;
            this.metaChanged();
        }
        if (opts.history) return true;
        const prevReadUpTo = this.readUpTo.get(key) ?? 0;
        if (msg.isSelf) {
            if (msg.ts > prevReadUpTo) this.markRead(msg.threadKind, msg.threadId, msg.ts);
        } else if (msg.ts > prevReadUpTo) {
            // A new incoming message supersedes a manual "mark as unread".
            if (this.markedUnread.delete(key)) this.metaChanged();
        }
        return true;
    }

    /**
     * After importing old messages into a chat: marks them read, up to just before the first message that was there
     * already and is unread, so history never shows up as new unread messages.
     */
    settleHistory(kind: ThreadKind, id: string, importedIds: ReadonlySet<string>): void {
        const key = threadKey(kind, id);
        const list = this.messages.get(key) ?? [];
        const readTs = this.readUpTo.get(key) ?? 0;
        const firstLiveUnread = list.find((m) => !importedIds.has(m.id) && !m.isSelf && m.ts > readTs)?.ts ?? Infinity;
        let upTo = readTs;
        for (const m of list) if (importedIds.has(m.id) && m.ts < firstLiveUnread && m.ts > upTo) upTo = m.ts;
        if (upTo > readTs) {
            this.readUpTo.set(key, upTo);
            this.metaChanged();
        }
    }

    getMessage(msgId: string): StoredMessage | undefined {
        const key = this.index.get(msgId);
        return key ? this.messages.get(key)?.find((m) => m.id === msgId) : undefined;
    }

    getMessages(kind: ThreadKind, id: string, opts: { before?: number; limit?: number } = {}): ChatMessage[] {
        const limit = Math.min(Math.max(opts.limit ?? 50, 1), MAX_MESSAGES_PER_THREAD);
        let list = this.messages.get(threadKey(kind, id)) ?? [];
        if (opts.before !== undefined) list = list.filter((m) => m.ts < opts.before!);
        return list.slice(-limit).map((m) => this.toPublic(m));
    }

    /**
     * Searches cached messages across all conversations with diacritic-insensitive, case-insensitive matching.
     * Skips recalled messages and returns hits sorted newest-first, capped to [1, 100].
     */
    searchMessages(query: string, limit = 50): MessageSearchHit[] {
        const needle = foldForSearch(query.trim());
        if (!needle) return [];
        const cap = Math.min(Math.max(1, Number.isFinite(limit) ? Math.floor(limit) : 50), 100);
        const matches: { thread: ThreadInfo | undefined; msg: StoredMessage }[] = [];
        for (const [key, list] of this.messages) {
            const thread = this.threads.get(key);
            for (const m of list) {
                if (m.recalled) continue;
                const s = this.searchTextOf(m);
                const hit =
                    s.foldedText.includes(needle) ||
                    (s.foldedTitle !== undefined && s.foldedTitle.includes(needle)) ||
                    (s.foldedDescription !== undefined && s.foldedDescription.includes(needle));
                if (hit) matches.push({ thread, msg: m });
            }
        }
        matches.sort((a, b) => b.msg.ts - a.msg.ts);
        return matches.slice(0, cap).map(({ thread, msg }) => ({
            threadId: msg.threadId,
            threadKind: msg.threadKind,
            threadName: thread?.name || msg.threadId,
            threadAvatar: thread?.avatar ?? "",
            message: this.toPublic(msg),
        }));
    }

    /** Sets (or with an empty code, clears) one user's reaction. Returns the updated message. */
    applyReaction(msgId: string, uid: string, code: string): StoredMessage | undefined {
        return this.update(msgId, (m) => {
            const reactions = { ...m.reactions };
            if (code) reactions[uid] = code;
            else delete reactions[uid];
            m.reactions = reactions;
        });
    }

    recall(msgId: string): StoredMessage | undefined {
        return this.update(msgId, (m) => {
            m.recalled = true;
            m.text = "Message recalled";
            delete m.attachment;
            delete m.quote;
            delete m.mentions;
            delete m.reactions;
        });
    }

    removeMessage(msgId: string): StoredMessage | undefined {
        const key = this.index.get(msgId);
        const list = key ? this.messages.get(key) : undefined;
        const i = list?.findIndex((m) => m.id === msgId) ?? -1;
        if (!key || !list || i < 0) return undefined;
        const [removed] = list.splice(i, 1);
        this.index.delete(msgId);
        this.threadChanged(key);
        return removed;
    }

    /**
     * Records delivery/seen receipts for `msgId`. In 1:1 chats a receipt covers every earlier
     * message we sent; in groups it is per message. Returns the messages that changed.
     */
    applyReceipt(
        kind: ThreadKind,
        threadId: string,
        msgId: string,
        receipt: { deliveredTo?: string[]; seenBy?: string[] },
    ): StoredMessage[] {
        const key = threadKey(kind, threadId);
        const list = this.messages.get(key) ?? [];
        const target = list.find((m) => m.id === msgId);
        if (!target) return [];
        const affected = kind === "user" ? list.filter((m) => m.isSelf && m.ts <= target.ts) : [target];

        const changed: StoredMessage[] = [];
        for (const m of affected) {
            const deliveredTo = union(m.deliveredTo, receipt.deliveredTo);
            const seenBy = union(m.seenBy, receipt.seenBy);
            if (deliveredTo.length === (m.deliveredTo?.length ?? 0) && seenBy.length === (m.seenBy?.length ?? 0)) continue;
            m.deliveredTo = deliveredTo;
            m.seenBy = seenBy;
            changed.push(m);
        }
        if (changed.length) this.markDirty(key);
        return changed;
    }

    /** Incoming messages after the read marker, i.e. the ones a "seen" receipt should cover. */
    unreadMessages(kind: ThreadKind, id: string): StoredMessage[] {
        const key = threadKey(kind, id);
        const readTs = this.readUpTo.get(key) ?? 0;
        return (this.messages.get(key) ?? []).filter((m) => !m.isSelf && m.ts > readTs);
    }

    /** Returns whether the thread had been manually marked as unread. */
    markRead(kind: ThreadKind, id: string, upTo: number = Date.now()): boolean {
        const key = threadKey(kind, id);
        const wasMarkedUnread = this.markedUnread.delete(key);
        const advanced = (this.readUpTo.get(key) ?? 0) < upTo;
        if (advanced) this.readUpTo.set(key, upTo);
        if (wasMarkedUnread || advanced) this.metaChanged();
        return wasMarkedUnread;
    }

    markUnread(kind: ThreadKind, id: string): void {
        this.markedUnread.add(threadKey(kind, id));
        this.metaChanged();
    }

    setPinned(kind: ThreadKind, id: string, pinned: boolean): void {
        const key = threadKey(kind, id);
        if (pinned) this.pinned.add(key);
        else this.pinned.delete(key);
        this.metaChanged();
    }

    replacePinned(keys: string[]): void {
        this.pinned = new Set(keys);
        this.metaChanged();
    }

    /** `until`: epoch ms, -1 for indefinitely, null to unmute. */
    setMuted(kind: ThreadKind, id: string, until: number | null): void {
        const key = threadKey(kind, id);
        if (until === null) this.mutedUntil.delete(key);
        else this.mutedUntil.set(key, until);
        this.metaChanged();
    }

    replaceMuted(entries: [string, number][]): void {
        this.mutedUntil = new Map(entries);
        this.metaChanged();
    }

    isMuted(kind: ThreadKind, id: string, now = Date.now()): boolean {
        const until = this.mutedUntil.get(threadKey(kind, id));
        return until !== undefined && (until === -1 || until > now);
    }

    /** Drops a conversation's messages (after deleting it on Zalo). */
    clearThreadMessages(kind: ThreadKind, id: string): void {
        const key = threadKey(kind, id);
        for (const m of this.messages.get(key) ?? []) this.index.delete(m.id);
        this.messages.delete(key);
        this.threadChanged(key);
        this.markedUnread.delete(key);
        this.metaChanged();
    }

    /** Forgets a conversation entirely (a group I left): its messages, pin, mute and read state. */
    removeThread(kind: ThreadKind, id: string): void {
        const key = threadKey(kind, id);
        this.clearThreadMessages(kind, id);
        this.threads.delete(key);
        this.readUpTo.delete(key);
        this.pinned.delete(key);
        this.mutedUntil.delete(key);
        this.metaChanged();
    }

    lastMessage(kind: ThreadKind, id: string): StoredMessage | undefined {
        return this.messages.get(threadKey(kind, id))?.at(-1);
    }

    listThreads(now = Date.now()): ThreadSummary[] {
        const out: ThreadSummary[] = [];
        for (const [key, info] of this.threads) {
            const list = this.messages.get(key) ?? [];
            const unread = this.unreadCount(key, list);
            const last = list.at(-1);
            const summary: ThreadSummary = {
                ...info,
                lastMessage: last ? this.toPublic(last) : null,
                unread: Math.max(unread, this.markedUnread.has(key) ? 1 : 0),
                pinned: this.pinned.has(key),
            };
            const muted = this.mutedUntil.get(key);
            if (muted !== undefined && (muted === -1 || muted > now)) summary.mutedUntil = muted;
            out.push(summary);
        }
        return out.sort(
            (a, b) =>
                Number(b.pinned) - Number(a.pinned) ||
                (b.lastMessage?.ts ?? 0) - (a.lastMessage?.ts ?? 0) ||
                a.name.localeCompare(b.name),
        );
    }

    toPublic(m: StoredMessage): ChatMessage {
        const { raw: _raw, reactions, deliveredTo, seenBy, ...rest } = m;
        const out: ChatMessage = {
            ...rest,
            reactions: summarizeReactions(reactions, this.ownId),
            mentionsMe: !m.isSelf && (m.mentions ?? []).some((x) => x.uid === this.ownId || x.uid === "-1"),
        };
        if (m.isSelf) {
            out.status = seenBy?.length ? "seen" : deliveredTo?.length ? "delivered" : "sent";
            if (m.threadKind === "group") out.seenCount = seenBy?.length ?? 0;
        }
        return out;
    }

    clear(): void {
        this.threads.clear();
        this.messages.clear();
        this.readUpTo.clear();
        this.pinned.clear();
        this.mutedUntil.clear();
        this.markedUnread.clear();
        this.index.clear();
        this.unreadCounts.clear();
        this.wipe = true;
        this.metaChanged();
    }

    /**
     * Saves what changed, synchronously: for shutdown, logout and fatal errors. Otherwise changes are saved in the
     * background a moment after they happen.
     */
    flush(): void {
        if (this.saveTimer) {
            clearTimeout(this.saveTimer);
            this.saveTimer = null;
        }
        // A background save still writing these may not finish before the process exits: write them again now.
        for (const key of this.writingThreads) this.dirtyThreads.add(key);
        if (this.writingMeta) this.metaDirty = true;
        this.saveSync();
    }

    private update(msgId: string, fn: (m: StoredMessage) => void): StoredMessage | undefined {
        const key = this.index.get(msgId);
        const m = this.getMessage(msgId);
        if (!key || !m) return undefined;
        fn(m);
        this.searchText.delete(m);
        this.markDirty(key);
        return m;
    }

    /** The thread's messages changed (added, removed, moved or evicted). */
    private threadChanged(key: string): void {
        this.unreadCounts.delete(key);
        this.markDirty(key);
    }

    /** Something saved in the thread's file changed. */
    private markDirty(key: string): void {
        if (!this.paths) return;
        this.dirtyThreads.add(key);
        this.scheduleSave();
    }

    /** Threads, read markers, pins, mutes or the message limit changed. */
    private metaChanged(): void {
        if (!this.paths) return;
        this.metaDirty = true;
        this.scheduleSave();
    }

    /** Incoming messages after the read marker, as `list.filter((m) => !m.isSelf && m.ts > readTs).length`, cached. */
    private unreadCount(key: string, list: StoredMessage[]): number {
        const readTs = this.readUpTo.get(key) ?? 0;
        const cached = this.unreadCounts.get(key);
        if (cached && cached.readTs === readTs) return cached.count;
        let count = 0;
        for (const m of list) if (!m.isSelf && m.ts > readTs) count++;
        this.unreadCounts.set(key, { readTs, count });
        return count;
    }

    private searchTextOf(m: StoredMessage): SearchText {
        const title = m.attachment?.title;
        const description = m.attachment?.description;
        let s = this.searchText.get(m);
        if (!s || s.text !== m.text || s.title !== title || s.description !== description) {
            s = {
                text: m.text,
                title,
                description,
                foldedText: foldForSearch(m.text),
                foldedTitle: title === undefined ? undefined : foldForSearch(title),
                foldedDescription: description === undefined ? undefined : foldForSearch(description),
            };
            this.searchText.set(m, s);
        }
        return s;
    }

    private load(): void {
        if (!this.file || !this.paths) return;
        const { meta, threads } = this.paths;
        // Left by a crash in the middle of a write.
        for (const dir of [path.dirname(meta), threads]) {
            for (const name of listDir(dir)) if (name.endsWith(".tmp")) removeQuietly(path.join(dir, name));
        }
        if (!fs.existsSync(meta) && fs.existsSync(this.file)) {
            this.migrateLegacy(this.file);
            return;
        }
        let metaLoaded = false;
        if (fs.existsSync(meta)) {
            try {
                this.applyMeta(JSON.parse(fs.readFileSync(meta, "utf8")) as Meta);
                metaLoaded = true;
            } catch (err) {
                backUpCorrupt(meta, err);
            }
        }
        // One unreadable chat is backed up and skipped; the others still load.
        for (const name of listDir(threads)) {
            if (!THREAD_FILE.test(name)) continue;
            // The name is the key, so the file read is the one the next save of that chat replaces.
            const key = Buffer.from(name.slice(0, -".json".length), "hex").toString("utf8");
            const file = threadFile(this.paths, key);
            if (path.basename(file) !== name) continue;
            try {
                const data = JSON.parse(fs.readFileSync(file, "utf8")) as ThreadFile;
                if (!Array.isArray(data.messages)) throw new Error("No message list");
                this.messages.set(key, data.messages);
                for (const m of data.messages) this.index.set(m.id, key);
            } catch (err) {
                backUpCorrupt(file, err);
            }
        }
        if (!metaLoaded) {
            // Only a history import lets a chat grow past the usual limit; its raised limit was in meta.json too.
            for (const list of this.messages.values()) {
                if (list.length > this.messageCap) this.messageCap = Math.max(list.length, HISTORY_MESSAGES_PER_THREAD);
            }
        }
        // Messages whose thread is not in meta.json: saved just before a crash, or meta.json was unreadable.
        for (const [key, list] of this.messages) {
            const last = list.at(-1);
            if (!last || this.threads.has(key)) continue;
            this.threads.set(key, { id: last.threadId, kind: last.threadKind, name: last.threadId, avatar: "" });
            // Without the saved read markers, all of the history would show as unread.
            if (!metaLoaded) this.readUpTo.set(key, last.ts);
            this.metaChanged();
        }
    }

    private applyMeta(meta: Meta): void {
        const threads = new Map(Object.entries(meta.threads ?? {}));
        const readUpTo = new Map(Object.entries(meta.readUpTo ?? {}));
        const pinned = new Set(meta.pinned ?? []);
        const mutedUntil = new Map(Object.entries(meta.mutedUntil ?? {}));
        const markedUnread = new Set(meta.markedUnread ?? []);
        this.threads = threads;
        this.readUpTo = readUpTo;
        this.pinned = pinned;
        this.mutedUntil = mutedUntil;
        this.markedUnread = markedUnread;
        if (typeof meta.messageCap === "number" && meta.messageCap > MAX_MESSAGES_PER_THREAD) this.messageCap = meta.messageCap;
    }

    /** Reads the old single-file cache and saves it in the per-thread layout, keeping the old file as `.migrated`. */
    private migrateLegacy(file: string): void {
        try {
            const snapshot = JSON.parse(fs.readFileSync(file, "utf8")) as Snapshot;
            this.applyMeta(snapshot);
            this.messages = new Map(Object.entries(snapshot.messages ?? {}));
            for (const [key, list] of this.messages) for (const m of list) this.index.set(m.id, key);
        } catch (err) {
            this.threads.clear();
            this.messages.clear();
            this.readUpTo.clear();
            this.pinned.clear();
            this.mutedUntil.clear();
            this.markedUnread.clear();
            this.index.clear();
            this.unreadCounts.clear();
            backUpCorrupt(file, err);
            return;
        }
        // Until every chat is written, meta.json (whose presence means "moved") is not, and the old file stays.
        this.pendingMigration = new Set(this.messages.keys());
        for (const key of this.messages.keys()) this.dirtyThreads.add(key);
        this.metaDirty = true;
        if (!this.saveSync()) this.scheduleSave();
    }

    /** The moved cache is fully saved: keep the old single file only as a backup. */
    private retireLegacy(): void {
        if (!this.pendingMigration || !this.file) return;
        this.pendingMigration = null;
        try {
            fs.renameSync(this.file, `${this.file}.migrated`);
        } catch (err) {
            console.warn(`Could not rename the old message cache ${this.file}:`, err);
        }
    }

    private scheduleSave(): void {
        if (!this.paths || this.saveTimer) return;
        this.saveTimer = setTimeout(() => {
            this.saveTimer = null;
            this.saveInBackground().catch((err) => console.warn("Could not save message cache:", err));
        }, 2000);
        this.saveTimer.unref();
    }

    /** Writes the changed files now, blocking. Returns whether all of them were written. */
    private saveSync(): boolean {
        const paths = this.paths;
        if (!paths) return true;
        let ok = true;
        const fail = (err: unknown) => {
            if (ok) console.warn(`Could not save message cache ${path.dirname(paths.meta)}:`, err);
            ok = false;
        };
        try {
            fs.mkdirSync(paths.threads, { recursive: true, mode: 0o700 });
            if (this.wipe) this.wipeFiles(paths);
        } catch (err) {
            fail(err);
            return false;
        }
        for (const key of [...this.dirtyThreads]) {
            this.dirtyThreads.delete(key);
            try {
                const file = threadFile(paths, key);
                const json = this.threadJson(key);
                if (json === null) this.removeFile(file);
                else this.writeSync(file, json);
                this.pendingMigration?.delete(key);
            } catch (err) {
                this.dirtyThreads.add(key);
                fail(err);
            }
        }
        // Written last, so that a thread saved before a crash is found again (see [load]).
        if (this.metaDirty && !this.pendingMigration?.size) {
            this.metaDirty = false;
            try {
                this.writeSync(paths.meta, this.metaJson());
                this.retireLegacy();
            } catch (err) {
                this.metaDirty = true;
                fail(err);
            }
        }
        return ok && !this.metaDirty;
    }

    /**
     * Writes the changed files without blocking the event loop: one at a time, and never two saves at once. A sync
     * [flush] in the middle wins: this save then drops what it was writing.
     */
    private async saveInBackground(): Promise<void> {
        const paths = this.paths;
        if (!paths) return;
        if (this.saving) {
            this.scheduleSave();
            return;
        }
        this.saving = true;
        let ok = true;
        const fail = (err: unknown) => {
            if (ok) console.warn(`Could not save message cache ${path.dirname(paths.meta)}:`, err);
            ok = false;
        };
        try {
            fs.mkdirSync(paths.threads, { recursive: true, mode: 0o700 });
            if (this.wipe) this.wipeFiles(paths);
            for (const key of [...this.dirtyThreads]) {
                // Saved meanwhile by a sync flush.
                if (!this.dirtyThreads.delete(key)) continue;
                this.writingThreads.add(key);
                try {
                    const file = threadFile(paths, key);
                    const json = this.threadJson(key);
                    let written = true;
                    if (json === null) this.removeFile(file);
                    else written = await this.writeAsync(file, json);
                    if (written) this.pendingMigration?.delete(key);
                } catch (err) {
                    this.dirtyThreads.add(key);
                    fail(err);
                } finally {
                    this.writingThreads.delete(key);
                }
            }
            if (this.metaDirty && !this.pendingMigration?.size) {
                this.metaDirty = false;
                this.writingMeta = true;
                try {
                    if (await this.writeAsync(paths.meta, this.metaJson())) this.retireLegacy();
                } catch (err) {
                    this.metaDirty = true;
                    fail(err);
                } finally {
                    this.writingMeta = false;
                }
            }
        } catch (err) {
            fail(err);
        } finally {
            this.saving = false;
        }
    }

    /** The thread's file contents, or null when it has no messages left and its file should go. */
    private threadJson(key: string): string | null {
        const list = this.messages.get(key);
        if (!list?.length) return null;
        const data: ThreadFile = { key, messages: list };
        return JSON.stringify(data);
    }

    private metaJson(): string {
        const meta: Meta = {
            version: 1,
            threads: Object.fromEntries(this.threads),
            readUpTo: Object.fromEntries(this.readUpTo),
            pinned: [...this.pinned],
            mutedUntil: Object.fromEntries(this.mutedUntil),
            markedUnread: [...this.markedUnread],
            ...(this.messageCap > MAX_MESSAGES_PER_THREAD ? { messageCap: this.messageCap } : {}),
        };
        return JSON.stringify(meta);
    }

    /** After [clear] (logging out): removes every thread file and the old single-file cache. */
    private wipeFiles(paths: { threads: string }): void {
        for (const name of listDir(paths.threads)) if (THREAD_FILE.test(name)) this.removeFile(path.join(paths.threads, name));
        if (this.file) {
            fs.rmSync(this.file, { force: true });
            fs.rmSync(`${this.file}.migrated`, { force: true });
        }
        this.pendingMigration = null;
        this.wipe = false;
    }

    /** Each write or removal of a file gets a new number; an async write renames its file in only if it is still the latest. */
    private nextSeq(file: string): number {
        this.fileSeq.set(file, ++this.seq);
        return this.seq;
    }

    /** Writes through a temporary file, so a crash never leaves a half-written file. */
    private writeSync(file: string, data: string): void {
        const tmp = `${file}.${this.nextSeq(file)}.tmp`;
        try {
            fs.writeFileSync(tmp, data, { mode: 0o600 });
            fs.renameSync(tmp, file);
        } catch (err) {
            removeQuietly(tmp);
            throw err;
        }
    }

    /** Like [writeSync] without blocking on the write. Returns false when a newer write of the file overtook it. */
    private async writeAsync(file: string, data: string): Promise<boolean> {
        const seq = this.nextSeq(file);
        const tmp = `${file}.${seq}.tmp`;
        try {
            await fs.promises.writeFile(tmp, data, { mode: 0o600 });
            if (this.fileSeq.get(file) !== seq) {
                removeQuietly(tmp);
                return false;
            }
            // Renamed synchronously: nothing can write the file between this check and the rename.
            fs.renameSync(tmp, file);
            return true;
        } catch (err) {
            removeQuietly(tmp);
            throw err;
        }
    }

    private removeFile(file: string): void {
        this.nextSeq(file);
        fs.rmSync(file, { force: true });
    }
}

/** A thread's file: named by the hex of its key, which is safe on any filesystem, case-insensitive ones included. */
function threadFile(paths: { threads: string }, key: string): string {
    return path.join(paths.threads, `${Buffer.from(key, "utf8").toString("hex")}.json`);
}

function listDir(dir: string): string[] {
    try {
        return fs.readdirSync(dir);
    } catch {
        return [];
    }
}

function removeQuietly(file: string): void {
    try {
        fs.rmSync(file, { force: true });
    } catch {
        // Best effort: a leftover temporary file is removed on the next start.
    }
}

/** Moves an unreadable cache file aside instead of overwriting it. */
function backUpCorrupt(file: string, err: unknown): void {
    const backup = `${file}.corrupt.${Date.now()}`;
    try {
        fs.renameSync(file, backup);
        console.warn(`Backed up unreadable message cache ${file} to ${backup}:`, err);
    } catch {
        console.warn(`Ignoring unreadable message cache ${file}:`, err);
    }
}

function foldForSearch(text: string): string {
    return text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/đ/g, "d").replace(/Đ/g, "D").toLowerCase();
}

function union(a: string[] | undefined, b: string[] | undefined): string[] {
    return [...new Set([...(a ?? []), ...(b ?? [])])];
}

function summarizeReactions(reactions: Record<string, string> | undefined, ownId: string | null): ReactionSummary[] {
    const byCode = new Map<string, ReactionSummary>();
    for (const [uid, code] of Object.entries(reactions ?? {})) {
        const known = REACTION_BY_CODE.get(code);
        const summary = byCode.get(code) ?? { key: known?.key ?? code, emoji: known?.emoji ?? code, count: 0, mine: false };
        summary.count++;
        if (uid === ownId) summary.mine = true;
        byCode.set(code, summary);
    }
    return [...byCode.values()].sort((a, b) => b.count - a.count);
}
