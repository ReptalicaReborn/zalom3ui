import assert from "node:assert/strict";
import dgram from "node:dgram";
import { test } from "node:test";
import { SrtpContext } from "../src/srtp.js";
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
    const seen: { inits: Buffer[]; audio: number; audioOk: number; tokens: Set<number>; ends: number; pings: number } = {
        inits: [],
        audio: 0,
        audioOk: 0,
        tokens: new Set(),
        ends: 0,
        pings: 0,
    };
    server.on("message", (msg, from) => {
        const p = zrtc.parse(msg.subarray(0, 1)[0] === zrtc.Kind.Audio ? Buffer.from([zrtc.Kind.AudioInToken, ...msg.subarray(1)]) : msg);
        if (msg[0] === zrtc.Kind.Request && p.type === "control") {
            if (p.cmd === zrtc.Cmd.InitCallCallee) {
                seen.inits.push(msg);
                // Response: header, error 0, value38, value18 (the token), our address.
                const reply = Buffer.concat([msg.subarray(0, 21), Buffer.from([0, 0, 0, 0, 1, 0, 0, 0, 0x39, 0x30, 0, 0, 9, 0]), Buffer.from("127.0.0.1")]);
                reply[0] = zrtc.Kind.Response;
                server.send(reply, from.port, from.address);
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
    assert.deepEqual(heard, [[100, 1000, "78010203"], [101, 1320, "78010203"], [102, 1640, "78010203"]], "decrypted Opus handed over");
    assert.ok(seen.audio >= 3, "sent audio every 20 ms");
    assert.equal(seen.audioOk, seen.audio, "every packet decrypts with the session key");
    assert.deepEqual([...seen.tokens], [12345], "audio carries the server's token");
    assert.ok(seen.pings >= 1);
    assert.equal(seen.ends, 1);
    assert.ok(logs.some((l) => l.kind === "media_init_answer" && l.value18 === 12345));
});
