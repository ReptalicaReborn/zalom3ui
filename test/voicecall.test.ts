import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { incomingCallOf, parseJsonKeepingBigInts } from "../src/calls.js";
import { ZaloSession } from "../src/session.js";
import { ChatStore } from "../src/store.js";
import { DEFAULT_VOICE_CALL_DOMAIN, declineCall, ringRing, ringingCallOf, voiceCallDomain } from "../src/voicecall.js";

test("the voice call service comes from the login's service map, with Zalo PC's default", () => {
    assert.equal(voiceCallDomain({ voice_call: ["https://voicecall-wpa.chat.zalo.me/"] }), "https://voicecall-wpa.chat.zalo.me");
    assert.equal(voiceCallDomain({ voice_call: "https://vc.example" }), "https://vc.example");
    assert.equal(voiceCallDomain({}), DEFAULT_VOICE_CALL_DOMAIN);
    assert.equal(voiceCallDomain(undefined), DEFAULT_VOICE_CALL_DOMAIN);
});

test("declining is an answer with status 3, as Zalo PC's call program sends it", async () => {
    // The shape of a real 1:1 request (values made up): 32-bit zrtc ids, the noised user id, ids as text.
    const codec = '[{"frmPtime":20,"payload":112,"name":"opus/16000/1","dynamicFptime":0}]';
    const request = (callType: number) =>
        parseJsonKeepingBigInts(
            JSON.stringify({
                uidFrom: "465000001",
                uidN: "4459000000000000001",
                uidTo: "438000002",
                callId: "321000003",
                status: "0",
                ts: "1795617640181",
                codec,
                session: "SESSION-made-up",
                rtpAddress: "192.0.2.10:4200",
                rtcpAddress: "192.0.2.10:4200",
                params: JSON.stringify({
                    id: 321000003,
                    Dname: "Bảo",
                    extendData: JSON.stringify({ callType, packetMode: 2, srtpMode: 1, serverAddr: [{ rtp: "192.0.2.10:4200", rtcp: "192.0.2.10:4200" }], p2p: [{ ip: "198.51.100.7", port: 1 }] }),
                }),
            }),
        ) as Record<string, unknown>;
    for (const [callType, expected] of [[0, 1], [1, 3]]) {
        const control = { act: "request", data: request(callType), raw: {} };
        const incoming = incomingCallOf(control, 1795617640181)!;
        assert.equal(incoming.callerId, "4459000000000000001");
        assert.equal(incoming.video, callType === 1);
        assert.equal(incoming.displayName, "Bảo");
        const call = ringingCallOf(incoming, control, 1795617640181);
        assert.equal(call.callType, expected);
        const sent: [string, Record<string, unknown>][] = [];
        await declineCall(async (endpoint, params) => void sent.push([endpoint, params]), call);
        assert.deepEqual(sent, [
            [
                "answer",
                {
                    callerId: "4459000000000000001",
                    callId: 321000003,
                    status: 3,
                    codec,
                    rtcpAddress: "192.0.2.10:4200",
                    rtpAddress: "192.0.2.10:4200",
                    session_id: "SESSION-made-up",
                    extendData: sent[0][1].extendData,
                    subCommand: 3,
                },
            ],
        ]);
        const ours = JSON.parse(sent[0][1].extendData as string);
        assert.equal(ours.callType, callType);
        assert.equal(ours.platform, 2, "Zalo PC's platform");
        assert.deepEqual(ours.p2p, [], "never the caller's own addresses");
        assert.deepEqual(ours.serverAddr, [{ rtp: "192.0.2.10:4200", rtcp: "192.0.2.10:4200" }]);
        sent.length = 0;
        await ringRing(async (endpoint, params) => void sent.push([endpoint, params]), call, "imei-1");
        assert.deepEqual(sent, [["ringring", { callerId: "4459000000000000001", callId: 321000003, status: 0, imei: "imei-1" }]]);
    }
});

test("a call stays declinable while it rings, and not after it ended", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zaloo-decline-"));
    const session = new ZaloSession(new ChatStore(), dir);
    const ringing = () => (session as unknown as { ringing: Map<string, unknown> }).ringing;
    const now = Date.now();
    session.onCallControl({ act: "request", data: { callerId: "5", callId: "7" }, raw: {} }, "poll", now);
    assert.ok(ringing().has("7"));
    session.onCallControl({ act: "cancel", data: { callId: "7" }, raw: {} }, "poll", now);
    assert.equal(ringing().size, 0);
    // Logged out: declining is refused before anything is sent.
    await assert.rejects(session.declineCall("7"), /Not logged in/);
});
