import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { LzmaError, decodeLzmaAlone, decodeLzmaAuto, decodeXz } from "../src/lzma.js";

// Fixtures were made with liblzma (Python's lzma module); the inputs are rebuilt here from simple formulas.
const fixture = (name: string) => fs.readFileSync(fileURLToPath(new URL(`./fixtures/${name}`, import.meta.url)));

function lcg(n: number, seed: number): Buffer {
    const out = Buffer.alloc(n);
    let x = seed;
    for (let i = 0; i < n; i++) {
        x = (Math.imul(x, 1103515245) + 12345) & 0x7fffffff;
        out[i] = (x >> 16) & 255;
    }
    return out;
}

const TEXT = Buffer.from(("The quick brown fox jumps over the lazy dog. ".repeat(40) + "Xin chào, đây là tin nhắn thử nghiệm.\n").repeat(5));

test("xz: a small text", () => {
    assert.ok(decodeXz(fixture("lz_text.xz")).equals(TEXT));
});

test("lzma_alone: a small text", () => {
    assert.ok(decodeLzmaAlone(fixture("lz_text.lzma")).equals(TEXT));
});

test("auto-detects .xz and .lzma", () => {
    assert.ok(decodeLzmaAuto(fixture("lz_text.xz")).equals(TEXT));
    assert.ok(decodeLzmaAuto(fixture("lz_text.lzma")).equals(TEXT));
});

test("xz: several LZMA2 chunks keep the dictionary and the model between them", () => {
    const expected = Buffer.concat([lcg(997, 7)].flatMap((b) => Array.from({ length: 5000 }, () => b))).subarray(0, 4_500_000);
    assert.ok(decodeXz(fixture("lz_big.xz")).equals(expected));
});

test("xz: an incompressible input is stored in uncompressed LZMA2 chunks", () => {
    assert.ok(decodeXz(fixture("lz_rnd.xz")).equals(lcg(3000, 99)));
});

test("a limit on the output stops a stream that would expand too far", () => {
    assert.throws(() => decodeXz(fixture("lz_big.xz"), 1_000_000), LzmaError);
    assert.throws(() => decodeLzmaAlone(fixture("lz_text.lzma"), 100), LzmaError);
});

test("damaged streams fail with an LzmaError, not a crash", () => {
    const xz = fixture("lz_text.xz");
    assert.throws(() => decodeXz(xz.subarray(0, 40)), LzmaError);
    const broken = Buffer.from(xz);
    broken[30] ^= 0xff;
    // A flipped byte either makes the decoder fail or changes the output; it never silently gives the original.
    let decoded: Buffer | null = null;
    try {
        decoded = decodeXz(broken);
    } catch (err) {
        assert.ok(err instanceof LzmaError);
    }
    if (decoded) assert.ok(!decoded.equals(TEXT));
    assert.throws(() => decodeLzmaAuto(Buffer.from("not lzma at all, just some text")), LzmaError);
});
