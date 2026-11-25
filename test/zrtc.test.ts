import assert from "node:assert/strict";
import { test } from "node:test";
import { audio, endCall, initCall, parse, ping } from "../src/zrtc.js";

// Every expected byte string below was produced by running the official Zalo PC call program's own packet functions
// (zrtc::ZRTPPacket::init… then buildPacket) in an arm64 emulator, with marker values 0x11111111, 0x22222222…
const hex = (s: string) => Buffer.from(s.replace(/\s+/g, ""), "hex");
const A = 0x11111111;
const B = 0x22222222;
const C = 0x33333333;

test("init call, as the caller (command 11)", () => {
    // initZRTPPacketRequestInitCall(isCaller=1, userId=A, partnerId=B, callId=C, "SESS", attempt=0x55, "KEY", 0)
    assert.deepEqual(
        initCall({ isCaller: true, userId: A, partnerId: B, callId: C, session: "SESS", attempt: 0x55, extra: "KEY" }),
        hex("01 7e 00000000 00000000 11111111 00000000 0b00 55 33333333 22222222 0400 53455353"),
    );
});

test("init call, as the person called (command 12, with the extra string)", () => {
    assert.deepEqual(
        initCall({ isCaller: false, userId: A, partnerId: B, callId: C, session: "SESS", attempt: 0x55, extra: "KEY" }),
        hex("01 7e 00000000 00000000 11111111 00000000 0c00 55 33333333 22222222 0400 53455353 0300 4b4559"),
    );
    // Zalo PC's first attempt passes an empty extra string.
    assert.deepEqual(
        initCall({ isCaller: false, userId: A, partnerId: B, callId: C, session: "SESS" }).subarray(-8),
        hex("0400 53455353 0000"),
    );
});

test("ping and end call", () => {
    // initZRTPPacketRequestPing(A, B, "PING")
    assert.deepEqual(ping({ userId: A, serverId: B, session: "PING" }), hex("01 01 00000000 00000000 11111111 22222222 0200 01 0400 50494e47"));
    // initZRTPPacketRequestEndCall(userId=A, callId=B, serverId=C, flag)
    assert.deepEqual(endCall({ userId: A, callId: B, serverId: C, flag: true }), hex("01 7e 00000000 00000000 11111111 33333333 0300 01 22222222"));
    assert.deepEqual(endCall({ userId: A, callId: B, serverId: C }), hex("01 7e 00000000 00000000 11111111 33333333 0300 00 22222222"));
});

test("audio to the server: kind 3, token, RTP", () => {
    // initZRTPPacketAudio(1, A, B, 0, 0) builds "03 22222222" around the payload.
    assert.deepEqual(audio(B, Buffer.from("RTP")), hex("03 22222222 525450"));
    assert.throws(() => audio(1, Buffer.alloc(0x579)), /too large/);
});

// Parsed the same way the official program parses them (checked in the emulator with these exact bytes).
const hdr = (kind: number, cmd: number, sub: number) =>
    Buffer.concat([Buffer.from([kind, 0x7e]), hex("a1a1a1a1 b2b2b2b2 c3c3c3c3 d4d4d4d4"), Buffer.from([cmd & 0xff, cmd >> 8, sub])]);
const u32 = (v: number) => {
    const b = Buffer.alloc(4);
    b.writeUInt32LE(v);
    return b;
};
const s16 = (s: string) => Buffer.concat([Buffer.from([s.length, 0]), Buffer.from(s)]);

test("init call response: error, two server numbers, our address", () => {
    const ok = parse(Buffer.concat([hdr(2, 12, 1), u32(0), u32(0x11223344), u32(0x55667788), s16("1.2.3.4:5678")]));
    assert.equal(ok.type, "control");
    if (ok.type !== "control") return;
    assert.deepEqual(
        { error: ok.error, value38: ok.value38, value18: ok.value18, text: ok.text, cmd: ok.cmd, sub: ok.sub, userId: ok.userId },
        { error: 0, value38: 0x11223344, value18: 0x55667788, text: "1.2.3.4:5678", cmd: 12, sub: 1, userId: 0xc3c3c3c3 },
    );
    // What Zalo's media server really sends (33 bytes, no address); Zalo PC's parser takes it as a success.
    const real = parse(Buffer.concat([hex("027e00000000000000006a3d201a000000000c0001"), u32(0), u32(0x11223344), u32(0x55667788)]));
    assert.equal(real.type, "control");
    if (real.type === "control") {
        assert.deepEqual(
            { error: real.error, value38: real.value38, value18: real.value18, text: real.text, userId: real.userId, cmd: real.cmd },
            { error: 0, value38: 0x11223344, value18: 0x55667788, text: undefined, userId: 438320490, cmd: 12 },
        );
    }
    const failed = parse(Buffer.concat([hdr(2, 11, 1), u32(7)]));
    assert.equal(failed.type === "control" && failed.error, 7);
});

test("other responses and server requests", () => {
    const end = parse(Buffer.concat([hdr(2, 3, 0), u32(0)]));
    assert.equal(end.type === "control" && end.error, 0);
    const fwd = parse(Buffer.concat([hdr(1, 32, 7), u32(8), Buffer.from("FWD-JSON")]));
    assert.equal(fwd.type === "control" && fwd.text, "FWD-JSON");
    const fwd16 = parse(Buffer.concat([hdr(1, 32, 12), s16("S16")]));
    assert.equal(fwd16.type === "control" && fwd16.text, "S16");
    const echo = parse(Buffer.concat([hdr(1, 5, 0), s16("ECHO")]));
    assert.equal(echo.type === "control" && echo.text, "ECHO");
    const r50 = parse(Buffer.concat([hdr(2, 50, 0), u32(0), Buffer.from("PAYLOAD50")]));
    assert.equal(r50.type === "control" && r50.payload?.toString(), "PAYLOAD50");
});

test("media from the server, with and without a token", () => {
    assert.deepEqual(parse(Buffer.from("\x04RTPDATA4", "latin1")), { type: "media", kind: 4, payload: Buffer.from("RTPDATA4") });
    assert.deepEqual(parse(Buffer.concat([Buffer.from([5]), u32(0x99887766), Buffer.from("RTPDATA5")])), {
        type: "media",
        kind: 5,
        token: 0x99887766,
        payload: Buffer.from("RTPDATA5"),
    });
    const p2p = parse(Buffer.concat([Buffer.from([0x7f, 0, 1]), u32(0x12345678), Buffer.from([7, 0]), Buffer.from("P2PMEDIA")]));
    assert.deepEqual(p2p, { type: "p2p", cmd: 7, from: 0x12345678, flag1: 0, flag2: 1, payload: Buffer.from("P2PMEDIA"), rest: Buffer.alloc(0) });
    assert.throws(() => parse(Buffer.from([2, 0x7e, 1])), /cut short/);
});

test("Writer.str16 throws RangeError when string exceeds 65535 bytes", () => {
    assert.throws(() => ping({ userId: A, serverId: B, session: "a".repeat(0x10000) }), RangeError);
    assert.doesNotThrow(() => ping({ userId: A, serverId: B, session: "a".repeat(0xffff) }));
});
