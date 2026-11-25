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
