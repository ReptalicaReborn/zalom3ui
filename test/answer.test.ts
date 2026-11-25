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

const SESSION = "TEST-made-up-session-0123456789abcdefghijklmnop";

/** A fake Zalo media server on localhost: accepts the join, streams one audio packet, collects what the app sends. */
async function fakeMediaServer() {
    const server = dgram.createSocket("udp4");
    await new Promise<void>((r) => server.bind(0, "127.0.0.1", r));
    const caller = SrtpContext.fromSession(SESSION);
    const reader = SrtpContext.fromSession(SESSION);
    const got = { joins: 0, audio: [] as Buffer[], ends: 0 };
    server.on("message", (msg, from) => {
        if (msg[0] === zrtc.Kind.Request) {
            const cmd = msg.readUInt16LE(18);
            if (cmd === zrtc.Cmd.InitCallCallee) {
                got.joins++;
                const reply = Buffer.concat([msg.subarray(0, 21), Buffer.from([0, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 0, 0])]);
                reply[0] = zrtc.Kind.Response;
                server.send(reply, from.port, from.address);
                const rtp = Buffer.from("80700005000000640000beef", "hex");
                server.send(Buffer.concat([Buffer.from([zrtc.Kind.AudioIn]), caller.protect(Buffer.concat([rtp, Buffer.from("opus-frame")]))]), from.port, from.address);
            } else if (cmd === zrtc.Cmd.EndCall) got.ends++;
        } else if (msg[0] === zrtc.Kind.Audio) {
            const rtp = reader.unprotect(msg.subarray(5));
            if (rtp) got.audio.push(rtp.subarray(12));
        }
    });
    return { port: (server.address() as { port: number }).port, got, close: () => server.close() };
}

function sessionWithFakeZalo() {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-answer-"));
    const session = new ZaloSession(new ChatStore(), dir);
    const sent: [string, Record<string, unknown>][] = [];
    const api: Record<string, unknown> = {
        getContext: () => ({ imei: "imei-1" }),
        custom(name: string) {
            api[name] = async ({ endpoint, params }: { endpoint: string; params: Record<string, unknown> }) => {
                sent.push([endpoint, params]);
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

function ring(session: ZaloSession, port: number, callId: string) {
    session.onCallControl({
        act: "request",
        data: {
            uidFrom: "465000001",
            uidN: "4459000000000000001",
            uidTo: "438000002",
            callId,
            session: SESSION,
            rtpAddress: `127.0.0.1:${port}`,
            params: JSON.stringify({ userId: 438000002, Dname: "Bảo", extendData: JSON.stringify({ callType: 0 }) }),
        },
        raw: {},
    });
}

test("answering joins the media server, answers, carries audio both ways, and ends when the caller hangs up", async () => {
    const media = await fakeMediaServer();
    const { session, sent, events } = sessionWithFakeZalo();
    const heard: string[] = [];
    session.on("call_audio", (a) => heard.push(`${a.seq}:${a.opus.toString()}`));
    ring(session, media.port, "321000003");

    const call = await session.answerCall("321000003");
    assert.equal(call.state, "connected");
    assert.equal(call.callerName, "Bảo");
    assert.deepEqual(sent.map(([e]) => e), ["ringring", "answer"]);
    assert.equal(sent[1][1].status, 0);
    assert.equal(events.at(-1)?.type, "connected");
    await assert.rejects(session.answerCall("321000003"), /isn't ringing/);

    session.sendCallAudio(Buffer.from("my-voice"));
    await new Promise((r) => setTimeout(r, 120));
    assert.deepEqual(heard, ["5:opus-frame"], "the caller's audio, decrypted");
    assert.ok(media.got.audio.some((a) => a.toString() === "my-voice"), "our frame reached the media server, decryptable");
    assert.ok(media.got.audio.some((a) => a.toString("hex") === "f8fffe"), "silence fills in between frames");
    assert.equal(session.currentCall()?.callId, "321000003");

    session.onCallControl({ act: "endcall", data: { callId: "321000003" }, raw: {} });
    assert.equal(session.currentCall(), null);
    assert.deepEqual(events.at(-1), { type: "ended", callId: "321000003", act: "endcall" });
    assert.ok(!sent.some(([e]) => e === "endcall"), "the caller hung up: nothing to send");
    media.close();
});

test("hanging up sends Zalo's end call and the media server's", async () => {
    const media = await fakeMediaServer();
    const { session, sent, events } = sessionWithFakeZalo();
    ring(session, media.port, "321000004");
    await session.answerCall("321000004");
    session.hangUpCall();
    await new Promise((r) => setTimeout(r, 80));
    assert.equal(media.got.ends, 1);
    assert.deepEqual(sent.at(-1), ["endcall", { uidTo: "4459000000000000001", callId: 321000004, status: 3, imei: "imei-1" }]);
    assert.deepEqual(events.at(-1), { type: "ended", callId: "321000004", act: "hangup" });
    assert.throws(() => session.hangUpCall(), /No call/);
    media.close();
});
