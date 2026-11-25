import assert from "node:assert/strict";
import dgram from "node:dgram";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { ZaloSession, type CallEvent } from "../src/session.js";
import { parseRtcp, pli } from "../src/rtcp.js";
import { rtpHeader, SrtpContext } from "../src/srtp.js";
import { ChatStore } from "../src/store.js";
import * as zrtc from "../src/zrtc.js";

const SESSION = "TEST-made-up-session-0123456789abcdefghijklmnop";

/** A fake Zalo media server on localhost: accepts the join, streams one audio packet, collects what the app sends. */
async function fakeMediaServer() {
    const server = dgram.createSocket("udp4");
    await new Promise<void>((r) => server.bind(0, "127.0.0.1", r));
    const caller = SrtpContext.fromSession(SESSION);
    const reader = SrtpContext.fromSession(SESSION);
    const got = { joins: 0, audio: [] as Buffer[], ends: 0, video: [] as Buffer[], rtcpVideo: [] as Buffer[] };
    let peer: { port: number; address: string } | null = null;
    server.on("message", (msg, from) => {
        peer = from;
        if (msg[0] === zrtc.Kind.Request) {
            const cmd = msg.readUInt16LE(18);
            if (cmd === zrtc.Cmd.InitCallCallee) {
                got.joins++;
                const reply = Buffer.concat([msg.subarray(0, 21), Buffer.from([0, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 0, 0])]);
                reply[0] = zrtc.Kind.Response;
                server.send(reply, from.port, from.address);
                const rtp = Buffer.from("80700005000000640000beef", "hex");
                server.send(Buffer.concat([Buffer.from([zrtc.Kind.AudioIn]), caller.protect(Buffer.concat([rtp, Buffer.from("opus-frame")]))]), from.port, from.address);
                // One key frame of the caller's video: pt 98 + marker, timestamp 9000, SSRC 0x5151, Annex B IDR slice.
                const vrtp = Buffer.from("80e2000000002328" + "00005151" + "00000001658880", "hex");
                server.send(Buffer.concat([Buffer.from([zrtc.Kind.VideoIn]), caller.protect(vrtp)]), from.port, from.address);
            } else if (cmd === zrtc.Cmd.EndCall) got.ends++;
        } else if (msg[0] === zrtc.Kind.Video) {
            const rtp = reader.unprotect(msg.subarray(5));
            if (rtp) got.video.push(rtp);
        } else if (msg[0] === zrtc.Kind.VideoRtcp) {
            got.rtcpVideo.push(msg.subarray(5));
        } else if (msg[0] === zrtc.Kind.Audio) {
            const rtp = reader.unprotect(msg.subarray(5));
            if (rtp) got.audio.push(rtp.subarray(12));
        }
    });
    return {
        port: (server.address() as { port: number }).port,
        got,
        /** Sends a raw zrtc packet to the app side, as the media server would. */
        push: (packet: Buffer) => peer && server.send(packet, peer.port, peer.address),
        close: () => server.close(),
    };
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
    assert.deepEqual(sent.map(([e]) => e), ["ringring", "answer"], "a voice call: no camera state to announce");
    assert.equal(sent[1][1].status, 0);
    assert.equal(events.at(-1)?.type, "connected");
    await assert.rejects(session.answerCall("321000003"), /isn't ringing/);

    session.sendCallAudio(Buffer.from("my-voice"));
    await new Promise((r) => setTimeout(r, 120));
    assert.deepEqual(heard, ["5:opus-frame"], "the caller's audio, decrypted");
    assert.ok(media.got.audio.some((a) => a.toString() === "my-voice"), "our frame reached the media server, decryptable");
    assert.ok(media.got.audio.some((a) => a.toString("hex") === "f8fffe"), "silence fills in between frames");
    assert.equal(session.currentCall()?.callId, "321000003");

    session.setCallMuted(true);
    await new Promise((r) => setTimeout(r, 10));
    assert.deepEqual(sent.at(-1), ["conf", { calleeId: "4459000000000000001", callId: 321000003, isOnCam: -1, isOnMic: 1, data: "" }]);

    session.onCallControl({ act: "end_call", data: { callId: "321000003" }, raw: {} });
    assert.equal(session.currentCall(), null, "end_call (with the underscore) ends it too");
    assert.deepEqual(events.at(-1), { type: "ended", callId: "321000003", act: "end_call" });
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

test("answering a video call without a camera says the camera is off", async () => {
    const media = await fakeMediaServer();
    const { session, sent } = sessionWithFakeZalo();
    session.onCallControl({
        act: "request",
        data: {
            uidFrom: "465000001",
            uidN: "4459000000000000001",
            uidTo: "438000002",
            callId: "321000005",
            session: SESSION,
            rtpAddress: `127.0.0.1:${media.port}`,
            params: JSON.stringify({ userId: 438000002, extendData: JSON.stringify({ callType: 1 }) }),
        },
        raw: {},
    });
    await session.answerCall("321000005");
    await new Promise((r) => setTimeout(r, 10));
    assert.equal(JSON.parse(sent[1][1].extendData as string).callType, 1, "still a video call");
    assert.deepEqual(sent[2], ["conf", { calleeId: "4459000000000000001", callId: 321000005, isOnCam: 0, isOnMic: -1, data: "" }]);
    session.hangUpCall();
    media.close();
});

function ringVideo(session: ZaloSession, port: number, callId: string) {
    session.onCallControl({
        act: "request",
        data: {
            uidFrom: "465000001",
            uidN: "4459000000000000001",
            uidTo: "438000002",
            callId,
            session: SESSION,
            rtpAddress: `127.0.0.1:${port}`,
            params: JSON.stringify({ userId: 438000002, extendData: JSON.stringify({ callType: 1 }) }),
        },
        raw: {},
    });
}

test("video in a call: the other side's frames and key-frame requests reach the app, ours go out only while the camera is on", async () => {
    const media = await fakeMediaServer();
    const { session, sent } = sessionWithFakeZalo();
    const frames: string[] = [];
    let keyRequests = 0;
    session.on("call_video", (f) => frames.push(`${f.key}:${f.timestamp}:${f.data.toString("hex")}`));
    session.on("call_key_frame", () => keyRequests++);
    ringVideo(session, media.port, "321000006");
    await session.answerCall("321000006");
    await new Promise((r) => setTimeout(r, 120));
    assert.deepEqual(frames, ["true:9000:00000001658880"]);

    // The camera is off until the app says otherwise: its frames are not forwarded.
    const frame = { key: true, data: Buffer.from("00000001658880", "hex") };
    session.sendCallVideo(frame);
    await new Promise((r) => setTimeout(r, 40));
    assert.equal(media.got.video.length, 0);
    session.setCallCamera(true);
    await new Promise((r) => setTimeout(r, 10));
    assert.deepEqual(sent.at(-1), ["conf", { calleeId: "4459000000000000001", callId: 321000006, isOnCam: 1, isOnMic: -1, data: "" }]);
    session.sendCallVideo(frame);
    await new Promise((r) => setTimeout(r, 40));
    assert.equal(media.got.video.length, 1);
    const h = rtpHeader(media.got.video[0])!;
    assert.equal(h.payloadType, 98 | 0, "key frame payload type");
    assert.equal(h.marker, true);

    // The caller asks for a key frame (PLI aimed at our video SSRC): the app is told.
    media.push(zrtc.rtcp(7, pli(0x7777, h.ssrc), true));
    await new Promise((r) => setTimeout(r, 40));
    assert.equal(keyRequests, 1);

    // And the app asks for one from the caller: a PLI about the caller's video stream goes out.
    session.requestCallKeyFrame();
    await new Promise((r) => setTimeout(r, 40));
    const plis = media.got.rtcpVideo.flatMap((b) => parseRtcp(b)).filter((p) => p.type === "PLI");
    assert.ok(plis.some((p) => p.type === "PLI" && p.mediaSsrc === 0x5151), "PLI for the caller's video SSRC");

    session.setCallCamera(false);
    session.hangUpCall();
    media.close();
});

test("the other side's camera and microphone controls become peer events, and don't end the call", async () => {
    const media = await fakeMediaServer();
    const { session, events } = sessionWithFakeZalo();
    ring(session, media.port, "321000007");
    await session.answerCall("321000007");
    const control = (act: string, params: unknown, callId = "321000007") =>
        session.onCallControl({ act, data: { callId, params: JSON.stringify(params) }, raw: {} });
    control("onoff_camera", { type: 0 });
    assert.deepEqual(events.at(-1), { type: "peer", callId: "321000007", cameraOn: false, muted: false });
    control("onoff_camera", { type: 1 });
    assert.deepEqual(events.at(-1), { type: "peer", callId: "321000007", cameraOn: true, muted: false });
    control("onoff_camera", { type: 0 });
    assert.deepEqual(events.at(-1), { type: "peer", callId: "321000007", cameraOn: false, muted: false }, "toggling again is not a duplicate");
    control("mute_audio", { status: 1 });
    assert.deepEqual(events.at(-1), { type: "peer", callId: "321000007", cameraOn: false, muted: true });
    control("mute_audio", { type: 0 });
    assert.deepEqual(events.at(-1), { type: "peer", callId: "321000007", cameraOn: false, muted: false });
    const before = events.length;
    control("onoff_camera", { type: 0 }, "999");
    assert.equal(events.length, before, "another call's control is ignored");
    assert.equal(session.currentCall()?.callId, "321000007", "still in the call");
    assert.ok(!events.some((e) => e.type === "ended"));
    session.hangUpCall();
    media.close();
});

test("our own answer echo does not emit an ended event, and media_join trace omits raw token and natAddress", async () => {
    const media = await fakeMediaServer();
    const { session, events } = sessionWithFakeZalo();
    try {
        ring(session, media.port, "321000008");
        await session.answerCall("321000008");

        const before = events.length;
        session.onCallControl({ act: "answer", data: { callId: "321000008" }, raw: {} });
        assert.equal(events.length, before, "our own answer echo must not emit ended");
        assert.equal(session.currentCall()?.callId, "321000008");

        const lines = session.callLog().trim().split("\n").map((l) => JSON.parse(l) as Record<string, unknown>);
        const joinEntry = lines.find((e) => e.kind === "media_join");
        assert.ok(joinEntry);
        assert.equal(joinEntry.joined, true);
        assert.equal("token" in joinEntry, false);
        assert.equal("address" in joinEntry, false);
    } finally {
        if (session.currentCall()) session.hangUpCall();
        media.close();
    }
});

test("answerCall and declineCall reject expired ringing calls and group calls", async () => {
    const media = await fakeMediaServer();
    const { session } = sessionWithFakeZalo();
    try {
        // Expired ringing call (receivedAt > 2 minutes ago)
        const oldNow = Date.now() - 3 * 60_000;
        session.onCallControl(
            {
                act: "request",
                data: {
                    uidFrom: "465000001",
                    uidN: "4459000000000000001",
                    uidTo: "438000002",
                    callId: "321000009",
                    session: SESSION,
                    rtpAddress: `127.0.0.1:${media.port}`,
                    params: JSON.stringify({ userId: 438000002, extendData: JSON.stringify({ callType: 0 }) }),
                },
                raw: {},
            },
            "socket",
            oldNow,
        );
        await assert.rejects(session.answerCall("321000009"), /isn't ringing/);

        // Group call cannot be answered or declined here
        session.onCallControl({
            act: "group_request",
            data: {
                uidFrom: "465000001",
                callId: "321000010",
                groupId: "g99",
            },
            raw: {},
        });
        await assert.rejects(session.answerCall("321000010"), /Group calls/);
        await assert.rejects(session.declineCall("321000010"), /Group calls/);
    } finally {
        media.close();
    }
});

test("hanging up or toggling camera during connecting phase behaves cleanly", async () => {
    const media = await fakeMediaServer();
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-answer-race-"));
    try {
        // Case 1: hangUpCall while ringring is in flight aborts cleanly before joining media or accepting.
        let releaseRingRing!: () => void;
        const sent1: string[] = [];
        const session1 = new ZaloSession(new ChatStore(), dir);
        const api1: Record<string, unknown> = {
            getContext: () => ({ imei: "imei-1" }),
            custom(name: string) {
                api1[name] = async ({ endpoint }: { endpoint: string }) => {
                    sent1.push(endpoint);
                    if (endpoint === "ringring") {
                        await new Promise<void>((r) => (releaseRingRing = r));
                    }
                    return {};
                };
            },
        };
        Object.assign(session1 as unknown as { api: unknown; status: { state: string } }, { api: api1, status: { state: "ready" } });
        const events1: CallEvent[] = [];
        session1.on("call", (e) => events1.push(e));
        ring(session1, media.port, "321000011");

        const answering = session1.answerCall("321000011");
        // declineCall cannot race answerCall because ringing entry was already claimed
        await assert.rejects(session1.declineCall("321000011"), /isn't ringing/);
        session1.hangUpCall();
        assert.equal(session1.currentCall(), null);
        assert.deepEqual(events1.at(-1), { type: "ended", callId: "321000011", act: "hangup" });
        releaseRingRing();
        await assert.rejects(answering, /Call ended while connecting/);
        assert.deepEqual(sent1, ["ringring"]);

        // Case 2: setCallCamera(true) during connecting sends isOnCam: 1 instead of hardcoded 0.
        let releaseRingRing2!: () => void;
        const sent2: [string, Record<string, unknown>][] = [];
        const session2 = new ZaloSession(new ChatStore(), dir);
        const api2: Record<string, unknown> = {
            getContext: () => ({ imei: "imei-1" }),
            custom(name: string) {
                api2[name] = async ({ endpoint, params }: { endpoint: string; params: Record<string, unknown> }) => {
                    sent2.push([endpoint, params]);
                    if (endpoint === "ringring") {
                        await new Promise<void>((r) => (releaseRingRing2 = r));
                    }
                    return {};
                };
            },
        };
        Object.assign(session2 as unknown as { api: unknown; status: { state: string } }, { api: api2, status: { state: "ready" } });
        ringVideo(session2, media.port, "321000012");
        const answering2 = session2.answerCall("321000012");
        session2.setCallCamera(true);
        releaseRingRing2();
        await answering2;
        await new Promise((r) => setTimeout(r, 15));
        const conf = sent2.find(([e]) => e === "conf");
        assert.ok(conf);
        assert.equal(conf[1].isOnCam, 1);
        session2.hangUpCall();
    } finally {
        media.close();
        fs.rmSync(dir, { recursive: true, force: true });
    }
});
