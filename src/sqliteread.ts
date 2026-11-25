/**
 * A minimal read-only SQLite reader: finds a table in a database file held in memory and walks its rows.
 *
 * Why it exists: the phone's history backup is a set of SQLite files (one per conversation), and the phone's embedded
 * Node (18) has no SQLite module. Reads only what the backup needs: ordinary rowid tables (table b-trees, overflow
 * pages, all value types). No SQL, no indexes, no WAL. Format reference: https://www.sqlite.org/fileformat2.html
 */

export class SqliteError extends Error {}

/** A column value: NULL, an integer (a bigint when it doesn't fit a safe JS number), a float, text or bytes. */
export type SqlValue = null | number | bigint | string | Buffer;

export type SqlRow = Record<string, SqlValue>;

const MAX_DEPTH = 40;
const MAX_ROW_BYTES = 64 * 1024 * 1024;

type Header = { pageSize: number; usable: number; pageCount: number; encoding: number };

function readHeader(db: Buffer): Header {
    if (db.length < 100 || db.toString("latin1", 0, 15) !== "SQLite format 3") throw new SqliteError("Not a SQLite database");
    let pageSize = db.readUInt16BE(16);
    if (pageSize === 1) pageSize = 65536;
    if (pageSize < 512 || pageSize > 65536 || (pageSize & (pageSize - 1)) !== 0) throw new SqliteError("Invalid SQLite page size");
    const usable = pageSize - db[20];
    if (usable < 480) throw new SqliteError("Invalid SQLite page layout");
    const encoding = db.readUInt32BE(56) || 1;
    if (encoding < 1 || encoding > 3) throw new SqliteError("Unsupported SQLite text encoding");
    return { pageSize, usable, pageCount: Math.floor(db.length / pageSize), encoding };
}

function varint(buf: Buffer, pos: number): { value: bigint; next: number } {
    let v = 0n;
    for (let i = 0; i < 8; i++) {
        if (pos + i >= buf.length) throw new SqliteError("Truncated SQLite varint");
        const b = buf[pos + i];
        v = (v << 7n) | BigInt(b & 0x7f);
        if ((b & 0x80) === 0) return { value: v, next: pos + i + 1 };
    }
    if (pos + 8 >= buf.length) throw new SqliteError("Truncated SQLite varint");
    return { value: (v << 8n) | BigInt(buf[pos + 8]), next: pos + 9 };
}

function toNumeric(v: bigint): number | bigint {
    return v >= BigInt(Number.MIN_SAFE_INTEGER) && v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v;
}

function decodeText(bytes: Buffer, encoding: number): string {
    if (encoding === 1) return bytes.toString("utf8");
    if (encoding === 2) return bytes.toString("utf16le");
    const swapped = Buffer.from(bytes);
    swapped.swap16();
    return swapped.toString("utf16le");
}

/** Decodes one record (the payload of a table row) into column values. */
export function parseRecord(payload: Buffer, encoding = 1): SqlValue[] {
    const header = varint(payload, 0);
    const headerEnd = Number(header.value);
    if (headerEnd > payload.length || headerEnd < header.next) throw new SqliteError("Corrupt SQLite record");
    const types: number[] = [];
    let p = header.next;
    while (p < headerEnd) {
        const t = varint(payload, p);
        types.push(Number(t.value));
        p = t.next;
    }
    const values: SqlValue[] = [];
    let at = headerEnd;
    const need = (n: number) => {
        if (n < 0 || at + n > payload.length) throw new SqliteError("Corrupt SQLite record");
    };
    for (const t of types) {
        switch (t) {
            case 0:
                values.push(null);
                break;
            case 1:
            case 2:
            case 3:
            case 4:
            case 5:
            case 6: {
                const size = [0, 1, 2, 3, 4, 6, 8][t];
                need(size);
                let v = 0n;
                for (let i = 0; i < size; i++) v = (v << 8n) | BigInt(payload[at + i]);
                // Two's complement.
                if (payload[at] & 0x80) v -= 1n << BigInt(size * 8);
                values.push(toNumeric(v));
                at += size;
                break;
            }
            case 7:
                need(8);
                values.push(payload.readDoubleBE(at));
                at += 8;
                break;
            case 8:
                values.push(0);
                break;
            case 9:
                values.push(1);
                break;
            default: {
                if (t < 12) throw new SqliteError("Corrupt SQLite record");
                const size = t % 2 === 0 ? (t - 12) / 2 : (t - 13) / 2;
                need(size);
                const bytes = payload.subarray(at, at + size);
                values.push(t % 2 === 0 ? Buffer.from(bytes) : decodeText(bytes, encoding));
                at += size;
            }
        }
    }
    return values;
}

class Db {
    readonly h: Header;
    constructor(readonly buf: Buffer) {
        this.h = readHeader(buf);
    }

    page(n: number): Buffer {
        if (!Number.isInteger(n) || n < 1 || n > this.h.pageCount) throw new SqliteError("SQLite page out of range");
        return this.buf.subarray((n - 1) * this.h.pageSize, n * this.h.pageSize);
    }

    /** Full payload of a cell, following overflow pages. */
    payload(page: Buffer, cellAt: number, total: number): Buffer {
        const u = this.h.usable;
        const x = u - 35;
        if (total <= x) return page.subarray(cellAt, cellAt + total);
        const m = Math.floor(((u - 12) * 32) / 255) - 23;
        const k = m + ((total - m) % (u - 4));
        const local = k <= x ? k : m;
        if (total > MAX_ROW_BYTES) throw new SqliteError("SQLite row too large");
        const parts: Buffer[] = [page.subarray(cellAt, cellAt + local)];
        let remaining = total - local;
        let next = page.readUInt32BE(cellAt + local);
        const seen = new Set<number>();
        while (remaining > 0) {
            if (next === 0 || seen.has(next)) throw new SqliteError("Corrupt SQLite overflow chain");
            seen.add(next);
            const pg = this.page(next);
            const take = Math.min(remaining, u - 4);
            parts.push(pg.subarray(4, 4 + take));
            remaining -= take;
            next = pg.readUInt32BE(0);
        }
        return Buffer.concat(parts);
    }

    /** Rows of the table b-tree rooted at `root`, in rowid order. */
    *rows(root: number, depth = 0): Generator<{ rowid: bigint; record: Buffer }> {
        if (depth > MAX_DEPTH) throw new SqliteError("SQLite b-tree too deep");
        const page = this.page(root);
        const base = root === 1 ? 100 : 0;
        const type = page[base];
        const cells = page.readUInt16BE(base + 3);
        if (type === 0x0d) {
            for (let i = 0; i < cells; i++) {
                const ptr = page.readUInt16BE(base + 8 + i * 2);
                if (ptr >= this.h.usable) throw new SqliteError("Corrupt SQLite cell pointer");
                const size = varint(page, ptr);
                const rowid = varint(page, size.next);
                yield { rowid: rowid.value, record: this.payload(page, rowid.next, Number(size.value)) };
            }
        } else if (type === 0x05) {
            for (let i = 0; i < cells; i++) {
                const ptr = page.readUInt16BE(base + 12 + i * 2);
                if (ptr + 4 > this.h.usable) throw new SqliteError("Corrupt SQLite cell pointer");
                yield* this.rows(page.readUInt32BE(ptr), depth + 1);
            }
            yield* this.rows(page.readUInt32BE(base + 8), depth + 1);
        } else {
            throw new SqliteError("Unexpected SQLite page type");
        }
    }
}

/** Column names from a `CREATE TABLE` statement, and which one (if any) is the INTEGER PRIMARY KEY (the rowid). */
export function parseCreateTable(sql: string): { columns: string[]; rowidColumn: number } {
    const open = sql.indexOf("(");
    const close = sql.lastIndexOf(")");
    if (open < 0 || close < open) throw new SqliteError("Unreadable table definition");
    const body = sql.slice(open + 1, close);
    const defs: string[] = [];
    let depth = 0;
    let quote = "";
    let current = "";
    for (const c of body) {
        if (quote) {
            current += c;
            if (c === quote) quote = "";
            continue;
        }
        if (c === '"' || c === "'" || c === "`") quote = c;
        else if (c === "[") quote = "]";
        else if (c === "(") depth++;
        else if (c === ")") depth--;
        if (c === "," && depth === 0) {
            defs.push(current);
            current = "";
        } else {
            current += c;
        }
    }
    if (current.trim()) defs.push(current);
    const columns: string[] = [];
    let rowidColumn = -1;
    for (const def of defs) {
        const text = def.trim();
        if (/^(constraint|primary|unique|check|foreign)\b/i.test(text)) continue;
        const m = /^(?:"([^"]+)"|`([^`]+)`|\[([^\]]+)\]|'([^']+)'|(\S+))/.exec(text);
        if (!m) continue;
        const name = m[1] ?? m[2] ?? m[3] ?? m[4] ?? m[5];
        if (/\binteger\s+primary\s+key\b/i.test(text) && !/\bdesc\b/i.test(text)) rowidColumn = columns.length;
        columns.push(name);
    }
    return { columns, rowidColumn };
}

/** Whether the file is a SQLite database (by its header). */
export function isSqlite(buf: Buffer): boolean {
    return buf.length >= 100 && buf.toString("latin1", 0, 15) === "SQLite format 3";
}

/** The table's column names and its rows as objects, or null when the file has no such table. */
export function readTable(buf: Buffer, table: string): { columns: string[]; rows: () => Generator<SqlRow> } | null {
    const db = new Db(buf);
    let found: { root: number; sql: string } | null = null;
    for (const { record } of db.rows(1)) {
        const r = parseRecord(record, db.h.encoding);
        // sqlite_master: type, name, tbl_name, rootpage, sql
        if (r[0] === "table" && typeof r[1] === "string" && r[1].toLowerCase() === table.toLowerCase()) {
            found = { root: Number(r[3]), sql: String(r[4] ?? "") };
            break;
        }
    }
    if (!found) return null;
    const { columns, rowidColumn } = parseCreateTable(found.sql);
    return {
        columns,
        *rows() {
            for (const { rowid, record } of db.rows(found!.root)) {
                const values = parseRecord(record, db.h.encoding);
                const row: SqlRow = {};
                columns.forEach((name, i) => {
                    // The rowid alias column is stored as NULL in the record.
                    row[name] = i === rowidColumn && (values[i] === null || values[i] === undefined) ? toNumeric(rowid) : (values[i] ?? null);
                });
                yield row;
            }
        },
    };
}
