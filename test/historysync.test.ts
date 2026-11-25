import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
    DownloadError,
    HistorySync,
    backupDownloader,
    isZaloCookieHost,
    classifyTransferError,
    downloadRetryDelayMs,
    syncControlOf,
    syncControlsIn,
    syncControlsOfFrame,
    type HistorySyncDeps,
    type HistorySyncStatus,
} from "../src/historysync.js";
import { CookieJar } from "tough-cookie";
import { createServer } from "../src/server.js";
import { ZaloSession } from "../src/session.js";
import { ChatStore, HISTORY_MESSAGES_PER_THREAD, type StoredMessage } from "../src/store.js";
import { convertConversation, importConversation, parseBinNet, readConversation, toMillis } from "../src/syncimport.js";
import { BackupError, backupKeyText, conversationFiles, decryptChunks, deriveAesKey, generateSyncKeyPair, rsaDecryptPkcs1, unpackBackup } from "../src/zdb.js";
import { encryptChunks, packBackup } from "./synchelpers.js";

const fixture = (name: string) => fs.readFileSync(fileURLToPath(new URL(`./fixtures/${name}`, import.meta.url)));
const FILES = JSON.parse(fixture("sync_files.json").toString()) as [string, number][];
const BODY = fixture("sync_body.xz");
const OWN = "555";

// ---- Cryptography ----

test("the AES key is the first 32 key characters, upper-cased", () => {
    const key = deriveAesKey("0123456789abcdefABCDEF0123456789zzzz");
    assert.equal(key.length, 32);
    assert.equal(key.toString("latin1"), "0123456789ABCDEFABCDEF0123456789");
    // Shorter text is zero-padded.
    const short = deriveAesKey("abc");
    assert.equal(short.toString("latin1", 0, 3), "ABC");
    assert.ok(short.subarray(3).every((b) => b === 0));
});

test("the key text is hex for format 1 and base64 for format 0", () => {
    const k = Buffer.from([0xde, 0xad, 0xbe, 0xef]);
    assert.equal(backupKeyText(k, 1), "deadbeef");
    assert.equal(backupKeyText(k, 0), "3q2+7w==");
});

test("AES chunks restart the IV every 64 KiB", () => {
    const key = deriveAesKey("K".repeat(32));
    const plain = Buffer.alloc(0x10000 + 64, 7); // two chunks of identical blocks
    const enc = encryptChunks(plain, key);
    // First block of each chunk is encrypted with the zero IV, so they are equal.
    assert.ok(enc.subarray(0, 16).equals(enc.subarray(0x10000, 0x10000 + 16)));
    assert.ok(decryptChunks(enc, key).equals(plain));
    assert.throws(() => decryptChunks(enc.subarray(0, 30), key), BackupError);
});

test("RSA PKCS#1 v1.5: a key encrypted to our public key comes back; garbage does not", () => {
    const pair = generateSyncKeyPair();
    const spki = crypto.createPublicKey({ key: Buffer.from(pair.publicKey, "base64"), format: "der", type: "spki" });
    const secret = crypto.randomBytes(16);
    const cipher = crypto.publicEncrypt({ key: spki, padding: crypto.constants.RSA_PKCS1_PADDING }, secret);
    const back = rsaDecryptPkcs1(pair.privateKeyPem, cipher.toString("base64"));
    assert.ok(back && back.equals(secret));
    assert.equal(rsaDecryptPkcs1(pair.privateKeyPem, crypto.randomBytes(256).toString("base64")), null);
    assert.equal(rsaDecryptPkcs1(pair.privateKeyPem, ""), null);
    assert.equal(rsaDecryptPkcs1("not a key", cipher.toString("base64")), null);
    // OAEP is a different scheme: refused.
    const oaep = crypto.publicEncrypt({ key: spki, oaepHash: "sha1" }, secret);
    assert.equal(rsaDecryptPkcs1(pair.privateKeyPem, oaep.toString("base64")), null);
});

const KEY_TEXT = "00112233445566778899aabbccddeeff";
const specs = FILES.map(([name, size]) => ({ name, size }));

test("unpackBackup: decrypts, decompresses and splits the files", () => {
    const file = packBackup(specs, BODY, KEY_TEXT);
    const files = unpackBackup(file, KEY_TEXT);
    assert.deepEqual(files.map((f) => [f.name, f.data.length]), FILES);
    assert.ok(files[0].data.equals(fixture("sync_1001.db")));
    assert.ok(files[1].data.equals(fixture("sync_group_777.db")));
    assert.deepEqual(conversationFiles(files).map((c) => [c.id, c.isGroup]), [["1001", false], ["777", true]]);
});

test("unpackBackup: four extra bytes at the start or the end are tolerated", () => {
    for (const opts of [{ prefix: 4 }, { suffix: 4 }]) {
        const files = unpackBackup(packBackup(specs, BODY, KEY_TEXT, opts), KEY_TEXT);
        assert.equal(files.length, 2);
    }
});

test("unpackBackup: a wrong key, damaged data and unsafe names are refused", () => {
    const file = packBackup(specs, BODY, KEY_TEXT);
    assert.throws(() => unpackBackup(file, "ffeeddccbbaa99887766554433221100"), /Wrong key/);
    assert.throws(() => unpackBackup(file.subarray(0, 40), KEY_TEXT), BackupError);
    assert.throws(() => unpackBackup(Buffer.alloc(100, 1), KEY_TEXT), BackupError);
    assert.throws(() => unpackBackup(packBackup([{ name: "../evil.db", size: 1 }], BODY, KEY_TEXT), KEY_TEXT), /unsafe/);
    // The file table promises more bytes than the data holds.
    assert.throws(() => unpackBackup(packBackup([{ name: "1001.db", size: 10_000_000 }], BODY, KEY_TEXT), KEY_TEXT), BackupError);
    assert.throws(() => unpackBackup(packBackup(specs, Buffer.from("garbage that is not lzma"), KEY_TEXT), KEY_TEXT), BackupError);
});

test("conversationFiles: the three naming schemes, and only real SQLite", () => {
    const db = fixture("sync_1001.db");
    const found = conversationFiles([
        { name: "100.db", data: db },
        { name: "group_200.db", data: db },
        { name: "g300.db", data: db },
        { name: "db/400.db", data: db },
        { name: "abc.db", data: db },
        { name: "500.db", data: Buffer.from("not sqlite at all, but long enough to look like something") },
        { name: "readme.txt", data: db },
    ]);
    assert.deepEqual(found.map((c) => `${c.isGroup ? "g" : "u"}${c.id}`), ["u100", "g200", "g300", "u400"]);
});

// ---- BinNet ----

function tlv(items: [number, Buffer][], little = true): Buffer {
    return Buffer.concat(
        items.map(([k, v]) => {
            const head = Buffer.alloc(8);
            if (little) {
                head.writeUInt32LE(k, 0);
                head.writeUInt32LE(v.length, 4);
            } else {
                head.writeUInt32BE(k, 0);
                head.writeUInt32BE(v.length, 4);
            }
            return Buffer.concat([head, v]);
        }),
    );
}
const i32 = (n: number, little = true) => {
    const b = Buffer.alloc(4);
    if (little) b.writeInt32LE(n);
    else b.writeInt32BE(n);
    return b;
};
const i64 = (n: bigint, little = true) => {
    const b = Buffer.alloc(8);
    if (little) b.writeBigInt64LE(n);
    else b.writeBigInt64BE(n);
    return b;
};

test("parseBinNet: text, attachment, quote and mentions, in either byte order", () => {
    for (const little of [true, false]) {
        const attach = tlv([[0x28, Buffer.from("photo")], [0x2a, i32(77, little)], [0x2f, Buffer.from("pic.jpg")], [0x30, Buffer.from("https://x.invalid/p")], [0x2e, Buffer.from('{"width":1}')]], little);
        const quote = tlv([[0x50, i64(555n, little)], [0x51, i64(802n, little)], [0x52, i64(7351234567890123456n, little)], [0x54, i64(1700000000000n, little)], [0x56, Buffer.from("old text")], [0x5a, Buffer.from("Alice")]], little);
        const mention = tlv([[0x65, i64(1002n, little)], [0x66, i32(3, little)], [0x67, i32(4, little)], [0x68, i32(0, little)]], little);
        const bin = parseBinNet(tlv([[0x06, attach], [0x07, quote], [0x08, mention], [0x0b, Buffer.from("hello Bob")]], little));
        assert.equal(bin.text, "hello Bob");
        assert.equal(bin.attachs.length, 1);
        assert.equal(bin.attachs[0].type, "photo");
        assert.equal(bin.attachs[0].id, 77);
        assert.equal(bin.attachs[0].title, "pic.jpg");
        assert.equal(bin.attachs[0].params, '{"width":1}');
        assert.equal(bin.quote?.globalMsgId, "7351234567890123456");
        assert.equal(bin.quote?.ownerId, "555");
        assert.equal(bin.quote?.msg, "old text");
        assert.equal(bin.quote?.fromD, "Alice");
        assert.deepEqual(bin.mentions, [{ uid: "1002", pos: 3, len: 4, type: 0 }]);
    }
});

test("parseBinNet: UTF-8 text, UTF-16 text, and junk", () => {
    assert.equal(parseBinNet(tlv([[0x0b, Buffer.from("xin chào 😀")]])).text, "xin chào 😀");
    assert.equal(parseBinNet(tlv([[0x0b, Buffer.from("abc def", "utf16le")]])).text, "abc def");
    assert.deepEqual(parseBinNet(Buffer.from("junk")).attachs, []);
    assert.equal(parseBinNet(null).text, "");
    // A length that runs past the end is not a TLV box.
    const bad = Buffer.alloc(8);
    bad.writeUInt32LE(0x0b, 0);
    bad.writeUInt32LE(999, 4);
    assert.equal(parseBinNet(bad).text, "");
});

// ---- Rows to messages ----

test("readConversation: keeps delivered rows with a client id, oldest first", () => {
    const conv = readConversation(fixture("sync_1001.db"), "1001", false)!;
    assert.equal(conv.sourceRows, 12);
    // Dropped: status 0 and the row without a client message id.
    assert.equal(conv.rows.length, 10);
    assert.ok(conv.rows.every((r, i) => i === 0 || conv.rows[i - 1].ts <= r.ts));
    assert.equal(readConversation(fixture("sync_many.db"), "9", false)!.rows.length, 401);
    assert.equal(toMillis(1700000000), 1700000000000);
    assert.equal(toMillis(1700000000000), 1700000000000);
});

test("convertConversation: ids, senders, attachments, quotes, recalls and skipped types", () => {
    const conv = readConversation(fixture("sync_1001.db"), "1001", false)!;
    const { messages, recalled, skipped } = convertConversation(conv, OWN, (id) => (id === "1001" ? "Alice" : ""));
    assert.deepEqual(recalled, ["9001"]);
    assert.deepEqual(skipped, { type_33: 1 });
    const byId = new Map(messages.map((m) => [m.data.msgId, m]));
    // Both rows with global id 9001 are kept here (deduping is the store's job); the global id "0" falls back to the client id.
    assert.ok(byId.has("808"));
    const mine = byId.get("9002")!;
    assert.equal(mine.isSelf, true);
    assert.equal(mine.data.uidFrom, OWN);
    assert.equal(mine.data.idTo, "1001");
    assert.equal(mine.threadId, "1001");
    const theirs = byId.get("9001")!;
    assert.equal(theirs.isSelf, false);
    assert.equal(theirs.data.uidFrom, "1001");
    assert.equal(theirs.data.idTo, OWN);
    assert.equal(theirs.data.dName, "Alice");
    assert.equal(theirs.data.content, "hello");
    const photo = byId.get("9003")!;
    assert.equal(photo.data.msgType, "chat.photo");
    assert.equal((photo.data.content as { title: string }).title, "pic.jpg");
    const quoted = byId.get("9004")!;
    assert.equal(quoted.data.quote?.globalMsgId, "9002");
    assert.equal(quoted.data.quote?.msg, "hi back");
    assert.equal(byId.get("7351234567890123456")?.data.msgId, "7351234567890123456");
});

test("convertConversation: group senders and mentions", () => {
    const conv = readConversation(fixture("sync_group_777.db"), "777", true)!;
    const { messages } = convertConversation(conv, OWN);
    assert.equal(messages.length, 4);
    assert.ok(messages.every((m) => m.type === 1 && m.threadId === "777" && m.data.idTo === "777"));
    assert.equal(messages[0].data.uidFrom, "1001");
    assert.deepEqual(messages[1].data.mentions, [{ uid: "1002", pos: 0, len: 4, type: 0 }]);
    assert.equal(messages[2].isSelf, true);
    assert.equal(messages[2].data.uidFrom, OWN);
    // A sender recorded as our own id counts as ours, and "@all" is uid -1.
    assert.equal(messages[3].isSelf, true);
    assert.deepEqual(messages[3].data.mentions, [{ uid: "-1", pos: 0, len: 4, type: 0 }]);
});

// ---- Into the store ----

test("importConversation: dedupes by id, keeps order, applies recalls, leaves nothing unread", () => {
    const store = new ChatStore();
    store.ownId = OWN;
    const live: StoredMessage = {
        id: "9002",
        cliMsgId: "802",
        threadId: "1001",
        threadKind: "user",
        fromId: OWN,
        fromName: "",
        isSelf: true,
        ts: 1700000001000,
        msgType: "webchat",
        text: "hi back (live copy)",
    };
    store.addMessage(live);
    const conv = readConversation(fixture("sync_1001.db"), "1001", false)!;
    const counts = importConversation(conv, OWN, store);
    // 10 rows kept: 8 become messages (one undo row, one system type), of which 9001 repeats and 9002 is already stored.
    assert.deepEqual(counts, { imported: 6, duplicates: 2, skipped: 1, recalled: 1 });
    const list = store.getMessages("user", "1001", { limit: 100 });
    assert.deepEqual(list.map((m) => m.ts), [...list.map((m) => m.ts)].sort((a, b) => a - b));
    assert.equal(list.find((m) => m.id === "9002")!.text, "hi back (live copy)", "the stored copy wins");
    assert.equal(list.find((m) => m.id === "9001")!.recalled, true);
    assert.equal(store.listThreads().find((t) => t.id === "1001")!.unread, 0, "history is not unread");
    // Running it again adds nothing.
    const again = importConversation(readConversation(fixture("sync_1001.db"), "1001", false)!, OWN, store);
    assert.equal(again.imported, 0);
    // Imported messages carry no raw copy: they can't be quoted or reacted to.
    assert.equal(store.getMessage("9003")?.raw, undefined);
});

test("importConversation: a newer unread live message stays unread", () => {
    const store = new ChatStore();
    const newer: StoredMessage = {
        id: "live1",
        cliMsgId: "c",
        threadId: "1001",
        threadKind: "user",
        fromId: "1001",
        fromName: "Alice",
        isSelf: false,
        ts: 1800000000000,
        msgType: "webchat",
        text: "fresh",
    };
    store.addMessage(newer);
    importConversation(readConversation(fixture("sync_1001.db"), "1001", false)!, OWN, store);
    assert.equal(store.listThreads().find((t) => t.id === "1001")!.unread, 1);
});

test("a history import can exceed the usual per-chat limit, and the limit survives a restart", () => {
    const store = new ChatStore();
    store.allowMessages(HISTORY_MESSAGES_PER_THREAD);
    const conv = readConversation(fixture("sync_many.db"), "1001", false)!;
    // 401 rows only; widen it past 500 by importing the same rows under shifted ids.
    const wide = { ...conv, rows: Array.from({ length: 3 }, (_, k) => conv.rows.map((r) => ({ ...r, globalMsgId: `${r.globalMsgId}-${k}`, cliMsgId: `${r.cliMsgId}-${k}` }))).flat() };
    const counts = importConversation(wide, OWN, store);
    assert.equal(counts.imported, 1203);
    assert.equal(store.getMessages("user", "1001", { limit: 500 }).length, 500);
    // Without the raised limit the older messages fall off at once.
    const plain = new ChatStore();
    assert.ok(importConversation(wide, OWN, plain).imported < 1203);
    assert.equal(plain.getMessages("user", "1001", { limit: 500 }).length, 500);
    // The raised limit is saved and read back.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-cap-"));
    const file = path.join(dir, "messages.json");
    const saved = new ChatStore(file);
    saved.allowMessages(HISTORY_MESSAGES_PER_THREAD);
    importConversation(wide, OWN, saved);
    saved.flush();
    const reloaded = new ChatStore(file);
    reloaded.addMessage({ id: "new", cliMsgId: "n", threadId: "1001", threadKind: "user", fromId: "1001", fromName: "", isSelf: false, ts: Date.now(), msgType: "webchat", text: "x" });
    assert.equal(reloaded.getMessage("9001") === undefined, true);
    assert.ok(reloaded.getMessage("20000-0"), "the oldest imported message is still there after a restart");
    fs.rmSync(dir, { recursive: true, force: true });
});

// ---- Controls and small pure pieces ----

test("syncControlOf: reads the control shapes Zalo uses, and ignores others", () => {
    const control = syncControlOf({ act_type: "syncmsgmb", act: "user_confirm", data: JSON.stringify({ user_action: 1, uid: "123456789012345678" }) });
    assert.equal(control?.act, "user_confirm");
    assert.equal(control?.data.user_action, 1);
    assert.equal(control?.data.uid, "123456789012345678", "long ids stay exact");
    assert.equal(syncControlOf({ act_type: "voip", act: "request" }), null);
    assert.equal(syncControlOf(null), null);
    assert.equal(syncControlsIn([{ content: { act_type: "transfer_after_login", act: "x", data: { temp_key: "k" } } }, { content: { act_type: "fr" } }, null]).length, 1);
});

test("syncControlsOfFrame: reads a plain control frame (cmd 601)", () => {
    const payload = JSON.stringify({ data: JSON.stringify({ controls: [{ content: { act_type: "syncmsgmb", act: "user_confirm", data: { user_action: 1 } } }] }), encrypt: 0 });
    const frame = Buffer.concat([Buffer.from([1, 601 & 0xff, 601 >> 8, 0]), Buffer.from(payload)]);
    const found = syncControlsOfFrame(frame, undefined);
    assert.equal(found.length, 1);
    assert.equal(found[0].actType, "syncmsgmb");
    // Other commands, and frames that aren't JSON, give nothing (and don't throw for the first).
    assert.deepEqual(syncControlsOfFrame(Buffer.concat([Buffer.from([1, 0x5f, 0x02, 1]), Buffer.from(payload)]), undefined), []);
});

test("classifyTransferError", () => {
    assert.equal(classifyTransferError({ status: 2 }), "phone_idle");
    assert.equal(classifyTransferError({ status: 1, error_code: 5 }), "phone_active");
    assert.equal(classifyTransferError({ status: 0, error_code: 5 }), "failed");
    assert.equal(classifyTransferError({}), "ignore");
});

test("downloadRetryDelayMs: backoff, retryable statuses, Retry-After and the attempt limit", () => {
    assert.equal(downloadRetryDelayMs(404, 1, undefined), 1000);
    assert.equal(downloadRetryDelayMs(404, 2, undefined), 2000);
    assert.equal(downloadRetryDelayMs(500, 4, undefined), 8000);
    assert.equal(downloadRetryDelayMs(0, 1, undefined), 1000, "network failure");
    assert.equal(downloadRetryDelayMs(403, 1, undefined), -1);
    assert.equal(downloadRetryDelayMs(404, 5, undefined), -1);
    assert.equal(downloadRetryDelayMs(429, 1, "5"), 5000);
    assert.equal(downloadRetryDelayMs(429, 1, "3600"), 30_000);
    // A date: HTTP dates have whole-second resolution, so use one a few seconds ahead.
    const now = Date.UTC(2026, 0, 1, 12, 0, 0);
    assert.equal(downloadRetryDelayMs(503, 1, new Date(now + 6000).toUTCString(), now), 6000);
    assert.equal(downloadRetryDelayMs(503, 1, new Date(now - 6000).toUTCString(), now), 1000, "a date in the past falls back to the backoff");
    assert.equal(downloadRetryDelayMs(404, 1, "5"), 1000, "Retry-After only counts for 429 and 503");
});

// ---- The controller, with a fake phone ----


function harness(overrides: Partial<HistorySyncDeps> = {}, opts: { keyText?: string; ownId?: string | null } = {}) {
    const store = new ChatStore();
    store.ownId = OWN;
    const calls: { endpoint: string; params: Record<string, unknown> }[] = [];
    const states: HistorySyncStatus[] = [];
    const keyPair = generateSyncKeyPair();
    const spki = crypto.createPublicKey({ key: Buffer.from(keyPair.publicKey, "base64"), format: "der", type: "spki" });
    const keyText = opts.keyText ?? KEY_TEXT;
    // The phone sends the backup key (here 16 bytes, shown as hex) encrypted to our public key.
    const encryptedKey = crypto.publicEncrypt({ key: spki, padding: crypto.constants.RSA_PKCS1_PADDING }, Buffer.from(KEY_TEXT, "hex")).toString("base64");
    const backup = packBackup(specs, BODY, keyText);
    const downloads: string[] = [];
    const deps: HistorySyncDeps = {
        ownId: () => (opts.ownId === undefined ? OWN : opts.ownId),
        imei: () => "imei-1",
        pcName: "Zalo-Android",
        call: async (endpoint, params) => {
            calls.push({ endpoint, params });
            return {};
        },
        download: async (url, onProgress) => {
            downloads.push(url);
            onProgress(backup.length / 2, backup.length);
            onProgress(backup.length, backup.length);
            return backup;
        },
        sink: store,
        keyPair: () => keyPair,
        sleep: async () => {},
        ...overrides,
    };
    const sync = new HistorySync(deps);
    sync.on("change", (s) => states.push(s));
    const info = (extra: Record<string, unknown> = {}) => ({
        public_key: keyPair.publicKey,
        pc_name: "Zalo-Android",
        url: "https://files.example.invalid/backup/abc",
        encrypted_key: encryptedKey,
        file_size: backup.length,
        db_info: JSON.stringify({ db_format: 1, origin_db: { msg_total: 16 } }),
        ...extra,
    });
    const control = (act: string, data: Record<string, unknown>) =>
        sync.handleControl({ actType: "syncmsgmb", act, data: { public_key: keyPair.publicKey, pc_name: "Zalo-Android", ...data } });
    const settle = async (want: string) => {
        for (let i = 0; i < 200 && sync.getStatus().state !== want; i++) await new Promise((r) => setTimeout(r, 5));
        return sync.getStatus();
    };
    return { sync, store, calls, states, keyPair, info, control, settle, downloads, backup };
}

test("controller: the whole flow, from the request to the imported messages", async () => {
    const h = harness();
    const started = await h.sync.start();
    assert.equal(started.state, "waiting_for_phone");
    assert.equal(h.calls[0].endpoint, "pull_mobile_msg");
    assert.deepEqual(h.calls[0].params, {
        pc_name: "Zalo-Android",
        public_key: h.keyPair.publicKey,
        from_seq_id: 0,
        is_retry: 0,
        min_seq_id: 0,
        temp_key: "",
        imei: "imei-1",
    });
    // Someone else's sync (another public key) is ignored.
    h.sync.handleControl({ actType: "syncmsgmb", act: "user_confirm", data: { public_key: "other", user_action: 0 } });
    assert.equal(h.sync.getStatus().state, "waiting_for_phone");

    h.control("user_confirm", { user_action: 1 });
    assert.equal(h.sync.getStatus().progress, 15);
    h.control("syncmsg_info", h.info());
    const done = await h.settle("done");
    assert.equal(done.state, "done", done.error ?? "");
    assert.equal(done.progress, 100);
    assert.equal(done.counts?.conversations, 2);
    // 7 + 4 messages are new in the two conversations (see the import tests), nothing is unread.
    assert.equal(done.counts?.imported, 11);
    assert.equal(h.store.getMessages("group", "777").length, 4);
    assert.ok(h.store.getMessages("user", "1001").length >= 7);
    assert.equal(h.store.listThreads().reduce((n, t) => n + t.unread, 0), 0);
    assert.deepEqual(h.downloads, ["https://files.example.invalid/backup/abc"]);
    // Zalo is told to drop the snapshot.
    assert.ok(h.calls.some((c) => c.endpoint === "delete_snapshot_mobile_msg" && c.params.public_key === h.keyPair.publicKey));
    // Progress only goes forward through the states.
    const order = ["waiting_for_phone", "downloading", "importing", "done"];
    const seen = h.states.map((s) => s.state).filter((s, i, a) => a.indexOf(s) === i);
    assert.deepEqual(seen, order);
    // The log has no message text or key material.
    const log = h.sync.getLog().join("\n");
    assert.ok(!log.includes("hello") && !log.includes(h.keyPair.publicKey) && !log.includes("backup/abc"));
    assert.match(log, /backup info: format=1 msgs=16/);
});

test("controller: starting again while running does nothing; after done it can start again", async () => {
    const h = harness();
    await h.sync.start();
    await h.sync.start();
    assert.equal(h.calls.filter((c) => c.endpoint === "pull_mobile_msg").length, 1);
    h.control("syncmsg_info", h.info());
    assert.equal((await h.settle("done")).state, "done");
    await h.sync.start();
    assert.equal(h.sync.getStatus().state, "waiting_for_phone");
});

test("controller: declined, restoring, and error events", async () => {
    for (const [act, data, text] of [
        ["user_confirm", { user_action: 0 }, /declined/],
        ["user_confirm", { user_action: 2 }, /restoring/],
        ["transfer_error", { error_code: 12, status: 0 }, /code 12/],
    ] as const) {
        const h = harness();
        await h.sync.start();
        h.control(act, data);
        const s = h.sync.getStatus();
        assert.equal(s.state, "failed");
        assert.match(s.error ?? "", text);
        // The request is cancelled on Zalo's side.
        assert.ok(h.calls.some((c) => c.endpoint === "cancel_pull_mobile_msg"));
    }
});

test("controller: the phone being idle or busy keeps waiting", async () => {
    const h = harness();
    await h.sync.start();
    h.control("transfer_error", { status: 2 });
    assert.equal(h.sync.getStatus().state, "waiting_for_phone");
    assert.match(h.sync.getStatus().detail, /Keep Zalo open/);
    h.control("transfer_error", { status: 1 });
    assert.equal(h.sync.getStatus().progress, 20);
});

test("controller: Zalo refusing the request fails it; code 211 names sync v2", async () => {
    const h = harness({
        call: async () => {
            throw Object.assign(new Error("nope"), { code: 211 });
        },
    });
    const s = await h.sync.start();
    assert.equal(s.state, "failed");
    assert.match(s.error ?? "", /sync v2/);
    const h2 = harness({
        call: async () => {
            throw Object.assign(new Error("nope"), { code: 114 });
        },
    });
    assert.match((await h2.sync.start()).error ?? "", /code 114/);
});

test("controller: not logged in is a 409", async () => {
    const h = harness({}, { ownId: null });
    await assert.rejects(h.sync.start(), (err: { status?: number }) => err.status === 409);
});

test("controller: times out when the phone never answers", async () => {
    const h = harness({ timeouts: { confirmMs: 20 } });
    await h.sync.start();
    const s = await h.settle("failed");
    assert.match(s.error ?? "", /did not confirm/);
});

test("controller: an empty backup finishes at once", async () => {
    const h = harness();
    await h.sync.start();
    h.control("syncmsg_info", h.info({ db_info: JSON.stringify({ db_format: 1, origin_db: { msg_total: 0 } }) }));
    const s = await h.settle("done");
    assert.equal(s.state, "done");
    assert.equal(s.counts?.imported, 0);
    assert.equal(h.downloads.length, 0);
});

test("controller: unsupported or incomplete backup descriptions fail clearly", async () => {
    for (const [extra, text] of [
        [{ db_info: JSON.stringify({ db_format: 0, origin_db: { msg_total: 3 } }) }, /old format/],
        [{ url: "" }, /no download link/],
        [{ url: "http://insecure.example.invalid/x" }, /no download link/],
        [{ encrypted_key: "" }, /no download link or key/],
        [{ db_info: "not json" }, /description is missing/],
    ] as const) {
        const h = harness();
        await h.sync.start();
        h.control("syncmsg_info", h.info(extra as Record<string, unknown>));
        const s = await h.settle("failed");
        assert.equal(s.state, "failed");
        assert.match(s.error ?? "", text);
    }
});

test("controller: a wrong key fails without importing anything", async () => {
    const h = harness({}, { keyText: "ffeeddccbbaa99887766554433221100" });
    await h.sync.start();
    h.control("syncmsg_info", h.info());
    const s = await h.settle("failed");
    assert.match(s.error ?? "", /Wrong key/);
    assert.equal(h.store.listThreads().length, 0);
});

test("controller: the download is retried (the file may not be ready), then succeeds", async () => {
    let attempts = 0;
    const waits: number[] = [];
    const backup = packBackup(specs, BODY, KEY_TEXT);
    const h2 = harness({
        download: async () => {
            attempts++;
            if (attempts < 3) throw new DownloadError("HTTP 404", 404);
            return backup;
        },
        sleep: async (ms) => void waits.push(ms),
    });
    await h2.sync.start();
    h2.control("syncmsg_info", h2.info());
    const s = await h2.settle("done");
    assert.equal(s.state, "done", s.error ?? "");
    assert.equal(attempts, 3);
    assert.deepEqual(waits, [1000, 2000]);
});

test("controller: a download that keeps failing gives up with the HTTP status", async () => {
    const h = harness({
        download: async () => {
            throw new DownloadError("HTTP 403", 403);
        },
    });
    await h.sync.start();
    h.control("syncmsg_info", h.info());
    const s = await h.settle("failed");
    assert.match(s.error ?? "", /HTTP 403/);
});

test("controller: cancel stops the run, tells Zalo, and ignores later events", async () => {
    const h = harness();
    await h.sync.start();
    h.sync.cancel();
    assert.equal(h.sync.getStatus().state, "idle");
    assert.ok(h.calls.some((c) => c.endpoint === "cancel_pull_mobile_msg"));
    h.control("syncmsg_info", h.info());
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(h.sync.getStatus().state, "idle");
    assert.equal(h.downloads.length, 0);
});

test("controller: a temp key from transfer_after_login goes into the next request", async () => {
    const h = harness();
    h.sync.handleControl({ actType: "transfer_after_login", act: "x", data: { temp_key: "tk-1" } });
    await h.sync.start();
    assert.equal(h.calls[0].params.temp_key, "tk-1");
});

// ---- Wiring: session, routes and the real request builders on a fake zca-js API ----

function listen(session: ZaloSession) {
    const server = createServer({ host: "127.0.0.1", port: 0, dataDir: os.tmpdir(), accessToken: null, readReceipts: true }, session);
    return new Promise<{ port: number; close: () => Promise<void> }>((resolve) =>
        server.listen(0, "127.0.0.1", () =>
            resolve({
                port: (server.address() as net.AddressInfo).port,
                close: () =>
                    new Promise<void>((r) => {
                        server.closeAllConnections?.();
                        server.close(() => r());
                    }),
            }),
        ),
    );
}

function request(port: number, method: string, url: string) {
    return new Promise<{ status: number; body: Record<string, unknown> }>((resolve, reject) => {
        const r = http.request({ host: "127.0.0.1", port, method, path: url }, (res) => {
            let body = "";
            res.setEncoding("utf8");
            res.on("data", (c) => (body += c));
            res.on("end", () => resolve({ status: res.statusCode ?? 0, body: JSON.parse(body || "{}") }));
        });
        r.on("error", reject);
        r.end();
    });
}

test("session and routes: start, status, events, and the request the file service gets", async () => {
    const store = new ChatStore();
    store.ownId = OWN;
    const session = new ZaloSession(store, os.tmpdir());
    const requests: string[] = [];
    const fake: Record<string, unknown> = {
        getContext: () => ({ imei: "imei-9" }),
        zpwServiceMap: { file: ["https://files.example.invalid/"] },
        custom(name: string, fn: (a: { utils: unknown; props: unknown }) => Promise<unknown>) {
            const utils = {
                encodeAES: (s: string) => `enc(${s})`,
                makeURL: (base: string, params: Record<string, string>) => `${base}?params=${encodeURIComponent(params.params)}`,
                request: async (url: string) => {
                    requests.push(url);
                    return {};
                },
                resolve: async (r: unknown) => r,
            };
            fake[name] = (props: unknown) => fn({ utils, props });
        },
    };
    const internals = session as unknown as { api: unknown; status: { state: string; ownId: string } };
    internals.api = fake;
    internals.status.state = "ready";
    internals.status.ownId = OWN;
    const events: string[] = [];
    session.on("history_sync", (s) => events.push(s.state));

    const { port, close } = await listen(session);
    try {
        assert.equal((await request(port, "GET", "/api/history-sync/status")).body.state, "idle");

        const started = await request(port, "POST", "/api/history-sync/start");
        assert.equal(started.status, 202);
        assert.equal(started.body.state, "waiting_for_phone");
        // The request went to the service map's file host, `/api/message/pull_mobile_msg`, with encrypted parameters.
        assert.equal(requests.length, 1);
        const url = new URL(requests[0]);
        assert.equal(url.origin + url.pathname, "https://files.example.invalid/api/message/pull_mobile_msg");
        const params = decodeURIComponent(url.searchParams.get("params")!);
        assert.match(params, /^enc\(\{"pc_name":"Zalo-Android","public_key":"[A-Za-z0-9+/=]+","from_seq_id":0,"is_retry":0,"min_seq_id":0,"temp_key":"","imei":"imei-9"\}\)$/);

        const again = await request(port, "POST", "/api/history-sync/start");
        assert.equal(again.status, 200);
        assert.equal(requests.length, 1, "a second start while running makes no new request");

        const status = await request(port, "GET", "/api/history-sync/status");
        assert.equal(status.body.state, "waiting_for_phone");
        assert.ok(Array.isArray(status.body.log));

        // A decline arrives on the control channel.
        const publicKey = /public_key":"([^"]+)"/.exec(params)![1];
        session.historySync.onControls([{ content: { act_type: "syncmsgmb", act: "user_confirm", data: { public_key: publicKey, pc_name: "Zalo-Android", user_action: 0 } } }]);
        assert.equal((await request(port, "GET", "/api/history-sync/status")).body.state, "failed");
        assert.deepEqual(events, ["waiting_for_phone", "failed"]);

        assert.equal((await request(port, "POST", "/api/history-sync/cancel")).body.state, "idle");
    } finally {
        await close();
    }
});

test("route: starting while logged out is a 409", async () => {
    const { port, close } = await listen(new ZaloSession(new ChatStore(), os.tmpdir()));
    try {
        assert.equal((await request(port, "POST", "/api/history-sync/start")).status, 409);
    } finally {
        await close();
    }
});

test("backup download: chat.zalo.me cookies on another Zalo host, no form content type, safe redirects", async () => {
    assert.ok(isZaloCookieHost("zfile-sync.zadn.vn"));
    assert.ok(isZaloCookieHost("tt-files-wpa.chat.zalo.me"));
    assert.ok(!isZaloCookieHost("evil-zalo.me"));
    assert.ok(!isZaloCookieHost("zalo.me.example.com"));

    const jar = new CookieJar();
    await jar.setCookie("zpw_sek=SEK; Domain=chat.zalo.me; Path=/; Secure", "https://chat.zalo.me/");
    await jar.setCookie("zpsid=SID; Domain=.zalo.me; Path=/; Secure", "https://chat.zalo.me/");
    const seen: { url: string; headers: Record<string, string> }[] = [];
    const replies: Response[] = [
        new Response(null, { status: 302, headers: { location: "https://b.zadn.vn/file.zdb" } }),
        new Response(null, { status: 302, headers: { location: "https://cdn.example.com/file.zdb" } }),
        new Response(Buffer.from("BACKUP"), { status: 200, headers: { "content-length": "6" } }),
    ];
    const fake: Record<string, unknown> = {
        custom(name: string, fn: (a: { ctx: unknown; utils: unknown; props: unknown }) => Promise<unknown>) {
            const ctx = {
                uid: "4459",
                userAgent: "UA/1",
                cookie: jar,
                options: {
                    polyfill: async (url: string, init: { headers: Record<string, string>; redirect: string }) => {
                        assert.equal(init.redirect, "manual");
                        seen.push({ url, headers: init.headers });
                        return replies.shift()!;
                    },
                },
            };
            fake[name] = (props: unknown) => fn({ ctx, utils: {}, props });
        },
    };
    const download = backupDownloader(fake as never);
    const got = await download("https://a.zadn.vn/pull?k=1", () => {}, new AbortController().signal);
    assert.equal(got.toString(), "BACKUP");
    assert.deepEqual(seen.map((s) => s.url), ["https://a.zadn.vn/pull?k=1", "https://b.zadn.vn/file.zdb", "https://cdn.example.com/file.zdb"]);
    const first = seen[0].headers;
    assert.equal(first.Cookie, "zpw_sek=SEK; zpsid=SID; zpw_uin=4459");
    assert.equal(first.Origin, "https://chat.zalo.me");
    assert.equal(first.Referer, "https://chat.zalo.me/");
    assert.equal(first["User-Agent"], "UA/1");
    assert.equal(first.Accept, "*/*");
    assert.ok(!Object.keys(first).some((k) => k.toLowerCase() === "content-type"));
    assert.equal(seen[1].headers.Cookie, first.Cookie);
    assert.equal(seen[2].headers.Cookie, undefined, "no Zalo cookies to other hosts");

    // An error answer is kept, shortened, for the log.
    replies.push(new Response('{"error_code":-1,"error_message":"bad request"}', { status: 400 }));
    await assert.rejects(download("https://a.zadn.vn/pull", () => {}, new AbortController().signal), (err: unknown) => {
        assert.ok(err instanceof DownloadError);
        assert.equal(err.status, 400);
        assert.equal(err.answer, '{"error_code":-1,"error_message":"bad request"}');
        return true;
    });
});
