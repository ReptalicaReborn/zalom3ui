import assert from "node:assert/strict";
import crypto from "node:crypto";
import { test } from "node:test";
import { parseJsonKeepingBigInts } from "../src/calls.js";
import { decodeZaloAes, parseNoisedIdsReply } from "../src/historysync.js";
import { ChatStore } from "../src/store.js";
import { convertConversation, importConversation, isOutgoing, type BackupConversation, type BackupRow } from "../src/syncimport.js";

const row = (senderId: string, outgoing?: boolean, extra: Partial<BackupRow> = {}): BackupRow => ({
    globalMsgId: "1",
    cliMsgId: "1",
    msgType: 1,
    ts: 1,
    senderId,
    outgoing,
    text: "x",
    ttl: 0,
    bin: { attachs: [], mentions: [], quoteStatus: 0, dataSource: "", text: "" } as unknown as BackupRow["bin"],
    ...extra,
});

test("isOutgoing: sender \"0\", myPlainUid, or our noised id is us", () => {
    assert.equal(isOutgoing(row("0"), { id: "100", isGroup: false }, "9999999999999999999"), true);
    assert.equal(isOutgoing(row("9999999999999999999"), { id: "g1", isGroup: true }, "9999999999999999999"), true);
    assert.equal(isOutgoing(row("42424242"), { id: "777", isGroup: true }, "9999999999999999999", "42424242"), true);
});

test("isOutgoing: in a 1:1 chat only the other person's id is theirs", () => {
    const conv = { id: "100", isGroup: false };
    assert.equal(isOutgoing(row("100"), conv, "me"), false);
    assert.equal(isOutgoing(row("some-other-form-of-my-id"), conv, "me"), true);
    assert.equal(isOutgoing(row(""), conv, "me"), false);
});

test("isOutgoing: in a group a stranger's id is theirs", () => {
    assert.equal(isOutgoing(row("555"), { id: "g1", isGroup: true }, "me", "42424242"), false);
});

test("isOutgoing: an explicit direction column wins", () => {
    assert.equal(isOutgoing(row("100", true), { id: "100", isGroup: false }, "me"), true);
    assert.equal(isOutgoing(row("0", false), { id: "100", isGroup: false }, "me"), false);
});

test("parseNoisedIdsReply handles both parallel arrays and map objects", () => {
    const arr = parseNoisedIdsReply(["101", "102"], ["201"], {
        fids: ["1000000000000000101", "0"],
        gids: ["2000000000000000201"],
    });
    assert.equal(arr.users.get("101"), "1000000000000000101");
    assert.equal(arr.users.has("102"), false);
    assert.equal(arr.groups.get("201"), "2000000000000000201");

    const map = parseNoisedIdsReply(["101", "102"], ["201"], {
        fids: { "101": "1000000000000000101", "102": "" },
        gids: { "201": "2000000000000000201" },
    });
    assert.equal(map.users.get("101"), "1000000000000000101");
    assert.equal(map.users.has("102"), false);
    assert.equal(map.groups.get("201"), "2000000000000000201");

    const fromString = parseNoisedIdsReply(
        ["101"],
        ["201"],
        '{"fids":[1000000000000000101],"gids":{"201":2000000000000000201}}',
    );
    assert.equal(fromString.users.get("101"), "1000000000000000101");
    assert.equal(fromString.groups.get("201"), "2000000000000000201");
});

test("decodeZaloAes decrypts URL-encoded AES payloads and preserves 19-digit ids with parseJsonKeepingBigInts", () => {
    const key = crypto.randomBytes(16);
    const secretKey = key.toString("base64");
    const rawJson = '{"error_code":0,"data":{"fids":[1234567890123456789],"gids":{"777":9876543210987654321}}}';
    const cipher = crypto.createCipheriv("aes-128-cbc", key, Buffer.alloc(16, 0));
    const enc = encodeURIComponent(Buffer.concat([cipher.update(rawJson, "utf8"), cipher.final()]).toString("base64"));

    const plain = decodeZaloAes(secretKey, enc);
    assert.equal(plain, rawJson);
    const parsed = parseJsonKeepingBigInts(plain!) as { data: unknown };
    const reply = parseNoisedIdsReply(["324724890"], ["777"], parsed.data);
    assert.equal(reply.users.get("324724890"), "1234567890123456789");
    assert.equal(reply.groups.get("777"), "9876543210987654321");
});

test("convertConversation and importConversation map plain ids to noised ids for 1:1 and groups", () => {
    const ownNoised = "9000000000000000001";
    const peerNoised = "9000000000000000002";
    const groupNoised = "8000000000000000003";
    const idMap = {
        users: new Map([["324724890", peerNoised]]),
        groups: new Map([["721897407", groupNoised]]),
        myPlainUid: "111222333",
    };

    const groupConv: BackupConversation = {
        id: "721897407",
        isGroup: true,
        sourceRows: 2,
        rows: [
            row("111222333", undefined, { globalMsgId: "m1", cliMsgId: "c1", ts: 1000, text: "my group msg" }),
            row("324724890", undefined, {
                globalMsgId: "m2",
                cliMsgId: "c2",
                ts: 2000,
                text: "@Me hi",
                bin: {
                    attachs: [],
                    mentions: [{ uid: "111222333", pos: 0, len: 3, type: 0 }],
                    quote: { ownerId: "324724890", cliMsgId: "c1", globalMsgId: "m1", type: 0, ts: 1000, msg: "my group msg", attach: "", fromD: "" },
                    quoteStatus: 1,
                    dataSource: "",
                    text: "",
                },
            }),
        ],
    };

    const names: Record<string, string> = { [peerNoised]: "Alice" };
    const converted = convertConversation(groupConv, ownNoised, (id) => names[id] ?? "", idMap);
    assert.equal(converted.threadId, groupNoised);
    assert.equal(converted.messages.length, 2);
    assert.equal(converted.messages[0].threadId, groupNoised);
    assert.equal(converted.messages[0].isSelf, true);
    assert.equal(converted.messages[0].data.uidFrom, ownNoised);
    assert.equal(converted.messages[1].isSelf, false);
    assert.equal(converted.messages[1].data.uidFrom, peerNoised);
    assert.equal(converted.messages[1].data.dName, "Alice");
    assert.equal(converted.messages[1].data.quote?.ownerId, peerNoised);
    assert.equal(converted.messages[1].data.quote?.fromD, "Alice");
    assert.equal(converted.messages[1].data.mentions?.[0].uid, ownNoised);

    const store = new ChatStore();
    store.ownId = ownNoised;
    store.upsertThread({ id: groupNoised, kind: "group", name: "Study Group", avatar: "https://a/g.jpg" });
    const counts = importConversation(groupConv, ownNoised, store, (id) => names[id] ?? "", idMap);
    assert.equal(counts.imported, 2);
    assert.equal(store.getMessages("group", groupNoised).length, 2);
    assert.equal(store.getThread("group", "721897407"), undefined);

    // Unmapped conversation is skipped rather than creating a numeric-id thread.
    const unmapped: BackupConversation = { id: "99999999", isGroup: false, sourceRows: 1, rows: [row("99999999")] };
    const skipped = importConversation(unmapped, ownNoised, store, () => "", idMap);
    assert.equal(skipped.imported, 0);
    assert.equal(skipped.skipped, 1);
    assert.equal(store.getThread("user", "99999999"), undefined);
});

test("ChatStore.migrateThread merges old plain-id history into a named noised thread and dropPlainThread frees ids", () => {
    const store = new ChatStore();
    store.ownId = "9000000000000000001";
    const peerNoised = "9000000000000000002";
    store.upsertThread({ id: peerNoised, kind: "user", name: "Alice", avatar: "https://a/alice.jpg" });

    // Old sync imported under plain id (no `raw` field).
    store.addMessage(
        {
            id: "hist1",
            cliMsgId: "c1",
            threadId: "324724890",
            threadKind: "user",
            fromId: "324724890",
            fromName: "",
            isSelf: false,
            ts: 1000,
            msgType: "webchat",
            text: "hello",
        },
        { history: true },
    );
    store.settleHistory("user", "324724890", new Set(["hist1"]));

    assert.equal(store.migrateThread("user", "324724890", peerNoised), true);
    assert.equal(store.getThread("user", "324724890"), undefined);
    const msgs = store.getMessages("user", peerNoised);
    assert.equal(msgs.length, 1);
    assert.equal(msgs[0].threadId, peerNoised);
    assert.equal(msgs[0].fromId, peerNoised);
    assert.equal(msgs[0].fromName, "Alice");
    // Read state carried over: not unread.
    assert.equal(store.listThreads().find((t) => t.id === peerNoised)?.unread, 0);

    // dropPlainThread frees message ids for re-import, but refuses to touch named threads.
    assert.equal(store.dropPlainThread("user", peerNoised), false);
    store.addMessage(
        {
            id: "hist2",
            cliMsgId: "c2",
            threadId: "68386082",
            threadKind: "user",
            fromId: "68386082",
            fromName: "",
            isSelf: false,
            ts: 2000,
            msgType: "webchat",
            text: "old",
        },
        { history: true },
    );
    assert.equal(store.dropPlainThread("user", "68386082"), true);
    assert.equal(store.getThread("user", "68386082"), undefined);
    // Freed id can now be added to the mapped thread.
    assert.equal(
        store.addMessage(
            {
                id: "hist2",
                cliMsgId: "c2",
                threadId: peerNoised,
                threadKind: "user",
                fromId: peerNoised,
                fromName: "Alice",
                isSelf: false,
                ts: 2000,
                msgType: "webchat",
                text: "new",
            },
            { history: true },
        ),
        true,
    );
});
