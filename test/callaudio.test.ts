import assert from "node:assert/strict";
import { test } from "node:test";
import { decodeAudioFromApp, encodeAudioToApp } from "../src/server.js";

test("audio messages between server and app (docs/call-audio.md §3)", () => {
    assert.equal(encodeAudioToApp(0xfffe, 0x12345678, Buffer.from([0xaa, 0xbb])).toString("hex"), "01fffe12345678aabb");
    assert.equal(encodeAudioToApp(0x1fffe, 2 ** 32 + 5, Buffer.alloc(0)).toString("hex"), "01fffe00000005", "wraps like RTP");
    assert.equal(decodeAudioFromApp(Buffer.from([2, 0xf8, 0xff, 0xfe]))?.toString("hex"), "f8fffe");
    assert.equal(decodeAudioFromApp(Buffer.from([2])), null, "empty frame");
    assert.equal(decodeAudioFromApp(Buffer.from([1, 2, 3])), null, "wrong direction");
    assert.equal(decodeAudioFromApp(Buffer.alloc(2000, 2)), null, "larger than any Opus packet");
});
