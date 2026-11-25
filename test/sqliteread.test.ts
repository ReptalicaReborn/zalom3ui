import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { SqliteError, isSqlite, parseCreateTable, parseRecord, readTable } from "../src/sqliteread.js";

// The .db fixtures were written by SQLite itself (Python's sqlite3).
const fixture = (name: string) => fs.readFileSync(fileURLToPath(new URL(`./fixtures/${name}`, import.meta.url)));

test("reads a small table: columns, text, NULL and the rowid column", () => {
    const table = readTable(fixture("sync_1001.db"), "ChatContent");
    assert.ok(table);
    assert.deepEqual(table.columns, ["Id", "GlbMsgId", "CliMsgId", "MsgType", "TimeStamp", "SenderId", "MsgContent", "TTL", "BinNet", "MsgStatus"]);
    const rows = [...table.rows()];
    assert.equal(rows.length, 12);
    assert.equal(rows[0].Id, 1);
    assert.equal(rows[0].GlbMsgId, "9001");
    assert.equal(rows[0].TimeStamp, 1700000000000);
    assert.equal(rows[0].BinNet, null);
    assert.ok(Buffer.isBuffer(rows[2].BinNet));
    assert.equal(rows[9].CliMsgId, null);
    assert.equal(rows[10].GlbMsgId, "7351234567890123456");
    assert.ok(String(rows[11].MsgContent).startsWith("ünï 😀 x"));
    assert.equal(String(rows[11].MsgContent).length, "ünï 😀 ".length + 3000);
});

test("reads a table with interior pages and overflow pages (512-byte pages)", () => {
    const table = readTable(fixture("sync_many.db"), "chatcontent");
    assert.ok(table);
    const rows = [...table.rows()];
    assert.equal(rows.length, 401);
    // Rows come back in rowid order across the whole b-tree.
    assert.deepEqual(rows.map((r) => r.Id), Array.from({ length: 401 }, (_, i) => i + 1));
    assert.equal(rows[123].MsgContent, `message number 123 ${"lorem ipsum ".repeat(123 % 7)}`);
    const big = rows[400];
    assert.equal(big.MsgContent, `overflow ${"0123456789".repeat(700)}`);
    assert.equal((big.BinNet as Buffer).length, 1500);
});

test("a missing table is null; a file that is not SQLite is rejected", () => {
    assert.equal(readTable(fixture("sync_1001.db"), "Nope"), null);
    assert.equal(isSqlite(fixture("sync_1001.db")), true);
    assert.equal(isSqlite(Buffer.from("hello")), false);
    assert.throws(() => readTable(Buffer.alloc(200), "x"), SqliteError);
});

test("a truncated database fails with a SqliteError", () => {
    const db = fixture("sync_many.db");
    assert.throws(() => [...readTable(db.subarray(0, 512 * 6), "ChatContent")!.rows()], SqliteError);
});

test("parseRecord: every serial type", () => {
    // header size 12; types: NULL, int8 -2, int16 258, int24 -1, float, 0, 1, text "ab", blob 01 02
    const header = Buffer.from([12, 0, 1, 2, 3, 7, 8, 9, 17, 16]);
    const body = Buffer.concat([
        Buffer.from([0xfe]),
        Buffer.from([0x01, 0x02]),
        Buffer.from([0xff, 0xff, 0xff]),
        (() => {
            const b = Buffer.alloc(8);
            b.writeDoubleBE(1.5);
            return b;
        })(),
        Buffer.from("ab"),
        Buffer.from([1, 2]),
    ]);
    // Fix the header size to match the real header length (10 bytes + itself = 11).
    header[0] = header.length;
    const values = parseRecord(Buffer.concat([header, body]));
    assert.equal(values[0], null);
    assert.equal(values[1], -2);
    assert.equal(values[2], 258);
    assert.equal(values[3], -1);
    assert.equal(values[4], 1.5);
    assert.equal(values[5], 0);
    assert.equal(values[6], 1);
    assert.equal(values[7], "ab");
    assert.deepEqual(values[8], Buffer.from([1, 2]));
});

test("a 64-bit integer beyond JS safe range comes back as a bigint", () => {
    // header: size 2, type 6 (int64); body: 0x7351234567890123
    const body = Buffer.alloc(8);
    body.writeBigInt64BE(0x7351234567890123n);
    assert.equal(parseRecord(Buffer.concat([Buffer.from([2, 6]), body]))[0], 0x7351234567890123n);
});

test("parseCreateTable: quoting, constraints and the rowid alias", () => {
    const t = parseCreateTable('CREATE TABLE "T" (`a` INTEGER PRIMARY KEY AUTOINCREMENT, [b c] TEXT DEFAULT (1,2), d NUMERIC(10,2), PRIMARY KEY (a), UNIQUE (d))');
    assert.deepEqual(t.columns, ["a", "b c", "d"]);
    assert.equal(t.rowidColumn, 0);
    assert.equal(parseCreateTable("CREATE TABLE t (x TEXT, y INTEGER)").rowidColumn, -1);
});
