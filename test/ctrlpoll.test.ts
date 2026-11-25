import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { callControlsIn } from "../src/calls.js";
import { ControlPoller, controlsIn, decodeAesCbc, pollCursor, readPollAnswer, serverList } from "../src/ctrlpoll.js";
import { ZaloSession, type CallEvent } from "../src/session.js";
import { ChatStore } from "../src/store.js";

const KEY = crypto.randomBytes(32).toString("base64");

/** Encrypts like Zalo's servers do for zca-js's decodeAES: AES-CBC, zero IV, base64, URL-encoded. */
function sealCbc(text: string, key = KEY): string {
    const cipher = crypto.createCipheriv("aes-256-cbc", Buffer.from(key, "base64"), Buffer.alloc(16));
    return encodeURIComponent(Buffer.concat([cipher.update(text, "utf8"), cipher.final()]).toString("base64"));
}

const voipRequest = {
    content: { act_type: "voip", act: "request", data: '{"callerId":1234567890123456789,"callId":42,"callType":0}' },
};

test("the poll cursor has Zalo PC's format", () => {
    assert.equal(pollCursor("0", "1"), "0,1," + encodeURIComponent(Buffer.from("{}").toString("base64") + ",1"));
    assert.equal(pollCursor("0", "1"), "0,1,e30%3D%2C1");
});

test("control servers are read from a list or a single URL", () => {
    assert.deepEqual(serverList(["https://a.example/x", 3, "nope"]), ["https://a.example/x"]);
    assert.deepEqual(serverList("https://b.example"), ["https://b.example"]);
    assert.deepEqual(serverList(undefined), []);
});

test("poll answers: empty, encrypted data, plain JSON data, refusals", () => {
    assert.deepEqual(readPollAnswer("", KEY), { kind: "empty" });
    assert.deepEqual(readPollAnswer('{"error_code":0,"data":""}', KEY), { kind: "empty" });

    // Written as text: a JS number literal would already round the id.
    const inner = `{"data":{"lastActionId":98765432109876543210,"controls":${JSON.stringify([voipRequest])}}}`;
    const answer = readPollAnswer(JSON.stringify({ error_code: 0, data: sealCbc(inner) }), KEY);
    assert.equal(answer.kind, "data");
    if (answer.kind !== "data") return;
    assert.equal(answer.data.lastActionId, "98765432109876543210", "long ids keep every digit");
    assert.equal(controlsIn(answer.data).length, 1);

    const plain = readPollAnswer(JSON.stringify({ error_code: 0, data: inner }), KEY);
    assert.equal(plain.kind, "data");

    assert.deepEqual(readPollAnswer('{"error_code":-69,"error_message":"x"}', KEY), { kind: "error", code: -69, message: "x" });
    assert.equal(readPollAnswer(JSON.stringify({ error_code: 0, data: sealCbc("{}", crypto.randomBytes(32).toString("base64")) }), KEY).kind, "error");
    assert.equal(decodeAesCbc(KEY, "not base64 at all"), null);
});

test("controls are found wherever the answer keeps them, in either shape", () => {
    const found = controlsIn({ a: { list: [voipRequest] }, b: [{ act_type: "voip", act: "cancel", data: { callId: 42 } }] });
    assert.deepEqual(found.map((c) => c.content.act), ["request", "cancel"]);
    const calls = callControlsIn(found);
    assert.equal(calls[0].data.callerId, "1234567890123456789");
    assert.equal(calls[1].data.callId, 42);
});

function poller(answers: { status: number; body: string }[], events: Record<string, unknown>[], urls: string[] = []) {
    const got: unknown[] = [];
    const p = new ControlPoller({
        servers: ["https://ctrl1.example/", "https://ctrl2.example"],
        secretKey: KEY,
        apiVersion: 685,
        apiType: 30,
        holdSeconds: 30,
        get: async (url) => {
            urls.push(url);
            const next = answers.shift();
            if (!next) throw new Error("no more answers");
            return next;
        },
        onControls: (c) => got.push(...c),
        onEvent: (e) => events.push(e),
        sleep: async () => {},
    });
    return { p, got };
}

test("a poll follows the action ids, hands over controls and asks Zalo PC's URL", async () => {
    const events: Record<string, unknown>[] = [];
    const urls: string[] = [];
    const body = (inner: unknown) => ({ status: 200, body: JSON.stringify({ error_code: 0, data: sealCbc(JSON.stringify(inner)) }) });
    const { p, got } = poller([body({ data: { lastActionId: 500, controls: [voipRequest] } }), body({ data: { resetLastActionId: 7 } })], events, urls);
    assert.equal(await p.pollOnce(), true);
    assert.equal(got.length, 1);
    assert.match(urls[0], /^https:\/\/ctrl1\.example\/\?zpw_ver=685&zpw_type=30&params=0,1,e30%3D%2C1&ts=\d+$/);
    assert.ok(p.cursor.startsWith("500,1,"));
    assert.equal(await p.pollOnce(), true);
    assert.ok(p.cursor.startsWith("7,1,"));
    assert.deepEqual(events.map((e) => e.kind), ["ctrl_poll", "ctrl_poll"]);
    assert.deepEqual(events[0].controls, ["voip/request"]);
});

test("network failures move to the next server; a refusal stops polling for good", async () => {
    const events: Record<string, unknown>[] = [];
    const urls: string[] = [];
    const { p } = poller([{ status: 502, body: "" }, { status: 200, body: '{"error_code":-69}' }], events, urls);
    // pollOnce on its own (not started) treats a failure as the end.
    p.start();
    await new Promise((r) => setTimeout(r, 20));
    assert.ok(urls[1].startsWith("https://ctrl2.example/?"), "rotates servers after a failure");
    assert.deepEqual(events.map((e) => e.kind), ["ctrl_poll_start", "ctrl_poll_error", "ctrl_poll_refused"]);
    assert.equal(urls.length, 2, "no poll after the refusal");
});

test("no control servers: says so in the call log and does nothing", () => {
    const events: Record<string, unknown>[] = [];
    const p = new ControlPoller({
        servers: [],
        secretKey: KEY,
        apiVersion: 1,
        apiType: 1,
        holdSeconds: 30,
        get: async () => assert.fail("must not poll"),
        onControls: () => {},
        onEvent: (e) => events.push(e),
    });
    p.start();
    assert.equal(events[0].kind, "ctrl_poll_unavailable");
});

test("a call seen both ways rings once; an old one doesn't ring at all", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-ctrl-"));
    const session = new ZaloSession(new ChatStore(), dir);
    const events: CallEvent[] = [];
    session.on("call", (e) => events.push(e));
    const now = 1_800_000_000_000;
    const request = { act: "request", data: { callerId: "5", callId: "1", ts: now - 1000 }, raw: {} };
    session.onCallControl(request, "socket", now);
    session.onCallControl(request, "poll", now);
    assert.equal(events.length, 1);
    session.onCallControl({ act: "request", data: { callerId: "5", callId: "2", ts: (now - 10 * 60_000) / 1000 }, raw: {} }, "poll", now);
    assert.equal(events.length, 1, "a ten-minute-old request (in seconds) is history");
    assert.match(session.callLog(), /call_too_old/);
});

test("an answer that only repeats controls already handed over hands over nothing and waits", async () => {
    const control = { actionId: "1", controlId: "1", content: { act_type: "voip", act: "request", data: "{}" } };
    const body = { status: 200, body: JSON.stringify({ error_code: 0, data: sealCbc(JSON.stringify({ data: { lastActionId: 5, controls: [control] } })) }) };
    const slept: number[] = [];
    const got: unknown[] = [];
    const events: Record<string, unknown>[] = [];
    const answers = [body, body];
    const p = new ControlPoller({
        servers: ["https://ctrl.example"],
        secretKey: KEY,
        apiVersion: 1,
        apiType: 1,
        holdSeconds: 30,
        get: async () => answers.shift()!,
        onControls: (c) => got.push(...c),
        onEvent: (e) => events.push(e),
        sleep: async (ms) => void slept.push(ms),
    });
    await p.pollOnce();
    await p.pollOnce();
    assert.equal(got.length, 1, "handed over once");
    assert.equal(events[1].repeated, 1);
    assert.ok(slept.includes(30_000), "waits before polling again");
});

test("serverList requires HTTPS and rejects plain HTTP", () => {
    assert.deepEqual(serverList(["http://insecure.example", "https://secure.example"]), ["https://secure.example"]);
    assert.deepEqual(serverList("http://insecure.example"), []);
});

test("readPollAnswer rejects null, array, and primitive decrypted data", () => {
    for (const payload of ["null", "[1,2,3]", "42", '"hello"', "true", '{"data":null}', '{"data":[1,2]}']) {
        const res = readPollAnswer(JSON.stringify({ error_code: 0, data: sealCbc(payload) }), KEY);
        assert.equal(res.kind, "error", `expected error for ${payload}`);
    }
    assert.equal(readPollAnswer("null", KEY).kind, "error");
    assert.equal(readPollAnswer("[1,2]", KEY).kind, "error");
});

test("controlsIn preserves controlId and actionId on top-level controls for deduplication", async () => {
    const topLevel = { act_type: "voip", act: "request", controlId: "cid-9", actionId: "aid-9", data: "{}" };
    const found = controlsIn({ list: [topLevel] });
    assert.equal(found.length, 1);
    assert.equal(found[0].controlId, "cid-9");
    assert.equal(found[0].actionId, "aid-9");

    const body = { status: 200, body: JSON.stringify({ error_code: 0, data: sealCbc(JSON.stringify({ data: { list: [topLevel] } })) }) };
    const got: unknown[] = [];
    const events: Record<string, unknown>[] = [];
    const answers = [body, body];
    const p = new ControlPoller({
        servers: ["https://ctrl.example"],
        secretKey: KEY,
        apiVersion: 1,
        apiType: 1,
        holdSeconds: 30,
        get: async () => answers.shift()!,
        onControls: (c) => got.push(...c),
        onEvent: (e) => events.push(e),
        sleep: async () => {},
    });
    await p.pollOnce();
    await p.pollOnce();
    assert.equal(got.length, 1, "top-level control deduplicated across polls");
    assert.equal(events[1].repeated, 1);
});

test("stop() cancels active sleep timers and prevents stale in-flight polls from firing after stop/start", async () => {
    let resolveFirst!: (v: { status: number; body: string }) => void;
    let calls = 0;
    const got: unknown[] = [];
    const events: Record<string, unknown>[] = [];
    const body = (callId: number) =>
        JSON.stringify({
            error_code: 0,
            data: sealCbc(JSON.stringify({ data: { controls: [{ content: { act_type: "voip", act: "request", data: JSON.stringify({ callId }) } }] } })),
        });

    const p = new ControlPoller({
        servers: ["https://ctrl.example"],
        secretKey: KEY,
        apiVersion: 1,
        apiType: 1,
        holdSeconds: 30,
        get: async () => {
            calls++;
            if (calls === 1) {
                return await new Promise<{ status: number; body: string }>((r) => {
                    resolveFirst = r;
                });
            }
            if (calls === 2) return { status: 200, body: body(2) };
            // Trigger refusal to stop the second run cleanly.
            return { status: 200, body: '{"error_code":-69}' };
        },
        onControls: (c) => got.push(...c),
        onEvent: (e) => events.push(e),
    });

    p.start();
    // Wait a tick for the first poll to be in-flight.
    await new Promise((r) => setTimeout(r, 10));
    assert.equal(calls, 1);
    p.stop();
    p.start();
    // Resolve the stale first poll after restart: it must be ignored completely.
    resolveFirst({ status: 200, body: body(1) });
    await new Promise((r) => setTimeout(r, 30));
    // Stop while the second run is sleeping in MIN_GAP_MS so no timer lingers.
    p.stop();
    assert.equal(got.length, 1, "only the restarted run delivered controls");
    assert.match(String((got[0] as { content: { data: string } }).content.data), /"callId":2/);
});

test("errors thrown by onControls or onEvent never cause unhandled rejections", async () => {
    const unhandled: unknown[] = [];
    const onUnhandled = (err: unknown) => unhandled.push(err);
    process.on("unhandledRejection", onUnhandled);
    try {
        const answers = [
            { status: 200, body: JSON.stringify({ error_code: 0, data: sealCbc(JSON.stringify({ data: { controls: [voipRequest] } })) }) },
            { status: 200, body: '{"error_code":-69}' },
        ];
        const p = new ControlPoller({
            servers: ["https://ctrl.example"],
            secretKey: KEY,
            apiVersion: 1,
            apiType: 1,
            holdSeconds: 30,
            get: async () => answers.shift() ?? { status: 200, body: '{"error_code":-69}' },
            onControls: () => {
                throw new Error("boom controls");
            },
            onEvent: () => {
                throw new Error("boom event");
            },
            sleep: async () => {},
        });
        p.start();
        await new Promise((r) => setTimeout(r, 30));
        p.stop();
        assert.deepEqual(unhandled, []);
    } finally {
        process.off("unhandledRejection", onUnhandled);
    }
});

