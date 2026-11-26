import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { test } from "node:test";
import * as shim from "../src/shims/iconv-lite.js";

// The real package (a dependency of express), to compare against.
const iconv = createRequire(import.meta.url)("iconv-lite") as {
    encodingExists(encoding: string): boolean;
    decode(buf: Buffer, encoding: string): string;
    getDecoder(encoding: string): { write(buf: Buffer): string; end(): string | undefined };
};

const SUPPORTED = [
    "utf8", "UTF-8", "utf-8", "unicode-1-1-utf-8", "ucs2", "UCS-2", "utf16le", "UTF-16LE",
    "binary", "latin1", "ISO-8859-1", "iso_8859-1:1987", "l1", "cp28591", "28591",
    "ascii", "US-ASCII", "csASCII", "ascii-8bit", "ANSI_X3.4-1968", "ansi_x3.4-1986", "ansix34",
];

const SAMPLES = [
    Buffer.from("plain ascii {\"a\":1}"),
    Buffer.from("﻿{\"text\":\"Xin chào, Đức 💬\"}"),
    Buffer.from("Tiếng Việt", "utf16le"),
    Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from("bom 16", "utf16le")]),
    Buffer.from([0x41, 0x7f, 0x80, 0xe9, 0xff, 0x00, 0xc3]),
    Buffer.alloc(0),
];

test("iconv shim knows the same encodings as iconv-lite for what it supports", () => {
    for (const name of SUPPORTED) {
        assert.equal(iconv.encodingExists(name), true, `iconv-lite knows ${name}`);
        assert.equal(shim.encodingExists(name), true, name);
    }
    for (const name of ["koi8-r", "windows-1258", "utf-16", "nope", "", "constructor", "__proto__"]) {
        assert.equal(shim.encodingExists(name), false, name);
    }
});

test("iconv shim decodes exactly like iconv-lite, whole and in chunks", () => {
    for (const name of SUPPORTED) {
        for (const buf of SAMPLES) {
            assert.equal(shim.decode(buf, name), iconv.decode(buf, name), `${name}: ${buf.toString("hex")}`);
            // Every split point, so multi-byte characters and the BOM straddle chunks.
            for (let cut = 0; cut <= buf.length; cut++) {
                const run = (d: { write(b: Buffer): string; end(): string | undefined }) =>
                    d.write(buf.subarray(0, cut)) + d.write(buf.subarray(cut)) + (d.end() || "");
                assert.equal(run(shim.getDecoder(name)), run(iconv.getDecoder(name)), `${name} cut ${cut}: ${buf.toString("hex")}`);
            }
        }
    }
});

test("iconv shim rejects other charsets with iconv-lite's message, which raw-body turns into 415", () => {
    assert.throws(() => shim.getDecoder("koi8-r"), /^Error: Encoding not recognized: 'koi8-r' \(searched as: 'koi8r'\)$/);
    assert.throws(() => shim.decode(Buffer.from("x"), "windows-1258"), /^Error: Encoding not recognized: /);
});
