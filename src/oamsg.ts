import { decodeEventPayload, frameHeader } from "./calls.js";
import type { RawMessage } from "./store.js";

/**
 * Extracts messages that zca-js's `listen.js` silently drops from Zalo WebSocket frames:
 *  - `pageMsgs` (Official Account / ZNS / OTP messages) on live 501, history 510, and offline/re-push frames.
 *  - `msgs` / `groupMsgs` on mixed offline/re-push frames (504, 513, 514, 524, 534, 551, 570) that zca-js ignores.
 *
 * We intentionally do NOT re-emit `msgs` from 501/510 or `groupMsgs` from 521/511 because zca-js already emits those.
 * Any overlap is still harmless because ChatStore deduplicates by `msgId`.
 */
const PAGE_MSG_CMDS = new Set([501, 504, 510, 513, 514, 524, 534, 551, 570]);
const OFFLINE_PUSH_CMDS = new Set([504, 513, 514, 524, 534, 551, 570]);

function isE2eeType(msgType: unknown): boolean {
    return typeof msgType === "string" && /(^|\.)e2ee(\.|$)/i.test(msgType);
}

function isUndoContent(content: unknown): boolean {
    return Boolean(content && typeof content === "object" && !Array.isArray(content) && "deleteMsg" in (content as Record<string, unknown>));
}

function toRawMessage(item: unknown, isGroup: boolean, ownId: string): RawMessage | null {
    if (!item || typeof item !== "object" || Array.isArray(item)) return null;
    const m = item as Record<string, unknown>;
    const msgId = m.msgId ?? m.globalMsgId ?? m.cliMsgId;
    if (msgId === undefined || msgId === null || String(msgId) === "") return null;
    if (isUndoContent(m.content)) return null;

    const rawUidFrom = String(m.uidFrom ?? "");
    const rawIdTo = String(m.idTo ?? "");
    const isSelf = rawUidFrom === "0" || (ownId !== "" && rawUidFrom === ownId);
    const threadId = isGroup ? rawIdTo : isSelf ? rawIdTo : rawUidFrom;
    if (!threadId || threadId === "0") return null;

    const uidFrom = rawUidFrom === "0" ? ownId : rawUidFrom;
    const idTo = rawIdTo === "0" ? ownId : rawIdTo;
    const content = isE2eeType(m.msgType) ? "[Encrypted message]" : m.content;

    const quoteRaw = m.quote && typeof m.quote === "object" ? (m.quote as Record<string, unknown>) : undefined;
    const quote = quoteRaw
        ? {
              ownerId: String(quoteRaw.ownerId ?? ""),
              globalMsgId: String(quoteRaw.globalMsgId ?? ""),
              fromD: typeof quoteRaw.fromD === "string" ? quoteRaw.fromD : undefined,
              msg: typeof quoteRaw.msg === "string" ? quoteRaw.msg : undefined,
              attach: typeof quoteRaw.attach === "string" ? quoteRaw.attach : undefined,
          }
        : undefined;

    return {
        type: isGroup ? 1 : 0,
        threadId,
        isSelf,
        data: {
            msgId: String(msgId),
            cliMsgId: String(m.cliMsgId ?? msgId),
            msgType: String(m.msgType ?? "webchat"),
            uidFrom,
            idTo,
            dName: typeof m.dName === "string" ? m.dName : typeof m.oaName === "string" ? m.oaName : "",
            ts: (typeof m.ts === "string" || typeof m.ts === "number" ? m.ts : Date.now()) as string | number,
            content,
            propertyExt: m.propertyExt,
            ttl: typeof m.ttl === "number" ? m.ttl : undefined,
            st: typeof m.st === "number" ? m.st : undefined,
            at: typeof m.at === "number" ? m.at : undefined,
            cmd: typeof m.cmd === "number" ? m.cmd : undefined,
            quote,
            mentions: Array.isArray(m.mentions) ? (m.mentions as RawMessage["data"]["mentions"]) : undefined,
        },
    };
}

/** Extracts OA (`pageMsgs`) and offline/re-push messages from a raw Zalo WebSocket frame. */
export function extractDroppedMessages(frame: Buffer, cipherKey: string | undefined, ownId: string): RawMessage[] {
    const header = frameHeader(frame);
    if (!header || header.version !== 1 || !PAGE_MSG_CMDS.has(header.cmd)) return [];
    const text = frame.subarray(4).toString("utf8");
    if (!text) return [];

    let parsed: unknown;
    try {
        parsed = JSON.parse(text);
    } catch {
        return [];
    }
    let decoded: unknown;
    try {
        decoded = decodeEventPayload(parsed as { data?: unknown; encrypt?: unknown }, cipherKey);
    } catch {
        return [];
    }
    if (!decoded || typeof decoded !== "object") return [];
    const outer = decoded as Record<string, unknown>;
    const data = outer.data && typeof outer.data === "object" ? (outer.data as Record<string, unknown>) : outer;

    const out: RawMessage[] = [];
    const pushList = (list: unknown, isGroup: boolean) => {
        if (!Array.isArray(list)) return;
        for (const item of list) {
            const raw = toRawMessage(item, isGroup, ownId);
            if (raw) out.push(raw);
        }
    };

    // Always capture pageMsgs (Official Accounts, ZNS notifications, OTPs) — zca-js never reads this array.
    pushList(data.pageMsgs, false);

    // For offline/re-push commands zca-js ignores completely, also capture standard 1:1 and group arrays.
    if (OFFLINE_PUSH_CMDS.has(header.cmd)) {
        pushList(data.msgs, false);
        pushList(data.groupMsgs, true);
    }

    return out;
}
