import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import zlib from "node:zlib";
import { CallTrace, callControlsOf, decodeEventPayload, frameHeader, incomingCallOf, inflateAny, inspectFrame, parseJsonKeepingBigInts, redactForTrace } from "../src/calls.js";
import { ZaloSession, type CallEvent } from "../src/session.js";
import { ChatStore } from "../src/store.js";

const KEY = crypto.randomBytes(32).toString("base64");

/** A control frame the way Zalo sends it: header + {data, encrypt: 2} with AES-GCM over zlib'd JSON. */
function controlFrame(controls: unknown[], cmd = 601): Buffer {
    return eventFrame({ data: { controls } }, cmd);
}

function eventFrame(body: unknown, cmd: number, compress: (b: Buffer) => Buffer = zlib.deflateSync): Buffer {
    const json = Buffer.from(JSON.stringify(body));
    const iv = crypto.randomBytes(16);
    const aad = crypto.randomBytes(16);
    const cipher = crypto.createCipheriv("aes-256-gcm", Buffer.from(KEY, "base64"), iv);
    cipher.setAAD(aad);
    const sealed = Buffer.concat([cipher.update(compress(json)), cipher.final(), cipher.getAuthTag()]);
    const payload = JSON.stringify({ data: encodeURIComponent(Buffer.concat([iv, aad, sealed]).toString("base64")), encrypt: 2 });
    const header = Buffer.alloc(4);
    header[0] = 1;
    header.writeUInt16LE(cmd, 1);
    header[3] = 0;
    return Buffer.concat([header, Buffer.from(payload)]);
}

const requestControl = {
    content: {
        act_type: "voip",
        act: "request",
        // Written as text: a JS number literal would already round the id.
        data: '{"callerId":1234567890123456789,"callId":77,"callType":1,"ts":1700000000000}',
    },
};

test("frames are read like zca-js: header, then the AES-GCM + zlib payload", () => {
    const frame = controlFrame([requestControl, { content: { act_type: "file_done", act: "x", data: {} } }]);
    assert.deepEqual(frameHeader(frame), { version: 1, cmd: 601, subCmd: 0 });
    const controls = callControlsOf(frame, KEY);
    assert.equal(controls.length, 1, "only voip controls");
    assert.equal(controls[0].act, "request");
    // Zalo ids longer than a JS number keep every digit.
    assert.equal(controls[0].data.callerId, "1234567890123456789");
    assert.equal(controls[0].data.callId, 77);
});

test("other frames and missing keys are ignored, not thrown", () => {
    assert.deepEqual(callControlsOf(controlFrame([requestControl], 501), KEY), []);
    assert.deepEqual(callControlsOf(Buffer.from([1, 2]), KEY), []);
    assert.deepEqual(callControlsOf(controlFrame([requestControl]), undefined), []);
    assert.equal(decodeEventPayload({ data: '{"a":1}', encrypt: 0 }, undefined) !== null, true);
});

test("big integers survive parsing, small ones stay numbers", () => {
    assert.deepEqual(parseJsonKeepingBigInts('{"a":12345678901234567890,"b":[1,98765432109876543210],"c":42}'), {
        a: "12345678901234567890",
        b: [1, "98765432109876543210"],
        c: 42,
    });
});

test("an incoming call names its caller, call id and whether it is video", () => {
    const [control] = callControlsOf(controlFrame([requestControl]), KEY);
    assert.deepEqual(incomingCallOf(control), {
        callId: "77",
        callerId: "1234567890123456789",
        video: true,
        group: false,
        groupId: undefined,
        ts: 1_700_000_000_000,
    });
    assert.equal(incomingCallOf({ act: "endcall", data: {}, raw: {} }), null);
    assert.equal(incomingCallOf({ act: "request", data: {}, raw: {} }), null, "no caller, no call");
});

test("the session announces incoming calls, their end, and keeps a call log", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-calls-"));
    const store = new ChatStore();
    store.upsertThread({ id: "555", kind: "user", name: "Bảo", avatar: "" });
    const session = new ZaloSession(store, dir);
    const events: CallEvent[] = [];
    session.on("call", (e) => events.push(e));

    session.onCallControl({ act: "request", data: { callerId: "555", callId: "9", callType: 0 }, raw: { content: { act: "request" } } });
    session.onCallControl({ act: "cancel", data: { callId: "9" }, raw: { content: { act: "cancel" } } });
    session.onCallControl({ act: "ringring", data: { callId: "9" }, raw: {} });

    assert.equal(events.length, 2);
    assert.deepEqual(events[0], {
        type: "incoming",
        call: { callId: "9", callerId: "555", video: false, group: false, groupId: undefined, ts: (events[0] as { call: { ts: number } }).call.ts, callerName: "Bảo" },
    });
    assert.deepEqual(events[1], { type: "ended", callId: "9", act: "cancel" });

    const log = session.callLog().trim().split("\n").map((l) => JSON.parse(l));
    assert.deepEqual(log.map((l) => l.act), ["request", "cancel", "ringring"]);
    session.clearCallLog();
    assert.equal(session.callLog(), "");
});

test("the call log rotates instead of growing without end", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-trace-"));
    const trace = new CallTrace(dir);
    const big = "x".repeat(300_000);
    for (let i = 0; i < 9; i++) trace.record({ i, big });
    assert.ok(fs.existsSync(path.join(dir, "call-trace.jsonl.1")));
    assert.ok(trace.read().length < 5 * 1024 * 1024);
});

test("the call log notes every control's type and unknown frames' field names, never their content", () => {
    const info = inspectFrame(controlFrame([requestControl, { content: { act_type: "fr", act: "add", data: { secret: "x" } } }]), KEY);
    assert.deepEqual(info?.controlTypes, ["voip/request", "fr/add"]);
    assert.equal(info?.calls.length, 1);

    const unknown = inspectFrame(eventFrame({ data: { callSignal: { sdp: "v=0" }, n: 1 } }, 777), KEY);
    assert.deepEqual(unknown?.unknown, { version: 1, cmd: 777, subCmd: 0, bytes: unknown?.unknown?.bytes, keys: ["data", "data.callSignal", "data.n"] });
    assert.ok(!JSON.stringify(unknown).includes("v=0"), "values are not recorded");

    assert.equal(inspectFrame(eventFrame({ data: { msgs: [] } }, 501), KEY), null, "message frames are none of the log's business");
});

test("gzipped and raw-deflated payloads decode too (call controls from a real phone came gzipped)", () => {
    const body = { data: { controls: [requestControl] } };
    for (const compress of [zlib.gzipSync, zlib.deflateRawSync, zlib.deflateSync]) {
        const controls = callControlsOf(eventFrame(body, 601, compress), KEY);
        assert.equal(controls.length, 1, compress.name);
        assert.equal(controls[0].data.callerId, "1234567890123456789");
    }
});

test("a payload that isn't compressed at all says what it starts with", () => {
    assert.throws(() => callControlsOf(eventFrame({ data: {} }, 601, (b) => Buffer.concat([Buffer.from([9, 9, 9, 9]), b])), KEY), /starting 09 09 09 09/);
});

test("redactForTrace and CallTrace.record strip sensitive secrets/PII while keeping debugging fields", () => {
    const sample = {
        kind: "media_join",
        callId: "77",
        callerId: "12345",
        server: "1.2.3.4:4200",
        rtpAddress: "1.2.3.4:4200",
        token: 998877,
        address: "198.51.100.2:55555",
        natAddress: "198.51.100.2:55555",
        session: "SECRET-SESSION-KEY-0123456789abcdef",
        sessId: "SECRET-SESS-ID",
        session_id: "SECRET-SESSION-ID",
        srtpKey: "SECRET-SRTP-KEY",
        srtp_key: "SECRET-SRTP-KEY-2",
        p2p: [{ ip: "198.51.100.7", port: 1234 }],
        displayName: "Alice",
        callerName: "Alice",
        avatar: "https://cdn.example/avatar.jpg",
        callerAvatar: "https://cdn.example/caller.jpg",
        stats: { sent: { audio: 10 }, received: { audio: 12 } },
        control: {
            content: {
                act_type: "voip",
                act: "request",
                data: JSON.stringify({
                    callId: "77",
                    session: "NESTED-SECRET-SESSION",
                    params: JSON.stringify({
                        sessId: "NESTED-SESS-ID",
                        Dname: "Alice",
                        avatar: "https://cdn.example/nested.jpg",
                        extendData: JSON.stringify({ p2p: [{ ip: "10.0.0.1", port: 5000 }], callType: 0 }),
                    }),
                }),
            },
        },
    };

    const redacted = redactForTrace(sample) as Record<string, unknown>;
    assert.equal(redacted.callId, "77");
    assert.equal(redacted.callerId, "12345");
    assert.equal(redacted.server, "1.2.3.4:4200");
    assert.equal(redacted.rtpAddress, "1.2.3.4:4200");
    assert.deepEqual(redacted.stats, { sent: { audio: 10 }, received: { audio: 12 } });
    for (const k of ["token", "address", "natAddress", "session", "sessId", "session_id", "srtpKey", "srtp_key", "p2p", "displayName", "callerName", "avatar", "callerAvatar"]) {
        assert.equal(redacted[k], "[redacted]", `${k} should be redacted`);
    }

    const dir = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-redact-")), "nested");
    const trace = new CallTrace(dir);
    trace.record(sample);
    const dirMode = fs.statSync(dir).mode & 0o777;
    assert.equal(dirMode, 0o700, "trace directory created with 0700 mode");
    const raw = trace.read();
    for (const secret of ["SECRET-SESSION", "SECRET-SESS", "SECRET-SRTP", "998877", "198.51.100", "10.0.0.1", "Alice", "cdn.example", "NESTED-SECRET", "NESTED-SESS"]) {
        assert.ok(!raw.includes(secret), `trace must not contain ${secret}`);
    }
    assert.ok(raw.includes("1.2.3.4:4200"), "media server address kept");
    assert.ok(raw.includes('"callId":"77"'), "callId kept");
});

test("inflateAny enforces maxOutputLength against zip bombs", () => {
    const huge = Buffer.alloc(3 * 1024 * 1024);
    for (const compress of [zlib.gzipSync, zlib.deflateSync, zlib.deflateRawSync]) {
        const bomb = compress(huge);
        assert.throws(() => inflateAny(bomb), { code: "ERR_BUFFER_TOO_LARGE" }, compress.name);
    }
});

test("decodeEventPayload rejects unknown encrypt modes and invalid cipherKey lengths", () => {
    assert.equal(decodeEventPayload({ data: '{"a":1}', encrypt: 4 }, KEY), null);
    assert.equal(decodeEventPayload({ data: '{"a":1}', encrypt: -1 }, KEY), null);
    assert.equal(decodeEventPayload({ data: '{"a":1}', encrypt: Number.NaN }, KEY), null);
    const frame = controlFrame([requestControl]);
    const parsed = JSON.parse(frame.subarray(4).toString("utf8")) as { data: string; encrypt: number };
    assert.equal(decodeEventPayload(parsed, Buffer.alloc(15).toString("base64")), null, "15-byte key rejected");
    assert.equal(decodeEventPayload(parsed, Buffer.alloc(20).toString("base64")), null, "20-byte key rejected");
    assert.equal(decodeEventPayload(parsed, ""), null, "empty key rejected");
});

test("incomingCallOf requires a non-empty callId", () => {
    assert.equal(incomingCallOf({ act: "request", data: { callerId: "555" }, raw: {} }), null);
    assert.equal(incomingCallOf({ act: "request", data: { callerId: "555", callId: "" }, raw: {} }), null);
    assert.equal(incomingCallOf({ act: "request", data: { callerId: "555", callId: "   " }, raw: {} }), null);
});

