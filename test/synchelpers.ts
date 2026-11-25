import crypto from "node:crypto";
import { MAGIC, deriveAesKey } from "../src/zdb.js";

/** AES-256-CBC, zero IV restarted every 64 KiB, no padding: the inverse of the backup's decryption. */
export function encryptChunks(data: Buffer, key: Buffer): Buffer {
    const parts: Buffer[] = [];
    for (let off = 0; off < data.length; off += 0x10000) {
        const c = crypto.createCipheriv("aes-256-cbc", key, Buffer.alloc(16));
        c.setAutoPadding(false);
        parts.push(Buffer.concat([c.update(data.subarray(off, Math.min(off + 0x10000, data.length))), c.final()]));
    }
    return Buffer.concat(parts);
}

/** Builds a backup file the way the phone does; the body is an already compressed LZMA stream. */
export function packBackup(
    files: { name: string; size: number }[],
    compressedBody: Buffer,
    keyText: string,
    opts: { prefix?: number; suffix?: number } = {},
): Buffer {
    const head = Buffer.alloc(18);
    MAGIC.copy(head);
    head.writeUInt32BE(files.length, 14);
    const parts: Buffer[] = [head];
    for (const f of files) {
        const name = Buffer.from(f.name);
        const len = Buffer.alloc(4);
        len.writeUInt32BE(name.length);
        const size = Buffer.alloc(4);
        size.writeUInt32BE(f.size);
        parts.push(len, name, size);
    }
    parts.push(compressedBody);
    let plain = Buffer.concat(parts);
    if (plain.length % 16) plain = Buffer.concat([plain, Buffer.alloc(16 - (plain.length % 16))]);
    return Buffer.concat([Buffer.alloc(opts.prefix ?? 0, 0x5a), encryptChunks(plain, deriveAesKey(keyText)), Buffer.alloc(opts.suffix ?? 0, 0x5a)]);
}
