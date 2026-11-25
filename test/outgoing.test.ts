import assert from "node:assert/strict";
import dgram from "node:dgram";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { ZaloSession, type CallEvent } from "../src/session.js";
import { SrtpContext } from "../src/srtp.js";
import { ChatStore } from "../src/store.js";
import * as zrtc from "../src/zrtc.js";
import { outgoingMediaOf } from "../src/voicecall.js";

const SESSION = "TEST-made-up-session-0123456789abcdefghijklmnop";

/** A fake media server: accepts the caller's init call (command 11), streams one audio packet, counts audio and ends. */
async function fakeMediaServer(accept = true) {
    const server = dgram.createSocket("udp4");
    await new Promise<void>((r) => server.bind(0, "127.0.0.1", r));
    const theirs = SrtpContext.fromSession(SESSION);
    const reader = SrtpContext.fromSession(SESSION);
    const got = { joins: 0, callerJoins: 0, audio: [] as Buffer[], ends: 0 };
    server.on("message", (msg, from) => {
        if (msg[0] === zrtc.Kind.Request) {
            const cmd = msg.readUInt16LE(18);
            if (cmd === zrtc.Cmd.InitCallCaller || cmd === zrtc.Cmd.InitCallCallee) {
                got.joins++;
                if (cmd === zrtc.Cmd.InitCallCaller) got.callerJoins++;
                if (!accept) return;
                const reply = Buffer.concat([msg.subarray(0, 21), Buffer.from([0, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 0, 0])]);
                reply[0] = zrtc.Kind.Response;
                server.send(reply, from.port, from.address);
                const rtp = Buffer.from("80700005000000640000beef", "hex");
                server.send(Buffer.concat([Buffer.from([zrtc.Kind.AudioIn]), theirs.protect(Buffer.concat([rtp, Buffer.from("opus-frame")]))]), from.port, from.address);
            } else if (cmd === zrtc.Cmd.EndCall) got.ends++;
        } else if (msg[0] === zrtc.Kind.Audio) {
            const rtp = reader.unprotect(msg.subarray(5));
            if (rtp) got.audio.push(rtp.subarray(12));
        }
    });
    return { port: (server.address() as { port: number }).port, got, close: () => server.close() };
}

function sessionWithFakeZalo(port: number, overrides: Partial<Record<string, unknown>> = {}) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-outgoing-"));
    const session = new ZaloSession(new ChatStore(), dir);
    const sent: [string, Record<string, unknown>][] = [];
    const api: Record<string, unknown> = {
        getContext: () => ({ imei: "imei-1" }),
        custom(name: string) {
            api[name] = async ({ endpoint, params }: { endpoint: string; params: Record<string, unknown> }) => {
                sent.push([endpoint, params]);
                if (endpoint === "requestcall") {
                    return {
                        id: params.callId,
                        status: 0,
                        fromId: 438000002,
                        toId: 465000001,
                        sessId: SESSION,
                        zrtc_config: { srtpMode: 1 },
                        servers: [{ rtpaddr: `127.0.0.1:${port}`, rtcpaddr: `127.0.0.1:${port}` }],
                        ...overrides,
                    };
                }
                return {};
            };
        },
    };
    const internals = session as unknown as { api: unknown; status: { state: string } };
    internals.api = api;
    internals.status.state = "ready";
    const events: CallEvent[] = [];
    session.on("call", (e) => events.push(e));
    return { session, sent, events };
}

const control = (act: string, callId: string, extra: Record<string, unknown> = {}) => ({ act, data: { callId, ...extra }, raw: {} });
const tick = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("placing a call: requestcall, join as caller, request, ring, answer, audio both ways, hang up", async () => {
    const media = await fakeMediaServer();
    const { session, sent, events } = sessionWithFakeZalo(media.port);
    const heard: string[] = [];
    session.on("call_audio", (a) => heard.push(a.opus.toString()));

    const call = await session.startCall("465000001", false);
    assert.equal(call.state, "calling");
    assert.equal(call.outgoing, true);
    assert.equal(call.callerId, "465000001");
    assert.equal(media.got.callerJoins, 1, "the caller's init call (command 11)");
    assert.deepEqual(sent.map(([e]) => e), ["requestcall", "request"]);
    assert.equal(sent[0][1].calleeId, "465000001");
    assert.equal(sent[0][1].typeRequest, 1);
    assert.equal(sent[0][1].imei, "imei-1");
    assert.equal(sent[1][1].rtpAddress, `127.0.0.1:${media.port}`);
    assert.equal(sent[1][1].subCommand, 3);
    assert.equal(sent[1][1].callId, Number(call.callId));
    assert.equal(JSON.parse(sent[1][1].extendData as string).callType, 0);
    assert.equal(events[0].type, "outgoing");
    assert.equal(session.currentCall()?.state, "calling");
    await assert.rejects(session.startCall("465000001", false), /Already in a call/);

    session.onCallControl(control("ring_ring", call.callId));
    assert.equal(events.at(-1)?.type, "ringing");
    assert.equal(session.currentCall()?.state, "ringing");

    session.onCallControl(control("answer", call.callId, { params: JSON.stringify({ status: 0, rtpSerIp: `127.0.0.1:${media.port}` }) }));
    assert.equal(events.at(-1)?.type, "connected");
    assert.equal(session.currentCall()?.state, "connected");
    await tick(10);
    assert.equal(sent.at(-1)?.[0], "answerack");
    assert.deepEqual(sent.at(-1)?.[1], { calleeId: "465000001", callId: Number(call.callId), status: 0, imei: "imei-1" });

    session.sendCallAudio(Buffer.from("my-voice"));
    await tick(120);
    assert.deepEqual(heard, ["opus-frame"]);
    assert.ok(media.got.audio.some((a) => a.toString() === "my-voice"));

    session.hangUpCall();
    await tick(80);
    assert.equal(media.got.ends, 1);
    assert.equal(sent.at(-1)?.[0], "endcall");
    assert.deepEqual(events.at(-1), { type: "ended", callId: call.callId, act: "hangup" });
    assert.equal(session.currentCall(), null);
    media.close();
});

test("a video call asks for type 3, says the camera is off after connecting, and can be cancelled while ringing", async () => {
    const media = await fakeMediaServer();
    const { session, sent, events } = sessionWithFakeZalo(media.port);
    const call = await session.startCall("465000001", true);
    assert.equal(sent[0][1].typeRequest, 3);
    assert.equal(JSON.parse(sent[1][1].extendData as string).callType, 1);
    session.onCallControl(control("ring_ring", call.callId));
    session.onCallControl(control("answer", call.callId, { status: 0 }));
    await tick(10);
    assert.deepEqual(sent.find(([e]) => e === "conf")?.[1], { calleeId: "465000001", callId: Number(call.callId), isOnCam: 0, isOnMic: -1, data: "" });
    session.setCallCamera(true);
    await tick(10);
    assert.equal(sent.at(-1)?.[1].isOnCam, 1);
    session.hangUpCall();
    media.close();

    const second = await fakeMediaServer();
    const b = sessionWithFakeZalo(second.port);
    const c2 = await b.session.startCall("465000001", true);
    b.session.onCallControl(control("ring_ring", c2.callId));
    b.session.hangUpCall();
    await tick(80);
    const cancel = b.sent.find(([e]) => e === "cancel");
    assert.deepEqual(cancel?.[1], { callerId: "465000001", callId: Number(c2.callId), callType: 1, status: 0, imei: "imei-1" });
    assert.ok(!b.sent.some(([e]) => e === "endcall"));
    assert.deepEqual(b.events.at(-1), { type: "ended", callId: c2.callId, act: "cancel" });
    assert.equal(events.length > 0, true);
    second.close();
});

test("declined, busy, cancelled and rejected calls end with a reason and send nothing more", async () => {
    const cases: [string, Record<string, unknown>, string][] = [
        ["answer", { params: JSON.stringify({ status: 3 }) }, "declined"],
        ["answer", { status: 1 }, "busy"],
        ["busy", {}, "busy"],
        ["reject", {}, "declined"],
        ["cancel", {}, "cancel"],
    ];
    for (const [act, extra, reason] of cases) {
        const media = await fakeMediaServer();
        const { session, sent, events } = sessionWithFakeZalo(media.port);
        const call = await session.startCall("465000001", false);
        session.onCallControl(control("ring_ring", call.callId));
        session.onCallControl(control(act, call.callId, extra));
        assert.deepEqual(events.at(-1), { type: "ended", callId: call.callId, act: reason }, `${act} -> ${reason}`);
        assert.equal(session.currentCall(), null);
        await tick(60);
        assert.ok(!sent.some(([e]) => e === "cancel" || e === "endcall"));
        media.close();
    }
});

test("nobody picks up: it times out as no answer and cancels; no ring at all is unreachable", async () => {
    const media = await fakeMediaServer();
    const { session, sent, events } = sessionWithFakeZalo(media.port);
    const call = await session.startCall("465000001", false, { setupMs: 40, ringMs: 60 });
    session.onCallControl(control("ring_ring", call.callId));
    await tick(150);
    assert.deepEqual(events.at(-1), { type: "ended", callId: call.callId, act: "no_answer" });
    assert.ok(sent.some(([e]) => e === "cancel"));

    const other = sessionWithFakeZalo(media.port);
    const c2 = await other.session.startCall("465000001", false, { setupMs: 40, ringMs: 60 });
    await tick(120);
    assert.deepEqual(other.events.at(-1), { type: "ended", callId: c2.callId, act: "unreachable" });
    media.close();
});

test("when Zalo or the media server refuses, the call fails cleanly", async () => {
    const media = await fakeMediaServer();
    const bad = sessionWithFakeZalo(media.port, { status: -1, msg: "nope" });
    await assert.rejects(bad.session.startCall("465000001", false), /status -1/);
    assert.equal(bad.session.currentCall(), null);
    assert.deepEqual(bad.events.map((e) => e.type), ["outgoing", "ended"]);
    assert.ok(!bad.sent.some(([e]) => e === "cancel"));

    const noServers = sessionWithFakeZalo(media.port, { servers: [] });
    await assert.rejects(noServers.session.startCall("465000001", false), /no media server/);

    const deaf = await fakeMediaServer(false);
    const refused = sessionWithFakeZalo(deaf.port);
    await assert.rejects(refused.session.startCall("465000001", false), /media server didn't let us join/);
    assert.equal(refused.session.currentCall(), null);
    assert.ok(!refused.sent.some(([e]) => e === "request"));

    await assert.rejects(sessionWithFakeZalo(media.port).session.startCall("not-a-number", false), /isn't a person/);
    media.close();
    deaf.close();
});

test("controls for another call are ignored; incoming calls still ring while we are calling", async () => {
    const media = await fakeMediaServer();
    const { session, events } = sessionWithFakeZalo(media.port);
    const call = await session.startCall("465000001", false);
    session.onCallControl(control("busy", "999"));
    assert.equal(session.currentCall()?.callId, call.callId);
    session.onCallControl({ act: "request", data: { uidN: "777", callId: "888", uidFrom: "1", uidTo: "2", params: "{}" }, raw: {} });
    assert.equal(events.at(-1)?.type, "incoming");
    session.hangUpCall();
    media.close();
});

test("outgoingMediaOf reads the answer and lists only field names for the log", () => {
    const ok = outgoingMediaOf({ id: 1_500_000_000, fromId: 5, toId: 6, sessId: SESSION, servers: [{ rtpaddr: "1.2.3.4:4200" }], extra: 1 }, 1_500_000_000);
    assert.ok("media" in ok);
    if ("media" in ok) {
        assert.deepEqual(ok.media.servers, [{ rtp: "1.2.3.4:4200", rtcp: "1.2.3.4:4200" }]);
        assert.deepEqual(ok.shape, ["extra", "fromId", "id", "servers", "sessId", "toId"]);
    }
    assert.ok("error" in outgoingMediaOf({ id: 2, fromId: 5, toId: 6, sessId: SESSION, servers: [{ rtpaddr: "1.2.3.4:4200" }] }, 1));
});
