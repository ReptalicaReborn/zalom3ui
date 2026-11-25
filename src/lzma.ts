/**
 * A small LZMA decoder: raw LZMA ("lzma_alone", the old `.lzma` file), LZMA2 and the `.xz` container around it.
 *
 * Why it exists: the phone's history backup is an LZMA stream (the Zalos project reads it with liblzma's
 * `lzma_auto_decoder`, which accepts `.xz` and `.lzma`), and Node has no LZMA in its standard library. Written from the
 * LZMA specification (the reference `LzmaSpec.cpp`) and the XZ file format; it decodes into one growing buffer, so the
 * "dictionary" is simply the output so far. Checksums inside `.xz` are skipped: the backup is already authenticated by
 * its decryption and by the SQLite files it has to contain.
 */

export class LzmaError extends Error {}

/** Default cap on decoded bytes (the Zalos project uses 256 MB too): a damaged stream must not eat a phone's memory. */
export const DEFAULT_MAX_OUTPUT = 256 * 1024 * 1024;

const PROB_INIT = 1024;
const NUM_STATES = 12;
const POS_STATES_MAX = 16;
const END_POS_MODEL_INDEX = 14;
const NUM_FULL_DISTANCES = 1 << (END_POS_MODEL_INDEX >> 1);
const NUM_ALIGN_BITS = 4;
const MATCH_MIN_LEN = 2;

class Output {
    buf: Buffer;
    pos = 0;
    /** Where the current dictionary starts (LZMA2 can reset it): matches must not reach before it. */
    dictStart = 0;
    constructor(
        initial: number,
        readonly max: number,
    ) {
        this.buf = Buffer.allocUnsafe(Math.max(64, Math.min(initial, max)));
    }
    ensure(extra: number): void {
        const need = this.pos + extra;
        if (need > this.max) throw new LzmaError("Decoded data is larger than the allowed maximum");
        if (need <= this.buf.length) return;
        let size = this.buf.length;
        while (size < need) size = Math.min(this.max, size * 2);
        const next = Buffer.allocUnsafe(size);
        this.buf.copy(next, 0, 0, this.pos);
        this.buf = next;
    }
    result(): Buffer {
        return this.buf.subarray(0, this.pos);
    }
}

class RangeDecoder {
    range = 0xffffffff;
    code = 0;
    private pos: number;
    constructor(
        private readonly src: Buffer,
        start: number,
        private readonly end: number,
    ) {
        this.pos = start;
        if (end - start < 5) throw new LzmaError("Truncated LZMA data");
        if (src[this.pos++] !== 0) throw new LzmaError("Corrupt LZMA data (range coder start)");
        for (let i = 0; i < 4; i++) this.code = ((this.code << 8) | src[this.pos++]) >>> 0;
        if (this.code === this.range) throw new LzmaError("Corrupt LZMA data");
    }
    get position(): number {
        return this.pos;
    }
    private normalize(): void {
        if (this.range < 0x1000000) {
            if (this.pos >= this.end) throw new LzmaError("Truncated LZMA data");
            this.range = (this.range << 8) >>> 0;
            this.code = ((this.code << 8) | this.src[this.pos++]) >>> 0;
        }
    }
    bit(probs: Uint16Array, i: number): number {
        const p = probs[i];
        const bound = (this.range >>> 11) * p;
        let b: number;
        if (this.code < bound) {
            this.range = bound;
            probs[i] = p + ((2048 - p) >>> 5);
            b = 0;
        } else {
            this.range -= bound;
            this.code -= bound;
            probs[i] = p - (p >>> 5);
            b = 1;
        }
        this.normalize();
        return b;
    }
    direct(n: number): number {
        let res = 0;
        for (; n > 0; n--) {
            this.range >>>= 1;
            let b = 0;
            if (this.code >= this.range) {
                this.code -= this.range;
                b = 1;
            }
            this.normalize();
            res = res * 2 + b;
        }
        return res;
    }
    tree(probs: Uint16Array, offset: number, bits: number): number {
        let m = 1;
        for (let i = 0; i < bits; i++) m = (m << 1) | this.bit(probs, offset + m);
        return m - (1 << bits);
    }
    reverseTree(probs: Uint16Array, offset: number, bits: number): number {
        let m = 1;
        let sym = 0;
        for (let i = 0; i < bits; i++) {
            const b = this.bit(probs, offset + m);
            m = (m << 1) | b;
            sym |= b << i;
        }
        return sym;
    }
}

class LenDecoder {
    private readonly choice = new Uint16Array(2).fill(PROB_INIT);
    private readonly low = new Uint16Array(POS_STATES_MAX << 3).fill(PROB_INIT);
    private readonly mid = new Uint16Array(POS_STATES_MAX << 3).fill(PROB_INIT);
    private readonly high = new Uint16Array(256).fill(PROB_INIT);
    reset(): void {
        this.choice.fill(PROB_INIT);
        this.low.fill(PROB_INIT);
        this.mid.fill(PROB_INIT);
        this.high.fill(PROB_INIT);
    }
    decode(rc: RangeDecoder, posState: number): number {
        if (rc.bit(this.choice, 0) === 0) return rc.tree(this.low, posState << 3, 3);
        if (rc.bit(this.choice, 1) === 0) return 8 + rc.tree(this.mid, posState << 3, 3);
        return 16 + rc.tree(this.high, 0, 8);
    }
}

/** The LZMA model: probabilities, state and the four last distances. Survives across LZMA2 chunks. */
class LzmaState {
    lc = 0;
    lp = 0;
    pb = 0;
    private literal = new Uint16Array(0);
    private readonly isMatch = new Uint16Array(NUM_STATES << 4);
    private readonly isRep = new Uint16Array(NUM_STATES);
    private readonly isRepG0 = new Uint16Array(NUM_STATES);
    private readonly isRepG1 = new Uint16Array(NUM_STATES);
    private readonly isRepG2 = new Uint16Array(NUM_STATES);
    private readonly isRep0Long = new Uint16Array(NUM_STATES << 4);
    private readonly posSlot = new Uint16Array(4 << 6);
    private readonly posDecoders = new Uint16Array(1 + NUM_FULL_DISTANCES - END_POS_MODEL_INDEX);
    private readonly align = new Uint16Array(1 << NUM_ALIGN_BITS);
    private readonly lenDec = new LenDecoder();
    private readonly repLenDec = new LenDecoder();
    private state = 0;
    private rep0 = 0;
    private rep1 = 0;
    private rep2 = 0;
    private rep3 = 0;

    setProps(byte: number): void {
        if (byte >= 9 * 5 * 5) throw new LzmaError("Invalid LZMA properties");
        this.lc = byte % 9;
        const rest = Math.floor(byte / 9);
        this.lp = rest % 5;
        this.pb = Math.floor(rest / 5);
        this.literal = new Uint16Array(0x300 << (this.lc + this.lp));
    }

    reset(): void {
        this.literal.fill(PROB_INIT);
        for (const p of [this.isMatch, this.isRep, this.isRepG0, this.isRepG1, this.isRepG2, this.isRep0Long, this.posSlot, this.posDecoders, this.align]) {
            p.fill(PROB_INIT);
        }
        this.lenDec.reset();
        this.repLenDec.reset();
        this.state = 0;
        this.rep0 = this.rep1 = this.rep2 = this.rep3 = 0;
    }

    /**
     * Decodes until `limit` bytes were produced (`limit` < 0: until the end marker). Returns whether an end marker was
     * seen.
     */
    decode(rc: RangeDecoder, out: Output, limit: number): boolean {
        const pbMask = (1 << this.pb) - 1;
        const lpMask = (1 << this.lp) - 1;
        const target = limit < 0 ? Infinity : out.pos + limit;
        let { state, rep0, rep1, rep2, rep3 } = this;
        const save = () => {
            this.state = state;
            this.rep0 = rep0;
            this.rep1 = rep1;
            this.rep2 = rep2;
            this.rep3 = rep3;
        };
        try {
            while (out.pos < target) {
                const posState = out.pos & pbMask;
                if (rc.bit(this.isMatch, (state << 4) + posState) === 0) {
                    out.ensure(1);
                    const prev = out.pos > out.dictStart ? out.buf[out.pos - 1] : 0;
                    const base = 0x300 * (((out.pos & lpMask) << this.lc) + (prev >> (8 - this.lc)));
                    let sym = 1;
                    if (state >= 7) {
                        const from = out.pos - rep0 - 1;
                        let matchByte = from >= out.dictStart ? out.buf[from] : 0;
                        while (sym < 0x100) {
                            const matchBit = (matchByte >> 7) & 1;
                            matchByte <<= 1;
                            const b = rc.bit(this.literal, base + ((1 + matchBit) << 8) + sym);
                            sym = (sym << 1) | b;
                            if (matchBit !== b) break;
                        }
                    }
                    while (sym < 0x100) sym = (sym << 1) | rc.bit(this.literal, base + sym);
                    out.buf[out.pos++] = sym & 0xff;
                    state = state < 4 ? 0 : state < 10 ? state - 3 : state - 6;
                    continue;
                }
                let len: number;
                if (rc.bit(this.isRep, state) !== 0) {
                    if (out.pos === out.dictStart) throw new LzmaError("Corrupt LZMA data (repeat at start)");
                    if (rc.bit(this.isRepG0, state) === 0) {
                        if (rc.bit(this.isRep0Long, (state << 4) + posState) === 0) {
                            if (rep0 >= out.pos - out.dictStart) throw new LzmaError("Corrupt LZMA data (distance)");
                            out.ensure(1);
                            state = state < 7 ? 9 : 11;
                            out.buf[out.pos] = out.buf[out.pos - rep0 - 1];
                            out.pos++;
                            continue;
                        }
                    } else {
                        let dist: number;
                        if (rc.bit(this.isRepG1, state) === 0) {
                            dist = rep1;
                        } else {
                            if (rc.bit(this.isRepG2, state) === 0) {
                                dist = rep2;
                            } else {
                                dist = rep3;
                                rep3 = rep2;
                            }
                            rep2 = rep1;
                        }
                        rep1 = rep0;
                        rep0 = dist;
                    }
                    len = this.repLenDec.decode(rc, posState);
                    state = state < 7 ? 8 : 11;
                } else {
                    rep3 = rep2;
                    rep2 = rep1;
                    rep1 = rep0;
                    len = this.lenDec.decode(rc, posState);
                    state = state < 7 ? 7 : 10;
                    const lenState = len < 4 ? len : 3;
                    const slot = rc.tree(this.posSlot, lenState << 6, 6);
                    if (slot < 4) {
                        rep0 = slot;
                    } else {
                        const direct = (slot >> 1) - 1;
                        let dist = (2 | (slot & 1)) * 2 ** direct;
                        if (slot < END_POS_MODEL_INDEX) {
                            dist += rc.reverseTree(this.posDecoders, dist - slot, direct);
                        } else {
                            dist += rc.direct(direct - NUM_ALIGN_BITS) * (1 << NUM_ALIGN_BITS);
                            dist += rc.reverseTree(this.align, 0, NUM_ALIGN_BITS);
                        }
                        rep0 = dist;
                        if (rep0 >= 0xffffffff) {
                            // End marker.
                            save();
                            return true;
                        }
                    }
                }
                len += MATCH_MIN_LEN;
                if (rep0 >= out.pos - out.dictStart) throw new LzmaError("Corrupt LZMA data (distance)");
                if (out.pos + len > target) throw new LzmaError("Corrupt LZMA data (match past the end)");
                out.ensure(len);
                // Matches may overlap their own output, so copy byte by byte.
                let from = out.pos - rep0 - 1;
                const buf = out.buf;
                for (let i = 0; i < len; i++) buf[out.pos++] = buf[from++];
            }
        } finally {
            save();
        }
        return false;
    }
}

/** Raw LZMA ("lzma_alone"): 13 byte header (properties, dictionary size, uncompressed size), then the stream. */
export function decodeLzmaAlone(src: Buffer, maxOutput = DEFAULT_MAX_OUTPUT): Buffer {
    if (src.length < 13 + 5) throw new LzmaError("Truncated LZMA data");
    const model = new LzmaState();
    model.setProps(src[0]);
    model.reset();
    const sizeBig = src.readBigUInt64LE(5);
    const known = sizeBig !== 0xffffffffffffffffn;
    if (known && sizeBig > BigInt(maxOutput)) throw new LzmaError("Decoded data is larger than the allowed maximum");
    const size = known ? Number(sizeBig) : -1;
    const out = new Output(known ? size : 1 << 20, maxOutput);
    const rc = new RangeDecoder(src, 13, src.length);
    const marker = model.decode(rc, out, size);
    if (known && !marker && out.pos !== size) throw new LzmaError("LZMA data ended early");
    return out.result();
}

/** LZMA2 chunks starting at `start`; returns where the stream ended (after the 0x00 end byte). */
function decodeLzma2(src: Buffer, start: number, out: Output): number {
    const model = new LzmaState();
    let pos = start;
    let needProps = true;
    let needDictReset = true;
    for (;;) {
        if (pos >= src.length) throw new LzmaError("Truncated LZMA2 data");
        const control = src[pos++];
        if (control === 0) return pos;
        if (control >= 0xe0 || control === 1) {
            needProps = true;
            needDictReset = false;
            out.dictStart = out.pos;
        } else if (needDictReset) {
            throw new LzmaError("Corrupt LZMA2 data (missing dictionary reset)");
        }
        if (control >= 0x80) {
            if (pos + 4 > src.length) throw new LzmaError("Truncated LZMA2 data");
            const unpacked = ((control & 0x1f) << 16) + src.readUInt16BE(pos) + 1;
            const packed = src.readUInt16BE(pos + 2) + 1;
            pos += 4;
            if (control >= 0xc0) {
                if (pos >= src.length) throw new LzmaError("Truncated LZMA2 data");
                model.setProps(src[pos++]);
                if (model.lc + model.lp > 4) throw new LzmaError("Invalid LZMA2 properties");
                needProps = false;
            } else if (needProps) {
                throw new LzmaError("Corrupt LZMA2 data (missing properties)");
            }
            if (control >= 0xa0) model.reset();
            if (pos + packed > src.length) throw new LzmaError("Truncated LZMA2 data");
            const rc = new RangeDecoder(src, pos, pos + packed);
            const before = out.pos;
            model.decode(rc, out, unpacked);
            if (out.pos - before !== unpacked) throw new LzmaError("Corrupt LZMA2 data (chunk size)");
            pos += packed;
        } else if (control === 1 || control === 2) {
            if (pos + 2 > src.length) throw new LzmaError("Truncated LZMA2 data");
            const size = src.readUInt16BE(pos) + 1;
            pos += 2;
            if (pos + size > src.length) throw new LzmaError("Truncated LZMA2 data");
            out.ensure(size);
            src.copy(out.buf, out.pos, pos, pos + size);
            out.pos += size;
            pos += size;
        } else {
            throw new LzmaError("Corrupt LZMA2 data (control byte)");
        }
    }
}

const XZ_MAGIC = Buffer.from([0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]);

function checkSize(type: number): number {
    // Sizes of the integrity check fields; 0 = none, 1 = CRC32, 4 = CRC64, 10 = SHA-256.
    switch (type) {
        case 0:
            return 0;
        case 1:
            return 4;
        case 4:
            return 8;
        case 10:
            return 32;
        default:
            throw new LzmaError("Unsupported .xz integrity check");
    }
}

function readVarint(src: Buffer, pos: number): { value: number; next: number } {
    let value = 0;
    for (let i = 0; i < 9; i++) {
        if (pos + i >= src.length) throw new LzmaError("Truncated .xz data");
        const b = src[pos + i];
        value += (b & 0x7f) * 2 ** (7 * i);
        if ((b & 0x80) === 0) return { value, next: pos + i + 1 };
    }
    throw new LzmaError("Corrupt .xz data (number)");
}

/** A `.xz` file: stream header, blocks that use only the LZMA2 filter, then the index and footer (not read). */
export function decodeXz(src: Buffer, maxOutput = DEFAULT_MAX_OUTPUT): Buffer {
    if (src.length < 12 || !src.subarray(0, 6).equals(XZ_MAGIC)) throw new LzmaError("Not an .xz stream");
    const check = checkSize(src[7] & 0x0f);
    const out = new Output(1 << 20, maxOutput);
    let pos = 12;
    let blocks = 0;
    while (pos < src.length) {
        const sizeByte = src[pos];
        if (sizeByte === 0) break; // The index starts here.
        const headerSize = (sizeByte + 1) * 4;
        if (pos + headerSize > src.length) throw new LzmaError("Truncated .xz data");
        const flags = src[pos + 1];
        let p = pos + 2;
        if (flags & 0x40) p = readVarint(src, p).next;
        if (flags & 0x80) p = readVarint(src, p).next;
        const filters = (flags & 0x03) + 1;
        if (filters !== 1) throw new LzmaError("Unsupported .xz filter chain");
        const id = readVarint(src, p);
        if (id.value !== 0x21) throw new LzmaError("Unsupported .xz filter");
        pos += headerSize;
        pos = decodeLzma2(src, pos, out);
        pos += (4 - (pos % 4)) % 4; // Block padding (blocks start 4-byte aligned).
        pos += check;
        blocks++;
    }
    if (blocks === 0) throw new LzmaError("Empty .xz stream");
    return out.result();
}

/** What liblzma's `lzma_auto_decoder` accepts: `.xz` (starts with its magic) or the old `.lzma`. */
export function decodeLzmaAuto(src: Buffer, maxOutput = DEFAULT_MAX_OUTPUT): Buffer {
    if (src.length >= 6 && src.subarray(0, 6).equals(XZ_MAGIC)) return decodeXz(src, maxOutput);
    return decodeLzmaAlone(src, maxOutput);
}
