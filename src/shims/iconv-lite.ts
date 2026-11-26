/**
 * A small stand-in for `iconv-lite` in the phone bundle (the `bundle` script aliases the package to this file).
 *
 * Express's body parsers (body-parser, raw-body) only use iconv-lite to turn request bodies into text, and the app's
 * clients send UTF-8 JSON. The real package adds ~0.5 MB of code-page tables to a bundle the app ships and copies out on
 * every start. This covers the encodings Node decodes itself (UTF-8, UTF-16LE, Latin-1 and ASCII) with iconv-lite's
 * results, BOM stripping included. Any other charset gets iconv-lite's own "Encoding not recognized" error, which the
 * body parsers answer with 415 Unsupported Media Type. The non-bundled server (`npm start`) still uses the real package.
 */
import { StringDecoder } from "node:string_decoder";

type Kind = "utf8" | "utf16le" | "latin1" | "ascii";

/** Names as iconv-lite canonicalizes them (see [canonical]), for the encodings above. */
const NAMES: Record<string, Kind> = {
    utf8: "utf8",
    unicode11utf8: "utf8",
    ucs2: "utf16le",
    utf16le: "utf16le",
    binary: "latin1",
    latin1: "latin1",
    iso88591: "latin1",
    l1: "latin1",
    cp28591: "latin1",
    "28591": "latin1",
    ascii: "ascii",
    usascii: "ascii",
    csascii: "ascii",
    ascii8bit: "ascii",
    ansix34: "ascii",
    ansix341968: "ascii",
    ansix341986: "ascii",
};

/** iconv-lite's lookup key: lower case, letters and digits only, without a ":1990"-style year suffix. */
function canonical(encoding: string): string {
    return `${encoding}`.toLowerCase().replace(/:\d{4}$|[^0-9a-z]/g, "");
}

function kindOf(encoding: string): Kind {
    const key = canonical(encoding);
    const kind = Object.hasOwn(NAMES, key) ? NAMES[key] : undefined;
    // Same message as iconv-lite: raw-body recognizes it (/^Encoding not recognized: /) and answers 415.
    if (!kind) throw new Error(`Encoding not recognized: '${encoding}' (searched as: '${key}')`);
    return kind;
}

export function encodingExists(encoding: string): boolean {
    try {
        kindOf(encoding);
        return true;
    } catch {
        return false;
    }
}

export interface Decoder {
    write(buf: Buffer): string;
    end(): string | undefined;
}

/** A streaming decoder: characters split across chunks come out whole, and a leading BOM is dropped (UTF-8/16). */
export function getDecoder(encoding: string): Decoder {
    const kind = kindOf(encoding);
    const inner = new StringDecoder(kind === "ascii" ? "latin1" : kind);
    const bomAware = kind === "utf8" || kind === "utf16le";
    let started = false;
    const finish = (text: string): string => {
        // iconv-lite's ASCII table maps bytes above 0x7F to U+FFFD (Node's own "ascii" would drop the high bit).
        if (kind === "ascii") text = text.replace(/[\u0080-ÿ]/g, "�");
        if (!started && text.length > 0) {
            started = true;
            if (bomAware && text.charCodeAt(0) === 0xfeff) text = text.slice(1);
        }
        return text;
    };
    return {
        write: (buf) => finish(inner.write(buf)),
        end: () => finish(inner.end()) || undefined,
    };
}

export function decode(buf: Buffer, encoding: string): string {
    const decoder = getDecoder(encoding);
    return decoder.write(buf) + (decoder.end() ?? "");
}
