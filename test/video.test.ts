import assert from "node:assert/strict";
import dgram from "node:dgram";
import { test } from "node:test";
import { fir, parseRtcp, pli } from "../src/rtcp.js";
import { decodeVideoFromApp, encodeVideoToApp, isKeyFrameWanted, KEY_FRAME_REQUEST } from "../src/server.js";
import { rtpHeader, SrtpContext } from "../src/srtp.js";
import { MAX_CHUNK, VideoAssembler, nalUnits, packetizeFrame, type VideoFrame, type VideoPacket } from "../src/video.js";
import * as zrtc from "../src/zrtc.js";
import { MediaCall } from "../src/zrtcclient.js";

const SC = Buffer.from([0, 0, 0, 1]);
const SPS = Buffer.from([0x67, 0x42, 0x00, 0x1f]);
const PPS = Buffer.from([0x68, 0xce, 0x3c, 0x80]);
const idr = (n = 6) => Buffer.concat([SC, Buffer.from([0x65, 0x88, ...Array(n).fill(0x84)])]);
const slice = (n = 6) => Buffer.concat([SC, Buffer.from([0x41, 0x9a, ...Array(n).fill(0x22)])]);
const keyFrame = () => Buffer.concat([SC, SPS, SC, PPS, idr()]);

/** Packets of one frame, the way Zalo sends them. */
function packets(frame: Buffer, key: boolean, seq: number, timestamp: number): VideoPacket[] {
    const chunks = packetizeFrame(frame);
    return chunks.map((payload, i) => ({ seq: (seq + i) & 0xffff, timestamp, payloadType: key ? 98 : 97, marker: i === chunks.length - 1, payload }));
}

function assembler() {
    const out: VideoFrame[] = [];
    const asks: string[] = [];
    const clock = { now: 1000 };
    const a = new VideoAssembler((f) => out.push(f), () => clock.now, (why) => asks.push(why));
    return { a, out, asks, clock };
}

test("packetizer: a small frame is one payload, a large one is cut into 1c chunks", () => {
    const small = Buffer.from("0000000165888000", "hex");
    assert.deepEqual(packetizeFrame(small), [small]);
    assert.equal(packetizeFrame(Buffer.alloc(MAX_CHUNK, 1)).length, 1, "exactly 1068 bytes still fits");

    const au = Buffer.from(Array.from({ length: 2500 }, (_, i) => i & 0xff));
    const chunks = packetizeFrame(au);
    assert.deepEqual(chunks.map((c) => c.length), [1070, 1070, 366]);
    assert.equal(chunks[0].subarray(0, 2).toString("hex"), "1c80");
    assert.equal(chunks[1].subarray(0, 2).toString("hex"), "1c00");
    assert.equal(chunks[2].subarray(0, 2).toString("hex"), "1c40");
    assert.deepEqual(Buffer.concat(chunks.map((c) => c.subarray(2))), au);
    assert.equal(chunks[1].subarray(2, 6).toString("hex"), au.subarray(1068, 1072).toString("hex"));
});

test("assembler: a single-packet key frame and a chunked delta frame", () => {
    const { a, out, asks } = assembler();
    const big = Buffer.concat([slice(0), Buffer.alloc(2500, 0x33)]);
    for (const p of packets(keyFrame(), true, 100, 9000)) a.push(p);
    for (const p of packets(big, false, 101, 12000)) a.push(p);
    assert.equal(out.length, 2);
    assert.deepEqual(out[0], { key: true, timestamp: 9000, data: keyFrame() });
    assert.equal(out[1].key, false);
    assert.equal(out[1].timestamp, 12000);
    assert.deepEqual(out[1].data, big);
    assert.deepEqual(asks, []);
    assert.deepEqual(a.stats, { frames: 2, keys: 1, gaps: 0, dropped: 0 });
});

test("assembler: sequence numbers wrap at 16 bits", () => {
    const { a, out } = assembler();
    for (const p of packets(keyFrame(), true, 65535, 1000)) a.push(p);
    for (const p of packets(slice(), false, 0, 4000)) a.push(p);
    // A three-packet frame straddling the wrap.
    for (const p of packets(Buffer.concat([slice(0), Buffer.alloc(2300, 7)]), false, 1, 7000)) a.push(p);
    assert.deepEqual(out.map((f) => f.timestamp), [1000, 4000, 7000]);
    assert.equal(a.stats.gaps, 0);
    const wrap = assembler();
    const first = packets(Buffer.concat([keyFrame(), Buffer.alloc(2200, 5)]), true, 65534, 500);
    assert.equal(first.length, 3);
    first.forEach((p) => wrap.a.push(p));
    assert.deepEqual(first.map((p) => p.seq), [65534, 65535, 0]);
    assert.equal(wrap.out.length, 1, "a frame that spans the wrap completes");
});

test("assembler: packets and frames arriving out of order are put right", () => {
    const { a, out } = assembler();
    for (const p of packets(keyFrame(), true, 9, 500)) a.push(p);
    const f1 = packets(Buffer.concat([slice(0), Buffer.alloc(2200, 5)]), false, 10, 1000);
    const f2 = packets(slice(), false, 13, 4000);
    a.push(f1[2]);
    a.push(f1[0]);
    a.push(f2[0]); // the next frame is complete before the first one is
    assert.equal(out.length, 1, "waits for the earlier frame");
    a.push(f1[1]);
    assert.deepEqual(out.map((f) => f.timestamp), [500, 1000, 4000]);
    assert.equal(a.stats.gaps, 0);
});

test("assembler: a lost frame is a gap, asks for a key frame, and delta frames wait for it", () => {
    const { a, out, asks, clock } = assembler();
    for (const p of packets(keyFrame(), true, 0, 1000)) a.push(p);
    // Frame at sequence 1 is lost; 2 arrives.
    for (const p of packets(slice(), false, 2, 7000)) a.push(p);
    assert.equal(out.length, 1, "still inside the reorder window");
    clock.now += 250;
    a.tick();
    assert.equal(a.stats.gaps, 1);
    assert.deepEqual(asks.slice(0, 1), ["gap"]);
    assert.equal(out.length, 1, "the delta frame after the loss is not delivered");
    assert.ok(a.waitingForKey);
    for (const p of packets(slice(), false, 3, 10000)) a.push(p);
    assert.equal(out.length, 1);
    for (const p of packets(keyFrame(), true, 4, 13000)) a.push(p);
    assert.deepEqual(out.map((f) => f.key), [true, true]);
    assert.ok(!a.waitingForKey);
    assert.equal(a.stats.dropped, 2);
});

test("assembler: a gap also shows when three frames are waiting", () => {
    const { a, out, asks } = assembler();
    for (const p of packets(keyFrame(), true, 0, 1000)) a.push(p);
    // Frames at 2, 3, 4, 5 arrive, 1 never does.
    [2, 3, 4, 5].forEach((seq, i) => packets(slice(), false, seq, 2000 + i * 1000).forEach((p) => a.push(p)));
    assert.equal(a.stats.gaps, 1);
    assert.ok(asks.includes("gap"));
    assert.equal(out.length, 1);
});

test("assembler: nothing is delivered before the first key frame", () => {
    const { a, out, asks } = assembler();
    for (const p of packets(slice(), false, 50, 1000)) a.push(p);
    for (const p of packets(slice(), false, 51, 2000)) a.push(p);
    assert.equal(out.length, 0);
    assert.equal(a.stats.dropped, 2);
    assert.ok(asks.includes("no_key"));
    assert.ok(a.waitingForKey);
    for (const p of packets(keyFrame(), true, 52, 3000)) a.push(p);
    for (const p of packets(slice(), false, 53, 4000)) a.push(p);
    assert.deepEqual(out.map((f) => f.key), [true, false]);
    assert.equal(a.stats.gaps, 0);
});

test("assembler: SPS/PPS are put in front of a key frame that lacks them", () => {
    const { a, out } = assembler();
    for (const p of packets(keyFrame(), true, 0, 1000)) a.push(p);
    for (const p of packets(idr(), true, 1, 2000)) a.push(p);
    assert.deepEqual(out[1].data, Buffer.concat([SC, SPS, SC, PPS, idr()]));
    // Only the PPS missing.
    for (const p of packets(Buffer.concat([SC, SPS, idr()]), true, 2, 3000)) a.push(p);
    assert.deepEqual(out[2].data, Buffer.concat([SC, PPS, SC, SPS, idr()]));
    assert.deepEqual(nalUnits(out[2].data).map((u) => u[0] & 0x1f), [8, 7, 5]);
    // Delta frames are left alone.
    for (const p of packets(slice(), false, 3, 4000)) a.push(p);
    assert.deepEqual(out[3].data, slice());
});

test("RTCP PLI and FIR: bytes and parsing", () => {
    assert.equal(pli(1, 2).toString("hex"), "81ce000200000001" + "00000002");
    assert.equal(fir(1, 2, 7).toString("hex"), "84ce000400000001" + "00000000" + "00000002" + "07000000");
    assert.deepEqual(parseRtcp(pli(0xaabbccdd, 5)), [{ type: "PLI", ssrc: 0xaabbccdd, mediaSsrc: 5 }]);
    assert.deepEqual(parseRtcp(Buffer.concat([fir(9, 77, 1), pli(9, 5)])), [
        { type: "FIR", ssrc: 9, mediaSsrc: 77 },
        { type: "PLI", ssrc: 9, mediaSsrc: 5 },
    ]);
});

test("video messages between server and app (docs/call-audio.md §3)", () => {
    assert.equal(encodeVideoToApp(true, 0x12345678, Buffer.from([0, 0, 0, 1, 0x65])).toString("hex"), "030112345678" + "0000000165");
    assert.equal(encodeVideoToApp(false, 2 ** 32 + 9, Buffer.from([1])).toString("hex"), "03000000000901", "timestamp wraps like RTP");
    assert.equal(KEY_FRAME_REQUEST.toString("hex"), "05");
    assert.deepEqual(decodeVideoFromApp(Buffer.from("040100000001" + "65", "hex")), { key: true, data: Buffer.from("0000000165", "hex") });
    assert.deepEqual(decodeVideoFromApp(Buffer.from("0400" + "41", "hex")), { key: false, data: Buffer.from("41", "hex") });
    assert.equal(decodeVideoFromApp(Buffer.from([4, 1])), null, "no access unit");
    assert.equal(decodeVideoFromApp(Buffer.from([2, 1, 2, 3])), null, "audio, not video");
    assert.equal(decodeVideoFromApp(Buffer.alloc(600 * 1024, 4)), null, "too large");
    assert.equal(isKeyFrameWanted(Buffer.from([6])), true);
    assert.equal(isKeyFrameWanted(Buffer.from([4, 1, 2])), false);
});

/**
 * A fake media server: gives the call its token, then plays "the caller": a delta frame first (no key frame yet, so
 * a PLI must come back), then a chunked key frame, and later a PLI aimed at our video stream. Everything we send is
 * collected and decrypted with the same session key.
 */
test("video both ways against a fake media server, with PLI each way", async () => {
    const session = "TEST-made-up-session-0123456789abcdefghijklmnop";
    const server = dgram.createSocket("udp4");
    await new Promise<void>((r) => server.bind(0, "127.0.0.1", r));
    const port = (server.address() as { port: number }).port;
    const caller = SrtpContext.fromSession(session);
    const reader = SrtpContext.fromSession(session);
    const CALLER_SSRC = 0x7777;
    const TOKEN = 12345;
    const seen = { video: [] as Buffer[], rtcp: [] as Buffer[], audioRtcp: [] as Buffer[], tokens: new Set<number>() };
    let peer: { port: number; address: string } | null = null;
    const toApp = (frame: Buffer, key: boolean, seq: number, ts: number) => {
        const chunks = packetizeFrame(frame);
        chunks.forEach((payload, i) => {
            const rtp = Buffer.alloc(12);
            rtp[0] = 0x80;
            rtp[1] = (key ? 98 : 97) | (i === chunks.length - 1 ? 0x80 : 0);
            rtp.writeUInt16BE(seq + i, 2);
            rtp.writeUInt32BE(ts, 4);
            rtp.writeUInt32BE(CALLER_SSRC, 8);
            server.send(Buffer.concat([Buffer.from([zrtc.Kind.VideoIn]), caller.protect(Buffer.concat([rtp, payload]))]), peer!.port, peer!.address);
        });
    };
    server.on("message", (msg, from) => {
        peer = from;
        if (msg[0] === zrtc.Kind.Request) {
            if (zrtc.parse(msg).type === "control" && msg.readUInt16LE(18) === zrtc.Cmd.InitCallCallee) {
                const reply = Buffer.concat([msg.subarray(0, 21), Buffer.from([0, 0, 0, 0, 1, 0, 0, 0, 0x39, 0x30, 0, 0, 9, 0]), Buffer.from("127.0.0.1")]);
                reply[0] = zrtc.Kind.Response;
                server.send(reply, from.port, from.address);
                toApp(slice(), false, 40, 1000);
            }
        } else if (msg[0] === zrtc.Kind.Video) {
            seen.tokens.add(msg.readUInt32LE(1));
            const rtp = reader.unprotect(msg.subarray(5));
            if (rtp) seen.video.push(rtp);
        } else if (msg[0] === zrtc.Kind.VideoRtcp) seen.rtcp.push(msg.subarray(5));
        else if (msg[0] === zrtc.Kind.AudioRtcp) seen.audioRtcp.push(msg.subarray(5));
    });

    const frames: VideoFrame[] = [];
    let keyRequests = 0;
    const call = new MediaCall({
        server: { host: "127.0.0.1", port },
        userId: 438000002,
        partnerId: 465000001,
        callId: 321000003,
        session,
        log: () => {},
        pingMs: 1000,
        rtcpMs: 60,
        onVideo: (f) => frames.push(f),
        onKeyFrameRequest: () => keyRequests++,
    });
    assert.equal(await call.join(2000, 100), TOKEN);
    await new Promise((r) => setTimeout(r, 450));

    // The delta frame had no key frame before it: dropped, and a PLI for the caller's SSRC went out on kind 15.
    assert.equal(frames.length, 0);
    const first = seen.rtcp.flatMap((b) => parseRtcp(b)).filter((p) => p.type === "PLI");
    assert.ok(first.length >= 1 && first.length <= 2, `PLI is rate limited (${first.length})`);
    assert.equal(first[0].type === "PLI" && first[0].mediaSsrc, CALLER_SSRC);

    // The caller's chunked key frame (SPS + PPS + a 2500-byte IDR) arrives whole.
    const bigKey = Buffer.concat([SC, SPS, SC, PPS, SC, Buffer.from([0x65]), Buffer.alloc(2500, 0x55)]);
    toApp(bigKey, true, 41, 4000);
    await new Promise((r) => setTimeout(r, 60));
    assert.equal(frames.length, 1);
    assert.deepEqual(frames[0], { key: true, timestamp: 4000, data: bigKey });
    assert.equal(call.stats.video.keys, 1);
    assert.equal(call.stats.video.dropped, 1);
    assert.deepEqual(call.stats.video.payloadTypes, { 97: 1, 98: 3 });
    assert.deepEqual(call.stats.video.nalTypes, { 7: 1, 8: 1, 5: 1 });

    // Our frames: a small delta frame and a 2500-byte key frame, chunked, protected, on kind 13 with the token.
    const myKey = Buffer.concat([SC, SPS, SC, PPS, SC, Buffer.from([0x65]), Buffer.alloc(2400, 0x66)]);
    call.sendVideoFrame({ key: true, data: myKey });
    call.sendVideoFrame({ key: false, data: slice() });
    await new Promise((r) => setTimeout(r, 80));
    assert.deepEqual([...seen.tokens], [TOKEN]);
    const heads = seen.video.map((r) => rtpHeader(r)!);
    assert.equal(heads.length, 4, "3 chunks + 1 packet");
    assert.deepEqual(heads.map((h) => h.payloadType), [98, 98, 98, 97]);
    assert.deepEqual(heads.map((h) => h.marker), [false, false, true, true]);
    assert.equal(new Set(heads.map((h) => h.ssrc)).size, 1);
    const mySsrc = heads[0].ssrc;
    assert.ok(mySsrc !== 0x7777);
    assert.deepEqual(heads.map((h) => (h.seq - heads[0].seq) & 0xffff), [0, 1, 2, 3]);
    assert.equal(heads[0].timestamp, heads[1].timestamp);
    assert.equal(heads[1].timestamp, heads[2].timestamp);
    assert.ok(((heads[3].timestamp - heads[0].timestamp) >>> 0) < 90_000, "90 kHz clock moves forward by a small step");
    // Put our own packets back together with the assembler.
    const mine: VideoFrame[] = [];
    const back = new VideoAssembler((f) => mine.push(f));
    seen.video.forEach((r, i) =>
        back.push({ seq: heads[i].seq, timestamp: heads[i].timestamp, payloadType: heads[i].payloadType, marker: heads[i].marker, payload: r.subarray(heads[i].headerLength) }),
    );
    assert.deepEqual(mine.map((f) => [f.key, f.data.length]), [[true, myKey.length], [false, slice().length]]);
    assert.deepEqual(mine[0].data, myKey);

    // Video reports go out on kind 15 (a sender report now that we've sent video), audio's stay on kind 5.
    await new Promise((r) => setTimeout(r, 100));
    const types = seen.rtcp.map((b) => parseRtcp(b).map((p) => p.type).join("+"));
    assert.ok(types.includes("SR+other"), `video sender report with CNAME (${types.join(",")})`);
    assert.ok(seen.audioRtcp.length >= 1);

    // The caller asks for a key frame from us (PLI, then FIR): the app is told (rate limited).
    server.send(zrtc.rtcp(TOKEN, pli(CALLER_SSRC, mySsrc), true), peer!.port, peer!.address);
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(keyRequests, 1);
    await new Promise((r) => setTimeout(r, 300));
    server.send(zrtc.rtcp(TOKEN, fir(CALLER_SSRC, mySsrc, 1), true), peer!.port, peer!.address);
    server.send(zrtc.rtcp(TOKEN, pli(CALLER_SSRC, 0x1234), true), peer!.port, peer!.address);
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(keyRequests, 2, "FIR counts, a PLI for some other stream doesn't");

    // The app asks for a key frame: one more PLI, but never closer than 500 ms to the last.
    const plisBefore = call.stats.video.plisSent;
    call.requestKeyFrame("app");
    call.requestKeyFrame("app");
    assert.ok(call.stats.video.plisSent - plisBefore <= 1);

    call.close();
    await new Promise((r) => setTimeout(r, 100));
    server.close();
});

test("packetizeFrame throws RangeError on non-positive maxChunk", () => {
    assert.throws(() => packetizeFrame(keyFrame(), 0), RangeError);
    assert.throws(() => packetizeFrame(keyFrame(), -10), RangeError);
    assert.throws(() => packetizeFrame(keyFrame(), Number.NaN), RangeError);
});

test("assembler: drops pending frame exceeding part count or byte cap", () => {
    const { a, out } = assembler();
    // Push 513 parts of one frame (exceeds MAX_FRAME_PARTS = 512).
    for (let i = 0; i < 513; i++) {
        a.push({ seq: i, timestamp: 1000, payloadType: 98, marker: false, payload: Buffer.from([0x1c, i === 0 ? 0x80 : 0x00, 1, 2]) });
    }
    assert.equal(out.length, 0);
    assert.equal(a.stats.dropped, 1);

    // Push packets exceeding MAX_FRAME_BYTES (512 * 1024).
    const { a: a2, out: out2 } = assembler();
    const bigChunk = Buffer.alloc(64 * 1024, 0xaa);
    for (let i = 0; i < 9; i++) {
        a2.push({ seq: i, timestamp: 2000, payloadType: 98, marker: false, payload: bigChunk });
    }
    assert.equal(out2.length, 0);
    assert.equal(a2.stats.dropped, 1);
});

test("assembler: startup wraparound near seq 0 handles pre-wrap straggler without jumping maxIdx", () => {
    const { a, out } = assembler();
    // First packet arrives at seq 2 (keyframe).
    for (const p of packets(keyFrame(), true, 2, 1000)) a.push(p);
    // Pre-wrap straggler at seq 65535 arrives — must be treated as before seq 2, not jumping maxIdx to 65535.
    for (const p of packets(slice(), false, 65535, 500)) a.push(p);
    // Next frames at seq 3 and 4 continue seamlessly with zero gaps.
    for (const p of packets(slice(), false, 3, 4000)) a.push(p);
    for (const p of packets(slice(), false, 4, 7000)) a.push(p);
    assert.deepEqual(out.map((f) => f.timestamp), [1000, 4000, 7000]);
    assert.equal(a.stats.gaps, 0);

    // Out-of-order startup keyframe straddling 65534, 65535, 0 where seq 0 arrives first.
    const { a: a2, out: out2 } = assembler();
    const kf = packets(Buffer.concat([keyFrame(), Buffer.alloc(2200, 5)]), true, 65534, 500);
    assert.deepEqual(kf.map((p) => p.seq), [65534, 65535, 0]);
    a2.push(kf[2]);
    a2.push(kf[0]);
    a2.push(kf[1]);
    assert.equal(out2.length, 1);
    assert.equal(out2[0].key, true);
});

test("assembler: extra packet with idx > f.end does not wedge frame completion", () => {
    const { a, out } = assembler();
    const kf = packets(Buffer.concat([keyFrame(), Buffer.alloc(1500, 9)]), true, 10, 1000);
    assert.equal(kf.length, 2); // seq 10, 11 (marker)
    // Marker arrives first (seq 11), then bogus packet at seq 12 with same timestamp, then seq 10.
    a.push(kf[1]);
    a.push({ seq: 12, timestamp: 1000, payloadType: 98, marker: false, payload: Buffer.from([0x1c, 0x00, 0xff]) });
    a.push(kf[0]);
    assert.equal(out.length, 1, "frame completes despite bogus packet after marker");
});

test("assembler: startup delta waits for in-flight keyframe within reorder window, drops after timeout", () => {
    const { a, out, clock } = assembler();
    const kf = packets(Buffer.concat([keyFrame(), Buffer.alloc(1500, 4)]), true, 10, 1000);
    assert.equal(kf.length, 2); // seq 10, 11
    // First packet of keyframe arrives (in flight).
    a.push(kf[0]);
    // Following delta frame at seq 12 arrives early.
    for (const p of packets(slice(), false, 12, 4000)) a.push(p);
    assert.equal(a.stats.dropped, 0, "delta waits while earlier keyframe is in flight");
    assert.equal(out.length, 0);
    // Remaining packet of keyframe arrives within reorder window.
    a.push(kf[1]);
    assert.deepEqual(out.map((f) => [f.key, f.timestamp]), [[true, 1000], [false, 4000]]);
    assert.equal(a.stats.dropped, 0);
    assert.equal(a.stats.gaps, 0);

    // If the in-flight keyframe never finishes, timeout drops the waiting delta.
    const { a: a2, out: out2, asks: asks2, clock: clock2 } = assembler();
    a2.push(kf[0]);
    for (const p of packets(slice(), false, 12, 4000)) a2.push(p);
    assert.equal(a2.stats.dropped, 0);
    clock2.now += 250;
    a2.tick();
    assert.equal(out2.length, 0);
    assert.equal(a2.stats.dropped, 1);
    assert.ok(asks2.includes("no_key"));
});

test("assembler: cached SPS/PPS are copied and do not pin the full keyframe buffer", () => {
    const { a, out } = assembler();
    const hugeKey = Buffer.concat([SC, SPS, SC, PPS, SC, Buffer.from([0x65]), Buffer.alloc(20_000, 0x55)]);
    for (const p of packets(hugeKey, true, 1, 1000)) a.push(p);
    const sps = (a as unknown as { sps: Buffer }).sps;
    const pps = (a as unknown as { pps: Buffer }).pps;
    assert.notEqual(sps.buffer, out[0].data.buffer, "SPS is copied off the keyframe buffer");
    assert.notEqual(pps.buffer, out[0].data.buffer, "PPS is copied off the keyframe buffer");
    assert.ok(sps.buffer.byteLength < 20_000, "SPS does not retain 20 KB keyframe slab");
    assert.ok(pps.buffer.byteLength < 20_000, "PPS does not retain 20 KB keyframe slab");
});
