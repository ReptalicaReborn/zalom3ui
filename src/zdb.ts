import crypto from "node:crypto";
import path from "node:path";
import { DEFAULT_MAX_OUTPUT, decodeLzmaAuto } from "./lzma.js";
import { isSqlite } from "./sqliteread.js";

/**
 * The cryptography and file format of the phone's history backup (docs/history-sync.md). Mirrors what the Zalos
 * project does (`MobileHistorySync` and `DbCross::decompressAndDecryptZdb4`, MIT), written independently.
 *
 *   1. We make a one-off RSA-2048 key pair and send its public key (SPKI DER, base64) to the phone.
 *   2. The phone sends back a random backup key encrypted to that public key (RSAES-PKCS1-v1_5, base64).
 *   3. The backup key, as text (hex for db_format 1, base64 for 0), gives the AES-256 key: its first 32 characters,
 *      lowercase letters upper-cased, as ASCII bytes.
 *   4. The file is AES-256-CBC, zero IV, in 64 KiB chunks that each restart the IV, no padding. Decrypted it starts with
 *      "ZDB4.0" (18 header bytes, the last four a big-endian file count), then a table of (name length, name, size)
 *      entries (big-endian u32), then one LZMA stream holding the files back to back.
 */

export class BackupError extends Error {}

export type SyncKeyPair = { publicKey: string; privateKeyPem: string };

/** A fresh key pair for one sync: the public half goes to the phone as SPKI DER in base64. */
export function generateSyncKeyPair(): SyncKeyPair {
    const { publicKey, privateKey } = crypto.generateKeyPairSync("rsa", {
        modulusLength: 2048,
        publicKeyEncoding: { type: "spki", format: "der" },
        privateKeyEncoding: { type: "pkcs1", format: "pem" },
    });
    return { publicKey: publicKey.toString("base64"), privateKeyPem: privateKey };
}

/**
 * RSAES-PKCS1-v1_5 decryption (what the phone used to encrypt the backup key). Done as a raw RSA operation plus our own
 * unpadding because newer Node versions refuse PKCS#1 v1.5 padding for private-key decryption. Returns null on any
 * failure, without saying which (it is an oracle otherwise).
 */
export function rsaDecryptPkcs1(privateKeyPem: string, cipherBase64: string): Buffer | null {
    try {
        const cipher = Buffer.from(cipherBase64, "base64");
        const key = crypto.createPrivateKey(privateKeyPem);
        const size = ((key.asymmetricKeyDetails?.modulusLength ?? 2048) + 7) >> 3;
        if (cipher.length === 0 || cipher.length > size) return null;
        const em = crypto.privateDecrypt({ key, padding: crypto.constants.RSA_NO_PADDING }, cipher);
        // Raw output can lose leading zero bytes in some builds: put them back.
        const block = em.length < size ? Buffer.concat([Buffer.alloc(size - em.length), em]) : em;
        if (block[0] !== 0x00 || block[1] !== 0x02) return null;
        const end = block.indexOf(0x00, 2);
        if (end < 10) return null; // At least 8 padding bytes.
        const message = block.subarray(end + 1);
        return message.length > 0 ? Buffer.from(message) : null;
    } catch {
        return null;
    }
}

/** The backup key as text: hex for `db_format` 1, base64 for the old format. */
export function backupKeyText(plainKey: Buffer, format: number): string {
    return format ? plainKey.toString("hex") : plainKey.toString("base64");
}

/** The AES-256 key: the key text's first 32 characters, ASCII lowercase made uppercase, zero-padded if shorter. */
export function deriveAesKey(keyText: string): Buffer {
    const key = Buffer.alloc(32);
    const bytes = Buffer.from(keyText, "latin1");
    for (let i = 0; i < 32 && i < bytes.length; i++) {
        const c = bytes[i];
        key[i] = c >= 0x61 && c <= 0x7a ? c - 0x20 : c;
    }
    return key;
}

const CHUNK = 0x10000;

/** AES-256-CBC with a zero IV, restarted every 64 KiB, no padding. `data.length` must be a multiple of 16. */
export function decryptChunks(data: Buffer, key: Buffer): Buffer {
    if (data.length % 16 !== 0) throw new BackupError("The backup is not a whole number of AES blocks");
    const out = Buffer.allocUnsafe(data.length);
    const iv = Buffer.alloc(16);
    for (let off = 0; off < data.length; off += CHUNK) {
        const part = data.subarray(off, Math.min(off + CHUNK, data.length));
        const d = crypto.createDecipheriv("aes-256-cbc", key, iv);
        d.setAutoPadding(false);
        const plain = Buffer.concat([d.update(part), d.final()]);
        plain.copy(out, off);
    }
    return out;
}

export type BackupFile = { name: string; data: Buffer };

export const MAGIC = Buffer.from("ZDB4.0", "latin1");
const MAX_FILES = 100_000;

/**
 * Decrypts and unpacks a backup. The encrypted file may carry four extra bytes at its start or end (the Zalos project
 * tries each layout), so each fit is probed by looking for the magic at the start of the first block.
 */
export function unpackBackup(encrypted: Buffer, keyText: string, maxOutput = DEFAULT_MAX_OUTPUT): BackupFile[] {
    const size = encrypted.length;
    if (size < 32) throw new BackupError("The backup file is too small");
    const key = deriveAesKey(keyText);
    const spans: { off: number; len: number }[] = [];
    if (size % 16 === 0) spans.push({ off: 0, len: size });
    if (size > 4 && (size - 4) % 16 === 0) spans.push({ off: 4, len: size - 4 }, { off: 0, len: size - 4 });
    if (!spans.length) throw new BackupError("The backup file is not a whole number of AES blocks");

    let span: { off: number; len: number } | null = null;
    for (const s of spans) {
        const probeLen = Math.min(s.len, 64);
        const d = crypto.createDecipheriv("aes-256-cbc", key, Buffer.alloc(16));
        d.setAutoPadding(false);
        const probe = Buffer.concat([d.update(encrypted.subarray(s.off, s.off + probeLen)), d.final()]);
        if (probe.length >= 18 && probe.subarray(0, 6).equals(MAGIC)) {
            span = s;
            break;
        }
    }
    if (!span) throw new BackupError("Wrong key, or not a ZDB4 backup");

    const plain = decryptChunks(encrypted.subarray(span.off, span.off + span.len), key);
    const count = plain.readUInt32BE(14);
    if (count > MAX_FILES) throw new BackupError("The backup lists too many files");
    const table: { name: string; size: number }[] = [];
    let pos = 18;
    let total = 0;
    for (let i = 0; i < count; i++) {
        if (pos + 4 > plain.length) throw new BackupError("The backup's file table is cut short");
        const nameLen = plain.readUInt32BE(pos);
        pos += 4;
        if (nameLen > 4096 || pos + nameLen + 4 > plain.length) throw new BackupError("The backup's file table is not valid");
        const name = plain.toString("utf8", pos, pos + nameLen);
        pos += nameLen;
        const fileSize = plain.readUInt32BE(pos);
        pos += 4;
        if (!name || name.includes("..") || name.startsWith("/") || name.includes("\\")) throw new BackupError("The backup has an unsafe file name");
        total += fileSize;
        if (total > maxOutput) throw new BackupError("The backup is larger than the allowed maximum");
        table.push({ name, size: fileSize });
    }
    if (table.length === 0) return [];
    let body: Buffer;
    try {
        body = decodeLzmaAuto(plain.subarray(pos), maxOutput);
    } catch (err) {
        throw new BackupError(`The backup's data is damaged (${err instanceof Error ? err.message : "decompression failed"})`);
    }
    if (body.length < total) throw new BackupError("The backup ended before all its files were read");
    const files: BackupFile[] = [];
    let at = 0;
    for (const { name, size: fileSize } of table) {
        files.push({ name, data: body.subarray(at, at + fileSize) });
        at += fileSize;
    }
    return files;
}

/** Conversation database files in a backup: `<id>.db`, `group_<id>.db` or `g<id>.db` holding SQLite. */
export function conversationFiles(files: BackupFile[]): { id: string; isGroup: boolean; data: Buffer }[] {
    const out: { id: string; isGroup: boolean; data: Buffer }[] = [];
    for (const f of files) {
        const base = path.posix.basename(f.name.replace(/\\/g, "/"));
        if (!base.endsWith(".db") || !isSqlite(f.data)) continue;
        let id = base.slice(0, -3);
        let isGroup = false;
        if (id.startsWith("group_")) {
            id = id.slice(6);
            isGroup = true;
        } else if (/^g\d+$/.test(id)) {
            id = id.slice(1);
            isGroup = true;
        }
        if (/^\d+$/.test(id)) out.push({ id, isGroup, data: f.data });
    }
    return out.sort((a, b) => a.id.localeCompare(b.id) || Number(a.isGroup) - Number(b.isGroup));
}
