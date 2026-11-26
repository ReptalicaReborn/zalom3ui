import { readTable, type SqlValue } from "./sqliteread.js";
import { normalizeMessage, type RawMessage, type StoredMessage, type ThreadKind } from "./store.js";

/**
 * Turns the phone's backup (one SQLite `ChatContent` table per conversation, with a TLV "BinNet" blob per message) into
 * messages for [ChatStore]. The column names, the BinNet tags and the message type table follow the Zalos project
 * (`MobileHistorySync::readBackupDirectory` / `toMessages` and `DbCross::parseBinNet`, MIT); the code is our own.
 */

// ---- BinNet: a flat TLV box (4-byte key, 4-byte length, value) whose byte order is guessed per blob ----

export type BinNetAttachment = {
    type: string;
    catId: number;
    id: number;
    extInfo: string;
    action: string;
    params: string;
    title: string;
    href: string;
    thumb: string;
    description: string;
    size: string;
    ext: string;
};

export type BinNetQuote = { ownerId: string; cliMsgType: number; fromD: string; cliMsgId: string; globalMsgId: string; ts: string; msg: string; attach: string };
export type BinNetMention = { uid: string; pos: number; len: number; type: number };

export type BinNet = {
    text: string;
    dataSource: string;
    quoteStatus: number;
    quote: BinNetQuote | null;
    attachs: BinNetAttachment[];
    mentions: BinNetMention[];
};

type Tlv = { values: Map<number, Buffer[]>; little: boolean };

function parseTlvWith(buf: Buffer, little: boolean): Map<number, Buffer[]> | null {
    const values = new Map<number, Buffer[]>();
    let off = 0;
    while (off + 8 <= buf.length) {
        const key = little ? buf.readUInt32LE(off) : buf.readUInt32BE(off);
        const len = little ? buf.readUInt32LE(off + 4) : buf.readUInt32BE(off + 4);
        off += 8;
        if (len > buf.length - off) return null;
        const list = values.get(key) ?? [];
        list.push(buf.subarray(off, off + len));
        values.set(key, list);
        off += len;
    }
    // A few trailing bytes are tolerated, as the Zalos project does.
    if (off !== buf.length && buf.length - off > 8) return null;
    return values;
}

const TOP_KEYS = [0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b];
const ATTACH_KEYS = [0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x40, 0x41, 0x42, 0x43, 0x44];

/** How much a parse looks like a BinNet: the byte order that fits the known tags wins. */
function score(values: Map<number, Buffer[]>): number {
    let s = 0;
    for (const k of TOP_KEYS) if (values.has(k)) s += 6;
    for (const k of ATTACH_KEYS) if (values.has(k)) s += 4;
    for (const k of values.keys()) {
        if (k <= 0x80) s += 1;
        if (k > 0x10000) s -= 3;
    }
    return s;
}

function parseTlv(buf: Buffer): Tlv | null {
    const le = parseTlvWith(buf, true);
    const be = parseTlvWith(buf, false);
    if (!le && !be) return null;
    const leScore = le ? score(le) : -1e9;
    const beScore = be ? score(be) : -1e9;
    return beScore > leScore ? { values: be!, little: false } : { values: le!, little: true };
}

function int32(t: Tlv, key: number): number | undefined {
    const v = t.values.get(key)?.[0];
    if (!v || v.length < 4) return undefined;
    return t.little ? v.readInt32LE(0) : v.readInt32BE(0);
}

/** An id or timestamp as text: 8 bytes when present, else 4 (unsigned), so 64-bit ids keep every digit. */
function int64Text(t: Tlv, key: number): string | undefined {
    const v = t.values.get(key)?.[0];
    if (!v) return undefined;
    if (v.length >= 8) return (t.little ? v.readBigInt64LE(0) : v.readBigInt64BE(0)).toString();
    if (v.length >= 4) return String(t.little ? v.readUInt32LE(0) : v.readUInt32BE(0));
    return undefined;
}

function looksLikeUtf8(v: Buffer): boolean {
    if (v.length === 0) return true;
    if (v.includes(0)) return false;
    // A strict decode round-trips only for valid UTF-8; control characters other than tab/CR/LF are suspicious.
    const text = v.toString("utf8");
    if (text.includes("�") && !v.includes(Buffer.from("�"))) return false;
    let bad = 0;
    for (const ch of text) {
        const c = ch.codePointAt(0)!;
        if (c < 0x20 && c !== 9 && c !== 10 && c !== 13) bad++;
    }
    return bad === 0;
}

/** Text from a TLV value: UTF-8 when it is, otherwise UTF-16LE when most high bytes are zero (ASCII-heavy). */
function decodeText(v: Buffer): string | undefined {
    if (looksLikeUtf8(v)) return v.toString("utf8");
    if (v.length >= 4 && v.length % 2 === 0) {
        let zero = 0;
        for (let i = 1; i < v.length; i += 2) if (v[i] === 0) zero++;
        if (zero * 100 >= (v.length / 2) * 60) return v.toString("utf16le");
    }
    return undefined;
}

function text(t: Tlv, key: number): string | undefined {
    const v = t.values.get(key)?.[0];
    return v ? decodeText(v) : undefined;
}

function parseAttachment(buf: Buffer): BinNetAttachment {
    const out: BinNetAttachment = { type: "", catId: 0, id: 0, extInfo: "", action: "", params: "", title: "", href: "", thumb: "", description: "", size: "", ext: "" };
    const t = parseTlv(buf);
    if (!t) return out;
    out.type = text(t, 0x28) ?? "";
    out.catId = int32(t, 0x29) ?? 0;
    out.id = int32(t, 0x2a) ?? 0;
    out.extInfo = text(t, 0x2b) ?? "";
    out.action = text(t, 0x2d) ?? "";
    out.params = text(t, 0x2e) ?? "";
    out.title = text(t, 0x2f) ?? "";
    out.href = text(t, 0x30) ?? "";
    out.thumb = text(t, 0x31) ?? "";
    out.description = text(t, 0x32) ?? "";
    out.size = text(t, 0x33) ?? "";
    out.ext = text(t, 0x34) ?? "";
    return out;
}

function parseQuote(buf: Buffer): BinNetQuote | null {
    const t = parseTlv(buf);
    if (!t) return null;
    return {
        ownerId: int64Text(t, 0x50) ?? "",
        cliMsgType: int32(t, 0x53) ?? 0,
        cliMsgId: int64Text(t, 0x51) ?? "0",
        globalMsgId: int64Text(t, 0x52) ?? "0",
        ts: int64Text(t, 0x54) ?? "0",
        msg: text(t, 0x56) ?? "",
        attach: text(t, 0x57) ?? "",
        fromD: text(t, 0x5a) ?? "",
    };
}

function parseMention(buf: Buffer): BinNetMention | null {
    const t = parseTlv(buf);
    if (!t) return null;
    return { uid: int64Text(t, 0x65) ?? "0", pos: int32(t, 0x66) ?? 0, len: int32(t, 0x67) ?? 0, type: int32(t, 0x68) ?? 0 };
}

/** Reads a message's BinNet blob. An unreadable blob gives an empty result (the message text column still works). */
export function parseBinNet(buf: Buffer | null | undefined): BinNet {
    const out: BinNet = { text: "", dataSource: "", quoteStatus: 0, quote: null, attachs: [], mentions: [] };
    if (!buf || buf.length === 0) return out;
    const t = parseTlv(buf);
    if (!t) return out;
    for (const v of t.values.get(0x06) ?? []) if (v.length >= 8) out.attachs.push(parseAttachment(v));
    const quote = t.values.get(0x07)?.[0];
    if (quote) out.quote = parseQuote(quote);
    for (const v of t.values.get(0x08) ?? []) {
        const m = parseMention(v);
        if (m) out.mentions.push(m);
    }
    out.quoteStatus = int32(t, 0x09) ?? 0;
    out.dataSource = text(t, 0x0a) ?? "";
    out.text = text(t, 0x0b) ?? "";
    return out;
}

// ---- The backup's rows ----

export type BackupRow = {
    globalMsgId: string;
    cliMsgId: string;
    msgType: number;
    ts: number;
    senderId: string;
    /** Set when the row has a column that says outright whether this account sent it (true = sent by us). */
    outgoing?: boolean;
    text: string;
    ttl: number;
    bin: BinNet;
};

export type BackupConversation = {
    id: string;
    isGroup: boolean;
    rows: BackupRow[];
    /** Rows in the table before filtering. */
    sourceRows: number;
    /** Column names of the `ChatContent` table (for the sync log: the phone's schema is only partly known). */
    columns?: string[];
};

function str(v: SqlValue | undefined): string {
    if (v === null || v === undefined) return "";
    return Buffer.isBuffer(v) ? v.toString("utf8") : String(v);
}

function num(v: SqlValue | undefined): number {
    if (typeof v === "number") return v;
    if (typeof v === "bigint") return Number(v);
    const n = Number(str(v));
    return Number.isFinite(n) ? n : 0;
}

/** Columns that, when present, tell directly whether this account sent the row (boolean-like). */
const OUTGOING_COLUMNS = ["IsSend", "IsSent", "IsOutgoing", "IsMine", "IsSelf", "FromMe", "IsFromMe", "Outgoing"];

function outgoingFlag(row: Record<string, SqlValue>): boolean | undefined {
    for (const name of OUTGOING_COLUMNS) {
        const v = row[name];
        if (v === null || v === undefined) continue;
        const n = typeof v === "number" ? v : typeof v === "bigint" ? Number(v) : Number(str(v));
        if (Number.isFinite(n)) return n !== 0;
    }
    return undefined;
}

function first(row: Record<string, SqlValue>, names: string[]): SqlValue | undefined {
    for (const n of names) if (row[n] !== null && row[n] !== undefined) return row[n];
    return undefined;
}

/** Epoch milliseconds; a value that looks like seconds is scaled up. */
export function toMillis(ts: number): number {
    return ts > 0 && ts < 1e11 ? ts * 1000 : ts;
}

/**
 * Reads one conversation database. Like the phone's own restore it keeps rows with a positive `MsgStatus` and a
 * client message id, oldest first. Returns null when the file has no `ChatContent` table.
 */
export function readConversation(db: Buffer, id: string, isGroup: boolean): BackupConversation | null {
    const table = readTable(db, "ChatContent");
    if (!table) return null;
    const hasStatus = table.columns.includes("MsgStatus");
    const rows: BackupRow[] = [];
    let sourceRows = 0;
    for (const r of table.rows()) {
        sourceRows++;
        if (hasStatus && num(r.MsgStatus) <= 0) continue;
        const cli = str(first(r, ["CliMsgId", "cliMsgId"]));
        if (!cli) continue;
        const glb = str(first(r, ["GlbMsgId", "GlobalMsgId", "MsgId"]));
        if (!glb && !cli) continue;
        const bin = r.BinNet;
        rows.push({
            globalMsgId: glb,
            cliMsgId: cli,
            msgType: num(r.MsgType),
            ts: toMillis(num(first(r, ["TimeStamp", "Ts"]))),
            senderId: str(first(r, ["SenderId", "FromId", "FromUid"])),
            outgoing: outgoingFlag(r),
            text: str(first(r, ["MsgContent", "Message"])),
            ttl: num(r.TTL),
            bin: parseBinNet(Buffer.isBuffer(bin) ? bin : null),
        });
    }
    // Stable sort: rows with equal timestamps keep the phone's order.
    rows.sort((a, b) => a.ts - b.ts);
    return { id, isGroup, rows, sourceRows, columns: table.columns };
}

// ---- Rows -> messages ----

/** Phone message types that are not chat content (system rows, call and group notices, ...). */
const SKIPPED_TYPES = new Set([21, 25, 29, 32, 33, 34, 35, 45, 51, 52]);

/** The zca-js `msgType` names the rest of the app understands. */
function typeName(t: number): string {
    switch (t) {
        case 2:
            return "chat.doodle";
        case 3:
        case 4:
        case 31:
            return "chat.photo";
        case 6:
            return "chat.voice";
        case 10:
            return "chat.sticker";
        case 12:
            return "chat.recommended";
        case 15:
            return "chat.list.action";
        case 18:
            return "chat.location.new";
        case 19:
            return "chat.video.msg";
        case 22:
            return "share.file";
        case 23:
            return "chat.gif";
        case 24:
            return "chat.webcontent";
        case 26:
            return "chat.poll";
        case 53:
            return "chat.webcontent.v2";
        default:
            return "webchat";
    }
}

/** "[Photo]"-style captions the phone writes for non-text messages. */
function isPlaceholderCaption(s: string): boolean {
    return /^\[[^\]]{1,24}\]$/.test(s.trim());
}

function jsonObject(raw: string): Record<string, unknown> {
    try {
        const v = JSON.parse(raw);
        return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
    } catch {
        return {};
    }
}

function pickId(o: Record<string, unknown>, keys: string[]): string {
    for (const k of keys) {
        const v = o[k];
        const s = typeof v === "string" || typeof v === "number" ? String(v) : "";
        if (s && s !== "0") return s;
    }
    return "";
}

/**
 * Plain-to-noised id tables returned by `/api/znoise`, plus our own plain user id from `syncmsg_info.data.uid`.
 * When `users` or `groups` is supplied, every conversation and participant id is translated to the noised form Zalo Web
 * (zca-js) uses; conversations with no mapping are skipped instead of creating broken numeric-id threads.
 */
export type BackupIdMap = {
    users?: ReadonlyMap<string, string>;
    groups?: ReadonlyMap<string, string>;
    myPlainUid?: string;
};

/**
 * Whether this account sent the row. An explicit direction column wins; next is `"0"`, our plain id from `syncmsg_info`,
 * or our noised id; in a 1:1 chat anyone other than the other person is us, and a row with no sender at all is theirs.
 */
export function isOutgoing(row: BackupRow, conv: { id: string; isGroup: boolean }, ownId: string, myPlainUid = ""): boolean {
    if (row.outgoing !== undefined) return row.outgoing;
    const sender = row.senderId;
    if (sender === "0" || (myPlainUid !== "" && sender === myPlainUid) || (ownId !== "" && sender === ownId)) return true;
    if (!conv.isGroup && sender !== "" && sender !== conv.id) return true;
    return false;
}

export type ConvertResult = {
    messages: RawMessage[];
    /** Ids of messages the backup says were recalled. */
    recalled: string[];
    skipped: Record<string, number>;
    /** The thread id the converted messages target (noised when `idMap` was supplied). */
    threadId?: string;
};

/**
 * Converts a conversation's rows to the raw message shape zca-js produces, so the same `normalizeMessage` as for live
 * messages applies. The phone's backup uses plain user and group ids, while Zalo Web (zca-js) uses noised ids mapped
 * through `/api/znoise` (`idMap`).
 */
export function convertConversation(
    conv: BackupConversation,
    ownId: string,
    nameOf: (userId: string) => string = () => "",
    idMap?: BackupIdMap,
): ConvertResult {
    const messages: RawMessage[] = [];
    const recalled: string[] = [];
    const skipped: Record<string, number> = {};
    const skip = (reason: string) => {
        skipped[reason] = (skipped[reason] ?? 0) + 1;
    };

    const hasNoiseMap = Boolean(idMap?.users || idMap?.groups);
    const threadId = hasNoiseMap
        ? (conv.isGroup ? idMap?.groups?.get(conv.id) : idMap?.users?.get(conv.id)) ?? ""
        : conv.id;
    if (!threadId) {
        if (conv.rows.length) skipped.no_thread = conv.rows.length;
        return { messages, recalled, skipped };
    }

    const myPlainUid = idMap?.myPlainUid ?? "";
    const mapUser = (plain: string): string => {
        if (!plain || plain === "0") return "";
        if (myPlainUid && plain === myPlainUid) return ownId;
        return idMap?.users?.get(plain) ?? plain;
    };

    for (const row of conv.rows) {
        if (SKIPPED_TYPES.has(row.msgType)) {
            skip(`type_${row.msgType}`);
            continue;
        }
        const rowId = row.globalMsgId && row.globalMsgId !== "0" ? row.globalMsgId : row.cliMsgId;
        const mine = isOutgoing(row, conv, ownId, myPlainUid);
        const bin = row.bin;

        if (row.msgType === 36) {
            const src = jsonObject(row.text);
            const target = pickId(src, ["globalMsgId", "msgId"]);
            if (target) recalled.push(target);
            else skip("undo_without_id");
            continue;
        }
        if (!row.senderId && !mine && conv.isGroup) {
            skip("no_sender");
            continue;
        }
        if (!rowId) {
            skip("no_id");
            continue;
        }

        const type = typeName(row.msgType);
        let content: unknown;
        if (row.msgType === 26) {
            content = jsonObject(row.text);
        } else if (type === "webchat" || bin.attachs.length === 0) {
            const body = row.text || bin.text;
            if ((!body && bin.attachs.length === 0) || (type !== "webchat" && isPlaceholderCaption(body))) {
                skip("empty_or_placeholder");
                continue;
            }
            content = body;
        } else {
            const a = bin.attachs[0];
            const obj: Record<string, unknown> = { title: a.title, href: a.href, thumb: a.thumb, description: a.description, action: a.action, params: a.params, type: a.type };
            if (type === "chat.sticker") {
                obj.id = a.id;
                obj.catId = a.catId;
            }
            if (!obj.title && !isPlaceholderCaption(row.text)) obj.title = row.text;
            content = obj;
        }
        // A text message that is only a placeholder with nothing else to show is not worth keeping.
        if (type === "webchat" && typeof content === "string" && content === "") {
            skip("empty_or_placeholder");
            continue;
        }

        const uidFrom = conv.isGroup ? (mine ? ownId : mapUser(row.senderId)) : mine ? ownId : threadId;
        const idTo = conv.isGroup ? threadId : mine ? threadId : ownId;

        const data: RawMessage["data"] = {
            msgId: rowId,
            cliMsgId: row.cliMsgId,
            msgType: type,
            uidFrom,
            idTo,
            dName: mine ? "" : nameOf(uidFrom),
            ts: String(row.ts),
            content,
            ttl: row.ttl,
        };
        if (bin.quote && (bin.quote.globalMsgId !== "0" || bin.quote.cliMsgId !== "0" || bin.quote.msg)) {
            const quoteOwner = bin.quote.ownerId;
            const quoteMine = quoteOwner === "0" || (myPlainUid !== "" && quoteOwner === myPlainUid) || (ownId !== "" && quoteOwner === ownId);
            const mappedQuoteOwner = quoteMine ? ownId : mapUser(quoteOwner);
            data.quote = {
                ownerId: mappedQuoteOwner || quoteOwner,
                globalMsgId: bin.quote.globalMsgId,
                fromD: bin.quote.fromD || (quoteMine ? "" : nameOf(mappedQuoteOwner) || (conv.isGroup ? "" : nameOf(threadId))),
                msg: bin.quote.msg,
                attach: bin.quote.attach,
            };
        }
        const mentions = bin.mentions
            .map((m) => ({ uid: m.uid.startsWith("-") ? "-1" : mapUser(m.uid), pos: m.pos, len: m.len, type: m.type }))
            .filter((m) => m.uid !== "0" && m.uid !== "");
        if (mentions.length) data.mentions = mentions;
        messages.push({ type: conv.isGroup ? 1 : 0, threadId, isSelf: mine, data });
    }
    return { messages, recalled, skipped, threadId };
}

// ---- Into the store ----

/** What an import needs from [ChatStore] (a narrow view, so tests can use a fake). */
export interface ImportSink {
    addMessage(msg: StoredMessage, opts: { history: true }): boolean;
    recall(msgId: string): unknown;
    /** After a conversation's import: its imported messages must not count as unread. */
    settleHistory(kind: ThreadKind, threadId: string, importedIds: ReadonlySet<string>): void;
    /** Removes a stale unnamed plain-id history thread before re-importing into its mapped noised id. */
    dropPlainThread?(kind: ThreadKind, plainId: string): boolean;
}

/** `duplicates`: messages already stored (or beyond the per-chat limit); `skipped`: rows that aren't chat content. */
export type ImportCounts = { imported: number; duplicates: number; skipped: number; recalled: number };

/**
 * Adds a conversation's messages to the store: new ids only, in timestamp order (the store keeps its lists sorted).
 * Messages already present (live, or from an earlier sync) are counted as duplicates and left alone.
 */
export function importConversation(
    conv: BackupConversation,
    ownId: string,
    sink: ImportSink,
    nameOf?: (userId: string) => string,
    idMap?: BackupIdMap,
): ImportCounts {
    const { messages, recalled, skipped, threadId } = convertConversation(conv, ownId, nameOf, idMap);
    const kind: ThreadKind = conv.isGroup ? "group" : "user";
    if (!threadId) {
        return { imported: 0, duplicates: 0, skipped: Object.values(skipped).reduce((a, b) => a + b, 0), recalled: 0 };
    }
    const ids = new Set<string>();
    let imported = 0;
    let duplicates = 0;
    for (const raw of messages) {
        const msg = normalizeMessage(raw);
        // Imported history is not acted on (no quote / reaction), so the raw copy would only cost memory.
        delete msg.raw;
        if (sink.addMessage(msg, { history: true })) {
            imported++;
            ids.add(msg.id);
        } else {
            duplicates++;
        }
    }
    let recalledCount = 0;
    for (const id of recalled) if (sink.recall(id)) recalledCount++;
    sink.settleHistory(kind, threadId, ids);
    return { imported, duplicates, skipped: Object.values(skipped).reduce((a, b) => a + b, 0), recalled: recalledCount };
}
