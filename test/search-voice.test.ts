import assert from "node:assert/strict";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import { test } from "node:test";
import { ThreadType } from "zca-js";
import { createServer } from "../src/server.js";
import { ZaloSession } from "../src/session.js";
import { ChatStore, normalizeMessage, type RawMessage, type StoredMessage } from "../src/store.js";

function makeMsg(
    id: string,
    ts: number,
    overrides: Partial<RawMessage["data"]> = {},
    top: Partial<Omit<RawMessage, "data">> = {},
): StoredMessage {
    return normalizeMessage({
        type: 0,
        threadId: "100",
        isSelf: false,
        ...top,
        data: {
            msgId: id,
            cliMsgId: `c${id}`,
            msgType: "webchat",
            uidFrom: "100",
            idTo: "me",
            dName: "Alice",
            ts: String(ts),
            content: "hello",
            ...overrides,
        },
    });
}

async function startTestServer(responses: Record<string, unknown | ((...args: unknown[]) => unknown)> = {}) {
    const store = new ChatStore();
    store.ownId = "me";
    const session = new ZaloSession(store, os.tmpdir());
    const calls: { method: string; args: unknown[] }[] = [];
    const api = new Proxy(
        {},
        {
            get: (_t, method: string) =>
                async (...args: unknown[]) => {
                    calls.push({ method, args });
                    const answer = responses[method];
                    if (answer instanceof Error) throw answer;
                    return typeof answer === "function" ? answer(...args) : (answer ?? {});
                },
        },
    );
    const internals = session as unknown as { api: unknown; status: { state: string; ownId: string } };
    internals.api = api;
    internals.status.state = "ready";
    internals.status.ownId = "me";

    const server = createServer(
        {
            host: "127.0.0.1",
            port: 0,
            dataDir: os.tmpdir(),
            accessToken: null,
            readReceipts: true,
        },
        session,
    );
    await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
    const port = (server.address() as net.AddressInfo).port;
    const close = () =>
        new Promise<void>((resolve) => {
            server.closeAllConnections?.();
            server.close(() => resolve());
        });
    return { port, session, store, calls, close };
}

function req(
    port: number,
    method: string,
    urlPath: string,
    opts: { headers?: Record<string, string>; body?: string | Buffer } = {},
): Promise<{ status: number; body: string }> {
    return new Promise((resolve, reject) => {
        const r = http.request({ host: "127.0.0.1", port, method, path: urlPath, headers: opts.headers }, (res) => {
            let body = "";
            res.setEncoding("utf8");
            res.on("data", (c) => (body += c));
            res.on("end", () => resolve({ status: res.statusCode ?? 0, body }));
        });
        r.on("error", reject);
        if (opts.body !== undefined) r.write(opts.body);
        r.end();
    });
}

test("ChatStore.searchMessages matches diacritics, đ/Đ, case, attachments, skips recalled, sorts newest-first", () => {
    const store = new ChatStore();
    store.ownId = "me";
    store.upsertThread({ id: "100", kind: "user", name: "Minh", avatar: "https://a/100.jpg" });
    store.upsertThread({ id: "g1", kind: "group", name: "Team Dev", avatar: "https://a/g1.jpg" });

    store.addMessage(makeMsg("1", 1000, { content: "Đi uống Cà Phê Sữa Đá nhé!" }));
    store.addMessage(makeMsg("2", 2000, { content: "ĐẶT lịch họp tiếng Việt" }, { type: 1, threadId: "g1" }));
    store.addMessage(makeMsg("3", 3000, { content: "Cà phê sáng nay bị thu hồi" }));
    store.recall("3");
    store.addMessage(
        makeMsg(
            "4",
            4000,
            {
                msgType: "chat.recommended",
                content: {
                    action: "recommened.link",
                    title: "Hướng dẫn Café Hà Nội",
                    description: "Quán đường Điện Biên Phủ rất ngon",
                    href: "https://example.com/cafe",
                    params: "{}",
                },
            },
            { type: 1, threadId: "g1" },
        ),
    );
    store.addMessage(
        makeMsg("5", 5000, {
            msgType: "share.file",
            content: { title: "Báo_cáo_Quý_4.pdf", href: "https://example.com/q4.pdf", params: "{}" },
        }),
    );

    // Blank query returns empty
    assert.deepEqual(store.searchMessages("   "), []);

    // Diacritic-insensitive + đ/Đ folding + skips recalled message #3
    const cafeHits = store.searchMessages("ca phe sua da");
    assert.equal(cafeHits.length, 1);
    assert.equal(cafeHits[0].message.id, "1");
    assert.equal(cafeHits[0].threadId, "100");
    assert.equal(cafeHits[0].threadKind, "user");
    assert.equal(cafeHits[0].threadName, "Minh");
    assert.equal(cafeHits[0].threadAvatar, "https://a/100.jpg");

    // Recalled message is never matched (neither original text nor "Message recalled")
    assert.deepEqual(store.searchMessages("thu hoi"), []);
    assert.deepEqual(store.searchMessages("recalled"), []);

    // Uppercase Đ folded to d, case-insensitive, matches group message
    const vietHits = store.searchMessages("dat lich hop TIENG VIET");
    assert.equal(vietHits.length, 1);
    assert.equal(vietHits[0].message.id, "2");
    assert.equal(vietHits[0].threadKind, "group");
    assert.equal(vietHits[0].threadName, "Team Dev");

    // Attachment title (Latin diacritics) & description (Vietnamese đ/Đ)
    const linkTitleHits = store.searchMessages("cafe ha noi");
    assert.equal(linkTitleHits.length, 1);
    assert.equal(linkTitleHits[0].message.id, "4");

    const linkDescHits = store.searchMessages("dien bien phu");
    assert.equal(linkDescHits.length, 1);
    assert.equal(linkDescHits[0].message.id, "4");

    const fileTitleHits = store.searchMessages("bao_cao_quy_4");
    assert.equal(fileTitleHits.length, 1);
    assert.equal(fileTitleHits[0].message.id, "5");

    // Broad match across threads is sorted newest-first
    store.addMessage(makeMsg("6", 1500, { content: "duong pho" }));
    store.addMessage(makeMsg("7", 6000, { content: "Đường lớn" }, { type: 1, threadId: "g1" }));
    const ordered = store.searchMessages("duong");
    assert.deepEqual(
        ordered.map((h) => h.message.id),
        ["7", "4", "6"],
    );
});

test("ChatStore.searchMessages respects limit and clamps to [1, 100]", () => {
    const store = new ChatStore();
    for (let i = 1; i <= 120; i++) {
        store.addMessage(makeMsg(String(i), i * 1000, { content: `item ${i}` }));
    }
    assert.equal(store.searchMessages("item").length, 50);
    assert.equal(store.searchMessages("item", 5).length, 5);
    assert.deepEqual(
        store.searchMessages("item", 3).map((h) => h.message.id),
        ["120", "119", "118"],
    );
    assert.equal(store.searchMessages("item", 0).length, 1);
    assert.equal(store.searchMessages("item", -10).length, 1);
    assert.equal(store.searchMessages("item", 500).length, 100);
});

test("GET /api/search validates q and limit and returns search results", async () => {
    const { port, store, close } = await startTestServer();
    try {
        store.upsertThread({ id: "100", kind: "user", name: "Lan", avatar: "https://a/lan.jpg" });
        store.addMessage(makeMsg("1", 1000, { content: "Chào buổi sáng" }));
        store.addMessage(makeMsg("2", 2000, { content: "Chào buổi tối" }));

        assert.equal((await req(port, "GET", "/api/search")).status, 400);
        assert.equal((await req(port, "GET", "/api/search?q=%20%20")).status, 400);
        assert.equal((await req(port, "GET", "/api/search?q=chao&limit=0")).status, 400);
        assert.equal((await req(port, "GET", "/api/search?q=chao&limit=abc")).status, 400);

        const res = await req(port, "GET", "/api/search?q=chao%20buoi&limit=1");
        assert.equal(res.status, 200);
        const body = JSON.parse(res.body) as { results: { threadId: string; threadName: string; message: { id: string } }[] };
        assert.equal(body.results.length, 1);
        assert.equal(body.results[0].threadId, "100");
        assert.equal(body.results[0].threadName, "Lan");
        assert.equal(body.results[0].message.id, "2");
    } finally {
        await close();
    }
});

test("ZaloSession.sendVoice uploads audio, normalizes extension, validates fileUrl, and calls sendVoice", async () => {
    const { port, session, calls, close } = await startTestServer({
        uploadAttachment: [{ fileType: "others", fileUrl: "https://cdn.zaloapp.com/voice/abc.m4a", fileId: "f1" }],
        sendVoice: { msgId: "900" },
    });
    try {
        const audio = Buffer.from([0x00, 0x01, 0x02, 0x0d, 0x0a, 0xff]);

        // Filename without audio extension gets .m4a appended
        await session.sendVoice("user", "100", { buffer: audio, originalname: "recording" });
        assert.equal(calls[0].method, "uploadAttachment");
        assert.deepEqual(calls[0].args, [
            [{ data: audio, filename: "recording.m4a", metadata: { totalSize: audio.length } }],
            "100",
            ThreadType.User,
        ]);
        assert.equal(calls[1].method, "sendVoice");
        assert.deepEqual(calls[1].args, [{ voiceUrl: "https://cdn.zaloapp.com/voice/abc.m4a" }, "100", ThreadType.User]);

        // Existing audio extension is preserved for group threads
        await session.sendVoice("group", "g1", { buffer: audio, originalname: "clip.aac" });
        assert.deepEqual(calls[2].args, [
            [{ data: audio, filename: "clip.aac", metadata: { totalSize: audio.length } }],
            "g1",
            ThreadType.Group,
        ]);
        assert.deepEqual(calls[3].args, [{ voiceUrl: "https://cdn.zaloapp.com/voice/abc.m4a" }, "g1", ThreadType.Group]);

        // Empty buffer rejected before calling API
        await assert.rejects(session.sendVoice("user", "100", { buffer: Buffer.alloc(0), originalname: "v.m4a" }), /empty/);

        // HTTP: missing body -> 400
        assert.equal((await req(port, "POST", "/api/threads/user/100/voice")).status, 400);

        // HTTP: raw binary body upload -> 201
        const rawRes = await req(port, "POST", "/api/threads/user/100/voice?name=memo.m4a", {
            headers: { "Content-Type": "audio/mp4" },
            body: audio,
        });
        assert.equal(rawRes.status, 201);
        const rawUploadArgs = calls[4].args[0] as [{ data: Buffer; filename: string }];
        assert.equal(rawUploadArgs[0].filename, "memo.m4a");
        assert.deepEqual(rawUploadArgs[0].data, audio);

        // HTTP: multipart/form-data upload with field "voice" -> 201 with exact binary bytes
        const boundary = "----ZaloVoiceBoundary1234";
        const multipartBody = Buffer.concat([
            Buffer.from(`--${boundary}\r\nContent-Disposition: form-data; name="voice"; filename="note.m4a"\r\nContent-Type: audio/mp4\r\n\r\n`),
            audio,
            Buffer.from(`\r\n--${boundary}--\r\n`),
        ]);
        const mpRes = await req(port, "POST", "/api/threads/group/g1/voice", {
            headers: { "Content-Type": `multipart/form-data; boundary=${boundary}` },
            body: multipartBody,
        });
        assert.equal(mpRes.status, 201);
        const mpUploadArgs = calls[6].args[0] as [{ data: Buffer; filename: string }];
        assert.equal(mpUploadArgs[0].filename, "note.m4a");
        assert.deepEqual(mpUploadArgs[0].data, audio);

        // HTTP: multipart/form-data missing "voice" field -> 400
        const wrongField = Buffer.from(
            `--${boundary}\r\nContent-Disposition: form-data; name="other"; filename="a.m4a"\r\n\r\nabc\r\n--${boundary}--\r\n`,
        );
        const wrongRes = await req(port, "POST", "/api/threads/user/100/voice", {
            headers: { "Content-Type": `multipart/form-data; boundary=${boundary}` },
            body: wrongField,
        });
        assert.equal(wrongRes.status, 400);
    } finally {
        await close();
    }
});

test("ZaloSession.sendVoice fails with 502 if uploadAttachment returns no fileUrl", async () => {
    const { session, calls, close } = await startTestServer({
        uploadAttachment: [{}],
    });
    try {
        await assert.rejects(
            session.sendVoice("user", "100", { buffer: Buffer.from([1, 2, 3]), originalname: "v.m4a" }),
            /Failed to upload voice recording/,
        );
        assert.equal(calls.length, 1);
        assert.equal(calls[0].method, "uploadAttachment");
    } finally {
        await close();
    }
});
