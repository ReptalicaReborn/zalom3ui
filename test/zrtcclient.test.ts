import assert from "node:assert/strict";
import dgram from "node:dgram";
import { test } from "node:test";
import { SrtpContext } from "../src/srtp.js";
import { parseRtcp } from "../src/rtcp.js";
import * as zrtc from "../src/zrtc.js";
import { MediaCall, OPUS_PT, parseServer } from "../src/zrtcclient.js";

test("media server addresses", () => {
    assert.deepEqual(parseServer("192.0.2.10:4200"), { host: "192.0.2.10", port: 4200 });
    assert.deepEqual(parseServer("2001:db8::7:4200"), { host: "2001:db8::7", port: 4200 });
    assert.equal(parseServer("nope"), null);
});

/**
 * A fake media server on localhost: answers init call with a token, then streams SRTP audio from "the caller", and
 * checks the audio Zalỏ sends back decrypts with the same session key.
 */
test("joins, receives and decrypts the caller's audio, sends its own, hangs up", async () => {
    const session = "TEST-made-up-session-0123456789abcdefghijklmnop";
    const server = dgram.createSocket("udp4");
    await new Promise<void>((r) => server.bind(0, "127.0.0.1", r));
    const port = (server.address() as { port: number }).port;
    const callerSrtp = SrtpContext.fromSession(session);
    const serverSide = SrtpContext.fromSession(session);
    const seen: { inits: Buffer[]; audio: number; audioOk: number; tokens: Set<number>; ends: number; pings: number; rtcp: string[] } = {
        inits: [],
        audio: 0,
        audioOk: 0,
        tokens: new Set(),
        ends: 0,
        pings: 0,
        rtcp: [],
    };
    server.on("message", (msg, from) => {
        if (msg[0] === zrtc.Kind.AudioRtcp) {
            seen.rtcp.push(parseRtcp(msg.subarray(5)).map((r) => r.type).join("+"));
            return;
        }
        const p = msg[0] === zrtc.Kind.Request ? zrtc.parse(msg) : null;
        if (p && p.type === "control") {
            if (p.cmd === zrtc.Cmd.InitCallCallee) {
                seen.inits.push(msg);
                // Response: header, error 0, value38, value18 (the token), our address.
                const reply = Buffer.concat([msg.subarray(0, 21), Buffer.from([0, 0, 0, 0, 1, 0, 0, 0, 0x39, 0x30, 0, 0, 9, 0]), Buffer.from("127.0.0.1")]);
                reply[0] = zrtc.Kind.Response;
                server.send(reply, from.port, from.address);
                // A video packet: a key frame (pt 98, marker) holding one IDR slice in Annex B.
                const vrtp = Buffer.alloc(12);
                vrtp[0] = 0x80;
                vrtp[1] = 0x80 | 98;
                vrtp.writeUInt32BE(0x5151, 8);
                server.send(Buffer.concat([Buffer.from([zrtc.Kind.VideoIn]), callerSrtp.protect(Buffer.concat([vrtp, Buffer.from([0, 0, 0, 1, 0x65, 0x88])]))]), from.port, from.address);
                // Then three packets of the caller's audio.
                for (let i = 0; i < 3; i++) {
                    const rtp = Buffer.alloc(12);
                    rtp[0] = 0x80;
                    rtp[1] = OPUS_PT;
                    rtp.writeUInt16BE(100 + i, 2);
                    rtp.writeUInt32BE(1000 + 320 * i, 4);
                    rtp.writeUInt32BE(0xabcdef, 8);
                    const srtp = callerSrtp.protect(Buffer.concat([rtp, Buffer.from([0x78, 1, 2, 3])]));
                    server.send(Buffer.concat([Buffer.from([zrtc.Kind.AudioIn]), srtp]), from.port, from.address);
                }
            } else if (p.cmd === zrtc.Cmd.Ping) seen.pings++;
            else if (p.cmd === zrtc.Cmd.EndCall) seen.ends++;
        } else if (msg[0] === zrtc.Kind.Audio) {
            seen.audio++;
            seen.tokens.add(msg.readUInt32LE(1));
            if (serverSide.unprotect(msg.subarray(5))) seen.audioOk++;
        }
    });

    const logs: Record<string, unknown>[] = [];
    const heard: [number, number, string][] = [];
    const call = new MediaCall({
        server: { host: "127.0.0.1", port },
        userId: 438000002,
        partnerId: 465000001,
        callId: 321000003,
        session,
        log: (e) => logs.push(e),
        pingMs: 30,
        rtcpMs: 40,
        onAudio: (seq, ts, opus) => heard.push([seq, ts, opus.toString("hex")]),
    });
    assert.equal(await call.join(2000, 100), 12345);
    call.startSilence();
    await new Promise((r) => setTimeout(r, 150));
    call.close();
    await new Promise((r) => setTimeout(r, 100));
    server.close();

    assert.deepEqual(seen.inits[0], zrtc.initCall({ isCaller: false, userId: 438000002, partnerId: 465000001, callId: 321000003, session, attempt: 1 }));
    assert.equal(call.natAddress, "127.0.0.1");
    assert.equal(call.stats.audio.packets, 3);
    assert.equal(call.stats.audio.decrypted, 3);
    assert.deepEqual(call.stats.audio.timestampSteps, [320, 320]);
    assert.deepEqual(call.stats.audio.opusToc, ["78", "78", "78"]);
    const v = call.stats.video;
    assert.deepEqual(
        { packets: v.packets, decrypted: v.decrypted, failed: v.failed, ssrcs: v.ssrcs, payloadTypes: v.payloadTypes, nalTypes: v.nalTypes, frames: v.frames, keys: v.keys },
        { packets: 1, decrypted: 1, failed: 0, ssrcs: 1, payloadTypes: { 98: 1 }, nalTypes: { 5: 1 }, frames: 1, keys: 1 },
        "video is assembled into frames",
    );
    assert.deepEqual(heard, [[100, 1000, "78010203"], [101, 1320, "78010203"], [102, 1640, "78010203"]], "decrypted Opus handed over");
    assert.ok(seen.audio >= 3, "sent audio every 20 ms");
    assert.equal(seen.audioOk, seen.audio, "every packet decrypts with the session key");
    assert.deepEqual([...seen.tokens], [12345], "audio carries the server's token");
    assert.ok(seen.pings >= 1);
    assert.ok(seen.rtcp.length >= 1, "sends RTCP");
    assert.equal(seen.rtcp.at(-1), "SR+other", "a sender report (with a CNAME) once audio went out");
    assert.equal(seen.ends, 1);
    assert.ok(logs.some((l) => l.kind === "media_init_answer" && l.value18 === 12345));
});

test("audio and video under one SSRC (as Zalo's apps send them) both decrypt: each stream has its own SRTP context", async () => {
    const session = "TEST-made-up-session-0123456789abcdefghijklmnop";
    const server = dgram.createSocket("udp4");
    await new Promise<void>((r) => server.bind(0, "127.0.0.1", r));
    const port = (server.address() as { port: number }).port;
    // The caller's two SRTP contexts, same key, same SSRC, sequence numbers far apart.
    const callerAudio = SrtpContext.fromSession(session);
    const callerVideo = SrtpContext.fromSession(session);
    const ssrc = 465000001;
    const rtp = (pt: number, seq: number, ts: number, marker: boolean, payload: Buffer) => {
        const h = Buffer.alloc(12);
        h[0] = 0x80;
        h[1] = pt | (marker ? 0x80 : 0);
        h.writeUInt16BE(seq, 2);
        h.writeUInt32BE(ts, 4);
        h.writeUInt32BE(ssrc, 8);
        return Buffer.concat([h, payload]);
    };
    server.on("message", (msg, from) => {
        if (msg[0] !== zrtc.Kind.Request || msg.readUInt16LE(18) !== zrtc.Cmd.InitCallCallee) return;
        const reply = Buffer.concat([msg.subarray(0, 21), Buffer.from([0, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0])]);
        reply[0] = zrtc.Kind.Response;
        server.send(reply, from.port, from.address);
        for (let i = 0; i < 3; i++) {
            server.send(Buffer.concat([Buffer.from([zrtc.Kind.AudioIn]), callerAudio.protect(rtp(OPUS_PT, 100 + i, 320 * i, false, Buffer.from([0x78, i])))]), from.port, from.address);
            // One key frame per packet: SPS, PPS, IDR in Annex B, payload type 98, marker set.
            const au = Buffer.from([0, 0, 0, 1, 0x67, 0x42, 0, 0x1e, 0, 0, 0, 1, 0x68, 0xce, 0, 0, 0, 1, 0x65, 0x88, i]);
            server.send(Buffer.concat([Buffer.from([zrtc.Kind.VideoIn]), callerVideo.protect(rtp(98, 40000 + i, 3000 * i, true, au))]), from.port, from.address);
        }
    });
    const frames: boolean[] = [];
    const call = new MediaCall({
        server: { host: "127.0.0.1", port },
        userId: 438000002,
        partnerId: ssrc,
        callId: 1,
        session,
        log: () => {},
        onVideo: (f) => frames.push(f.key),
    });
    assert.equal(await call.join(2000, 100), 7);
    await new Promise((r) => setTimeout(r, 150));
    call.close(false);
    server.close();
    assert.equal(call.stats.audio.decrypted, 3, "audio");
    assert.equal(call.stats.video.decrypted, 3, "video");
    assert.ok(frames.length >= 1 && frames.every((k) => k), "key frames came through");
});

test("parseServer rejects malformed IPs and accepts valid IPv4/IPv6 addresses", () => {
    assert.deepEqual(parseServer("[2001:db8::7]:4200"), { host: "2001:db8::7", port: 4200 });
    assert.deepEqual(parseServer("::1:4200"), { host: "::1", port: 4200 });
    assert.equal(parseServer("1.2.3.4.5:4200"), null);
    assert.equal(parseServer(":::4200"), null);
    assert.equal(parseServer("::::4200"), null);
    assert.equal(parseServer("[192.0.2.10:4200"), null);
    assert.equal(parseServer("192.0.2.10]:4200"), null);
    assert.equal(parseServer("192.0.2.10:0"), null);
    assert.equal(parseServer("192.0.2.10:65536"), null);
});

test("UDP source filtering, InitCall userId validation, RTCP token validation, unauthenticated isolation, and stats.received cap", async () => {
    const session = "TEST-made-up-session-0123456789abcdefghijklmnop";
    const { EventEmitter } = await import("node:events");
    let closedCount = 0;
    const fakeSocket = Object.assign(new EventEmitter(), {
        send: () => {},
        close: () => {
            closedCount++;
            if (closedCount > 1) throw new Error("already closed");
        },
    });
    let keyRequests = 0;
    const call = new MediaCall({
        server: { host: "192.0.2.10", port: 4200 },
        userId: 1001,
        partnerId: 2002,
        callId: 3003,
        session,
        log: () => {},
        createSocket: () => fakeSocket as unknown as dgram.Socket,
        onKeyFrameRequest: () => keyRequests++,
    });

    // Outbound SSRC never collides with videoSsrc or partnerId.
    const audioSsrc = (call as unknown as { ssrc: number }).ssrc;
    const videoSsrc = (call as unknown as { videoSsrc: number }).videoSsrc;
    assert.notEqual(audioSsrc, videoSsrc);
    assert.notEqual(audioSsrc, 2002);

    const joinPromise = call.join(500, 200);

    // Build an InitCall response helper.
    const makeInitReply = (userId: number, token: number) => {
        const req = zrtc.initCall({ isCaller: false, userId, partnerId: 2002, callId: 3003, session });
        const reply = Buffer.concat([req.subarray(0, 21), Buffer.alloc(12)]);
        reply[0] = zrtc.Kind.Response;
        reply.writeUInt32LE(0, 21); // error = 0
        reply.writeUInt32LE(1, 25); // value38
        reply.writeUInt32LE(token, 29); // value18
        return reply;
    };

    // Wrong source host suffix (old endsWith bug) and wrong source port are rejected.
    fakeSocket.emit("message", makeInitReply(1001, 111), { address: "10.192.0.2.10", port: 4200 });
    fakeSocket.emit("message", makeInitReply(1001, 111), { address: "192.0.2.10", port: 9999 });
    assert.equal(call.token, null);
    assert.equal(call.stats.received.other_source, 2);

    // Right source, wrong userId on InitCall response is rejected.
    fakeSocket.emit("message", makeInitReply(9999, 222), { address: "192.0.2.10", port: 4200 });
    assert.equal(call.token, null);
    assert.equal(call.stats.received.init_wrong_user, 1);

    // IPv4-mapped IPv6 address + exact port + matching userId succeeds.
    fakeSocket.emit("message", makeInitReply(1001, 777), { address: "::ffff:192.0.2.10", port: 4200 });
    assert.equal(await joinPromise, 777);

    // VideoRtcp with wrong token is rejected; matching token is accepted.
    const { pli } = await import("../src/rtcp.js");
    fakeSocket.emit("message", zrtc.rtcp(888, pli(0x5555, videoSsrc), true), { address: "192.0.2.10", port: 4200 });
    assert.equal(keyRequests, 0);
    assert.equal(call.stats.received.rtcp_wrong_token, 1);
    fakeSocket.emit("message", zrtc.rtcp(777, pli(0x5555, videoSsrc), true), { address: "192.0.2.10", port: 4200 });
    assert.equal(keyRequests, 1);

    // Unauthenticated AudioIn and VideoIn packets must NOT mutate SSRCs, payloadTypes, seq/ts stats, or ReceiveStats.
    const bogusRtp = Buffer.alloc(30);
    bogusRtp[0] = 0x80;
    bogusRtp[1] = OPUS_PT;
    bogusRtp.writeUInt16BE(55, 2);
    bogusRtp.writeUInt32BE(12345, 4);
    bogusRtp.writeUInt32BE(0xdeadbeef, 8);
    fakeSocket.emit("message", Buffer.concat([Buffer.from([zrtc.Kind.AudioIn]), bogusRtp]), { address: "192.0.2.10", port: 4200 });
    fakeSocket.emit("message", Buffer.concat([Buffer.from([zrtc.Kind.VideoIn]), bogusRtp]), { address: "192.0.2.10", port: 4200 });
    assert.equal(call.stats.audio.failed, 1);
    assert.equal(call.stats.audio.ssrcs, 0);
    assert.deepEqual(call.stats.audio.payloadTypes, {});
    assert.equal(call.stats.audio.firstSeq, undefined);
    assert.equal(call.stats.audio.lastSeq, undefined);
    assert.equal(call.stats.video.failed, 1);
    assert.equal(call.stats.video.ssrcs, 0);
    assert.deepEqual(call.stats.video.payloadTypes, {});

    // Unknown received keys are capped at 64, overflowing into "other_cmd".
    for (let k = 100; k < 220; k++) {
        fakeSocket.emit("message", Buffer.from([k, 0, 1]), { address: "192.0.2.10", port: 4200 });
    }
    assert.ok(Object.keys(call.stats.received).length <= 65, "received keys bounded");
    assert.ok((call.stats.received.other_cmd ?? 0) > 0, "overflow bucketed into other_cmd");

    // Pre-close socket to verify close()'s delayed socket.close() try/catch doesn't throw.
    fakeSocket.close();
    call.close(false);
    await new Promise((r) => setTimeout(r, 80));
});

test("close() during join() does not leak timers and startSilence() after close() is a no-op", async () => {
    const session = "TEST-made-up-session-0123456789abcdefghijklmnop";
    const { EventEmitter } = await import("node:events");
    const fakeSocket = Object.assign(new EventEmitter(), {
        send: () => {},
        close: () => {},
    });
    const call = new MediaCall({
        server: { host: "192.0.2.10", port: 4200 },
        userId: 1001,
        partnerId: 2002,
        callId: 3003,
        session,
        log: () => {},
        createSocket: () => fakeSocket as unknown as dgram.Socket,
    });
    const p = call.join(5000, 100);
    call.close(false);
    assert.equal(await p, null);
    call.startSilence();
    assert.equal((call as unknown as { timers: unknown[] }).timers.length, 0);
});
