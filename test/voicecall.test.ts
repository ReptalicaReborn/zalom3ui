import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { incomingCallOf, parseJsonKeepingBigInts } from "../src/calls.js";
import { ZaloSession } from "../src/session.js";
import { ChatStore } from "../src/store.js";
import { ANSWER_CODEC, DEFAULT_VOICE_CALL_DOMAIN, acceptCall, declineCall, hangUp, mediaParamsOf, ourExtendData, ringRing, ringingCallOf, voiceCallDomain } from "../src/voicecall.js";

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
        assert.deepEqual(ours.video, { codec: [{ name: "h264", payload: 97 }] }, "always, as the official apps send it");
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

test("picking up: media parameters from the request, answer status 0 with our codec, hang up", async () => {
    const data = parseJsonKeepingBigInts(
        JSON.stringify({
            uidFrom: "465000001",
            uidN: "4459000000000000001",
            uidTo: "438000002",
            callId: "321000003",
            ts: "1795617640181",
            session: "SESSION-made-up-0123456789abcdefghij",
            rtpAddress: "192.0.2.10:4200",
            params: JSON.stringify({ userId: 438000002, extendData: JSON.stringify({ callType: 0 }) }),
        }),
    ) as Record<string, unknown>;
    const control = { act: "request", data, raw: {} };
    const call = ringingCallOf(incomingCallOf(control, 1795617640181)!, control, 1795617640181);
    assert.deepEqual(mediaParamsOf(call), { server: "192.0.2.10:4200", userId: 438000002, partnerId: 465000001, callId: 321000003, session: "SESSION-made-up-0123456789abcdefghij" });
    const sent: [string, Record<string, unknown>][] = [];
    const send = async (endpoint: string, params: Record<string, unknown>) => void sent.push([endpoint, params]);
    await acceptCall(send, call, "192.0.2.10:4200");
    await hangUp(send, call, "imei-1");
    assert.equal(sent[0][0], "answer");
    assert.deepEqual({ ...sent[0][1], extendData: undefined }, {
        callerId: "4459000000000000001",
        callId: 321000003,
        status: 0,
        codec: ANSWER_CODEC,
        rtcpAddress: "192.0.2.10:4200",
        rtpAddress: "192.0.2.10:4200",
        session_id: "SESSION-made-up-0123456789abcdefghij",
        extendData: undefined,
        subCommand: 3,
    });
    assert.deepEqual(sent[1], ["endcall", { uidTo: "4459000000000000001", callId: 321000003, status: 3, imei: "imei-1" }]);
    assert.equal(mediaParamsOf({ ...call, data: {} }), null, "nothing to join without a server and session");
});

test("voiceCallDomain rejects plain HTTP and falls back to default", () => {
    assert.equal(voiceCallDomain({ voice_call: "http://insecure.example" }), DEFAULT_VOICE_CALL_DOMAIN);
    assert.equal(voiceCallDomain({ voice_call: ["http://insecure.example/"] }), DEFAULT_VOICE_CALL_DOMAIN);
});

test("ourExtendData always advertises only supported SRTP settings (srtpMode 1, srtcp 0)", () => {
    const control = {
        act: "request",
        data: {
            uidFrom: "465000001",
            uidN: "4459000000000000001",
            uidTo: "438000002",
            callId: "321000003",
            params: JSON.stringify({
                extendData: JSON.stringify({ callType: 0, srtpMode: 2, srtcp: 1, packetMode: 2 }),
            }),
        },
        raw: {},
    };
    const call = ringingCallOf(incomingCallOf(control)!, control);
    const ext = ourExtendData(call);
    assert.equal(ext.srtpMode, 1);
    assert.equal(ext.srtcp, 0);
});

test("mediaParamsOf validates >=30-byte session and supports zrtc_config.srtpKey fallback", () => {
    const makeCall = (data: Record<string, unknown>) => {
        const control = { act: "request", data, raw: {} };
        return ringingCallOf(incomingCallOf(control)!, control);
    };
    const base = {
        uidFrom: "465000001",
        uidN: "4459000000000000001",
        uidTo: "438000002",
        callId: "321000003",
        rtpAddress: "192.0.2.10:4200",
    };

    // Short/corrupt session (< 30 bytes) returns null instead of throwing later in SrtpContext.fromSession.
    assert.equal(mediaParamsOf(makeCall({ ...base, session: "too-short-session" })), null);

    // Fallback to zrtc_config.srtpKey (raw >= 30-byte string) when session/sessId are absent.
    const rawKey = "0123456789abcdef0123456789abcd";
    const fromRaw = mediaParamsOf(makeCall({ ...base, params: JSON.stringify({ userId: 438000002, zrtc_config: { srtpKey: rawKey } }) }));
    assert.deepEqual(fromRaw, {
        server: "192.0.2.10:4200",
        userId: 438000002,
        partnerId: 465000001,
        callId: 321000003,
        session: rawKey,
        srtpKey: rawKey,
    });

    // Fallback to base64 zrtc_config.srtpKey inside stringified zrtc_config.
    const b64Key = Buffer.alloc(30, 0x42).toString("base64");
    const fromB64 = mediaParamsOf(makeCall({ ...base, params: JSON.stringify({ userId: 438000002, zrtc_config: JSON.stringify({ srtpKey: b64Key }) }) }));
    assert.equal(fromB64?.session, b64Key);
    assert.equal(fromB64?.srtpKey, b64Key);

    // Short zrtc_config.srtpKey is rejected when no valid session exists.
    assert.equal(mediaParamsOf(makeCall({ ...base, params: JSON.stringify({ userId: 438000002, zrtc_config: { srtpKey: "short" } }) })), null);
});

