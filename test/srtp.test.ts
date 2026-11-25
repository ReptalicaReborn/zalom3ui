import assert from "node:assert/strict";
import { test } from "node:test";
import { SrtpContext, deriveKey, rtpHeader } from "../src/srtp.js";

const hex = (s: string) => Buffer.from(s, "hex");

test("key derivation matches RFC 3711 appendix B.3", () => {
    const key = hex("E1F97A0D3E018BE0D64FA32C06DE4139");
    const salt = hex("0EC675AD498AFEEBB6960B3AABE6");
    assert.equal(deriveKey(key, salt, 0, 16).toString("hex").toUpperCase(), "C61E7A93744F39EE10734AFE3FF7A087");
    assert.equal(deriveKey(key, salt, 2, 14).toString("hex").toUpperCase(), "30CBBC08863D8C85D49DB34A9AE1");
    assert.equal(deriveKey(key, salt, 1, 20).toString("hex").toUpperCase().slice(0, 32), "CEBE321F6FF7716B6FD4AB49AF256A15");
});

// Produced by libsrtp (the library Zalo's call program uses) with AES_CM_128_HMAC_SHA1_80 and the first 30 characters of
// a made-up session as master key + salt, the way Zalo derives a call's key.
const SESSION = "TEST-made-up-session-0123456789abcdefghijklmnop";
const LIBSRTP = {
    simple: [
        ["807000010000000011223344f8fffe", "807000010000000011223344152e16ffcce4a347bc5d0792c1"],
        ["80700002000001401122334468656c6c6f206f707573206672616d65", "8070000200000140112233440c8d5e62574bb7f06b7ee73e2fad856620f25e7396e79b9e4888"],
    ],
    rollover: [
        ["8070fffe000000001122334461", "8070fffe000000001122334496eb133da2fc1bec62583d"],
        ["8070ffff000001401122334462", "8070ffff0000014011223344f17ae5005c0e1cff79e805"],
        ["80700000000002801122334463", "80700000000002801122334454f4d7733f10dbe2c67a5c"],
        ["80700001000003c01122334464", "80700001000003c01122334428c18f3dfc89391d711c52"],
    ],
    csrc: [["817000070000000011223344aabbccdd776974682063737263", "817000070000000011223344aabbccdd4f02578623c179036154c2dc6b78b04693db11"]],
};

test("protect matches libsrtp byte for byte, including a sequence rollover and CSRCs", () => {
    for (const [name, packets] of Object.entries(LIBSRTP)) {
        const tx = SrtpContext.fromSession(SESSION);
        for (const [rtp, srtp] of packets) assert.equal(tx.protect(hex(rtp)).toString("hex"), srtp, name);
    }
});

test("unprotect reads libsrtp's packets and refuses damaged ones", () => {
    for (const [name, packets] of Object.entries(LIBSRTP)) {
        const rx = SrtpContext.fromSession(SESSION);
        for (const [rtp, srtp] of packets) assert.equal(rx.unprotect(hex(srtp))?.toString("hex"), rtp, name);
    }
    const rx = SrtpContext.fromSession(SESSION);
    const bad = hex(LIBSRTP.simple[0][1]);
    bad[bad.length - 1] ^= 1;
    assert.equal(rx.unprotect(bad), null);
    assert.equal(SrtpContext.fromSession("another-session-entirely-xxxxxxxxxx").unprotect(hex(LIBSRTP.simple[0][1])), null, "wrong key");
});

test("RTP headers", () => {
    assert.deepEqual(rtpHeader(hex("807000010000000011223344f8fffe")), {
        version: 2,
        payloadType: 112,
        marker: false,
        seq: 1,
        timestamp: 0,
        ssrc: 0x11223344,
        headerLength: 12,
    });
    assert.equal(rtpHeader(hex("00")), null);
});

function makeRtp(opts: { ssrc?: number; seq: number; ts?: number; pt?: number; padding?: number; payload?: Buffer }): Buffer {
    const body = opts.payload ?? Buffer.from([0x78, 1, 2, 3]);
    const pad = opts.padding ?? 0;
    const hdr = Buffer.alloc(12);
    hdr[0] = 0x80 | (pad > 0 ? 0x20 : 0);
    hdr[1] = opts.pt ?? 112;
    hdr.writeUInt16BE(opts.seq & 0xffff, 2);
    hdr.writeUInt32BE((opts.ts ?? opts.seq * 320) >>> 0, 4);
    hdr.writeUInt32BE((opts.ssrc ?? 0x11223344) >>> 0, 8);
    if (pad > 0) {
        const padBuf = Buffer.alloc(pad);
        padBuf[pad - 1] = pad;
        return Buffer.concat([hdr, body, padBuf]);
    }
    return Buffer.concat([hdr, body]);
}

test("TX and RX stream states are separate even when SSRC matches", () => {
    const ctx = SrtpContext.fromSession(SESSION);
    const peer = SrtpContext.fromSession(SESSION);
    const ssrc = 0x42424242;
    // TX rolls over from 0xffff -> 0x0000 (TX ROC becomes 1).
    ctx.protect(makeRtp({ ssrc, seq: 0xfffe }));
    ctx.protect(makeRtp({ ssrc, seq: 0xffff }));
    ctx.protect(makeRtp({ ssrc, seq: 0x0000 }));
    // RX on the same SrtpContext and same SSRC starts at ROC 0, seq 10 — must still decrypt.
    const incoming = peer.protect(makeRtp({ ssrc, seq: 10, payload: Buffer.from("hello") }));
    const plain = ctx.unprotect(incoming);
    assert.ok(plain);
    assert.equal(plain.subarray(12).toString(), "hello");
});

test("64-packet sliding replay window rejects duplicates and old packets without poisoning on forged packets", () => {
    const tx = SrtpContext.fromSession(SESSION);
    const rx = SrtpContext.fromSession(SESSION);
    const pkts = new Map<number, Buffer>();
    for (let seq = 1; seq <= 80; seq++) pkts.set(seq, tx.protect(makeRtp({ seq, payload: Buffer.from([seq]) })));

    assert.ok(rx.unprotect(pkts.get(1)!));
    assert.equal(rx.unprotect(pkts.get(1)!), null, "exact duplicate rejected");

    // Advance window to seq 70.
    assert.ok(rx.unprotect(pkts.get(70)!));
    // Forged packet at seq 200 must NOT advance state or poison window.
    const forged = Buffer.from(pkts.get(75)!);
    forged[forged.length - 1] ^= 0xff;
    assert.equal(rx.unprotect(forged), null);
    assert.ok(rx.unprotect(pkts.get(75)!), "authentic seq 75 still accepted after forged attempt");

    // Out-of-order within 64 packets of maxIdx (75): 75 - 63 = 12 is in window; 75 - 64 = 11 is outside window.
    assert.equal(rx.unprotect(pkts.get(11)!), null, "64 packets behind maxIdx rejected");
    assert.ok(rx.unprotect(pkts.get(12)!), "63 packets behind maxIdx accepted");
    assert.equal(rx.unprotect(pkts.get(12)!), null, "replay of out-of-order packet rejected");
});

test("estimateRoc rejects pre-wrap packets when roc === 0 (negative ROC)", () => {
    const tx = SrtpContext.fromSession(SESSION);
    const rx = SrtpContext.fromSession(SESSION);
    // Start RX at seq 5 (roc 0).
    assert.ok(rx.unprotect(tx.protect(makeRtp({ seq: 5 }))));
    // A packet with seq 65535 would estimate roc = -1 -> must return null.
    const tx2 = SrtpContext.fromSession(SESSION);
    const preWrap = tx2.protect(makeRtp({ seq: 65535 }));
    assert.equal(rx.unprotect(preWrap), null);
});

test("unauthenticated SSRCs do not populate rxStreams and authentic rxStreams are capped at 64", () => {
    const tx = SrtpContext.fromSession(SESSION);
    const rx = SrtpContext.fromSession(SESSION);
    // Flood 100 bogus packets with distinct SSRCs.
    for (let i = 1; i <= 100; i++) {
        const bad = tx.protect(makeRtp({ ssrc: 0x1000 + i, seq: 1 }));
        bad[bad.length - 1] ^= 0x55;
        assert.equal(rx.unprotect(bad), null);
    }
    // 64 authentic SSRCs should still succeed; the 65th is rejected by the cap.
    const txAuth = SrtpContext.fromSession(SESSION);
    for (let i = 1; i <= 64; i++) {
        assert.ok(rx.unprotect(txAuth.protect(makeRtp({ ssrc: 0x2000 + i, seq: 1 }))), `ssrc #${i}`);
    }
    assert.equal(rx.unprotect(txAuth.protect(makeRtp({ ssrc: 0x3000, seq: 1 }))), null, "65th authentic SSRC capped");
});

test("unprotect strips valid RTP padding and clears P bit, and rejects invalid padding", () => {
    const tx = SrtpContext.fromSession(SESSION);
    const rx = SrtpContext.fromSession(SESSION);
    const payload = Buffer.from("opus-data");
    const padded = makeRtp({ seq: 1, payload, padding: 4 });
    assert.equal((padded[0] & 0x20) !== 0, true);
    const plain = rx.unprotect(tx.protect(padded));
    assert.ok(plain);
    assert.equal((plain[0] & 0x20) !== 0, false, "P bit cleared");
    assert.deepEqual(plain.subarray(12), payload, "padding stripped");

    // Invalid padding: padLen = 0.
    const zeroPad = makeRtp({ seq: 2, payload: Buffer.from([1, 2, 0]) });
    zeroPad[0] |= 0x20;
    assert.equal(rx.unprotect(tx.protect(zeroPad)), null, "padLen 0 rejected");

    // Invalid padding: padLen > payload.length.
    const overPad = makeRtp({ seq: 3, payload: Buffer.from([1, 2, 10]) });
    overPad[0] |= 0x20;
    assert.equal(rx.unprotect(tx.protect(overPad)), null, "padLen > payload rejected");

    // Invalid padding: empty payload with P bit set.
    const emptyPad = makeRtp({ seq: 4, payload: Buffer.alloc(0) });
    emptyPad[0] |= 0x20;
    assert.equal(rx.unprotect(tx.protect(emptyPad)), null, "empty padded payload rejected");
});

test("protect throws RangeError when packet index reaches 2^48", () => {
    const tx = SrtpContext.fromSession(SESSION);
    const ssrc = 0x99887766;
    // Prime stream near the 48-bit index limit (roc = 0xffffffff, lastSeq = 0xffff).
    (tx as unknown as { txStreams: Map<number, { roc: number; lastSeq: number; started: boolean }> }).txStreams.set(ssrc, {
        roc: 0xffffffff,
        lastSeq: 0xffff,
        started: true,
    });
    assert.throws(() => tx.protect(makeRtp({ ssrc, seq: 0 })), RangeError);
});
