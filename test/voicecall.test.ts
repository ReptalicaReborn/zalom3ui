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

test("declining sends what Zalo PC's call program sends: noised caller id, numeric call id, call type 1/3", async () => {
    // The shape of a real 1:1 request (values made up): 32-bit zrtc ids, the noised user id, ids as text.
    const request = (callType: number) =>
        parseJsonKeepingBigInts(
            JSON.stringify({
                uidFrom: "465000001",
                uidN: "4459000000000000001",
                uidTo: "438000002",
                callId: "321000003",
                status: "0",
                ts: "1795617640181",
                params: JSON.stringify({ id: 321000003, Dname: "Bảo", extendData: JSON.stringify({ callType }) }),
            }),
        ) as Record<string, unknown>;
    for (const [callType, expected] of [[0, 1], [1, 3]]) {
        const control = { act: "request", data: request(callType), raw: {} };
        const incoming = incomingCallOf(control, 1795617640181)!;
        assert.equal(incoming.callerId, "4459000000000000001");
        assert.equal(incoming.video, callType === 1);
        assert.equal(incoming.displayName, "Bảo");
        const sent: [string, Record<string, unknown>][] = [];
        await declineCall(async (endpoint, params) => {
            sent.push([endpoint, params]);
            return {};
        }, ringingCallOf(incoming, control, 1795617640181), "imei-1");
        assert.deepEqual(sent, [["cancel", { callerId: "4459000000000000001", callId: 321000003, callType: expected, status: 0, imei: "imei-1" }]]);
        sent.length = 0;
        await ringRing(async (endpoint, params) => void sent.push([endpoint, params]), ringingCallOf(incoming, control, 1795617640181), "imei-1");
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
