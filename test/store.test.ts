import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { loadConfig } from "../src/config.js";
import { parseMentions } from "../src/server.js";
import { muteUntil } from "../src/session.js";
import { ChatStore, normalizeMessage, recalledMessageId, type RawMessage, type StoredMessage } from "../src/store.js";

export function raw(overrides: Partial<RawMessage["data"]> = {}, top: Partial<Omit<RawMessage, "data">> = {}): RawMessage {
    return {
        type: 0,
        threadId: "100",
        isSelf: false,
        ...top,
        data: {
            msgId: "1",
            cliMsgId: "c1",
            msgType: "webchat",
            uidFrom: "100",
            idTo: "me",
            dName: "Alice",
            ts: "1700000000000",
            content: "hello",
            ...overrides,
        },
    };
}

function msg(id: string, ts: number, extra: Partial<StoredMessage> = {}): StoredMessage {
    return { ...normalizeMessage(raw({ msgId: id, ts: String(ts) })), ...extra };
}

/** A message in the 1:1 chat with `threadId`. */
function inThread(id: string, ts: number, threadId: string, extra: Partial<StoredMessage> = {}): StoredMessage {
    return msg(id, ts, { threadId, fromId: threadId, ...extra });
}

test("normalizeMessage keeps plain text and the raw fields needed for actions", () => {
    const m = normalizeMessage(raw({ ttl: 0, st: 3, at: 5, cmd: 501 }));
    assert.equal(m.text, "hello");
    assert.equal(m.threadKind, "user");
    assert.equal(m.ts, 1700000000000);
    assert.equal(m.attachment, undefined);
    assert.deepEqual(m.raw, {
        msgType: "webchat",
        content: "hello",
        propertyExt: undefined,
        uidFrom: "100",
        idTo: "me",
        ts: "1700000000000",
        ttl: 0,
        st: 3,
        at: 5,
        cmd: 501,
    });
});

test("normalizeMessage labels attachments and group threads", () => {
    const m = normalizeMessage(
        raw(
            { msgType: "chat.photo", content: { title: "", href: "https://x/y.jpg", thumb: "https://x/t.jpg" } },
            { type: 1, threadId: "g1" },
        ),
    );
    assert.equal(m.threadKind, "group");
    assert.equal(m.threadId, "g1");
    assert.equal(m.text, "[Photo]");
    assert.deepEqual(m.attachment, { kind: "photo", href: "https://x/y.jpg", thumb: "https://x/t.jpg" });
});

test("normalizeMessage reads photo, video, voice and file details from params", () => {
    const photo = normalizeMessage(
        raw({
            msgType: "chat.photo",
            content: { href: "https://x/n.jpg", thumb: "https://x/t.jpg", params: JSON.stringify({ hd: "https://x/hd.jpg", width: 1280, height: "720" }) },
        }),
    );
    assert.deepEqual(photo.attachment, { kind: "photo", href: "https://x/hd.jpg", thumb: "https://x/t.jpg", width: 1280, height: 720 });

    const video = normalizeMessage(
        raw({
            msgType: "chat.video.msg",
            content: { href: "https://x/v.mp4", thumb: "https://x/v.jpg", params: JSON.stringify({ duration: 12500, video_width: 720, video_height: 1280 }) },
        }),
    );
    assert.equal(video.text, "[Video]");
    assert.deepEqual(video.attachment, { kind: "video", href: "https://x/v.mp4", thumb: "https://x/v.jpg", width: 720, height: 1280, duration: 12500 });

    const voice = normalizeMessage(raw({ msgType: "chat.voice", content: { href: "https://x/a.m4a", params: '{"duration":"4200"}' } }));
    assert.deepEqual(voice.attachment, { kind: "voice", href: "https://x/a.m4a", duration: 4200 });

    const file = normalizeMessage(
        raw({ msgType: "share.file", content: { title: "Báo cáo.PDF", href: "https://x/f", params: JSON.stringify({ fileSize: "20480" }) } }),
    );
    assert.equal(file.text, "[File] Báo cáo.PDF");
    assert.deepEqual(file.attachment, { kind: "file", title: "Báo cáo.PDF", href: "https://x/f", size: 20480, fileExt: "pdf" });
});

test("normalizeMessage reads call logs like the official client", () => {
    const call = (action: string, params: Record<string, unknown>) =>
        normalizeMessage(raw({ msgType: "chat.recommended", content: { action, title: "", params: JSON.stringify(params) } }));

    const answered = call("recommened.calltime", { calltype: 0, isCaller: 0, duration: 125, reason: 0 });
    assert.equal(answered.text, "[Incoming voice call]");
    assert.deepEqual(answered.attachment, {
        kind: "call",
        title: "Incoming voice call",
        call: { video: false, missed: false, outgoing: false },
        duration: 125_000,
    });

    const missed = call("recommened.misscall", { calltype: 1, isCaller: 0, reason: 2, duration: 0 });
    assert.equal(missed.text, "[Missed video call]");
    assert.deepEqual(missed.attachment?.call, { video: true, missed: true, outgoing: false });
    assert.equal(missed.attachment?.duration, undefined);

    assert.equal(call("recommened.misscall", { calltype: 0, isCaller: 1, reason: 3 }).text, "[Call declined]");
    assert.equal(call("recommened.misscall", { calltype: 0, isCaller: 1, reason: 4 }).text, "[You cancelled the call]");
    assert.equal(call("recommened.misscall", { calltype: 0, isCaller: 0, reason: 3 }).text, "[You declined the call]");
    assert.equal(call("recommened.calling", { calltype: 1, duration: 61 }).text, "[Outgoing video call]");
});

test("normalizeMessage reads stickers, links and contact cards", () => {
    const sticker = normalizeMessage(raw({ msgType: "chat.sticker", content: { id: 46991, catId: 10425, type: 7 } }));
    assert.equal(sticker.text, "[Sticker]");
    assert.deepEqual(sticker.attachment, {
        kind: "sticker",
        thumb: "https://api.zaloapp.com/api/emoticon/sticker/webpc?size=130&eid=46991",
        sticker: { id: 46991, catId: 10425, type: 7 },
    });

    const link = normalizeMessage(
        raw({ msgType: "chat.recommended", content: { action: "recommened.link", title: "Tin tức", description: "Mô tả", href: "https://news.example/a", thumb: "https://x/l.jpg", params: "{}" } }),
    );
    assert.equal(link.text, "[Link] Tin tức");
    assert.equal(link.attachment?.kind, "link");
    assert.equal(link.attachment?.description, "Mô tả");

    const card = normalizeMessage(
        raw({ msgType: "chat.recommended", content: { action: "recommened.user", title: "An", thumb: "https://x/a.jpg", params: "123456" } }),
    );
    assert.equal(card.text, "[Contact] An");
    assert.deepEqual(card.attachment, { kind: "contact", title: "An", thumb: "https://x/a.jpg", userId: "123456" });
});

test("normalizeMessage falls back for unknown content", () => {
    const m = normalizeMessage(raw({ msgType: "chat.something.new", content: {} }));
    assert.equal(m.text, "[Unsupported message]");
});

test("normalizeMessage reads quotes and mentions", () => {
    const m = normalizeMessage(
        raw(
            {
                content: "@Me look",
                quote: { ownerId: 200, globalMsgId: 42, fromD: "Bob", msg: "", attach: "{}" },
                mentions: [{ uid: "me", pos: 0, len: 3, type: 0 }],
            },
            { type: 1, threadId: "g1" },
        ),
    );
    assert.deepEqual(m.quote, { msgId: "42", fromId: "200", fromName: "Bob", text: "[Attachment]" });
    assert.deepEqual(m.mentions, [{ uid: "me", pos: 0, len: 3 }]);

    const store = new ChatStore();
    store.ownId = "me";
    assert.equal(store.toPublic(m).mentionsMe, true);
    store.ownId = "someone-else";
    assert.equal(store.toPublic(m).mentionsMe, false);
});

test("recalledMessageId recognises recall records", () => {
    assert.equal(recalledMessageId(raw({ content: { deleteMsg: 1, globalMsgId: 77, cliMsgId: 1 } })), "77");
    assert.equal(recalledMessageId(raw()), null);
});

test("ChatStore dedupes, sorts and counts unread", () => {
    const store = new ChatStore();
    assert.equal(store.addMessage(msg("2", 2000)), true);
    assert.equal(store.addMessage(msg("1", 1000)), true);
    assert.equal(store.addMessage(msg("1", 1000)), false);

    assert.deepEqual(
        store.getMessages("user", "100").map((m) => m.id),
        ["1", "2"],
    );
    const [thread] = store.listThreads();
    assert.equal(thread.name, "Alice");
    assert.equal(thread.unread, 2);
    assert.equal(thread.lastMessage?.id, "2");

    store.markRead("user", "100", 1500);
    assert.equal(store.listThreads()[0].unread, 1);
    assert.deepEqual(
        store.unreadMessages("user", "100").map((m) => m.id),
        ["2"],
    );

    // Replying marks everything up to the reply as read.
    store.addMessage(msg("3", 3000, { isSelf: true }));
    assert.equal(store.listThreads()[0].unread, 0);
});

test("ChatStore paginates with before/limit", () => {
    const store = new ChatStore();
    for (let i = 1; i <= 10; i++) store.addMessage(msg(String(i), i * 1000));
    assert.deepEqual(
        store.getMessages("user", "100", { before: 6000, limit: 2 }).map((m) => m.id),
        ["4", "5"],
    );
});

test("ChatStore keeps friend names over message display names", () => {
    const store = new ChatStore();
    store.upsertThread({ id: "100", kind: "user", name: "Alice Nguyen", avatar: "a.png" });
    store.addMessage(msg("1", 1000));
    assert.equal(store.listThreads()[0].name, "Alice Nguyen");
});

test("reactions are summarised per emoji with our own marked", () => {
    const store = new ChatStore();
    store.ownId = "me";
    store.addMessage(msg("1", 1000));
    store.applyReaction("1", "100", "/-heart");
    store.applyReaction("1", "me", "/-heart");
    store.applyReaction("1", "300", ":>");
    store.applyReaction("1", "300", "/-strong"); // changes their reaction

    let [m] = store.getMessages("user", "100");
    assert.deepEqual(m.reactions, [
        { key: "heart", emoji: "❤️", count: 2, mine: true },
        { key: "like", emoji: "👍", count: 1, mine: false },
    ]);

    store.applyReaction("1", "me", ""); // removal
    [m] = store.getMessages("user", "100");
    assert.deepEqual(m.reactions[0], { key: "heart", emoji: "❤️", count: 1, mine: false });
});

test("recall blanks the message, delete removes it", () => {
    const store = new ChatStore();
    store.addMessage(msg("1", 1000));
    store.addMessage(msg("2", 2000));
    store.applyReaction("1", "100", "/-heart");

    store.recall("1");
    const [m] = store.getMessages("user", "100");
    assert.equal(m.recalled, true);
    assert.equal(m.text, "Message recalled");
    assert.deepEqual(m.reactions, []);

    assert.ok(store.removeMessage("2"));
    assert.equal(store.getMessage("2"), undefined);
    assert.equal(store.addMessage(msg("2", 2000)), true, "a removed id can come back if Zalo replays it");
});

test("receipts: 1:1 covers earlier own messages, groups count per message", () => {
    const store = new ChatStore();
    store.ownId = "me";
    store.addMessage(msg("1", 1000, { isSelf: true }));
    store.addMessage(msg("2", 2000, { isSelf: true }));
    store.addMessage(msg("3", 3000, { isSelf: true }));

    store.applyReceipt("user", "100", "2", { deliveredTo: ["100"] });
    store.applyReceipt("user", "100", "1", { seenBy: ["100"] });
    assert.deepEqual(
        store.getMessages("user", "100").map((m) => m.status),
        ["seen", "delivered", "sent"],
    );
    assert.deepEqual(store.applyReceipt("user", "100", "1", { seenBy: ["100"] }), [], "no-op receipts change nothing");

    const g = normalizeMessage(raw({ msgId: "g1m", uidFrom: "me" }, { type: 1, threadId: "g1", isSelf: true }));
    store.addMessage(g);
    store.applyReceipt("group", "g1", "g1m", { seenBy: ["a", "b"] });
    store.applyReceipt("group", "g1", "g1m", { seenBy: ["b", "c"] });
    const [gm] = store.getMessages("group", "g1");
    assert.equal(gm.status, "seen");
    assert.equal(gm.seenCount, 3);
});

test("pinned threads sort first; mute and manual unread show in summaries", () => {
    const store = new ChatStore();
    store.upsertThread({ id: "a", kind: "user", name: "A", avatar: "" });
    store.upsertThread({ id: "b", kind: "user", name: "B", avatar: "" });
    store.addMessage({ ...msg("1", 5000), threadId: "a" });
    store.addMessage({ ...msg("2", 1000), threadId: "b" });
    store.markRead("user", "a");
    store.markRead("user", "b");

    assert.deepEqual(
        store.listThreads().map((t) => t.id),
        ["a", "b"],
    );
    store.setPinned("user", "b", true);
    assert.deepEqual(
        store.listThreads().map((t) => t.id),
        ["b", "a"],
    );

    store.setMuted("user", "a", -1);
    store.setMuted("user", "b", 500); // already expired
    const now = 1_000;
    const byId = Object.fromEntries(store.listThreads(now).map((t) => [t.id, t]));
    assert.equal(byId.a.mutedUntil, -1);
    assert.equal(byId.b.mutedUntil, undefined);
    assert.equal(store.isMuted("user", "a"), true);

    store.markUnread("user", "a");
    assert.equal(store.listThreads().find((t) => t.id === "a")?.unread, 1);
    assert.equal(store.markRead("user", "a"), true);
    assert.equal(store.listThreads().find((t) => t.id === "a")?.unread, 0);
});

test("ChatStore persists messages and settings to disk", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "watchhrept-"));
    const file = path.join(dir, "messages.json");
    try {
        const a = new ChatStore(file);
        a.addMessage(msg("1", 1000));
        a.applyReaction("1", "100", "/-heart");
        a.setPinned("user", "100", true);
        a.flush();
        // messages/meta.json and one file per chat, named by the hex of the thread key ("user:100").
        const cache = path.join(dir, "messages");
        assert.equal(fs.existsSync(file), false);
        assert.equal(fs.statSync(cache).mode & 0o777, 0o700);
        assert.equal(fs.statSync(path.join(cache, "threads")).mode & 0o777, 0o700);
        assert.equal(fs.statSync(path.join(cache, "meta.json")).mode & 0o777, 0o600);
        assert.deepEqual(fs.readdirSync(path.join(cache, "threads")), ["757365723a313030.json"]);
        assert.equal(fs.statSync(path.join(cache, "threads", "757365723a313030.json")).mode & 0o777, 0o600);

        const b = new ChatStore(file);
        assert.equal(b.getMessages("user", "100").length, 1);
        assert.equal(b.getMessage("1")?.reactions?.["100"], "/-heart", "message index is rebuilt on load");
        assert.equal(b.listThreads()[0].pinned, true);
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("muteUntil computes the end of each mute option", () => {
    const now = new Date(2026, 0, 1, 9, 30);
    assert.equal(muteUntil("1h", now), now.getTime() + 3_600_000);
    assert.equal(muteUntil("forever", now), -1);
    assert.equal(muteUntil("8am", now), new Date(2026, 0, 2, 8, 0).getTime());
    assert.equal(muteUntil("8am", new Date(2026, 0, 1, 7, 0)), new Date(2026, 0, 1, 8, 0).getTime());
});

test("parseMentions validates positions against the text", () => {
    assert.deepEqual(parseMentions([{ uid: "u1", pos: 0, len: 4 }], "@Bob hi"), [{ uid: "u1", pos: 0, len: 4 }]);
    assert.equal(parseMentions(undefined, "x"), undefined);
    assert.throws(() => parseMentions([{ uid: "", pos: 0, len: 4 }], "@Bob hi"), /uid/);
    assert.throws(() => parseMentions([{ uid: "u1", pos: 5, len: 4 }], "@Bob hi"), /outside/);
    assert.throws(() => parseMentions("nope", "x"), /array/);
});

test("ChatStore backs up corrupt messages.json instead of silently overwriting it", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "watchhrept-corrupt-"));
    const file = path.join(dir, "messages.json");
    try {
        fs.writeFileSync(file, "{not valid json");
        const store = new ChatStore(file);
        assert.deepEqual(store.listThreads(), []);
        const backups = fs.readdirSync(dir).filter((f) => f.startsWith("messages.json.corrupt."));
        assert.equal(backups.length, 1);
        assert.equal(fs.readFileSync(path.join(dir, backups[0]!), "utf8"), "{not valid json");
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore.flush catches filesystem errors without crashing", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "watchhrept-flush-"));
    const blocker = path.join(dir, "blocker");
    try {
        fs.writeFileSync(blocker, "not a directory");
        const store = new ChatStore(path.join(blocker, "messages.json"));
        store.addMessage(msg("1", 1000));
        assert.doesNotThrow(() => store.flush());
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore.addMessage returns false when an older message is immediately evicted", () => {
    const store = new ChatStore();
    for (let i = 1; i <= 500; i++) {
        assert.equal(store.addMessage(msg(String(i), i * 1000)), true);
    }
    assert.equal(store.addMessage(msg("old", 500)), false);
    assert.equal(store.getMessage("old"), undefined);
});

test("ChatStore.addMessage does not clear markedUnread when replaying an old self message", () => {
    const store = new ChatStore();
    store.addMessage(msg("1", 2000));
    store.markRead("user", "100", 2000);
    store.markUnread("user", "100");
    assert.equal(store.listThreads()[0].unread, 1);

    // Replaying an older self message (ts <= readUpTo) must keep markedUnread intact.
    assert.equal(store.addMessage(msg("self-old", 1500, { isSelf: true })), true);
    assert.equal(store.listThreads()[0].unread, 1);

    // A newer self message clears markedUnread.
    assert.equal(store.addMessage(msg("self-new", 3000, { isSelf: true })), true);
    assert.equal(store.listThreads()[0].unread, 0);
});

test("loadConfig refuses a public host without a token", () => {
    assert.throws(() => loadConfig({ HOST: "0.0.0.0" }), /ACCESS_TOKEN/);
    assert.equal(loadConfig({ HOST: "0.0.0.0", ACCESS_TOKEN: "s3cret" }).accessToken, "s3cret");
    assert.equal(loadConfig({}).host, "127.0.0.1");
    assert.equal(loadConfig({}).readReceipts, true);
    assert.equal(loadConfig({ READ_RECEIPTS: "false" }).readReceipts, false);
});

test("normalizeMessage shows the text of notices of an unmodelled type instead of a placeholder", () => {
    const m = normalizeMessage(
        raw({ msgType: "chat.webcontent", content: { title: "Đăng nhập Zalo trên máy tính thành công", href: "https://zalo.me" } }),
    );
    assert.equal(m.text, "Đăng nhập Zalo trên máy tính thành công");
    const empty = normalizeMessage(raw({ msgType: "chat.something.new", content: {} }));
    assert.equal(empty.text, "[Unsupported message]");
});

test("unnamedThreads lists chats still named by their id", () => {
    const store = new ChatStore();
    store.upsertThread({ id: "1", kind: "user", name: "", avatar: "" });
    store.upsertThread({ id: "2", kind: "user", name: "An", avatar: "" });
    store.upsertThread({ id: "3", kind: "group", name: "3", avatar: "" });
    assert.deepEqual(store.unnamedThreads().map((t) => t.id).sort(), ["1", "3"]);
});

/** listThreads' unread counts and searchMessages computed the slow way, from the store's own state. */
function reference(store: ChatStore) {
    const s = store as unknown as {
        threads: Map<string, { id: string; name: string; avatar: string }>;
        messages: Map<string, StoredMessage[]>;
        readUpTo: Map<string, number>;
        markedUnread: Set<string>;
    };
    const fold = (t: string) => t.normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/đ/g, "d").replace(/Đ/g, "D").toLowerCase();
    return {
        unread: Object.fromEntries(
            [...s.threads.keys()].map((key) => {
                const readTs = s.readUpTo.get(key) ?? 0;
                const n = (s.messages.get(key) ?? []).filter((m) => !m.isSelf && m.ts > readTs).length;
                return [key, Math.max(n, s.markedUnread.has(key) ? 1 : 0)];
            }),
        ),
        search(query: string): string[] {
            const needle = fold(query.trim());
            if (!needle) return [];
            const hits: StoredMessage[] = [];
            for (const list of s.messages.values()) {
                for (const m of list) {
                    if (m.recalled) continue;
                    const hit =
                        fold(m.text).includes(needle) ||
                        (m.attachment?.title !== undefined && fold(m.attachment.title).includes(needle)) ||
                        (m.attachment?.description !== undefined && fold(m.attachment.description).includes(needle));
                    if (hit) hits.push(m);
                }
            }
            return hits
                .sort((a, b) => b.ts - a.ts)
                .slice(0, 50)
                .map((m) => m.id);
        },
    };
}

test("cached unread counts and search text match recomputing them on every call", () => {
    const store = new ChatStore();
    const queries = ["cà phê", "ca phe", "DUONG", "báo cáo", "recalled", "hẹn", "tin tuc", "x"];
    const check = (step: string) => {
        const ref = reference(store);
        const unread = Object.fromEntries(store.listThreads(1_000).map((t) => [`${t.kind}:${t.id}`, t.unread]));
        assert.deepEqual(unread, ref.unread, `unread after ${step}`);
        for (const q of queries) {
            assert.deepEqual(
                store.searchMessages(q).map((h) => h.message.id),
                ref.search(q),
                `search "${q}" after ${step}`,
            );
        }
    };
    store.addMessage(inThread("a1", 1000, "a", { text: "Cà phê sữa đá nhé" }));
    store.addMessage(inThread("a2", 2000, "a", { text: "Đường Điện Biên Phủ" }));
    store.addMessage(inThread("a3", 3000, "a", { text: "ok", isSelf: true }));
    store.addMessage(inThread("a4", 4000, "a", { text: "hẹn gặp" }));
    store.addMessage(
        inThread("b1", 1500, "b", { text: "[Link] Tin tức", attachment: { kind: "link", title: "Tin tức", description: "Cà phê Hà Nội" } }),
    );
    store.addMessage(inThread("b2", 2500, "b", { text: "[File] Báo cáo.pdf", attachment: { kind: "file", title: "Báo cáo.pdf" } }));
    check("adding");

    store.markRead("user", "a", 2000);
    check("markRead");
    store.markUnread("user", "b");
    store.markRead("user", "b", 1500);
    check("markUnread then markRead");
    store.applyReaction("a4", "x", "/-heart");
    store.recall("a1");
    check("react and recall");
    store.removeMessage("a4");
    check("remove");
    store.addMessage(inThread("a0", 500, "a", { text: "older cà phê" }));
    check("older message");

    // A plain-id history thread merged into its real thread.
    store.addMessage(inThread("p1", 1200, "123", { text: "đường cũ", fromName: "", raw: undefined }), { history: true });
    store.addMessage(inThread("p2", 5000, "123", { text: "hẹn mai", fromName: "", raw: undefined }), { history: true });
    check("history");
    store.settleHistory("user", "123", new Set(["p1"]));
    check("settleHistory");
    assert.equal(store.migrateThread("user", "123", "b"), true);
    check("migrateThread");

    // Filling a chat past the limit evicts its oldest messages.
    for (let i = 0; i < 505; i++) {
        store.addMessage(inThread(`c${i}`, 10_000 + i, "c", { text: i % 2 ? "x đường" : "y", isSelf: i % 3 === 0 }));
    }
    check("eviction");
    store.markRead("user", "c", 10_300);
    check("markRead after eviction");
    assert.equal(store.addMessage(inThread("c-old", 1, "c")), false);
    check("evicting the new message at once");

    store.clearThreadMessages("user", "a");
    check("clearThreadMessages");
    store.removeThread("user", "b");
    check("removeThread");
    store.clear();
    check("clear");
});

// ---- Saving one file per chat ----

type StoreInternals = {
    threads: Map<string, unknown>;
    messages: Map<string, StoredMessage[]>;
    readUpTo: Map<string, number>;
    pinned: Set<string>;
    mutedUntil: Map<string, number>;
    markedUnread: Set<string>;
    index: Map<string, string>;
    messageCap: number;
    saveInBackground(): Promise<void>;
};

const internals = (store: ChatStore) => store as unknown as StoreInternals;

/** Everything the store saves, as it reads back from JSON. A chat with no messages left has no file. */
function savedState(store: ChatStore) {
    const s = internals(store);
    return JSON.parse(
        JSON.stringify({
            threads: Object.fromEntries(s.threads),
            messages: Object.fromEntries([...s.messages].filter(([, list]) => list.length > 0)),
            readUpTo: Object.fromEntries(s.readUpTo),
            pinned: [...s.pinned].sort(),
            mutedUntil: Object.fromEntries(s.mutedUntil),
            markedUnread: [...s.markedUnread].sort(),
            index: Object.fromEntries(s.index),
            messageCap: s.messageCap,
        }),
    );
}

/** What callers see. */
function publicView(store: ChatStore) {
    return {
        threads: store.listThreads(1_000),
        messages: store.listThreads(1_000).map((t) => store.getMessages(t.kind, t.id, { limit: 500 })),
        search: ["ca phe", "hen", "x"].map((q) => store.searchMessages(q)),
    };
}

/** Pins, mutes, read state, reactions, receipts, a recall, a removal, history and a moved plain-id thread. */
function fillStore(store: ChatStore) {
    store.ownId = "me";
    store.allowMessages(2000);
    store.upsertThread({ id: "a", kind: "user", name: "An", avatar: "https://x/a.jpg" });
    store.upsertThread({ id: "empty", kind: "user", name: "Nobody", avatar: "" });
    store.addMessage(inThread("a1", 1000, "a", { text: "Cà phê sữa đá" }));
    store.addMessage(inThread("a2", 2000, "a", { text: "hẹn mai", isSelf: true, fromId: "me" }));
    store.addMessage(inThread("a3", 3000, "a", { text: "x" }));
    store.applyReaction("a1", "me", "/-heart");
    store.applyReceipt("user", "a", "a2", { deliveredTo: ["a"], seenBy: ["a"] });
    store.addMessage(inThread("b1", 1500, "b", { text: "[Link] Tin", attachment: { kind: "link", title: "Tin", description: "Cà phê" } }));
    store.addMessage(inThread("b2", 2500, "b", { text: "bye" }));
    store.recall("b2");
    store.addMessage(inThread("b3", 2600, "b", { text: "gone" }));
    store.removeMessage("b3");
    store.addMessage({ ...normalizeMessage(raw({ msgId: "g1", uidFrom: "u9", content: "hẹn nhóm" }, { type: 1, threadId: "g" })) });
    store.upsertThread({ id: "g", kind: "group", name: "Nhóm", avatar: "" });
    store.addMessage(inThread("p1", 500, "123", { text: "old x", fromName: "", raw: undefined }), { history: true });
    store.migrateThread("user", "123", "a");
    store.addMessage(inThread("h1", 400, "a", { text: "history x", raw: undefined }), { history: true });
    store.settleHistory("user", "a", new Set(["h1"]));
    store.setPinned("user", "b", true);
    store.setMuted("user", "a", -1);
    store.setMuted("group", "g", 5_000);
    store.markUnread("user", "b");
}

const threadFile = (dir: string, key: string) => path.join(dir, "messages", "threads", `${Buffer.from(key).toString("hex")}.json`);

test("ChatStore reloads exactly what it saved", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-roundtrip-"));
    const file = path.join(dir, "messages.json");
    try {
        const a = new ChatStore(file);
        fillStore(a);
        a.flush();
        const b = new ChatStore(file);
        b.ownId = "me";
        assert.deepEqual(savedState(b), savedState(a));
        assert.deepEqual(publicView(b), publicView(a));
        // The plain-id thread moved into "a" has no file left.
        assert.equal(fs.existsSync(threadFile(dir, "user:123")), false);
        assert.equal(fs.existsSync(threadFile(dir, "user:a")), true);
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore rewrites only the files whose contents changed", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-dirty-"));
    const file = path.join(dir, "messages.json");
    // Every write goes through a new temporary file renamed into place, so a rewritten file has a new inode.
    const inode = (f: string) => fs.statSync(f).ino;
    try {
        const store = new ChatStore(file);
        store.addMessage(inThread("a1", 1000, "a"));
        store.addMessage(inThread("b1", 1000, "b"));
        store.flush();
        const fileA = threadFile(dir, "user:a");
        const fileB = threadFile(dir, "user:b");
        const meta = path.join(dir, "messages", "meta.json");
        const before = { a: inode(fileA), b: inode(fileB), meta: inode(meta) };

        store.applyReaction("a1", "100", "/-heart");
        store.flush();
        assert.notEqual(inode(fileA), before.a, "the changed chat is rewritten");
        assert.equal(inode(fileB), before.b, "the other chat is not");
        assert.equal(inode(meta), before.meta, "a reaction does not touch meta.json");

        store.markRead("user", "b", 5000);
        store.flush();
        assert.notEqual(inode(meta), before.meta, "read markers live in meta.json");
        assert.equal(inode(fileB), before.b, "marking a chat read does not rewrite its messages");

        const now = { a: inode(fileA), b: inode(fileB), meta: inode(meta) };
        store.flush();
        store.markRead("user", "b", 4000); // already read further
        store.flush();
        assert.deepEqual({ a: inode(fileA), b: inode(fileB), meta: inode(meta) }, now, "nothing changed, nothing written");

        store.removeThread("user", "b");
        store.flush();
        assert.equal(fs.existsSync(fileB), false, "a removed chat's file is deleted");
        assert.equal(inode(fileA), now.a);
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore moves an old single-file cache to one file per chat, keeping everything", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-migrate-"));
    const file = path.join(dir, "messages.json");
    try {
        // What the previous version saved: one snapshot of everything.
        const old = new ChatStore();
        fillStore(old);
        const s = internals(old);
        fs.writeFileSync(
            file,
            JSON.stringify({
                threads: Object.fromEntries(s.threads),
                messages: Object.fromEntries(s.messages),
                readUpTo: Object.fromEntries(s.readUpTo),
                pinned: [...s.pinned],
                mutedUntil: Object.fromEntries(s.mutedUntil),
                markedUnread: [...s.markedUnread],
                messageCap: s.messageCap,
            }),
        );

        const migrated = new ChatStore(file);
        migrated.ownId = "me";
        assert.deepEqual(savedState(migrated), savedState(old));
        assert.deepEqual(publicView(migrated), publicView(old));
        assert.equal(fs.existsSync(file), false, "the old file is moved aside");
        assert.ok(fs.existsSync(`${file}.migrated`), "and kept as a backup");
        assert.ok(fs.existsSync(path.join(dir, "messages", "meta.json")));
        assert.equal(fs.readdirSync(path.join(dir, "messages", "threads")).length, 3);

        // The next start reads the new layout.
        const reloaded = new ChatStore(file);
        reloaded.ownId = "me";
        assert.deepEqual(savedState(reloaded), savedState(old));

        // Logging out deletes the backup too.
        reloaded.clear();
        reloaded.flush();
        assert.equal(fs.existsSync(`${file}.migrated`), false);
        assert.deepEqual(fs.readdirSync(path.join(dir, "messages", "threads")), []);
        assert.deepEqual(new ChatStore(file).listThreads(), []);
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore keeps the old cache until every chat of it is written", (t) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-migrate-fail-"));
    const file = path.join(dir, "messages.json");
    try {
        const old = new ChatStore();
        old.addMessage(inThread("a1", 1000, "a"));
        old.addMessage(inThread("b1", 1000, "b"));
        const s = internals(old);
        fs.writeFileSync(file, JSON.stringify({ threads: Object.fromEntries(s.threads), messages: Object.fromEntries(s.messages), readUpTo: {} }));

        // The disk fills up while writing chat "b".
        const write = fs.writeFileSync;
        const failB = t.mock.method(fs, "writeFileSync", (f: fs.PathOrFileDescriptor, ...rest: unknown[]) => {
            if (String(f).includes(Buffer.from("user:b").toString("hex"))) throw new Error("ENOSPC");
            return (write as (...args: unknown[]) => void)(f, ...rest);
        });
        const store = new ChatStore(file);
        assert.equal(store.getMessages("user", "b").length, 1, "the old cache still loads");
        assert.ok(fs.existsSync(file), "the old file stays");
        assert.equal(fs.existsSync(path.join(dir, "messages", "meta.json")), false, "the move is not marked done");
        failB.mock.restore();

        store.flush();
        assert.ok(fs.existsSync(path.join(dir, "messages", "meta.json")));
        assert.equal(fs.existsSync(file), false);
        assert.deepEqual(savedState(new ChatStore(file)), savedState(old));
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore backs up an unreadable chat file and still loads the others", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-corrupt-thread-"));
    const file = path.join(dir, "messages.json");
    try {
        const a = new ChatStore(file);
        a.addMessage(inThread("a1", 1000, "a"));
        a.addMessage(inThread("b1", 1000, "b"));
        a.flush();
        fs.writeFileSync(threadFile(dir, "user:b"), "{not valid json");

        const b = new ChatStore(file);
        assert.equal(b.getMessages("user", "a").length, 1);
        assert.equal(b.getMessages("user", "b").length, 0);
        assert.deepEqual(
            b.listThreads().map((t) => t.id).sort(),
            ["a", "b"],
            "the chat itself is still listed",
        );
        const threads = fs.readdirSync(path.join(dir, "messages", "threads"));
        const backup = threads.find((f) => f.startsWith(`${Buffer.from("user:b").toString("hex")}.json.corrupt.`));
        assert.ok(backup);
        assert.equal(fs.readFileSync(path.join(dir, "messages", "threads", backup), "utf8"), "{not valid json");
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore keeps every chat's messages when meta.json is unreadable", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-corrupt-meta-"));
    const file = path.join(dir, "messages.json");
    try {
        const a = new ChatStore(file);
        a.allowMessages(2000);
        for (let i = 0; i < 600; i++) a.addMessage(inThread(`a${i}`, 1000 + i, "a"));
        a.flush();
        const meta = path.join(dir, "messages", "meta.json");
        fs.writeFileSync(meta, "garbage");

        const b = new ChatStore(file);
        assert.equal(b.getMessages("user", "a", { limit: 500 }).length, 500);
        assert.ok(b.getMessage("a0"), "all 600 messages are there");
        assert.deepEqual(
            b.listThreads().map((t) => [t.id, t.unread]),
            [["a", 0]],
            "the chat is listed again, its history read",
        );
        b.addMessage(inThread("new", 5000, "a"));
        assert.ok(b.getMessage("a0"), "the raised limit is kept");
        assert.ok(fs.readdirSync(path.join(dir, "messages")).some((f) => f.startsWith("meta.json.corrupt.")));
        b.flush();
        assert.deepEqual(savedState(new ChatStore(file)), savedState(b));
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test("ChatStore saves in the background, and a sync flush in the middle wins", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-async-"));
    const file = path.join(dir, "messages.json");
    try {
        const store = new ChatStore(file);
        store.addMessage(inThread("a1", 1000, "a"));
        await internals(store).saveInBackground();
        assert.deepEqual(savedState(new ChatStore(file)), savedState(store), "saved without a flush");

        // A newer message while the background save is still writing: flushing at shutdown keeps it.
        store.addMessage(inThread("a2", 2000, "a"));
        const saving = internals(store).saveInBackground();
        store.addMessage(inThread("a3", 3000, "a"));
        store.flush();
        await saving;
        assert.deepEqual(
            new ChatStore(file).getMessages("user", "a").map((m) => m.id),
            ["a1", "a2", "a3"],
        );

        // Exiting right after a flush, before the background write lands: the flush wrote it already.
        store.addMessage(inThread("a4", 4000, "a"));
        const pending = internals(store).saveInBackground();
        store.flush();
        assert.deepEqual(
            new ChatStore(file).getMessages("user", "a").map((m) => m.id),
            ["a1", "a2", "a3", "a4"],
        );
        await pending;

        // Logging out while a background save is writing: the old messages must not come back.
        store.addMessage(inThread("b1", 1000, "b"));
        const writing = internals(store).saveInBackground();
        store.clear();
        store.flush();
        await writing;
        assert.deepEqual(new ChatStore(file).listThreads(), []);
        assert.deepEqual(fs.readdirSync(path.join(dir, "messages", "threads")), [], "no files or temporary files left");
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});
