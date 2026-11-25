import assert from "node:assert/strict";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import { test } from "node:test";
import WebSocket from "ws";
import { createServer, decodeAudioFromApp, encodeAudioToApp } from "../src/server.js";
import { ZaloSession } from "../src/session.js";
import { ChatStore, normalizeMessage } from "../src/store.js";

test("audio messages between server and app (docs/call-audio.md §3)", () => {
    assert.equal(encodeAudioToApp(0xfffe, 0x12345678, Buffer.from([0xaa, 0xbb])).toString("hex"), "01fffe12345678aabb");
    assert.equal(encodeAudioToApp(0x1fffe, 2 ** 32 + 5, Buffer.alloc(0)).toString("hex"), "01fffe00000005", "wraps like RTP");
    assert.equal(decodeAudioFromApp(Buffer.from([2, 0xf8, 0xff, 0xfe]))?.toString("hex"), "f8fffe");
    assert.equal(decodeAudioFromApp(Buffer.from([2])), null, "empty frame");
    assert.equal(decodeAudioFromApp(Buffer.from([1, 2, 3])), null, "wrong direction");
    assert.equal(decodeAudioFromApp(Buffer.alloc(2000, 2)), null, "larger than any Opus packet");
});

async function startTestServer(opts: { host?: string; accessToken?: string | null; loggedIn?: boolean } = {}) {
    const store = new ChatStore();
    store.ownId = "me";
    const session = new ZaloSession(store, os.tmpdir());
    const sent: { method: string; args: unknown[] }[] = [];
    if (opts.loggedIn) {
        const api = new Proxy(
            {},
            {
                get: (_t, method: string) =>
                    async (...args: unknown[]) => {
                        sent.push({ method, args });
                        if (method === "sendMessage") return { message: { msgId: 10 }, attachment: [{ msgId: 20 }] };
                        if (method === "forwardMessage") return { success: [], fail: [] };
                        if (method === "lastOnline") throw new Error("secret upstream stack detail");
                        if (method === "getGroupInfo") throw new Error("secret internal database detail");
                        return {};
                    },
            },
        );
        const internals = session as unknown as { api: unknown; status: { state: string; ownId: string } };
        internals.api = api;
        internals.status.state = "ready";
        internals.status.ownId = "me";
    }
    const server = createServer(
        {
            host: opts.host ?? "127.0.0.1",
            port: 0,
            dataDir: os.tmpdir(),
            accessToken: opts.accessToken ?? null,
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
    return { server, port, session, store, sent, close };
}

function rawHttp(port: number, reqText: string): Promise<string> {
    return new Promise((resolve, reject) => {
        const sock = net.createConnection(port, "127.0.0.1", () => sock.write(reqText));
        let data = "";
        sock.setEncoding("utf8");
        sock.on("data", (c) => (data += c));
        sock.on("end", () => resolve(data));
        sock.on("error", reject);
        setTimeout(() => {
            sock.destroy();
            resolve(data);
        }, 500).unref();
    });
}

function req(
    port: number,
    method: string,
    urlPath: string,
    opts: { headers?: Record<string, string>; body?: string | Buffer } = {},
): Promise<{ status: number; headers: http.IncomingHttpHeaders; body: string }> {
    return new Promise((resolve, reject) => {
        const r = http.request({ host: "127.0.0.1", port, method, path: urlPath, headers: opts.headers }, (res) => {
            let body = "";
            res.setEncoding("utf8");
            res.on("data", (c) => (body += c));
            res.on("end", () => resolve({ status: res.statusCode ?? 0, headers: res.headers, body }));
        });
        r.on("error", reject);
        if (opts.body !== undefined) r.write(opts.body);
        r.end();
    });
}

test("server enforces Host, Origin, Sec-Fetch-Site, and baseline security headers (H1, M4)", async () => {
    const { port, close } = await startTestServer();
    try {
        // Security headers on static page and API
        const home = await req(port, "GET", "/");
        assert.equal(home.status, 200);
        assert.equal(home.headers["x-content-type-options"], "nosniff");
        assert.equal(home.headers["x-frame-options"], "DENY");
        assert.equal(home.headers["referrer-policy"], "no-referrer");
        assert.ok(home.headers["content-security-policy"]);

        // Native client without Origin is allowed
        const ok = await req(port, "GET", "/api/status");
        assert.equal(ok.status, 200);
        assert.equal(ok.headers["content-security-policy"], undefined);

        // Matching Origin is allowed
        const sameOrigin = await req(port, "GET", "/api/status", {
            headers: { Origin: `http://127.0.0.1:${port}`, "Sec-Fetch-Site": "same-origin" },
        });
        assert.equal(sameOrigin.status, 200);

        // Cross-origin Origin is rejected with 403
        const badOrigin = await req(port, "GET", "/api/status", {
            headers: { Origin: "https://evil.example" },
        });
        assert.equal(badOrigin.status, 403);

        // Sec-Fetch-Site: cross-site is rejected with 403
        const crossSite = await req(port, "GET", "/api/status", {
            headers: { "Sec-Fetch-Site": "cross-site" },
        });
        assert.equal(crossSite.status, 403);

        // Sec-Fetch-Site: same-site on mutation without matching Origin is rejected with 403
        const sameSiteMut = await req(port, "PUT", "/api/presence", {
            headers: { "Sec-Fetch-Site": "same-site", "Content-Type": "application/json" },
            body: "{}",
        });
        assert.equal(sameSiteMut.status, 403);

        // DNS rebinding Host header is rejected with 403 on loopback server
        const rebind = await req(port, "GET", "/api/status", {
            headers: { Host: `evil.example:${port}` },
        });
        assert.equal(rebind.status, 403);
    } finally {
        await close();
    }
});

test("server allows .json attachment uploads and validates inputs & errors (H3, M4, M6)", async () => {
    const { port, store, sent, close } = await startTestServer({ loggedIn: true });
    try {
        // .json attachment with Content-Type: application/json arrives as raw Buffer
        const jsonPayload = JSON.stringify({ hello: "world" });
        const upload = await req(port, "POST", "/api/threads/user/100/attachments?name=data.json", {
            headers: { "Content-Type": "application/json" },
            body: jsonPayload,
        });
        assert.equal(upload.status, 201);
        assert.deepEqual(JSON.parse(upload.body), { msgIds: ["20"] });

        // Malformed JSON returns 400 (not 500)
        const badJson = await req(port, "POST", "/api/threads/user/100/messages", {
            headers: { "Content-Type": "application/json" },
            body: "{not json",
        });
        assert.equal(badJson.status, 400);

        // Oversized JSON body returns 413 (not 500)
        const bigJson = await req(port, "POST", "/api/threads/user/100/messages", {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ text: "a".repeat(80 * 1024) }),
        });
        assert.equal(bigJson.status, 413);

        // Internal 500 does not leak raw err.message
        const err500 = await req(port, "GET", "/api/threads/group/g1/info");
        assert.equal(err500.status, 500);
        assert.deepEqual(JSON.parse(err500.body), { error: "Internal server error" });

        // GET /messages rejects NaN before and invalid limit
        assert.equal((await req(port, "GET", "/api/threads/user/100/messages?before=abc")).status, 400);
        assert.equal((await req(port, "GET", "/api/threads/user/100/messages?limit=0")).status, 400);
        assert.equal((await req(port, "GET", "/api/threads/user/100/messages?limit=abc")).status, 400);

        // POST /messages treats quoteId: null and "" as absent
        const quoteNull = await req(port, "POST", "/api/threads/user/100/messages", {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ text: "hi", quoteId: null }),
        });
        assert.equal(quoteNull.status, 201);
        const quoteEmpty = await req(port, "POST", "/api/threads/user/100/messages", {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ text: "hi", quoteId: "" }),
        });
        assert.equal(quoteEmpty.status, 201);

        // PUT /presence treats { kind: null, id: null } as clearing presence
        const clearPresence = await req(port, "PUT", "/api/presence", {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ kind: null, id: null }),
        });
        assert.equal(clearPresence.status, 204);

        // Forward deduplicates targets and caps at 20
        store.addMessage(
            normalizeMessage({
                type: 0,
                threadId: "100",
                isSelf: false,
                data: { msgId: "m1", cliMsgId: "cm1", msgType: "webchat", uidFrom: "100", dName: "A", ts: "1000", content: "hi" },
            }),
        );
        sent.length = 0;
        const dupFwd = await req(port, "POST", "/api/threads/user/100/messages/m1/forward", {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                to: [
                    { kind: "user", id: "u1" },
                    { kind: "user", id: "u1" },
                    { kind: "user", id: "u2" },
                ],
            }),
        });
        assert.equal(dupFwd.status, 204);
        assert.deepEqual(sent[0]?.args[1], ["u1", "u2"]);

        const tooMany = await req(port, "POST", "/api/threads/user/100/messages/m1/forward", {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                to: Array.from({ length: 21 }, (_, i) => ({ kind: "user", id: `u${i}` })),
            }),
        });
        assert.equal(tooMany.status, 400);
    } finally {
        await close();
    }
});

test("GET /threads/user/:id/info propagates 409 Not logged in instead of returning null", async () => {
    const { port, close } = await startTestServer({ loggedIn: false });
    try {
        const res = await req(port, "GET", "/api/threads/user/100/info");
        assert.equal(res.status, 409);
    } finally {
        await close();
    }
});

test("WebSocket upgrade enforces Origin/Host, returns 404 for unknown paths, replaces duplicate audio clients, and preserves audioWss on unrelated call end (H1, H2, H5)", async () => {
    const { port, session, close } = await startTestServer();
    try {
        // Cross-origin WS upgrade rejected with 403
        const wsCrossOrigin = await rawHttp(
            port,
            `GET /ws HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nOrigin: https://evil.example\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n`,
        );
        assert.match(wsCrossOrigin, /^HTTP\/1\.1 403 Forbidden/);

        // Unknown WS path returns 404 (not 401)
        const wsUnknown = await rawHttp(
            port,
            `GET /ws/does-not-exist HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n`,
        );
        assert.match(wsUnknown, /^HTTP\/1\.1 404 Not Found/);

        // Simulate an active call to test /ws/call-audio replacement and unrelated call end
        const internals = session as unknown as { activeCall: unknown };
        internals.activeCall = {
            info: { callId: "active-1", callerId: "100", callerName: "A", video: false, state: "connected", connectedAt: Date.now() },
            media: null,
            finish: () => {},
            camOn: false,
            peer: { cameraOn: false, muted: false },
            peerHeard: new Set(),
        };

        const ws1 = new WebSocket(`ws://127.0.0.1:${port}/ws/call-audio`);
        await new Promise<void>((r) => ws1.once("open", () => r()));

        // Ending an unrelated ringing call does NOT close ws1 because session.currentCall() is still active
        session.emit("call", { type: "ended", callId: "other-ringing", act: "declined" });
        await new Promise((r) => setTimeout(r, 30));
        assert.equal(ws1.readyState, WebSocket.OPEN);

        // Connecting a second audio client replaces the first with code 1000 "replaced"
        const ws1Closed = new Promise<{ code: number; reason: string }>((resolve) =>
            ws1.once("close", (code, reason) => resolve({ code, reason: reason.toString() })),
        );
        const ws2 = new WebSocket(`ws://127.0.0.1:${port}/ws/call-audio`);
        await new Promise<void>((r) => ws2.once("open", () => r()));
        const closedInfo = await ws1Closed;
        assert.equal(closedInfo.code, 1000);
        assert.equal(closedInfo.reason, "replaced");
        assert.equal(ws2.readyState, WebSocket.OPEN);

        // Ending the active call closes ws2
        const ws2Closed = new Promise<{ code: number; reason: string }>((resolve) =>
            ws2.once("close", (code, reason) => resolve({ code, reason: reason.toString() })),
        );
        internals.activeCall = null;
        session.emit("call", { type: "ended", callId: "active-1", act: "hangup" });
        const closed2 = await ws2Closed;
        assert.equal(closed2.code, 1000);
        assert.equal(closed2.reason, "call ended");
    } finally {
        await close();
    }

    // Closing the server removes all session listeners attached by attachWebSocket
    assert.equal(session.listenerCount("status"), 0);
    assert.equal(session.listenerCount("message"), 0);
    assert.equal(session.listenerCount("call"), 0);
    assert.equal(session.listenerCount("call_audio"), 0);
});
