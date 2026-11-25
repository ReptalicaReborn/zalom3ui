import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { incomingCallOf, parseJsonKeepingBigInts } from "../src/calls.js";
import { ZaloSession } from "../src/session.js";
import { ChatStore } from "../src/store.js";
import { DEFAULT_VOICE_CALL_DOMAIN, declineCall, ringingCallOf, voiceCallDomain } from "../src/voicecall.js";

test("the voice call service comes from the login's service map, with Zalo PC's default", () => {
    assert.equal(voiceCallDomain({ voice_call: ["https://voicecall-wpa.chat.zalo.me/"] }), "https://voicecall-wpa.chat.zalo.me");
    assert.equal(voiceCallDomain({ voice_call: "https://vc.example" }), "https://vc.example");
    assert.equal(voiceCallDomain({}), DEFAULT_VOICE_CALL_DOMAIN);
    assert.equal(voiceCallDomain(undefined), DEFAULT_VOICE_CALL_DOMAIN);
});

test("declining sends Zalo PC's cancel request, echoing the call id and type as Zalo sent them", async () => {
    // Written as text: a JS number literal would already round the id.
    const data = parseJsonKeepingBigInts('{"callerId":1234567890123456789,"callId":987654,"callType":0}') as Record<string, unknown>;
    const control = { act: "request", data, raw: {} };
    const incoming = incomingCallOf(control, 1000)!;
    const call = ringingCallOf(incoming, control, 1000);
    const sent: [string, Record<string, unknown>][] = [];
    await declineCall(async (endpoint, params) => {
        sent.push([endpoint, params]);
        return {};
    }, call, "imei-1");
    assert.deepEqual(sent, [["cancel", { callerId: "1234567890123456789", callId: 987654, callType: 0, status: 0, imei: "imei-1" }]]);
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
