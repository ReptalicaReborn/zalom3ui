import assert from "node:assert/strict";
import { test } from "node:test";
import { ReceiveStats, ntp, ntpMiddle, parseRtcp, receiverReport, sdesCname, senderReport } from "../src/rtcp.js";

const block = { ssrc: 0xabcdef01, fractionLost: 25, cumulativeLost: 3, highestSeq: 70000, jitter: 42, lsr: 0x12345678, dlsr: 65536 };

test("sender report, receiver report and SDES are laid out as RFC 3550 says (checked by hand)", () => {
    assert.equal(
        senderReport({ ssrc: 0x11223344, nowMs: 1795632118951, rtpTimestamp: 160000, packets: 50, octets: 1234, blocks: [block] }).toString("hex"),
        "81c8000c11223344eeb1aa76f374bc6a0002710000000032000004d2abcdef0119000003000111700000002a1234567800010000",
    );
    assert.equal(receiverReport({ ssrc: 0x11223344, blocks: [block] }).toString("hex"), "81c9000711223344abcdef0119000003000111700000002a1234567800010000");
    assert.equal(sdesCname(0x11223344, "zaloo").toString("hex"), "81ca00031122334401057a616c6f6f00");
    assert.equal(ntp(0).sec, 2_208_988_800);
    assert.equal(ntpMiddle(0x0001aaaa, 0xbbbb0000), 0xaaaabbbb);
});

test("compound packets parse back", () => {
    const compound = Buffer.concat([
        senderReport({ ssrc: 1, nowMs: 1000, rtpTimestamp: 2, packets: 3, octets: 4, blocks: [block] }),
        sdesCname(1, "x"),
        Buffer.from("81cd0002000000010000000a", "hex"), // a NACK (RTPFB)
    ]);
    const parts = parseRtcp(compound);
    assert.equal(parts.length, 3);
    assert.equal(parts[0].type, "SR");
    if (parts[0].type === "SR") assert.deepEqual(parts[0].blocks, [block]);
    assert.deepEqual(parts[2], { type: "other", packetType: 205, ssrc: 1, bytes: 12 });
    assert.deepEqual(parseRtcp(Buffer.from("00c80001", "hex")), [], "not version 2");
});

test("receive statistics: loss, sequence wrap, jitter, LSR/DLSR", () => {
    const s = new ReceiveStats(0xabc, 16000);
    assert.equal(s.block(0), null);
    // 65534, 65535, (0 lost), 1 — 20 ms apart, 320 ticks apart: no jitter.
    s.onPacket(65534, 0, 1000);
    s.onPacket(65535, 320, 1020);
    s.onPacket(1, 960, 1060);
    s.onSenderReport(0x0001aaaa, 0xbbbb0000, 1000);
    const b = s.block(1500)!;
    assert.equal(b.highestSeq, 65536 + 1);
    assert.equal(b.cumulativeLost, 1);
    assert.equal(b.fractionLost, Math.floor((1 << 8) / 4));
    assert.equal(b.jitter, 0);
    assert.equal(b.lsr, 0xaaaabbbb);
    assert.equal(b.dlsr, 32768, "half a second in 1/65536 s");
    // Nothing new: the next report says no fresh loss.
    assert.equal(s.block(2000)!.fractionLost, 0);
});

test("ReceiveStats guards dlsr against negative clock jump and fractionLost against 32-bit overflow", () => {
    const s = new ReceiveStats(0x123, 16000);
    s.onPacket(1, 0, 1000);
    s.onSenderReport(0x0001aaaa, 0xbbbb0000, 5000);
    // Clock jumped backwards (nowMs < SR arrivalMs): dlsr must clamp to 0.
    const b1 = s.block(4000)!;
    assert.equal(b1.dlsr, 0);

    // Simulate > 2^23 lost packets in one interval so (lostInterval << 8) would have overflowed int32.
    (s as unknown as { cycles: number; maxSeq: number }).cycles = 0x900000;
    const b2 = s.block(6000)!;
    assert.ok(b2.fractionLost >= 250 && b2.fractionLost <= 255, `fractionLost=${b2.fractionLost} should be near 255, not negative`);
});

test("parseRtcp honors padding bit and parses all FIR FCI entries", () => {
    // FIR with 2 FCI entries (header 12 bytes + 2 * 8 = 28 bytes -> length word = 6).
    const multiFir = Buffer.alloc(28);
    multiFir[0] = 0x84; // V=2, P=0, FMT=4
    multiFir[1] = 206;  // PSFB
    multiFir.writeUInt16BE(6, 2);
    multiFir.writeUInt32BE(0x1111, 4);
    multiFir.writeUInt32BE(0xaaaa, 12);
    multiFir[16] = 1;
    multiFir.writeUInt32BE(0xbbbb, 20);
    multiFir[24] = 2;
    assert.deepEqual(parseRtcp(multiFir), [
        { type: "FIR", ssrc: 0x1111, mediaSsrc: 0xaaaa },
        { type: "FIR", ssrc: 0x1111, mediaSsrc: 0xbbbb },
    ]);

    // Padded FIR: 1 FCI entry (20 bytes) + 8 bytes padding (last byte = 8) -> total 28 bytes, P bit set (0xa4).
    const paddedFir = Buffer.alloc(28);
    paddedFir[0] = 0xa4; // V=2, P=1, FMT=4
    paddedFir[1] = 206;
    paddedFir.writeUInt16BE(6, 2);
    paddedFir.writeUInt32BE(0x2222, 4);
    paddedFir.writeUInt32BE(0xcccc, 12);
    paddedFir[16] = 3;
    paddedFir.writeUInt32BE(0xdddd, 20); // inside padding, must NOT be parsed as a second FIR
    paddedFir[27] = 8;
    assert.deepEqual(parseRtcp(paddedFir), [{ type: "FIR", ssrc: 0x2222, mediaSsrc: 0xcccc }]);

    // Invalid padding (padLen = 0 or > len - 4) stops parsing.
    const badPad = Buffer.from(paddedFir);
    badPad[27] = 0;
    assert.deepEqual(parseRtcp(badPad), []);
    badPad[27] = 25; // > 28 - 4
    assert.deepEqual(parseRtcp(badPad), []);
});
