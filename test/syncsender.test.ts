import assert from "node:assert/strict";
import { test } from "node:test";
import { isOutgoing, type BackupRow } from "../src/syncimport.js";

const row = (senderId: string, outgoing?: boolean): BackupRow => ({
    globalMsgId: "1",
    cliMsgId: "1",
    msgType: 1,
    ts: 1,
    senderId,
    outgoing,
    text: "x",
    ttl: 0,
    bin: { attachs: [], mentions: [], quoteStatus: 0, dataSource: "", text: "" } as unknown as BackupRow["bin"],
});

test("isOutgoing: sender \"0\" or our id is us", () => {
    assert.equal(isOutgoing(row("0"), { id: "100", isGroup: false }, "me"), true);
    assert.equal(isOutgoing(row("me"), { id: "g1", isGroup: true }, "me"), true);
});

test("isOutgoing: in a 1:1 chat only the other person's id is theirs", () => {
    const conv = { id: "100", isGroup: false };
    assert.equal(isOutgoing(row("100"), conv, "me"), false);
    assert.equal(isOutgoing(row("some-other-form-of-my-id"), conv, "me"), true);
    assert.equal(isOutgoing(row(""), conv, "me"), false);
});

test("isOutgoing: in a group a stranger's id is theirs", () => {
    assert.equal(isOutgoing(row("555"), { id: "g1", isGroup: true }, "me"), false);
});

test("isOutgoing: an explicit direction column wins", () => {
    assert.equal(isOutgoing(row("100", true), { id: "100", isGroup: false }, "me"), true);
    assert.equal(isOutgoing(row("0", false), { id: "100", isGroup: false }, "me"), false);
});
