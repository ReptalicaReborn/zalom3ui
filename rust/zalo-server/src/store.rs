//! Message normalization, attachment parsing, OTP detection, Vietnamese diacritic-insensitive
//! search, and crash-safe per-thread persistence (`ChatStore`), matching `src/store.ts`.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex_lite::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::log::create_dir_private;
use crate::types::{
    is_local_device_uri, reaction_by_code, sanitize_remote_url, thread_key, Attachment,
    AttachmentKind, CallInfo, ChatMessage, MentionInfo, MessageSearchHit, MessageStatus, PollRef,
    QuoteInfo, RawRef, ReactionInfo, SearchCache, StickerRef, StoredMessage, ThreadInfo,
    ThreadKind, ThreadSummary, STICKER_IMAGE_URL,
};

pub const MAX_MESSAGES_PER_THREAD: usize = 500;
/// Per-chat limit once a history import has run: the phone's older messages would otherwise be evicted at once.
pub const HISTORY_MESSAGES_PER_THREAD: usize = 2000;

const META_FILE: &str = "meta.json";
/// Most "deleted for me" message ids remembered per thread.
const MAX_TOMBSTONES_PER_THREAD: usize = 1000;
const THREADS_DIR: &str = "threads";
const SAVE_DEBOUNCE: Duration = Duration::from_secs(2);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---- Vietnamese & Latin diacritic folding ----

/// Maps a single Unicode character to its ASCII base letter if it is a precomposed Latin or
/// Vietnamese diacritic character. Returns `None` for combining marks (which should be stripped)
/// or `Some(c)` for characters to emit.
fn fold_char(ch: char) -> Option<char> {
    let code = ch as u32;
    // Combining Diacritical Marks (NFD marks) -> strip completely.
    if (0x0300..=0x036F).contains(&code) {
        return None;
    }
    if ch.is_ascii() {
        return Some(ch);
    }
    // Vietnamese đ / Đ (matches JS `.replace(/[đĐ]/g, "d")`)
    if ch == 'đ' || ch == 'Đ' {
        return Some('d');
    }
    // Latin Extended Additional — Vietnamese double-diacritic vowels (U+1EA0..=U+1EF9)
    let mapped = match code {
        0x1EA0..=0x1EB7 => Some(if code % 2 == 0 { 'A' } else { 'a' }),
        0x1EB8..=0x1EC7 => Some(if code % 2 == 0 { 'E' } else { 'e' }),
        0x1EC8..=0x1ECB => Some(if code % 2 == 0 { 'I' } else { 'i' }),
        0x1ECC..=0x1EE3 => Some(if code % 2 == 0 { 'O' } else { 'o' }),
        0x1EE4..=0x1EF1 => Some(if code % 2 == 0 { 'U' } else { 'u' }),
        0x1EF2..=0x1EF9 => Some(if code % 2 == 0 { 'Y' } else { 'y' }),
        // Latin-1 Supplement (U+00C0..=U+00FF)
        0x00C0..=0x00C5 => Some('A'),
        0x00C7 => Some('C'),
        0x00C8..=0x00CB => Some('E'),
        0x00CC..=0x00CF => Some('I'),
        0x00D1 => Some('N'),
        0x00D2..=0x00D6 => Some('O'),
        0x00D9..=0x00DC => Some('U'),
        0x00DD => Some('Y'),
        0x00E0..=0x00E5 => Some('a'),
        0x00E7 => Some('c'),
        0x00E8..=0x00EB => Some('e'),
        0x00EC..=0x00EF => Some('i'),
        0x00F1 => Some('n'),
        0x00F2..=0x00F6 => Some('o'),
        0x00F9..=0x00FC => Some('u'),
        0x00FD | 0x00FF => Some('y'),
        // Latin Extended-A (U+0100..=U+017F)
        0x0100 | 0x0102 | 0x0104 => Some('A'),
        0x0101 | 0x0103 | 0x0105 => Some('a'),
        0x0106 | 0x0108 | 0x010A | 0x010C => Some('C'),
        0x0107 | 0x0109 | 0x010B | 0x010D => Some('c'),
        0x010E => Some('D'),
        0x010F => Some('d'),
        0x0112 | 0x0114 | 0x0116 | 0x0118 | 0x011A => Some('E'),
        0x0113 | 0x0115 | 0x0117 | 0x0119 | 0x011B => Some('e'),
        0x011C | 0x011E | 0x0120 | 0x0122 => Some('G'),
        0x011D | 0x011F | 0x0121 | 0x0123 => Some('g'),
        0x0124 => Some('H'),
        0x0125 => Some('h'),
        0x0128 | 0x012A | 0x012C | 0x012E | 0x0130 => Some('I'),
        0x0129 | 0x012B | 0x012D | 0x012F => Some('i'),
        0x0134 => Some('J'),
        0x0135 => Some('j'),
        0x0136 => Some('K'),
        0x0137 => Some('k'),
        0x0139 | 0x013B | 0x013D => Some('L'),
        0x013A | 0x013C | 0x013E => Some('l'),
        0x0143 | 0x0145 | 0x0147 => Some('N'),
        0x0144 | 0x0146 | 0x0148 => Some('n'),
        0x014C | 0x014E | 0x0150 => Some('O'),
        0x014D | 0x014F | 0x0151 => Some('o'),
        0x0154 | 0x0156 | 0x0158 => Some('R'),
        0x0155 | 0x0157 | 0x0159 => Some('r'),
        0x015A | 0x015C | 0x015E | 0x0160 => Some('S'),
        0x015B | 0x015D | 0x015F | 0x0161 => Some('s'),
        0x0162 | 0x0164 => Some('T'),
        0x0163 | 0x0165 => Some('t'),
        0x0168 | 0x016A | 0x016C | 0x016E | 0x0170 | 0x0172 => Some('U'),
        0x0169 | 0x016B | 0x016D | 0x016F | 0x0171 | 0x0173 => Some('u'),
        0x0174 => Some('W'),
        0x0175 => Some('w'),
        0x0176 | 0x0178 => Some('Y'),
        0x0177 => Some('y'),
        0x0179 | 0x017B | 0x017D => Some('Z'),
        0x017A | 0x017C | 0x017E => Some('z'),
        // Latin Extended-B (Vietnamese Ơ/ơ, Ư/ư and common caron/diaeresis vowels)
        0x01A0 => Some('O'),
        0x01A1 => Some('o'),
        0x01AF => Some('U'),
        0x01B0 => Some('u'),
        0x01CD => Some('A'),
        0x01CE => Some('a'),
        0x01CF => Some('I'),
        0x01D0 => Some('i'),
        0x01D1 => Some('O'),
        0x01D2 => Some('o'),
        0x01D3 | 0x01D5 | 0x01D7 | 0x01D9 | 0x01DB => Some('U'),
        0x01D4 | 0x01D6 | 0x01D8 | 0x01DA | 0x01DC => Some('u'),
        _ => None,
    };
    Some(mapped.unwrap_or(ch))
}

/// Strips combining marks and folds Vietnamese / Latin diacritics (including `đ`/`Đ` -> `d`).
pub fn fold_diacritics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if let Some(c) = fold_char(ch) {
            out.push(c);
        }
    }
    out
}

/// Folds text for diacritic-insensitive, case-insensitive Vietnamese search in a single pass.
pub fn fold_for_search(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if let Some(c) = fold_char(ch) {
            if c.is_ascii() {
                out.push(c.to_ascii_lowercase());
            } else {
                for lc in c.to_lowercase() {
                    out.push(lc);
                }
            }
        }
    }
    out
}

// ---- OTP extraction ----
//
// The same algorithm (and the same test vectors) lives in Kotlin: `extractOtpCode` in
// `ui/MessageContent.kt`. Keep them in step. It works on diacritic-folded lowercase text, one line
// at a time, and never uses look-around (the `regex-lite` crate has none).
//
//  1. a strong keyword (OTP, "ma xac thuc", "verification code", ...) followed, in the same
//     sentence and within 100 characters, by the first *code-like* number;
//  2. a weak keyword that must be directly followed by the code ("code: 123456", "your code is
//     123456", "ma: 123456", "ma cua ban la 1234");
//  3. a code-like number directly before a keyword ("739201 la ma xac nhan", "123456 is your
//     verification code", "G-123456 is your Google verification code");
//  4. a line that is only a code, right under a line with a strong keyword.
//
// A code-like number is 4-8 plain digits ("482910") or two groups of three ("482 910", "482-910",
// returned as "482910"). Amounts ("5.000.000", "5000d"), dates, times, 9+ digit numbers (phone and
// account numbers), numbers after tk/stk/sdt/hotline/..., and a year after "nam"/"year" are not.

const OTP_STRONG: &str = r"\b(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich|pin)|verification\s*code|security\s*code|auth(?:entication)?\s*code|confirm(?:ation)?\s*code|passcode|mat\s*khau\s*(?:dung\s*)?mot\s*lan|one[- ]time(?:\s*(?:code|password|pin))?)\b";
const OTP_WEAK: &str = r"(?:^|[.!?:;,]\s*|\byour\s+|\bur\s+|\bsms\s+|\bthe\s+)code\s*(?::|\bis\b)\s*(?:g-)?|\bma\s*:\s*(?:g-)?|\bma\s+cua\s+(?:ban|quy\s+khach|minh)\s*(?:\bla\b|:)\s*(?:g-)?";
const OTP_BEFORE: &str = r"\b(?:(?:la|is)\s+(?:your|the)\s+(?:[a-z]+\s+)?(?:(?:verification|security|login|confirmation|authentication)\s+)?code|(?:la\s+)?(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich)|verification\s*code|security\s*code|passcode)|la\s+ma\s+cua\s+ban)\b";
const OTP_TOKEN: &str = r"[0-9]+(?:[.,/:\-][0-9]+)*";
const OTP_UNIT_AFTER: &str = r"^\s*(?:d|vnd|dong|usd|eur|k|tr|trieu|nghin|ngan|ty|%)(?:[^a-z0-9]|$)";
const OTP_ACCOUNT_BEFORE: &str = r"\b(?:tk|stk|sdt|hotline|tel|phone|account|acc|a/c|card|cmnd|cccd|mst|tai\s+khoan|so\s+tk|so\s+the)\s*(?:so\b|no\b|number\b)?\s*[:#.]?\s*$";
const OTP_YEAR_BEFORE: &str = r"\b(?:nam|year)\s*$";
const OTP_SENTENCE_END: &str = r"[.!?](?:\s|$)";
const OTP_WINDOW: usize = 100;
const OTP_BEFORE_WINDOW: usize = 24;

fn otp_re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid OTP regex"))
}

fn otp_strong_re() -> &'static Regex {
    static STRONG: OnceLock<Regex> = OnceLock::new();
    otp_re(&STRONG, OTP_STRONG)
}

/// A numeric token (`[0-9]+` with `.,/:-` joins) in a folded line.
struct OtpToken {
    start: usize,
    end: usize,
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The code a token stands for, or `None` if it is not code-like (see the section comment).
fn otp_candidate(line: &str, toks: &[OtpToken], i: usize) -> Option<String> {
    static UNIT: OnceLock<Regex> = OnceLock::new();
    static ACCOUNT: OnceLock<Regex> = OnceLock::new();
    static YEAR: OnceLock<Regex> = OnceLock::new();
    let t = &toks[i];
    let text = &line[t.start..t.end];
    let prefix = &line[..t.start];
    let bytes = line.as_bytes();

    // Glued to letters ("abc1234", "1234abc"): not a standalone code.
    if t.start > 0 && bytes[t.start - 1].is_ascii_alphabetic() {
        return None;
    }
    if t.end < line.len() && bytes[t.end].is_ascii_alphabetic() {
        return None;
    }
    if prefix.ends_with('$') {
        return None;
    }

    let code = if all_digits(text) && (4..=8).contains(&text.len()) {
        text.to_string()
    } else if text.len() == 7 && text.as_bytes()[3] == b'-' && all_digits(&text.replace('-', "")) && text.split('-').all(|g| g.len() == 3) {
        text.replace('-', "")
    } else if all_digits(text) && text.len() == 3 {
        // "482 910": two groups of three, one space apart, and not part of a longer chain.
        let next = toks.get(i + 1)?;
        let gap_ok = next.start == t.end + 1 && bytes[t.end] == b' ';
        let next_text = &line[next.start..next.end];
        if !(gap_ok && all_digits(next_text) && next_text.len() == 3) {
            return None;
        }
        let chained_before = i > 0 && {
            let p = &toks[i - 1];
            p.end + 1 == t.start && bytes[p.end] == b' ' && all_digits(&line[p.start..p.end]) && p.end - p.start <= 3
        };
        let chained_after = toks.get(i + 2).is_some_and(|n| {
            n.start == next.end + 1 && bytes[next.end] == b' ' && all_digits(&line[n.start..n.end]) && n.end - n.start == 3
        });
        if chained_before || chained_after {
            return None;
        }
        format!("{text}{next_text}")
    } else {
        return None;
    };

    // The token that ends the code (the second group for "482 910").
    let end = if text.len() == 3 && all_digits(text) { toks[i + 1].end } else { t.end };
    if end < line.len() && bytes[end].is_ascii_alphabetic() {
        return None;
    }
    if end < line.len() && matches!(bytes[end], b'.' | b',') && end + 1 < line.len() && bytes[end + 1].is_ascii_digit() {
        return None;
    }
    if otp_re(&UNIT, OTP_UNIT_AFTER).is_match(&line[end..]) {
        return None;
    }
    if otp_re(&ACCOUNT, OTP_ACCOUNT_BEFORE).is_match(prefix) {
        return None;
    }
    if code.len() == 4
        && (code.starts_with("19") || code.starts_with("20"))
        && otp_re(&YEAR, OTP_YEAR_BEFORE).is_match(prefix)
    {
        return None;
    }
    Some(code)
}

fn otp_clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut e = max;
    while !s.is_char_boundary(e) {
        e -= 1;
    }
    &s[..e]
}

fn otp_in_line(line: &str) -> Option<String> {
    static WEAK: OnceLock<Regex> = OnceLock::new();
    static BEFORE: OnceLock<Regex> = OnceLock::new();
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    static SENTENCE: OnceLock<Regex> = OnceLock::new();
    let strong = otp_strong_re();
    let weak = otp_re(&WEAK, OTP_WEAK);
    let before = otp_re(&BEFORE, OTP_BEFORE);
    let sentence = otp_re(&SENTENCE, OTP_SENTENCE_END);
    let toks: Vec<OtpToken> = otp_re(&TOKEN, OTP_TOKEN)
        .find_iter(line)
        .map(|m| OtpToken { start: m.start(), end: m.end() })
        .collect();
    if toks.is_empty() {
        return None;
    }

    // 1. strong keyword, then the first code-like number in the same sentence.
    for m in strong.find_iter(line) {
        let limit = otp_clip(&line[m.end()..], OTP_WINDOW).len() + m.end();
        for (i, t) in toks.iter().enumerate() {
            if t.start < m.end() || t.start > limit {
                continue;
            }
            if sentence.is_match(&line[m.end()..t.start]) {
                break;
            }
            if let Some(code) = otp_candidate(line, &toks, i) {
                return Some(code);
            }
        }
    }

    // 2. weak keyword directly followed by the code.
    for m in weak.find_iter(line) {
        if let Some(i) = toks.iter().position(|t| t.start == m.end()) {
            if let Some(code) = otp_candidate(line, &toks, i) {
                return Some(code);
            }
        }
    }

    // 3. a code directly before a keyword.
    for m in before.find_iter(line) {
        let Some(i) = toks.iter().rposition(|t| t.end <= m.start()) else {
            continue;
        };
        // "482 910 la ..." ends on the second group; step back to the first.
        let first = if i > 0 && toks[i].end - toks[i].start == 3 && toks[i - 1].end + 1 == toks[i].start { i - 1 } else { i };
        let t = &toks[i];
        let gap = &line[t.end..m.start()];
        if gap.chars().count() > OTP_BEFORE_WINDOW || sentence.is_match(gap) {
            continue;
        }
        if let Some(code) = otp_candidate(line, &toks, first) {
            return Some(code);
        }
    }
    None
}

/// Extracts a one-time code (4-8 digits) from message or notification text; see the section
/// comment for the rules. Returns the digits only ("482 910" -> "482910").
pub fn extract_otp_code(text: &str) -> Option<String> {
    if text.bytes().filter(|b| b.is_ascii_digit()).take(3).count() < 3 {
        return None;
    }
    let folded = fold_for_search(text);
    let strong = otp_strong_re();
    let mut prev_strong = false;
    for line in folded.lines() {
        let line = line.trim();
        if let Some(code) = otp_in_line(line) {
            return Some(code);
        }
        // A line that is only a code, under a line with a strong keyword.
        if prev_strong {
            let toks = [OtpToken { start: 0, end: line.len() }];
            if all_digits(line) {
                if let Some(code) = otp_candidate(line, &toks, 0) {
                    return Some(code);
                }
            }
        }
        prev_strong = strong.is_match(line);
    }
    None
}

// ---- Message normalization & attachment parsing ----

fn kind_by_msg_type(msg_type: &str) -> AttachmentKind {
    match msg_type {
        "chat.photo" => AttachmentKind::Photo,
        "chat.sticker" => AttachmentKind::Sticker,
        "chat.gif" => AttachmentKind::Gif,
        "chat.video.msg" => AttachmentKind::Video,
        "chat.voice" => AttachmentKind::Voice,
        "share.file" => AttachmentKind::File,
        "chat.link" => AttachmentKind::Link,
        "chat.location.new" => AttachmentKind::Location,
        _ => AttachmentKind::Other,
    }
}

fn msg_type_label(msg_type: &str) -> Option<&'static str> {
    match msg_type {
        "chat.photo" => Some("[Photo]"),
        "chat.video.msg" => Some("[Video]"),
        "chat.voice" => Some("[Voice message]"),
        "chat.sticker" => Some("[Sticker]"),
        "chat.gif" => Some("[GIF]"),
        "share.file" => Some("[File]"),
        "chat.recommended" | "chat.link" => Some("[Link]"),
        "chat.location.new" => Some("[Location]"),
        "chat.doodle" => Some("[Drawing]"),
        "chat.ecard" | "chat.list.action" | "chat.webcontent" | "chat.webcontent.v2" => {
            Some("[Card]")
        }
        "chat.todo" => Some("[Task]"),
        "chat.poll" | "group.poll" => Some("[Poll]"),
        _ => None,
    }
}

fn non_empty_str(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn val_to_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn val_to_i64(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| u as i64))
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
            .unwrap_or(0),
        Some(Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(|f| f as i64)
            .unwrap_or(0),
        _ => 0,
    }
}

fn first_pos_f64(values: &[Option<&Value>]) -> Option<f64> {
    for v in values {
        let n = match v {
            Some(Value::Number(num)) => num.as_f64(),
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().parse::<f64>().ok(),
            _ => None,
        };
        if let Some(x) = n {
            if x.is_finite() && x > 0.0 {
                return Some(x);
            }
        }
    }
    None
}

fn first_pos_i64(values: &[Option<&Value>]) -> Option<i64> {
    first_pos_f64(values).map(|f| f as i64)
}

fn first_pos_u32(values: &[Option<&Value>]) -> Option<u32> {
    first_pos_f64(values).and_then(|f| {
        if f <= u32::MAX as f64 {
            Some(f as u32)
        } else {
            None
        }
    })
}

fn first_pos_u64(values: &[Option<&Value>]) -> Option<u64> {
    first_pos_f64(values).map(|f| f as u64)
}

fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn parse_json_obj(v: Option<&Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m.clone(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.starts_with('{') {
                if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(trimmed) {
                    return m;
                }
            }
            Map::new()
        }
        _ => Map::new(),
    }
}

fn web_url(v: Option<&Value>) -> Option<String> {
    non_empty_str(v).and_then(|s| sanitize_remote_url(&s))
}

fn remote_str(v: Option<&Value>) -> Option<String> {
    non_empty_str(v).filter(|s| !is_local_device_uri(s))
}

/// The message id a raw recall ("undo") record points at, if it is one.
pub fn recalled_message_id(raw_data: &Value) -> Option<String> {
    let content = raw_data.get("content")?.as_object()?;
    if content.contains_key("deleteMsg") {
        let id = val_to_string(content.get("globalMsgId"));
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

/// A recall record (`content.deleteMsg`) that names no target message id: it can only become a
/// stray placeholder, so ingest drops it.
pub fn raw_is_undo_without_target(raw_data: &Value) -> bool {
    let Some(content) = raw_data.get("content").and_then(|c| c.as_object()) else {
        return false;
    };
    content.contains_key("deleteMsg") && recalled_message_id(raw_data).is_none()
}

/// Parses a Zalo message's non-text attachment payload (`content` may be a JSON object or an
/// array for `chat.list.action`), matching `parseAttachment` in `src/store.ts`.
pub fn parse_attachment(msg_type: &str, content: &Value, is_self: bool) -> Option<Attachment> {
    let owned_map: Map<String, Value>;
    let content_map: &Map<String, Value> = match content {
        Value::Object(m) => m,
        Value::Array(arr) => {
            let mut m = Map::new();
            m.insert("items".to_string(), Value::Array(arr.clone()));
            owned_map = m;
            &owned_map
        }
        _ => return None,
    };

    let mut params = Map::new();
    if let Some(Value::String(s)) = content_map.get("params") {
        if s.starts_with('{') {
            if let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(s) {
                params = parsed;
            }
        }
    } else if let Some(Value::Object(m)) = content_map.get("params") {
        params = m.clone();
    }

    // 1. Sticker
    if msg_type == "chat.sticker" {
        let id = first_pos_i64(&[content_map.get("id")])?;
        let cat_id =
            first_pos_i64(&[content_map.get("catId"), content_map.get("cateId")]).unwrap_or(0);
        let sticker_type = first_pos_i64(&[content_map.get("type")]).unwrap_or(0) as i32;
        return Some(Attachment {
            kind: Some(AttachmentKind::Sticker),
            thumb: Some(format!("{STICKER_IMAGE_URL}{id}")),
            sticker: Some(StickerRef {
                id,
                cat_id,
                sticker_type,
            }),
            ..Default::default()
        });
    }

    // 2. Poll
    let has_poll_key = content_map.get("poll_id").is_some_and(|v| !v.is_null())
        || params.get("pollId").is_some_and(|v| !v.is_null());
    if msg_type == "group.poll" || msg_type == "chat.poll" || has_poll_key {
        let poll_id = first_pos_i64(&[
            content_map.get("poll_id"),
            content_map.get("pollId"),
            params.get("pollId"),
            params.get("poll_id"),
        ])?;
        let question = non_empty_str(content_map.get("question"))
            .or_else(|| non_empty_str(params.get("question")))
            .or_else(|| non_empty_str(content_map.get("title")))
            .or_else(|| non_empty_str(params.get("title")))
            .unwrap_or_else(|| "Poll".to_string());
        let act = non_empty_str(content_map.get("action"));
        let event = act.filter(|a| a != "create" && a != "group.poll" && a != "chat.poll");
        return Some(Attachment {
            kind: Some(AttachmentKind::Poll),
            title: Some(question.clone()),
            poll: Some(PollRef {
                poll_id,
                question,
                event,
            }),
            ..Default::default()
        });
    }

    // 3. Call log (`recommened.misscall|calling|receivecall|calltime` or `call.voice`/`call.video`)
    let action = non_empty_str(content_map.get("action")).unwrap_or_default();
    let is_call_action = matches!(
        action.as_str(),
        "recommened.misscall"
            | "recommened.calling"
            | "recommened.receivecall"
            | "recommened.calltime"
    );
    let is_call_msg_type = msg_type == "call.voice" || msg_type == "call.video";
    if is_call_action || is_call_msg_type {
        let reason = val_to_i64(params.get("reason"));
        let seconds = first_pos_f64(&[params.get("duration"), content_map.get("duration")]);
        let missed = if !action.is_empty() {
            action.ends_with("misscall")
        } else {
            reason != 0 || seconds.is_none()
        };
        let video = msg_type == "call.video" || val_to_i64(params.get("calltype")) == 1;
        let outgoing = match params.get("isCaller") {
            Some(v) if !v.is_null() => js_truthy(v),
            _ => {
                if !action.is_empty() {
                    action.ends_with("calling")
                } else {
                    is_self
                }
            }
        };
        let what = if video { "video call" } else { "voice call" };
        let title = if outgoing {
            if !missed {
                format!("Outgoing {what}")
            } else if reason == 4 || reason == 5 {
                "You cancelled the call".to_string()
            } else if reason == 3 {
                "Call declined".to_string()
            } else if reason == 1 {
                "Busy".to_string()
            } else {
                format!("Outgoing {what}, no answer")
            }
        } else if !missed {
            format!("Incoming {what}")
        } else if reason == 3 {
            "You declined the call".to_string()
        } else {
            format!("Missed {what}")
        };
        let duration = if !missed {
            seconds.map(|s| (s * 1000.0).round() as u64)
        } else {
            None
        };
        return Some(Attachment {
            kind: Some(AttachmentKind::Call),
            title: Some(title),
            duration,
            call: Some(CallInfo {
                video,
                missed,
                outgoing,
            }),
            ..Default::default()
        });
    }

    // 4. OA list / interactive card messages (`chat.list.action`)
    if msg_type == "chat.list.action" {
        let items: Vec<&Map<String, Value>> = match content_map.get("items") {
            Some(Value::Array(arr)) => arr.iter().filter_map(|x| x.as_object()).collect(),
            _ => vec![content_map],
        };
        if !items.is_empty() {
            let lead_idx = items
                .iter()
                .position(|x| {
                    x.get("type")
                        .and_then(|v| v.as_str())
                        .is_some_and(|t| t.starts_with("l.a.header."))
                })
                .unwrap_or(0);
            let lead = items[lead_idx];
            let title = non_empty_str(lead.get("title"))
                .or_else(|| items.iter().find_map(|x| non_empty_str(x.get("title"))));
            let description = non_empty_str(lead.get("description")).or_else(|| {
                items.iter().find_map(|x| {
                    non_empty_str(x.get("description")).filter(|d| Some(d) != title.as_ref())
                })
            });
            let description = description.filter(|d| Some(d) != title.as_ref());
            let thumb = web_url(lead.get("thumb"))
                .or_else(|| items.iter().find_map(|x| web_url(x.get("thumb"))));
            let action_idx = items
                .iter()
                .enumerate()
                .find(|(idx, x)| *idx != lead_idx && web_url(x.get("href")).is_some())
                .map(|(idx, _)| idx)
                .or_else(|| items.iter().position(|x| web_url(x.get("href")).is_some()));
            let href = web_url(lead.get("href"))
                .or_else(|| action_idx.and_then(|i| web_url(items[i].get("href"))));
            let action_label = action_idx
                .filter(|&i| i != lead_idx)
                .and_then(|i| non_empty_str(items[i].get("title")));
            let otp_src: Vec<&str> = [title.as_deref(), description.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            let otp = extract_otp_code(&otp_src.join("\n"));
            if title.is_some() || description.is_some() || thumb.is_some() || href.is_some() {
                return Some(Attachment {
                    kind: Some(AttachmentKind::Link),
                    title,
                    description,
                    thumb,
                    href,
                    action_label,
                    otp,
                    ..Default::default()
                });
            }
        }
    }

    // 5. ZInstant / ZNS cards (`chat.webcontent`, `chat.webcontent.v2`)
    if msg_type == "chat.webcontent" || msg_type == "chat.webcontent.v2" {
        let z_msg = parse_json_obj(content_map.get("zinstant_msg"));
        let meta = if !z_msg.is_empty() { &z_msg } else { &params };
        let custom_obj = parse_json_obj(meta.get("customMsg").or_else(|| params.get("customMsg")));
        let msg_map = parse_json_obj(custom_obj.get("msg"));
        let mut localized = non_empty_str(msg_map.get("vi"))
            .or_else(|| non_empty_str(msg_map.get("en")))
            .unwrap_or_default();
        if !localized.is_empty() {
            if let Some(Value::Array(highlights)) = custom_obj.get("highLightsV2") {
                for (idx, h) in highlights.iter().enumerate() {
                    let ho = parse_json_obj(Some(h));
                    let replacement = non_empty_str(ho.get("text"))
                        .or_else(|| non_empty_str(ho.get("value")))
                        .or_else(|| non_empty_str(ho.get("name")));
                    if let Some(rep) = replacement {
                        let placeholder = format!("${idx}$");
                        localized = localized.replacen(&placeholder, &rep, 1);
                    }
                }
            }
        }
        let raw_actions: &[Value] = meta
            .get("actions")
            .and_then(|v| v.as_array())
            .or_else(|| params.get("actions").and_then(|v| v.as_array()))
            .or_else(|| meta.get("buttons").and_then(|v| v.as_array()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        let mut action_label = non_empty_str(meta.get("actionTitle"))
            .or_else(|| non_empty_str(params.get("actionTitle")));
        let mut href = web_url(content_map.get("href"))
            .or_else(|| web_url(meta.get("href")))
            .or_else(|| web_url(meta.get("url")))
            .or_else(|| web_url(params.get("href")))
            .or_else(|| web_url(params.get("url")));
        let mut otp_from_action: Option<String> = None;

        for entry in raw_actions {
            let act = parse_json_obj(Some(entry));
            let act_data = parse_json_obj(act.get("actionData").or_else(|| act.get("data")));
            let label = non_empty_str(act.get("actionTitle"))
                .or_else(|| non_empty_str(act.get("title")))
                .or_else(|| non_empty_str(act.get("name")));
            let url = web_url(act.get("actionUrl"))
                .or_else(|| web_url(act.get("href")))
                .or_else(|| web_url(act.get("url")))
                .or_else(|| web_url(act_data.get("url")))
                .or_else(|| web_url(act_data.get("href")))
                .or_else(|| web_url(act_data.get("link")));
            if action_label.is_none() && label.is_some() {
                action_label = label;
            }
            if href.is_none() && url.is_some() {
                href = url;
            }
            let candidate_code = non_empty_str(act_data.get("otp"))
                .or_else(|| non_empty_str(act_data.get("code")))
                .or_else(|| non_empty_str(act_data.get("copyText")))
                .or_else(|| non_empty_str(act_data.get("text")));
            if otp_from_action.is_none() {
                if let Some(code) = candidate_code {
                    let trimmed = code.trim();
                    if (4..=8).contains(&trimmed.len())
                        && trimmed.bytes().all(|b| b.is_ascii_digit())
                    {
                        otp_from_action = Some(trimmed.to_string());
                    }
                }
            }
        }

        let lines: Vec<&str> = localized
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let title = non_empty_str(content_map.get("title"))
            .or_else(|| non_empty_str(meta.get("title")))
            .or_else(|| lines.first().map(|s| (*s).to_string()));
        let body_from_localized = if lines.len() > 1 {
            Some(lines[1..].join("\n"))
        } else {
            lines.first().map(|s| (*s).to_string())
        };
        let raw_desc = non_empty_str(content_map.get("description"))
            .or_else(|| non_empty_str(meta.get("description")))
            .or(body_from_localized);
        let description = raw_desc.filter(|d| Some(d) != title.as_ref());
        let thumb = web_url(content_map.get("thumb"))
            .or_else(|| web_url(meta.get("thumb")))
            .or_else(|| web_url(meta.get("logo")))
            .or_else(|| web_url(params.get("thumb")));
        let otp = otp_from_action.or_else(|| {
            let mut parts: Vec<&str> = Vec::new();
            if let Some(t) = title.as_deref() {
                parts.push(t);
            }
            if let Some(d) = description.as_deref() {
                parts.push(d);
            }
            if !localized.is_empty() {
                parts.push(&localized);
            }
            extract_otp_code(&parts.join("\n"))
        });

        if title.is_some()
            || description.is_some()
            || thumb.is_some()
            || href.is_some()
            || action_label.is_some()
            || otp.is_some()
        {
            return Some(Attachment {
                kind: Some(AttachmentKind::Link),
                title,
                description,
                thumb,
                href,
                action_label,
                otp,
                ..Default::default()
            });
        }
    }

    // 6. E-card (`chat.ecard`)
    if msg_type == "chat.ecard" {
        let title = non_empty_str(content_map.get("title"));
        let description = non_empty_str(content_map.get("description"))
            .filter(|d| Some(d) != title.as_ref());
        let thumb =
            web_url(content_map.get("thumb")).or_else(|| web_url(content_map.get("href")));
        let href = web_url(content_map.get("href"));
        let first_action = params
            .get("actions")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .map(|v| parse_json_obj(Some(v)))
            .unwrap_or_default();
        let action_label = non_empty_str(first_action.get("name"))
            .or_else(|| non_empty_str(first_action.get("title")));
        if title.is_some() || description.is_some() || thumb.is_some() {
            return Some(Attachment {
                kind: Some(AttachmentKind::Link),
                title,
                description,
                thumb,
                href,
                action_label,
                ..Default::default()
            });
        }
    }

    // 7. Standard media, files, links, contacts, locations, generic
    let mut kind = kind_by_msg_type(msg_type);
    if msg_type == "chat.recommended" {
        kind = match action.as_str() {
            "recommened.user" => AttachmentKind::Contact,
            "recommened.link" => AttachmentKind::Link,
            _ => AttachmentKind::Other,
        };
    }

    let is_media = matches!(
        kind,
        AttachmentKind::Photo | AttachmentKind::Gif | AttachmentKind::Video
    );
    // Link previews, todos and anything unrecognised read their fields from several places.
    let embed_like = matches!(kind, AttachmentKind::Link | AttachmentKind::Other)
        || msg_type == "chat.todo"
        || msg_type == "chat.location.new";
    let raw_title = non_empty_str(content_map.get("title"));
    let mut title = raw_title.filter(|t| !(is_media && is_local_device_uri(t)));
    let mut description = non_empty_str(content_map.get("description"));
    let mut href = if is_media {
        (if kind == AttachmentKind::Photo {
            remote_str(params.get("hd"))
        } else {
            None
        })
        .or_else(|| remote_str(content_map.get("href")))
        .or_else(|| remote_str(params.get("oriUrl")))
        .or_else(|| remote_str(params.get("origUrl")))
        .or_else(|| remote_str(params.get("hdUrl")))
        .or_else(|| remote_str(params.get("normalUrl")))
        .or_else(|| remote_str(params.get("rawUrl")))
        .or_else(|| remote_str(params.get("url")))
    } else {
        non_empty_str(content_map.get("href"))
    };
    let mut thumb = if is_media {
        remote_str(content_map.get("thumb"))
            .or_else(|| remote_str(params.get("thumbUrl")))
            .or_else(|| remote_str(params.get("thumb")))
    } else {
        non_empty_str(content_map.get("thumb"))
    };
    let mut domain: Option<String> = None;
    if embed_like {
        title = title.or_else(|| first_str(content_map, &params, EMBED_TITLE_KEYS));
        description = description
            .or_else(|| first_str(content_map, &params, EMBED_DESC_KEYS))
            .filter(|d| Some(d) != title.as_ref());
        href = href.or_else(|| first_url(content_map, &params, EMBED_HREF_KEYS));
        if msg_type == "chat.location.new" && href.is_none() {
            let lat = first_pos_or_neg_f64(params.get("latitude").or_else(|| params.get("lat")))
                .or_else(|| first_pos_or_neg_f64(content_map.get("latitude")));
            let lng = first_pos_or_neg_f64(params.get("longitude").or_else(|| params.get("lng")))
                .or_else(|| first_pos_or_neg_f64(content_map.get("longitude")));
            if let (Some(lat), Some(lng)) = (lat, lng) {
                href = Some(format!("https://maps.google.com/?q={lat},{lng}"));
            }
        }
        thumb = thumb
            .filter(|t| sanitize_remote_url(t).is_some())
            .or_else(|| first_url(content_map, &params, EMBED_THUMB_KEYS));
        domain = embed_domain(&params, href.as_deref());
    }
    let width = first_pos_u32(&[params.get("width"), params.get("video_width"), params.get("w")]);
    let height =
        first_pos_u32(&[params.get("height"), params.get("video_height"), params.get("h")]);
    let duration = first_pos_u64(&[params.get("duration")]);
    let size = first_pos_u64(&[params.get("fileSize"), params.get("size")]);
    let mut file_ext = non_empty_str(params.get("fileExt")).map(|s| s.to_ascii_lowercase());
    let mut user_id = None;
    let mut phone = None;

    if kind == AttachmentKind::Contact {
        // `params` is either the bare user id, or a JSON object (zca-js style) with the user id
        // and sometimes a phone number. The key names in the object form are best guesses.
        match content_map.get("params") {
            Some(Value::String(p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                user_id = Some(p.clone());
            }
            _ => {
                user_id = ["user_id", "userId", "uid", "contactUid", "id"]
                    .iter()
                    .find_map(|k| {
                        let v = val_to_string(params.get(*k));
                        (!v.is_empty()).then_some(v)
                    });
            }
        }
        phone = ["phone", "mobile", "phoneNumber", "phone_number", "sdt"]
            .iter()
            .find_map(|k| non_empty_str(params.get(*k)).or_else(|| non_empty_str(content_map.get(*k))))
            .filter(|p| looks_like_phone(p))
            .or_else(|| description.as_deref().filter(|d| looks_like_phone(d)).map(str::to_string))
            .or_else(|| href.as_deref().and_then(phone_from_zalo_me_url));
    }
    if kind == AttachmentKind::File && file_ext.is_none() {
        if let Some(t) = title.as_deref() {
            if let Some((_, ext)) = t.rsplit_once('.') {
                if !ext.is_empty() {
                    file_ext = Some(ext.to_ascii_lowercase());
                }
            }
        }
    }

    // Anything that is neither a known kind nor plain text is shown as a generic embed card.
    if kind == AttachmentKind::Other {
        kind = AttachmentKind::Embed;
    }
    let shows =
        title.is_some() || href.is_some() || thumb.is_some() || description.is_some();
    if shows || kind != AttachmentKind::Embed {
        Some(Attachment {
            kind: Some(kind),
            title,
            description,
            href,
            thumb,
            width,
            height,
            duration,
            size,
            file_ext,
            user_id,
            domain,
            phone,
            ..Default::default()
        })
    } else {
        None
    }
}

const EMBED_TITLE_KEYS: &[&str] = &["mediaTitle", "title", "name", "headline", "caption"];
const EMBED_DESC_KEYS: &[&str] = &["description", "desc", "mediaDesc", "content", "summary", "note"];
const EMBED_HREF_KEYS: &[&str] = &["href", "url", "link", "src", "redirect_url", "redirectUrl", "actionUrl"];
const EMBED_THUMB_KEYS: &[&str] = &["thumb", "thumbUrl", "thumb_url", "image", "mediaImage", "icon", "banner"];

/// First non-empty string among `keys`, looked up in the message content and then in `params`.
fn first_str(content: &Map<String, Value>, params: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        non_empty_str(content.get(*k))
            .or_else(|| non_empty_str(params.get(*k)))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// Like [`first_str`] but only http(s) URLs qualify.
fn first_url(content: &Map<String, Value>, params: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| web_url(content.get(*k)).or_else(|| web_url(params.get(*k))))
}

fn first_pos_or_neg_f64(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

/// Host of a URL without `www.`, for the "source" line of an embed.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    let host = host.split(':').next()?.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (!host.is_empty()).then_some(host)
}

fn embed_domain(params: &Map<String, Value>, href: Option<&str>) -> Option<String> {
    let explicit = ["domain", "src", "source", "provider"]
        .iter()
        .find_map(|k| non_empty_str(params.get(*k)))
        .map(|s| host_of(&s).unwrap_or(s))
        .filter(|s| !s.contains(' ') && s.len() <= 80);
    explicit.or_else(|| href.and_then(host_of))
}

fn looks_like_phone(s: &str) -> bool {
    let t = s.trim();
    let digits = t.chars().filter(char::is_ascii_digit).count();
    (8..=15).contains(&digits)
        && t.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '.' | '(' | ')'))
}

/// `https://zalo.me/84901234567` style profile links carry the phone number (guess).
fn phone_from_zalo_me_url(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let (host, path) = rest.split_once('/')?;
    if host.trim_start_matches("www.") != "zalo.me" {
        return None;
    }
    let seg = path.split(['/', '?', '#']).next()?;
    (seg.len() >= 8 && seg.len() <= 15 && seg.bytes().all(|b| b.is_ascii_digit()))
        .then(|| seg.to_string())
}

/// Whether a raw message is a recall ("undo") record: zca-js (`listen.js`) treats a 501/521 item
/// whose `content` object has a `deleteMsg` key as an `Undo` event. No other shape is guessed;
/// text content is never taken as a deletion marker (someone can type "Tin nhắn đã bị xóa").
pub fn is_deleted_payload(_msg_type: &str, content: &Value) -> bool {
    match content {
        Value::Object(m) => m.contains_key("deleteMsg"),
        _ => false,
    }
}

/// Keys retained inside `RawRef.content` for non-text/attachment messages (the exact subset read
/// by `prepare_qmsg`, `prepare_qmsg_attach`, and `forward_message`, dropping bulky card/layout
/// metadata arrays and tracking blobs).
const RAW_CONTENT_KEEP_KEYS: &[&str] = &[
    "href",
    "thumb",
    "title",
    "description",
    "params",
    "action",
    "id",
    "catId",
    "cateId",
    "type",
    "childnumber",
    "properties",
];

/// Returns `Some(Box<Value>)` only when `is_text` is true (only string-content quotes inspect
/// `property_ext`) and the value is non-null, non-empty.
fn slim_property_ext(is_text: bool, raw_prop: Option<&Value>) -> Option<Box<Value>> {
    if !is_text {
        return None;
    }
    let val = raw_prop?;
    match val {
        Value::Null => None,
        Value::Object(m) if m.is_empty() => None,
        Value::String(s) if s.is_empty() => None,
        other => Some(Box::new(other.clone())),
    }
}

/// Slims `raw.content` for storage:
/// - Plain-text string content -> `None` (already stored verbatim in `StoredMessage.text`).
/// - `chat.todo` -> retains only `"params"` (read by `prepare_qmsg`).
/// - Other object attachments -> retains only `RAW_CONTENT_KEEP_KEYS` with non-null values.
/// - Non-string/non-object -> empty object `{}` so `None` unambiguously means plain text.
fn slim_raw_content(msg_type: &str, content: &Value) -> Option<Box<Value>> {
    if content.is_string() {
        return None;
    }
    let mut kept = Map::new();
    if let Some(obj) = content.as_object() {
        if msg_type == "chat.todo" {
            if let Some(params) = obj.get("params").filter(|v| !v.is_null()) {
                kept.insert("params".to_string(), params.clone());
            }
        } else {
            for &key in RAW_CONTENT_KEEP_KEYS {
                if let Some(v) = obj.get(key) {
                    if v.is_null() || matches!(v, Value::String(s) if s.is_empty()) {
                        continue;
                    }
                    kept.insert(key.to_string(), v.clone());
                }
            }
        }
    }
    Some(Box::new(Value::Object(kept)))
}

/// Deduplicates and slims `StoredMessage.raw` after loading an older on-disk file into RAM.
fn slim_loaded_raw(msg: &mut StoredMessage) {
    let Some(raw) = msg.raw.as_deref_mut() else {
        return;
    };
    if raw.msg_type.as_deref() == Some(msg.msg_type.as_str()) {
        raw.msg_type = None;
    }
    if raw.uid_from.as_deref() == Some(msg.from_id.as_str()) {
        raw.uid_from = None;
    }
    if raw.id_to.as_deref() == Some(msg.thread_id.as_str()) {
        raw.id_to = None;
    }
    if raw.ts.as_deref().is_some_and(|t| t == msg.ts.to_string()) {
        raw.ts = None;
    }
    if msg.recalled {
        raw.content = None;
        raw.property_ext = None;
        return;
    }
    let is_text = match raw.content.as_deref() {
        Some(Value::String(s)) => {
            if s == &msg.text && msg.attachment.is_none() {
                raw.content = None;
            }
            true
        }
        None => msg.attachment.is_none(),
        Some(other) => {
            let msg_type = raw
                .msg_type
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(msg.msg_type.as_str());
            raw.content = slim_raw_content(msg_type, other);
            false
        }
    };
    raw.property_ext = slim_property_ext(is_text, raw.property_ext.as_deref());
}

/// Normalizes a raw Zalo message `data` payload into a `StoredMessage`, matching
/// `normalizeMessage` in `src/store.ts`.
pub fn normalize_message(
    raw_data: &Value,
    thread_kind: ThreadKind,
    thread_id: &str,
    is_self: bool,
    own_id: Option<&str>,
) -> StoredMessage {
    let raw_uid_from = val_to_string(raw_data.get("uidFrom"));
    let raw_id_to = val_to_string(raw_data.get("idTo"));
    let own = own_id.filter(|s| !s.is_empty());
    let uid_from = if raw_uid_from == "0" {
        own.map(str::to_string).unwrap_or(raw_uid_from)
    } else {
        raw_uid_from
    };
    let id_to = if raw_id_to == "0" {
        own.map(str::to_string).unwrap_or(raw_id_to)
    } else {
        raw_id_to
    };

    let msg_id = {
        let m = val_to_string(raw_data.get("msgId"));
        if !m.is_empty() {
            m
        } else {
            let g = val_to_string(raw_data.get("globalMsgId"));
            if !g.is_empty() {
                g
            } else {
                val_to_string(raw_data.get("cliMsgId"))
            }
        }
    };
    let cli_msg_id = {
        let c = val_to_string(raw_data.get("cliMsgId"));
        if !c.is_empty() {
            c
        } else {
            msg_id.clone()
        }
    };
    let msg_type = non_empty_str(raw_data.get("msgType")).unwrap_or_else(|| "webchat".to_string());
    let from_name = non_empty_str(raw_data.get("dName"))
        .or_else(|| non_empty_str(raw_data.get("oaName")))
        .unwrap_or_default();
    let ts_str = val_to_string(raw_data.get("ts"));
    let ts = val_to_i64(raw_data.get("ts"));
    let content_val = raw_data.get("content").cloned().unwrap_or(Value::Null);
    let raw_content = slim_raw_content(&msg_type, &content_val);
    let raw_property_ext = slim_property_ext(content_val.is_string(), raw_data.get("propertyExt"));

    let quote = raw_data.get("quote").and_then(|q| q.as_object()).map(|q| {
        let q_text = non_empty_str(q.get("msg")).unwrap_or_else(|| "[Attachment]".to_string());
        QuoteInfo {
            msg_id: val_to_string(q.get("globalMsgId")),
            from_id: val_to_string(q.get("ownerId")),
            from_name: non_empty_str(q.get("fromD")).unwrap_or_default(),
            text: q_text,
        }
    });

    let mentions: Vec<MentionInfo> = raw_data
        .get("mentions")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let obj = item.as_object()?;
                    Some(MentionInfo {
                        uid: val_to_string(obj.get("uid")),
                        pos: val_to_i64(obj.get("pos")) as i32,
                        len: val_to_i64(obj.get("len")) as i32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_id_to_opt = if id_to == thread_id {
        None
    } else {
        Some(id_to)
    };
    let raw_ts_opt = if ts_str == ts.to_string() {
        None
    } else {
        Some(ts_str)
    };

    let mut msg = StoredMessage {
        id: msg_id,
        cli_msg_id,
        thread_id: thread_id.to_string(),
        thread_kind,
        from_id: uid_from,
        from_name,
        is_self,
        ts,
        msg_type: msg_type.clone(),
        text: String::new(),
        attachment: None,
        quote,
        mentions,
        recalled: false,
        reactions: None,
        delivered_to: None,
        seen_by: None,
        raw: Some(Box::new(RawRef {
            msg_type: None,
            content: raw_content,
            property_ext: raw_property_ext,
            uid_from: None,
            id_to: raw_id_to_opt,
            ts: raw_ts_opt,
            ttl: val_to_i64(raw_data.get("ttl")),
            st: val_to_i64(raw_data.get("st")),
            at: val_to_i64(raw_data.get("at")),
            cmd: val_to_i64(raw_data.get("cmd")),
        })),
        search_folded: SearchCache::default(),
    };

    if is_deleted_payload(&msg_type, &content_val) {
        msg.recalled = true;
        msg.text = "Message recalled".to_string();
        msg.quote = None;
        msg.mentions = Vec::new();
        if let Some(raw) = msg.raw.as_deref_mut() {
            raw.content = None;
            raw.property_ext = None;
        }
        return msg;
    }

    if let Value::String(s) = content_val {
        msg.text = s;
        return msg;
    }

    let wrapped_content = match content_val {
        Value::Array(arr) => {
            let mut m = Map::new();
            m.insert("items".to_string(), Value::Array(arr));
            Value::Object(m)
        }
        Value::Object(m) => Value::Object(m),
        _ => Value::Object(Map::new()),
    };

    let attachment = parse_attachment(&msg_type, &wrapped_content, is_self);
    msg.attachment = attachment.clone();

    if let Some(ref att) = attachment {
        if att.kind == Some(AttachmentKind::Call) {
            msg.text = format!("[{}]", att.title.as_deref().unwrap_or("Call"));
            return msg;
        }
        if att.kind == Some(AttachmentKind::Poll) {
            msg.text = format!("[Poll] {}", att.title.as_deref().unwrap_or("Poll"));
            return msg;
        }
        if matches!(
            msg_type.as_str(),
            "chat.webcontent" | "chat.webcontent.v2" | "chat.list.action"
        ) {
            if let Some(preview) = att.title.as_deref().or(att.description.as_deref()) {
                msg.text = preview.to_string();
                return msg;
            }
        }
    }

    let known = if attachment.as_ref().and_then(|a| a.kind) == Some(AttachmentKind::Contact) {
        Some("[Contact]")
    } else {
        msg_type_label(&msg_type)
    };

    if known.is_none() {
        if let Some(title) = attachment.as_ref().and_then(|a| a.title.as_deref()) {
            msg.text = title.to_string();
            return msg;
        }
    }

    let att_kind = attachment.as_ref().and_then(|a| a.kind);
    let label = known.unwrap_or(if att_kind == Some(AttachmentKind::Embed) {
        "[Link]"
    } else {
        "[Unsupported message]"
    });
    let named_title = attachment
        .as_ref()
        .and_then(|a| a.title.as_deref())
        .filter(|_| att_kind != Some(AttachmentKind::Photo) && att_kind != Some(AttachmentKind::Gif));
    msg.text = match named_title {
        Some(t) => format!("{label} {t}"),
        None => label.to_string(),
    };
    msg
}

// ---- Reaction summary & public message projection ----

fn summarize_reactions(
    reactions: Option<&BTreeMap<String, String>>,
    own_id: Option<&str>,
) -> Vec<ReactionInfo> {
    let Some(map) = reactions else {
        return Vec::new();
    };
    let mut by_code: HashMap<&str, ReactionInfo> = HashMap::new();
    for (uid, code) in map {
        let entry = by_code.entry(code.as_str()).or_insert_with(|| {
            if let Some(def) = reaction_by_code(code) {
                ReactionInfo {
                    key: def.key.to_string(),
                    emoji: def.emoji.to_string(),
                    count: 0,
                    mine: false,
                }
            } else {
                ReactionInfo {
                    key: code.clone(),
                    emoji: code.clone(),
                    count: 0,
                    mine: false,
                }
            }
        });
        entry.count += 1;
        if own_id == Some(uid.as_str()) {
            entry.mine = true;
        }
    }
    let mut out: Vec<ReactionInfo> = by_code.into_values().collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    out
}

/// Converts a `StoredMessage` into the client-facing `ChatMessage` (stripping `raw`, `deliveredTo`,
/// and `seenBy`).
pub fn to_public_message(m: &StoredMessage, own_id: Option<&str>) -> ChatMessage {
    let reactions = summarize_reactions(m.reactions.as_ref(), own_id);
    let mentions_me = !m.is_self
        && m.mentions
            .iter()
            .any(|x| own_id == Some(x.uid.as_str()) || x.uid == "-1");
    let (status, seen_count) = if m.is_self {
        let seen_len = m.seen_by.as_ref().map_or(0, Vec::len);
        let deliv_len = m.delivered_to.as_ref().map_or(0, Vec::len);
        let st = if seen_len > 0 {
            MessageStatus::Seen
        } else if deliv_len > 0 {
            MessageStatus::Delivered
        } else {
            MessageStatus::Sent
        };
        let sc = if m.thread_kind == ThreadKind::Group {
            Some(seen_len)
        } else {
            None
        };
        (Some(st), sc)
    } else {
        (None, None)
    };

    // Incoming messages only: the card's code, else one found in the plain text.
    let otp = if m.is_self || m.recalled {
        None
    } else {
        m.attachment
            .as_ref()
            .and_then(|a| a.otp.clone())
            .or_else(|| extract_otp_code(&m.text))
    };

    ChatMessage {
        id: m.id.clone(),
        cli_msg_id: m.cli_msg_id.clone(),
        thread_id: m.thread_id.clone(),
        thread_kind: m.thread_kind,
        from_id: m.from_id.clone(),
        from_name: m.from_name.clone(),
        is_self: m.is_self,
        ts: m.ts,
        msg_type: m.msg_type.clone(),
        text: m.text.clone(),
        attachment: m.attachment.clone(),
        quote: m.quote.clone(),
        mentions: m.mentions.clone(),
        recalled: m.recalled,
        reactions,
        mentions_me,
        status,
        seen_count,
        otp,
    }
}

// ---- On-disk JSON schemas ----

fn de_map_i64_flexible<'de, D>(deserializer: D) -> Result<HashMap<String, i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<HashMap<String, Value>>::deserialize(deserializer)?;
    let mut out = HashMap::new();
    if let Some(m) = raw {
        for (k, v) in m {
            out.insert(k, val_to_i64(Some(&v)));
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetaFile {
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<u32>,
    #[serde(default)]
    threads: HashMap<String, ThreadInfo>,
    #[serde(default, deserialize_with = "de_map_i64_flexible")]
    read_up_to: HashMap<String, i64>,
    #[serde(default)]
    pinned: Vec<String>,
    #[serde(default, deserialize_with = "de_map_i64_flexible")]
    muted_until: HashMap<String, i64>,
    #[serde(default)]
    marked_unread: Vec<String>,
    #[serde(default)]
    message_cap: Option<usize>,
    /// Thread keys detected as Official Accounts (news / shops / ZNS senders).
    #[serde(default)]
    official_accounts: Vec<String>,
    /// Per-thread ids of messages deleted "for me" (tombstones, so replay can't resurrect them).
    #[serde(default)]
    deleted_ids: HashMap<String, Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetaFileRef<'a> {
    version: u32,
    threads: &'a HashMap<String, ThreadInfo>,
    read_up_to: &'a HashMap<String, i64>,
    pinned: Vec<&'a str>,
    muted_until: &'a HashMap<String, i64>,
    marked_unread: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_cap: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    official_accounts: Vec<&'a str>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    deleted_ids: &'a HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacySnapshot {
    #[serde(default)]
    threads: HashMap<String, ThreadInfo>,
    #[serde(default)]
    messages: HashMap<String, Vec<StoredMessage>>,
    #[serde(default, deserialize_with = "de_map_i64_flexible")]
    read_up_to: HashMap<String, i64>,
    #[serde(default)]
    pinned: Vec<String>,
    #[serde(default, deserialize_with = "de_map_i64_flexible")]
    muted_until: HashMap<String, i64>,
    #[serde(default)]
    marked_unread: Vec<String>,
    #[serde(default)]
    message_cap: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadFile {
    #[serde(default)]
    #[allow(dead_code)]
    key: String,
    messages: Vec<StoredMessage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadFileRef<'a> {
    key: &'a str,
    messages: &'a [StoredMessage],
}

// ---- ChatStore ----

#[derive(Debug, Clone)]
struct StorePaths {
    legacy_file: PathBuf,
    meta: PathBuf,
    threads: PathBuf,
}

impl StorePaths {
    fn from_legacy_file(legacy_file: PathBuf) -> Self {
        let dir = match legacy_file.to_str() {
            Some(s) if s.ends_with(".json") => PathBuf::from(&s[..s.len() - ".json".len()]),
            _ => {
                let mut os = OsString::from(legacy_file.as_os_str());
                os.push(".d");
                PathBuf::from(os)
            }
        };
        Self {
            legacy_file,
            meta: dir.join(META_FILE),
            threads: dir.join(THREADS_DIR),
        }
    }

    fn thread_file(&self, key: &str) -> PathBuf {
        self.threads
            .join(format!("{}.json", hex::encode(key.as_bytes())))
    }

    fn migrated_legacy_file(&self) -> PathBuf {
        let mut os = OsString::from(self.legacy_file.as_os_str());
        os.push(".migrated");
        PathBuf::from(os)
    }
}

#[derive(Debug, Default)]
struct StoreState {
    threads: HashMap<String, ThreadInfo>,
    messages: HashMap<String, Vec<StoredMessage>>,
    read_up_to: HashMap<String, i64>,
    pinned: HashSet<String>,
    muted_until: HashMap<String, i64>,
    marked_unread: HashSet<String>,
    /// Thread keys detected as Official Accounts.
    official_accounts: HashSet<String>,
    /// thread key -> ids deleted for me (insertion order, capped at `MAX_TOMBSTONES_PER_THREAD`).
    deleted_ids: HashMap<String, Vec<String>>,
    /// message id -> thread key
    index: HashMap<String, String>,
    own_id: Option<String>,
    message_cap: usize,
    /// Dirty tracking for incremental persistence.
    dirty_threads: HashSet<String>,
    meta_dirty: bool,
    wipe: bool,
    pending_migration: Option<HashSet<String>>,
}

impl StoreState {
    fn is_tombstoned(&self, key: &str, msg_id: &str) -> bool {
        self.deleted_ids
            .get(key)
            .is_some_and(|ids| ids.iter().any(|i| i == msg_id))
    }

    fn add_tombstone(&mut self, key: &str, msg_id: &str, has_paths: bool) {
        if self.is_tombstoned(key, msg_id) {
            return;
        }
        let ids = self.deleted_ids.entry(key.to_string()).or_default();
        ids.push(msg_id.to_string());
        if ids.len() > MAX_TOMBSTONES_PER_THREAD {
            let excess = ids.len() - MAX_TOMBSTONES_PER_THREAD;
            ids.drain(0..excess);
        }
        self.meta_changed(has_paths);
    }

    fn new() -> Self {
        Self {
            message_cap: MAX_MESSAGES_PER_THREAD,
            ..Default::default()
        }
    }

    fn thread_changed(&mut self, key: &str, has_paths: bool) {
        self.mark_dirty(key, has_paths);
    }

    fn mark_dirty(&mut self, key: &str, has_paths: bool) {
        if has_paths {
            self.dirty_threads.insert(key.to_string());
        }
    }

    fn meta_changed(&mut self, has_paths: bool) {
        if has_paths {
            self.meta_dirty = true;
        }
    }

    fn has_only_history(&self, kind: ThreadKind, id: &str) -> bool {
        let key = thread_key(kind, id);
        match self.messages.get(&key) {
            None => true,
            Some(list) => list.iter().all(|m| m.raw.is_none()),
        }
    }

    fn unread_count(&self, key: &str) -> usize {
        let read_ts = self.read_up_to.get(key).copied().unwrap_or(0);
        self.messages
            .get(key)
            .map(|list| {
                let start = list.partition_point(|m| m.ts <= read_ts);
                list[start..].iter().filter(|m| !m.is_self).count()
            })
            .unwrap_or(0)
    }

    fn meta_json(&self) -> String {
        let mut pinned: Vec<&str> = self.pinned.iter().map(String::as_str).collect();
        pinned.sort_unstable();
        let mut marked_unread: Vec<&str> = self.marked_unread.iter().map(String::as_str).collect();
        marked_unread.sort_unstable();
        let mut official_accounts: Vec<&str> =
            self.official_accounts.iter().map(String::as_str).collect();
        official_accounts.sort_unstable();
        let meta = MetaFileRef {
            version: 1,
            threads: &self.threads,
            read_up_to: &self.read_up_to,
            pinned,
            muted_until: &self.muted_until,
            marked_unread,
            message_cap: if self.message_cap > MAX_MESSAGES_PER_THREAD {
                Some(self.message_cap)
            } else {
                None
            },
            official_accounts,
            deleted_ids: &self.deleted_ids,
        };
        serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string())
    }

    fn thread_json(&self, key: &str) -> Option<String> {
        let list = self.messages.get(key)?;
        if list.is_empty() {
            return None;
        }
        let tf = ThreadFileRef {
            key,
            messages: list.as_slice(),
        };
        serde_json::to_string(&tf).ok()
    }

    fn apply_meta(&mut self, meta: MetaFile) {
        self.threads = meta.threads;
        self.read_up_to = meta.read_up_to;
        self.pinned = meta.pinned.into_iter().collect();
        self.muted_until = meta.muted_until;
        self.marked_unread = meta.marked_unread.into_iter().collect();
        self.official_accounts = meta.official_accounts.into_iter().collect();
        self.deleted_ids = meta.deleted_ids;
        if let Some(cap) = meta.message_cap {
            if cap > MAX_MESSAGES_PER_THREAD {
                self.message_cap = cap;
            }
        }
    }

    fn mark_read_inner(&mut self, kind: ThreadKind, id: &str, up_to: i64, has_paths: bool) -> bool {
        let key = thread_key(kind, id);
        let was_marked_unread = self.marked_unread.remove(&key);
        let prev = self.read_up_to.get(&key).copied().unwrap_or(0);
        let advanced = prev < up_to;
        if advanced {
            self.read_up_to.insert(key, up_to);
        }
        if was_marked_unread || advanced {
            self.meta_changed(has_paths);
        }
        was_marked_unread
    }
}

struct ChatStoreInner {
    paths: Option<StorePaths>,
    state: RwLock<StoreState>,
    io_lock: Mutex<()>,
    seq: AtomicU64,
    save_scheduled: AtomicBool,
}

/// Thread-safe, crash-safe in-memory and per-thread on-disk message store.
/// Cheaply cloneable via `Arc`.
#[derive(Clone)]
pub struct ChatStore {
    inner: Arc<ChatStoreInner>,
}

impl ChatStore {
    /// Creates an in-memory-only store (no disk I/O).
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(ChatStoreInner {
                paths: None,
                state: RwLock::new(StoreState::new()),
                io_lock: Mutex::new(()),
                seq: AtomicU64::new(0),
                save_scheduled: AtomicBool::new(false),
            }),
        }
    }

    /// Opens a store backed by `<data_dir>/messages/` (migrating `<data_dir>/messages.json` if present).
    pub fn open(data_dir: impl AsRef<Path>) -> Self {
        Self::open_file(data_dir.as_ref().join("messages.json"))
    }

    /// Opens a store given the legacy single-file path (e.g. `<data_dir>/messages.json`),
    /// storing the per-thread layout beside it in `<stem>/meta.json` and `<stem>/threads/<hex>.json`.
    pub fn open_file(legacy_file: impl AsRef<Path>) -> Self {
        let paths = StorePaths::from_legacy_file(legacy_file.as_ref().to_path_buf());
        let store = Self {
            inner: Arc::new(ChatStoreInner {
                paths: Some(paths),
                state: RwLock::new(StoreState::new()),
                io_lock: Mutex::new(()),
                seq: AtomicU64::new(0),
                save_scheduled: AtomicBool::new(false),
            }),
        };
        store.load();
        store
    }

    pub fn set_own_id(&self, own_id: Option<String>) {
        if let Ok(mut s) = self.inner.state.write() {
            s.own_id = own_id;
        }
    }

    pub fn own_id(&self) -> Option<String> {
        self.inner.state.read().ok().and_then(|s| s.own_id.clone())
    }

    /// Lets chats keep up to `cap` messages (saved with the cache), for imported history. Never lowers the limit.
    pub fn allow_messages(&self, cap: usize) {
        let mut schedule = false;
        if let Ok(mut s) = self.inner.state.write() {
            if cap > s.message_cap {
                s.message_cap = cap;
                s.meta_changed(self.inner.paths.is_some());
                schedule = true;
            }
        }
        if schedule {
            self.schedule_save();
        }
    }

    pub fn upsert_thread(&self, info: ThreadInfo) {
        if let Ok(mut s) = self.inner.state.write() {
            let key = thread_key(info.kind, &info.id);
            let prev = s.threads.get(&key).cloned();
            let name = if !info.name.is_empty() {
                info.name
            } else if let Some(p) = prev.as_ref().filter(|p| !p.name.is_empty()) {
                p.name.clone()
            } else {
                info.id.clone()
            };
            let avatar = if !info.avatar.is_empty() {
                info.avatar
            } else {
                prev.map(|p| p.avatar).unwrap_or_default()
            };
            s.threads.insert(
                key,
                ThreadInfo {
                    id: info.id,
                    kind: info.kind,
                    name,
                    avatar,
                },
            );
            s.meta_changed(self.inner.paths.is_some());
        }
        self.schedule_save();
    }

    pub fn get_thread(&self, kind: ThreadKind, id: &str) -> Option<ThreadInfo> {
        let key = thread_key(kind, id);
        self.inner
            .state
            .read()
            .ok()
            .and_then(|s| s.threads.get(&key).cloned())
    }

    /// Threads still named by their raw id (e.g. created by a history import for someone who is not a friend).
    pub fn unnamed_threads(&self) -> Vec<ThreadInfo> {
        self.inner
            .state
            .read()
            .map(|s| {
                s.threads
                    .values()
                    .filter(|t| t.name.is_empty() || t.name == t.id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True when the thread has no live messages (every message came from a history import, or it is empty).
    pub fn has_only_history(&self, kind: ThreadKind, id: &str) -> bool {
        self.inner
            .state
            .read()
            .map(|s| s.has_only_history(kind, id))
            .unwrap_or(true)
    }

    /// Removes an unnamed thread that holds only imported history, freeing its message ids in the dedupe index.
    pub fn drop_plain_thread(&self, kind: ThreadKind, plain_id: &str) -> bool {
        let has_paths = self.inner.paths.is_some();
        let dropped = {
            let Ok(mut s) = self.inner.state.write() else {
                return false;
            };
            let key = thread_key(kind, plain_id);
            let Some(info) = s.threads.get(&key) else {
                return false;
            };
            if !info.name.is_empty() && info.name != info.id {
                return false;
            }
            if !s.has_only_history(kind, plain_id) {
                return false;
            }
            if let Some(list) = s.messages.remove(&key) {
                for m in list {
                    if s.index.get(&m.id).map(String::as_str) == Some(key.as_str()) {
                        s.index.remove(&m.id);
                    }
                }
            }
            s.thread_changed(&key, has_paths);
            s.threads.remove(&key);
            s.read_up_to.remove(&key);
            s.marked_unread.remove(&key);
            s.pinned.remove(&key);
            s.muted_until.remove(&key);
            s.official_accounts.remove(&key);
            s.meta_changed(has_paths);
            true
        };
        if dropped {
            self.schedule_save();
        }
        dropped
    }

    /// Moves messages from an unnamed plain-id history thread into its real noised-id thread
    /// and removes the old thread. Preserves read state so migrated history never becomes unread.
    pub fn migrate_plain_thread(&self, kind: ThreadKind, from_id: &str, to_id: &str) -> bool {
        if from_id.is_empty() || to_id.is_empty() || from_id == to_id {
            return false;
        }
        let has_paths = self.inner.paths.is_some();
        let migrated = {
            let Ok(mut s) = self.inner.state.write() else {
                return false;
            };
            let from_key = thread_key(kind, from_id);
            let to_key = thread_key(kind, to_id);
            let Some(old_info) = s.threads.get(&from_key) else {
                return false;
            };
            if !old_info.name.is_empty() && old_info.name != old_info.id {
                return false;
            }
            if !s.has_only_history(kind, from_id) {
                return false;
            }

            let old_list = s.messages.remove(&from_key).unwrap_or_default();
            let target_info = s.threads.get(&to_key).cloned();
            let mut target_list = s.messages.remove(&to_key).unwrap_or_default();
            let mut existing_ids: HashSet<String> =
                target_list.iter().map(|m| m.id.clone()).collect();

            for m in old_list {
                if s.index.get(&m.id).map(String::as_str) == Some(from_key.as_str()) {
                    s.index.remove(&m.id);
                }
                if existing_ids.contains(&m.id) || s.index.contains_key(&m.id) {
                    continue;
                }
                let mut moved = m;
                if let Some(raw) = moved.raw.as_deref_mut() {
                    if raw.id_to.is_none() {
                        raw.id_to = Some(moved.thread_id.clone());
                    }
                    if kind == ThreadKind::User && !moved.is_self && raw.uid_from.is_none() {
                        raw.uid_from = Some(moved.from_id.clone());
                    }
                }
                moved.thread_id = to_id.to_string();
                if kind == ThreadKind::User && !moved.is_self {
                    moved.from_id = to_id.to_string();
                    if moved.from_name.is_empty() {
                        if let Some(ref ti) = target_info {
                            if ti.name != to_id {
                                moved.from_name = ti.name.clone();
                            }
                        }
                    }
                }
                existing_ids.insert(moved.id.clone());
                s.index.insert(moved.id.clone(), to_key.clone());
                target_list.push(moved);
            }

            target_list.sort_by_key(|m| m.ts);
            if target_list.len() > s.message_cap {
                let excess = target_list.len() - s.message_cap;
                for dropped in target_list.drain(0..excess) {
                    if s.index.get(&dropped.id).map(String::as_str) == Some(to_key.as_str()) {
                        s.index.remove(&dropped.id);
                    }
                }
            }
            if !target_list.is_empty() {
                s.messages.insert(to_key.clone(), target_list);
            }
            s.thread_changed(&to_key, has_paths);
            if target_info.is_none() {
                s.threads.insert(
                    to_key.clone(),
                    ThreadInfo {
                        id: to_id.to_string(),
                        kind,
                        name: to_id.to_string(),
                        avatar: String::new(),
                    },
                );
            }

            let merged_read = s
                .read_up_to
                .get(&to_key)
                .copied()
                .unwrap_or(0)
                .max(s.read_up_to.get(&from_key).copied().unwrap_or(0));
            if merged_read > 0 {
                s.read_up_to.insert(to_key.clone(), merged_read);
            }

            s.thread_changed(&from_key, has_paths);
            s.threads.remove(&from_key);
            s.read_up_to.remove(&from_key);
            s.marked_unread.remove(&from_key);
            s.pinned.remove(&from_key);
            s.muted_until.remove(&from_key);
            s.meta_changed(has_paths);
            true
        };
        if migrated {
            self.schedule_save();
        }
        migrated
    }

    /// Adds a live message; returns `false` if already stored or immediately evicted as too old.
    pub fn add_message(&self, msg: StoredMessage) -> bool {
        self.add_message_opts(msg, false)
    }

    /// Adds an imported historical message without altering read state (see [`Self::settle_history`]).
    pub fn add_history_message(&self, msg: StoredMessage) -> bool {
        self.add_message_opts(msg, true)
    }

    /// Adds a message; when `history` is true, read state is not modified.
    pub fn add_message_opts(&self, msg: StoredMessage, history: bool) -> bool {
        let has_paths = self.inner.paths.is_some();
        let (added, dirty) = {
            let Ok(mut s) = self.inner.state.write() else {
                return false;
            };
            if s.index.contains_key(&msg.id) {
                return false;
            }
            let key = thread_key(msg.thread_kind, &msg.thread_id);
            if s.is_tombstoned(&key, &msg.id) {
                return false;
            }
            let msg_id = msg.id.clone();
            let msg_ts = msg.ts;
            let msg_is_self = msg.is_self;
            let msg_thread_kind = msg.thread_kind;
            let msg_thread_id = msg.thread_id.clone();
            let msg_from_name = msg.from_name.clone();
            let cap = s.message_cap;

            let list = s.messages.entry(key.clone()).or_default();
            let mut idx = list.len();
            while idx > 0 && list[idx - 1].ts > msg_ts {
                idx -= 1;
            }
            list.insert(idx, msg);

            let mut evicted_self = false;
            let mut evicted_others = false;
            let mut dropped_ids = Vec::new();
            if list.len() > cap {
                let excess = list.len() - cap;
                for dropped in list.drain(0..excess) {
                    if dropped.id == msg_id {
                        evicted_self = true;
                    } else {
                        evicted_others = true;
                    }
                    dropped_ids.push(dropped.id);
                }
            }
            s.index.insert(msg_id.clone(), key.clone());
            for dropped_id in dropped_ids {
                s.index.remove(&dropped_id);
            }

            let mut any_dirty = false;
            if !evicted_self || evicted_others {
                s.thread_changed(&key, has_paths);
                any_dirty = true;
            }
            if evicted_self {
                return false;
            }

            if let Some(existing) = s.threads.get_mut(&key) {
                if msg_thread_kind == ThreadKind::User
                    && !msg_is_self
                    && !msg_from_name.is_empty()
                    && existing.name == existing.id
                {
                    existing.name = msg_from_name;
                    s.meta_changed(has_paths);
                    any_dirty = true;
                }
            } else {
                let name = if msg_thread_kind == ThreadKind::User
                    && !msg_is_self
                    && !msg_from_name.is_empty()
                {
                    msg_from_name
                } else {
                    msg_thread_id.clone()
                };
                s.threads.insert(
                    key.clone(),
                    ThreadInfo {
                        id: msg_thread_id.clone(),
                        kind: msg_thread_kind,
                        name,
                        avatar: String::new(),
                    },
                );
                s.meta_changed(has_paths);
                any_dirty = true;
            }

            if !history {
                let prev_read = s.read_up_to.get(&key).copied().unwrap_or(0);
                if msg_is_self {
                    if msg_ts > prev_read {
                        s.mark_read_inner(msg_thread_kind, &msg_thread_id, msg_ts, has_paths);
                        any_dirty = true;
                    }
                } else if msg_ts > prev_read && s.marked_unread.remove(&key) {
                    s.meta_changed(has_paths);
                    any_dirty = true;
                }
            }

            (true, any_dirty)
        };

        if dirty {
            self.schedule_save();
        }
        added
    }

    /// After importing old messages into a chat: marks them read up to just before the first
    /// pre-existing unread message, so history never shows up as new unread messages.
    pub fn settle_history(&self, kind: ThreadKind, id: &str, imported_ids: &HashSet<String>) {
        let has_paths = self.inner.paths.is_some();
        let mut changed = false;
        if let Ok(mut s) = self.inner.state.write() {
            let key = thread_key(kind, id);
            let read_ts = s.read_up_to.get(&key).copied().unwrap_or(0);
            let up_to = if let Some(list) = s.messages.get(&key) {
                let first_live_unread = list
                    .iter()
                    .find(|m| !imported_ids.contains(&m.id) && !m.is_self && m.ts > read_ts)
                    .map(|m| m.ts)
                    .unwrap_or(i64::MAX);
                let mut best = read_ts;
                for m in list {
                    if imported_ids.contains(&m.id) && m.ts < first_live_unread && m.ts > best {
                        best = m.ts;
                    }
                }
                best
            } else {
                read_ts
            };
            if up_to > read_ts {
                s.read_up_to.insert(key, up_to);
                s.meta_changed(has_paths);
                changed = true;
            }
        }
        if changed {
            self.schedule_save();
        }
    }

    /// The newest stored message of a conversation kind, as `(ts, id)` (ties go to the later
    /// stored one). Used by the replay report to see where our history ends.
    pub fn newest_message_of_kind(&self, kind: ThreadKind) -> Option<(i64, String)> {
        let s = self.inner.state.read().ok()?;
        let mut best: Option<(i64, &str)> = None;
        for list in s.messages.values() {
            let Some(last) = list.last() else { continue };
            if last.thread_kind != kind {
                continue;
            }
            if best.is_none_or(|(ts, _)| last.ts >= ts) {
                best = Some((last.ts, last.id.as_str()));
            }
        }
        best.map(|(ts, id)| (ts, id.to_string()))
    }

    /// Whether a message with this id is stored.
    pub fn has_message(&self, msg_id: &str) -> bool {
        self.inner
            .state
            .read()
            .is_ok_and(|s| s.index.contains_key(msg_id))
    }

    /// Whether the conversation's read marker already covers a message sent at `ts`.
    pub fn is_read_at(&self, kind: ThreadKind, id: &str, ts: i64) -> bool {
        let Ok(s) = self.inner.state.read() else {
            return false;
        };
        let key = thread_key(kind, id);
        s.read_up_to.get(&key).is_some_and(|&read| ts <= read)
    }

    /// Looks up a stored message by its message id (includes server-side `raw` fields).
    pub fn find_message(&self, msg_id: &str) -> Option<StoredMessage> {
        let s = self.inner.state.read().ok()?;
        let key = s.index.get(msg_id)?;
        s.messages
            .get(key)?
            .iter()
            .find(|m| m.id == msg_id)
            .cloned()
    }

    /// Alias for [`Self::find_message`].
    pub fn get_message(&self, msg_id: &str) -> Option<StoredMessage> {
        self.find_message(msg_id)
    }

    /// O(1) count of cached messages in a conversation.
    pub fn message_count(&self, kind: ThreadKind, id: &str) -> usize {
        let key = thread_key(kind, id);
        self.inner
            .state
            .read()
            .ok()
            .and_then(|s| s.messages.get(&key).map(Vec::len))
            .unwrap_or(0)
    }

    /// Returns up to `limit` (clamped to `1..=500`, default 50) public messages in chronological
    /// order, optionally older than `before` (exclusive).
    pub fn get_messages(
        &self,
        kind: ThreadKind,
        id: &str,
        before: Option<i64>,
        limit: Option<usize>,
    ) -> Vec<ChatMessage> {
        let cap = limit.unwrap_or(50).clamp(1, MAX_MESSAGES_PER_THREAD);
        let Ok(s) = self.inner.state.read() else {
            return Vec::new();
        };
        let key = thread_key(kind, id);
        let Some(list) = s.messages.get(&key) else {
            return Vec::new();
        };
        let end = match before {
            Some(ts) => list.partition_point(|m| m.ts < ts),
            None => list.len(),
        };
        let start = end.saturating_sub(cap);
        list[start..end]
            .iter()
            .map(|m| to_public_message(m, s.own_id.as_deref()))
            .collect()
    }

    /// Searches cached messages across all conversations with Vietnamese diacritic-insensitive,
    /// case-insensitive matching. Skips recalled messages and returns hits sorted newest-first,
    /// capped to `[1, 100]` (default 50).
    pub fn search_messages(&self, query: &str, limit: Option<usize>) -> Vec<MessageSearchHit> {
        let needle = fold_for_search(query.trim());
        if needle.is_empty() {
            return Vec::new();
        }
        let needle_has_nul = needle.contains('\0');
        let cap = limit.unwrap_or(50).clamp(1, 100);
        let Ok(s) = self.inner.state.read() else {
            return Vec::new();
        };
        let own = s.own_id.as_deref();

        // Min-heap of the top `cap` newest matches: `(Reverse(ts), seq, thread, msg)`.
        // A plain wrapper avoids requiring `Ord` on references.
        struct HeapEntry<'a> {
            ts: i64,
            seq: usize,
            thread: Option<&'a ThreadInfo>,
            msg: &'a StoredMessage,
        }
        impl PartialEq for HeapEntry<'_> {
            fn eq(&self, other: &Self) -> bool {
                self.ts == other.ts && self.seq == other.seq
            }
        }
        impl Eq for HeapEntry<'_> {}
        impl PartialOrd for HeapEntry<'_> {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for HeapEntry<'_> {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                // Min-heap by `ts` so `peek()` is the oldest match in the top-K.
                Reverse((self.ts, self.seq)).cmp(&Reverse((other.ts, other.seq)))
            }
        }

        let mut heap: BinaryHeap<HeapEntry<'_>> = BinaryHeap::with_capacity(cap + 1);
        let mut seq = 0usize;

        for (key, list) in &s.messages {
            let thread = s.threads.get(key);
            for m in list.iter().rev() {
                if m.recalled {
                    continue;
                }
                if heap.len() == cap {
                    if let Some(oldest) = heap.peek() {
                        if m.ts < oldest.ts {
                            continue;
                        }
                    }
                }
                let haystack = m.search_folded.get_or_init(|| build_search_folded(m));
                let hit = if !needle_has_nul {
                    haystack.contains(&needle)
                } else {
                    haystack.split('\0').any(|part| part.contains(&needle))
                };
                if hit {
                    seq += 1;
                    if heap.len() < cap {
                        heap.push(HeapEntry {
                            ts: m.ts,
                            seq,
                            thread,
                            msg: m,
                        });
                    } else if let Some(mut oldest) = heap.peek_mut() {
                        if m.ts > oldest.ts {
                            *oldest = HeapEntry {
                                ts: m.ts,
                                seq,
                                thread,
                                msg: m,
                            };
                        }
                    }
                }
            }
        }

        let mut top = heap.into_vec();
        top.sort_by(|a, b| b.ts.cmp(&a.ts));
        top.into_iter()
            .map(|e| MessageSearchHit {
                thread_id: e.msg.thread_id.clone(),
                thread_kind: e.msg.thread_kind,
                thread_name: e
                    .thread
                    .filter(|t| !t.name.is_empty())
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| e.msg.thread_id.clone()),
                thread_avatar: e.thread.map(|t| t.avatar.clone()).unwrap_or_default(),
                message: to_public_message(e.msg, own),
            })
            .collect()
    }

    /// Sets (or with an empty code, clears) one user's reaction on a message.
    pub fn apply_reaction(&self, msg_id: &str, uid: &str, code: &str) -> Option<StoredMessage> {
        let has_paths = self.inner.paths.is_some();
        let updated = {
            let mut s = self.inner.state.write().ok()?;
            let key = s.index.get(msg_id)?.clone();
            let msg = s
                .messages
                .get_mut(&key)?
                .iter_mut()
                .find(|m| m.id == msg_id)?;
            let reactions = msg.reactions.get_or_insert_with(BTreeMap::new);
            if code.is_empty() {
                reactions.remove(uid);
            } else {
                reactions.insert(uid.to_string(), code.to_string());
            }
            let cloned = msg.clone();
            s.mark_dirty(&key, has_paths);
            cloned
        };
        self.schedule_save();
        Some(updated)
    }

    /// Marks a message as recalled (undo), clearing attachment, quote, mentions, and reactions.
    pub fn apply_undo(&self, msg_id: &str) -> Option<StoredMessage> {
        let has_paths = self.inner.paths.is_some();
        let updated = {
            let mut s = self.inner.state.write().ok()?;
            let key = s.index.get(msg_id)?.clone();
            let msg = s
                .messages
                .get_mut(&key)?
                .iter_mut()
                .find(|m| m.id == msg_id)?;
            msg.recalled = true;
            msg.text = "Message recalled".to_string();
            msg.attachment = None;
            msg.quote = None;
            msg.mentions.clear();
            msg.reactions = None;
            if let Some(raw) = msg.raw.as_deref_mut() {
                raw.content = None;
                raw.property_ext = None;
            }
            msg.search_folded.clear();
            let cloned = msg.clone();
            s.mark_dirty(&key, has_paths);
            cloned
        };
        self.schedule_save();
        Some(updated)
    }

    /// Removes a message for good ("delete for me"): drops it if stored and remembers its id so
    /// history replay or old-message fetches cannot bring it back. Works for recalled messages too.
    pub fn delete_for_me(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
    ) -> Option<StoredMessage> {
        let has_paths = self.inner.paths.is_some();
        let removed = {
            let mut s = self.inner.state.write().ok()?;
            let key = thread_key(kind, thread_id);
            s.add_tombstone(&key, msg_id, has_paths);
            let removed = s.messages.get_mut(&key).and_then(|list| {
                let pos = list.iter().position(|m| m.id == msg_id)?;
                Some(list.remove(pos))
            });
            if removed.is_some() {
                s.index.remove(msg_id);
                s.thread_changed(&key, has_paths);
            }
            removed
        };
        self.schedule_save();
        removed
    }

    /// Whether `msg_id` was deleted for me in this thread.
    pub fn is_deleted_for_me(&self, kind: ThreadKind, thread_id: &str, msg_id: &str) -> bool {
        self.inner
            .state
            .read()
            .map(|s| s.is_tombstoned(&thread_key(kind, thread_id), msg_id))
            .unwrap_or(false)
    }

    /// Deletes a single message from the store and its dedupe index.
    pub fn delete_message(&self, msg_id: &str) -> Option<StoredMessage> {
        let has_paths = self.inner.paths.is_some();
        let removed = {
            let mut s = self.inner.state.write().ok()?;
            let key = s.index.get(msg_id)?.clone();
            let list = s.messages.get_mut(&key)?;
            let pos = list.iter().position(|m| m.id == msg_id)?;
            let removed = list.remove(pos);
            s.index.remove(msg_id);
            s.thread_changed(&key, has_paths);
            removed
        };
        self.schedule_save();
        Some(removed)
    }

    /// Records delivery/seen receipts for `msg_id`. In 1:1 chats a receipt covers every earlier
    /// message we sent; in groups it applies to that message only. Returns the messages that changed.
    pub fn apply_receipt(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
        delivered_to: &[String],
        seen_by: &[String],
    ) -> Vec<StoredMessage> {
        if delivered_to.is_empty() && seen_by.is_empty() {
            return Vec::new();
        }
        let has_paths = self.inner.paths.is_some();
        let changed = {
            let Ok(mut s) = self.inner.state.write() else {
                return Vec::new();
            };
            let key = thread_key(kind, thread_id);
            let Some(list) = s.messages.get_mut(&key) else {
                return Vec::new();
            };
            let Some(target_pos) = list.iter().rposition(|m| m.id == msg_id) else {
                return Vec::new();
            };
            let target_ts = list[target_pos].ts;
            let mut out = Vec::new();
            if kind == ThreadKind::Group {
                if merge_receipts(&mut list[target_pos], delivered_to, seen_by) {
                    out.push(list[target_pos].clone());
                }
            } else {
                let end = list.partition_point(|m| m.ts <= target_ts);
                for m in list[..end].iter_mut().rev() {
                    if !m.is_self {
                        continue;
                    }
                    if merge_receipts(m, delivered_to, seen_by) {
                        out.push(m.clone());
                    }
                }
                out.reverse();
            }
            if !out.is_empty() {
                s.mark_dirty(&key, has_paths);
            }
            out
        };
        if !changed.is_empty() {
            self.schedule_save();
        }
        changed
    }

    /// Convenience wrapper recording delivery receipts for `msg_id`.
    pub fn apply_delivery(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
        uids: &[String],
    ) -> Vec<StoredMessage> {
        self.apply_receipt(kind, thread_id, msg_id, uids, &[])
    }

    /// Convenience wrapper recording seen receipts for `msg_id`.
    pub fn apply_seen(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
        uids: &[String],
    ) -> Vec<StoredMessage> {
        self.apply_receipt(kind, thread_id, msg_id, &[], uids)
    }

    /// Incoming messages after the read marker (the ones a "seen" receipt should cover).
    pub fn unread_messages(&self, kind: ThreadKind, id: &str) -> Vec<StoredMessage> {
        let Ok(s) = self.inner.state.read() else {
            return Vec::new();
        };
        let key = thread_key(kind, id);
        let read_ts = s.read_up_to.get(&key).copied().unwrap_or(0);
        s.messages
            .get(&key)
            .map(|list| {
                list.iter()
                    .filter(|m| !m.is_self && m.ts > read_ts)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Advances the read marker for a thread; returns whether it had been manually marked unread.
    pub fn mark_read(&self, kind: ThreadKind, id: &str, up_to: Option<i64>) -> bool {
        let ts = up_to.unwrap_or_else(now_ms);
        let has_paths = self.inner.paths.is_some();
        let (was_marked, dirty) = {
            let Ok(mut s) = self.inner.state.write() else {
                return false;
            };
            let prev_dirty = s.meta_dirty;
            let was = s.mark_read_inner(kind, id, ts, has_paths);
            (was, s.meta_dirty && !prev_dirty || was)
        };
        if dirty {
            self.schedule_save();
        }
        was_marked
    }

    pub fn mark_unread(&self, kind: ThreadKind, id: &str) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            s.marked_unread.insert(thread_key(kind, id));
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    pub fn set_pinned(&self, kind: ThreadKind, id: &str, pinned: bool) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            let key = thread_key(kind, id);
            if pinned {
                s.pinned.insert(key);
            } else {
                s.pinned.remove(&key);
            }
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    /// Replaces the entire pinned-thread set (e.g. from Zalo's synced pin list).
    pub fn set_pinned_set(&self, keys: impl IntoIterator<Item = String>) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            s.pinned = keys.into_iter().collect();
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    /// Sets mute expiration (`Some(epoch_ms)`, `Some(-1)` for indefinitely, `None` to unmute).
    pub fn set_muted(&self, kind: ThreadKind, id: &str, until: Option<i64>) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            let key = thread_key(kind, id);
            match until {
                None => {
                    s.muted_until.remove(&key);
                }
                Some(u) => {
                    s.muted_until.insert(key, u);
                }
            }
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    /// Replaces the entire muted-thread map (e.g. from Zalo's synced mute list).
    pub fn set_mutes(&self, entries: impl IntoIterator<Item = (String, i64)>) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            s.muted_until = entries.into_iter().collect();
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    pub fn is_muted(&self, kind: ThreadKind, id: &str) -> bool {
        self.is_muted_at(kind, id, now_ms())
    }

    pub fn is_muted_at(&self, kind: ThreadKind, id: &str, now: i64) -> bool {
        let Ok(s) = self.inner.state.read() else {
            return false;
        };
        let key = thread_key(kind, id);
        s.muted_until
            .get(&key)
            .is_some_and(|&until| until == -1 || until > now)
    }

    /// Drops a conversation's messages (e.g. after deleting conversation history on Zalo).
    pub fn clear_thread_messages(&self, kind: ThreadKind, id: &str) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            let key = thread_key(kind, id);
            if let Some(list) = s.messages.remove(&key) {
                for m in list {
                    s.index.remove(&m.id);
                }
            }
            s.thread_changed(&key, has_paths);
            s.marked_unread.remove(&key);
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    /// Forgets a conversation entirely (messages, thread info, pin, mute, read state).
    pub fn delete_thread(&self, kind: ThreadKind, id: &str) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            let key = thread_key(kind, id);
            if let Some(list) = s.messages.remove(&key) {
                for m in list {
                    s.index.remove(&m.id);
                }
            }
            s.thread_changed(&key, has_paths);
            s.threads.remove(&key);
            s.read_up_to.remove(&key);
            s.marked_unread.remove(&key);
            s.pinned.remove(&key);
            s.muted_until.remove(&key);
            s.official_accounts.remove(&key);
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    /// Flags a 1:1 thread as an Official Account (news / shop / ZNS sender). Sticky: once set it
    /// is never cleared by later signals. Returns true when the flag was newly set.
    pub fn mark_official_account(&self, id: &str) -> bool {
        if id.is_empty() || id == "0" {
            return false;
        }
        let has_paths = self.inner.paths.is_some();
        let newly = match self.inner.state.write() {
            Ok(mut s) => {
                let added = s.official_accounts.insert(thread_key(ThreadKind::User, id));
                if added {
                    s.meta_changed(has_paths);
                }
                added
            }
            Err(_) => false,
        };
        if newly {
            self.schedule_save();
        }
        newly
    }

    pub fn is_official_account(&self, id: &str) -> bool {
        self.inner
            .state
            .read()
            .map(|s| s.official_accounts.contains(&thread_key(ThreadKind::User, id)))
            .unwrap_or(false)
    }

    pub fn last_message(&self, kind: ThreadKind, id: &str) -> Option<StoredMessage> {
        let s = self.inner.state.read().ok()?;
        let key = thread_key(kind, id);
        s.messages.get(&key)?.last().cloned()
    }

    /// Lists all conversations sorted by: pinned first, then newest `lastMessage.ts` desc, then name asc.
    pub fn list_threads(&self) -> Vec<ThreadSummary> {
        self.list_threads_at(now_ms())
    }

    /// Lists all conversations evaluated at timestamp `now` (for deterministic mute expiry checks).
    pub fn list_threads_at(&self, now: i64) -> Vec<ThreadSummary> {
        let mut keyed: Vec<(String, ThreadSummary)> = {
            let Ok(s) = self.inner.state.read() else {
                return Vec::new();
            };
            let own = s.own_id.as_deref();
            let mut out = Vec::with_capacity(s.threads.len());
            for (key, info) in &s.threads {
                let unread = s.unread_count(key);
                let last_message = s
                    .messages
                    .get(key)
                    .and_then(|list| list.last())
                    .map(|m| to_public_message(m, own));
                let marked = if s.marked_unread.contains(key) { 1 } else { 0 };
                let pinned = s.pinned.contains(key);
                let is_official_account = s.official_accounts.contains(key);
                let muted_until = s
                    .muted_until
                    .get(key)
                    .copied()
                    .filter(|&u| u == -1 || u > now);
                let lower = info.name.to_lowercase();
                out.push((
                    lower,
                    ThreadSummary {
                        id: info.id.clone(),
                        kind: info.kind,
                        name: info.name.clone(),
                        avatar: info.avatar.clone(),
                        last_message,
                        unread: unread.max(marked),
                        pinned,
                        muted_until,
                        is_official_account,
                    },
                ));
            }
            out
        };
        keyed.sort_by(|(a_low, a), (b_low, b)| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| {
                    let b_ts = b.last_message.as_ref().map_or(0, |m| m.ts);
                    let a_ts = a.last_message.as_ref().map_or(0, |m| m.ts);
                    b_ts.cmp(&a_ts)
                })
                .then_with(|| a_low.cmp(b_low).then_with(|| a.name.cmp(&b.name)))
        });
        keyed.into_iter().map(|(_, t)| t).collect()
    }

    /// Converts a `StoredMessage` to a public `ChatMessage` using this store's `own_id`.
    pub fn to_public(&self, m: &StoredMessage) -> ChatMessage {
        let own = self.own_id();
        to_public_message(m, own.as_deref())
    }

    /// Clears all cached conversations and marks all on-disk files for deletion (for logout).
    pub fn clear(&self) {
        let has_paths = self.inner.paths.is_some();
        if let Ok(mut s) = self.inner.state.write() {
            s.threads.clear();
            s.messages.clear();
            s.read_up_to.clear();
            s.pinned.clear();
            s.muted_until.clear();
            s.marked_unread.clear();
            s.official_accounts.clear();
            s.index.clear();
            s.dirty_threads.clear();
            s.pending_migration = None;
            s.wipe = true;
            s.meta_changed(has_paths);
        }
        self.schedule_save();
    }

    /// Synchronously persists all pending changes to disk. Never panics on I/O errors.
    pub fn flush(&self) {
        let _ = self.save_sync();
    }

    // ---- Disk persistence internals ----

    fn load(&self) {
        let Some(paths) = self.inner.paths.as_ref() else {
            return;
        };

        // Clean up any `.tmp` files left by a crash mid-write.
        if let Some(meta_dir) = paths.meta.parent() {
            for name in list_dir_names(meta_dir) {
                if name.ends_with(".tmp") {
                    remove_quietly(&meta_dir.join(name));
                }
            }
        }
        for name in list_dir_names(&paths.threads) {
            if name.ends_with(".tmp") {
                remove_quietly(&paths.threads.join(name));
            }
        }

        if !paths.meta.exists() && paths.legacy_file.exists() {
            self.migrate_legacy(&paths.legacy_file.clone());
            return;
        }

        let mut meta_loaded = false;
        if paths.meta.exists() {
            match fs::read_to_string(&paths.meta)
                .map_err(|e| e.to_string())
                .and_then(|text| serde_json::from_str::<MetaFile>(&text).map_err(|e| e.to_string()))
            {
                Ok(meta) => {
                    if let Ok(mut s) = self.inner.state.write() {
                        s.apply_meta(meta);
                        meta_loaded = true;
                    }
                }
                Err(err) => {
                    backup_corrupt(&paths.meta, &err);
                }
            }
        }

        let mut all_read = meta_loaded;
        let mut need_schedule = false;

        for name in list_dir_names(&paths.threads) {
            if !is_hex_thread_file(&name) {
                continue;
            }
            let hex_part = &name[..name.len() - ".json".len()];
            let Ok(bytes) = hex::decode(hex_part) else {
                continue;
            };
            let Ok(key) = String::from_utf8(bytes) else {
                continue;
            };
            let file = paths.thread_file(&key);
            if file.file_name().and_then(|f| f.to_str()) != Some(name.as_str()) {
                continue;
            }
            match fs::read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| {
                    serde_json::from_str::<ThreadFile>(&text).map_err(|e| e.to_string())
                }) {
                Ok(mut tf) => {
                    for m in &mut tf.messages {
                        slim_loaded_raw(m);
                    }
                    if let Ok(mut s) = self.inner.state.write() {
                        for m in &tf.messages {
                            s.index.insert(m.id.clone(), key.clone());
                        }
                        s.messages.insert(key, tf.messages);
                    }
                }
                Err(err) => {
                    all_read = false;
                    backup_corrupt(&file, &err);
                }
            }
        }

        // The new layout loaded cleanly after a migration: the old backup copy is no longer needed.
        if all_read {
            remove_quietly(&paths.migrated_legacy_file());
        }

        if let Ok(mut s) = self.inner.state.write() {
            if !meta_loaded {
                let max_len = s.messages.values().map(Vec::len).max().unwrap_or(0);
                if max_len > s.message_cap {
                    s.message_cap = max_len.max(HISTORY_MESSAGES_PER_THREAD);
                }
            }
            // Recover threads whose messages exist on disk without a matching meta.json entry.
            let orphan_keys: Vec<(String, ThreadInfo, i64)> = s
                .messages
                .iter()
                .filter_map(|(key, list)| {
                    if s.threads.contains_key(key) {
                        return None;
                    }
                    let last = list.last()?;
                    Some((
                        key.clone(),
                        ThreadInfo {
                            id: last.thread_id.clone(),
                            kind: last.thread_kind,
                            name: last.thread_id.clone(),
                            avatar: String::new(),
                        },
                        last.ts,
                    ))
                })
                .collect();
            for (key, info, last_ts) in orphan_keys {
                s.threads.insert(key.clone(), info);
                if !meta_loaded {
                    s.read_up_to.insert(key, last_ts);
                }
                s.meta_changed(true);
                need_schedule = true;
            }
        }

        if need_schedule {
            self.schedule_save();
        }
    }

    fn migrate_legacy(&self, legacy_file: &Path) {
        let parsed = fs::read_to_string(legacy_file)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                serde_json::from_str::<LegacySnapshot>(&text).map_err(|e| e.to_string())
            });
        let mut snapshot = match parsed {
            Ok(snap) => snap,
            Err(err) => {
                if let Ok(mut s) = self.inner.state.write() {
                    *s = StoreState::new();
                }
                backup_corrupt(legacy_file, &err);
                return;
            }
        };
        for list in snapshot.messages.values_mut() {
            for m in list {
                slim_loaded_raw(m);
            }
        }

        if let Ok(mut guard) = self.inner.state.write() {
            let s = &mut *guard;
            s.apply_meta(MetaFile {
                version: Some(1),
                threads: snapshot.threads,
                read_up_to: snapshot.read_up_to,
                pinned: snapshot.pinned,
                muted_until: snapshot.muted_until,
                marked_unread: snapshot.marked_unread,
                message_cap: snapshot.message_cap,
                official_accounts: Vec::new(),
                deleted_ids: HashMap::new(),
            });
            s.messages = snapshot.messages;
            s.index.clear();
            for (key, list) in &s.messages {
                for m in list {
                    s.index.insert(m.id.clone(), key.clone());
                }
            }
            let keys: HashSet<String> = s.messages.keys().cloned().collect();
            s.pending_migration = Some(keys.clone());
            s.dirty_threads.extend(keys);
            s.meta_dirty = true;
        }

        if !self.save_sync() {
            self.schedule_save();
        }
    }

    fn schedule_save(&self) {
        if self.inner.paths.is_none() {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self
            .inner
            .save_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let store = self.clone();
        handle.spawn(async move {
            tokio::time::sleep(SAVE_DEBOUNCE).await;
            store.inner.save_scheduled.store(false, Ordering::Release);
            let _ = tokio::task::spawn_blocking(move || {
                store.save_sync();
            })
            .await;
        });
    }

    fn next_seq(&self) -> u64 {
        self.inner.seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn save_sync(&self) -> bool {
        let Some(paths) = self.inner.paths.as_ref() else {
            return true;
        };
        let Ok(_io_guard) = self.inner.io_lock.lock() else {
            return false;
        };

        // Drain only dirty keys and flags under a brief write lock; serialize each thread under
        // a short read lock so writers and readers are never blocked across large serializations.
        let (do_wipe, dirty_keys, want_meta, mut pending_mig) = {
            let Ok(mut s) = self.inner.state.write() else {
                return false;
            };
            let do_wipe = s.wipe;
            let dirty_keys: Vec<String> = s.dirty_threads.drain().collect();
            let want_meta = s.meta_dirty;
            if want_meta {
                s.meta_dirty = false;
            }
            let pending_mig = s.pending_migration.clone();
            (do_wipe, dirty_keys, want_meta, pending_mig)
        };

        if !do_wipe && dirty_keys.is_empty() && !want_meta {
            return true;
        }

        let mut ok = true;
        let mut logged_err = false;
        let mut fail = |err: &dyn std::fmt::Display| {
            if !logged_err {
                let dir_display = paths
                    .meta
                    .parent()
                    .unwrap_or(paths.meta.as_path())
                    .display();
                tracing::warn!("Could not save message cache {dir_display}: {err}");
                logged_err = true;
            }
            ok = false;
        };

        if let Err(err) = create_dir_private(&paths.threads) {
            fail(&err);
            // Put everything back as dirty so a future flush retries.
            if let Ok(mut s) = self.inner.state.write() {
                if do_wipe {
                    s.wipe = true;
                }
                for key in dirty_keys {
                    s.dirty_threads.insert(key);
                }
                if want_meta {
                    s.meta_dirty = true;
                }
            }
            return false;
        }

        if do_wipe {
            for name in list_dir_names(&paths.threads) {
                if is_hex_thread_file(&name) {
                    remove_quietly(&paths.threads.join(name));
                }
            }
            remove_quietly(&paths.legacy_file);
            remove_quietly(&paths.migrated_legacy_file());
            pending_mig = None;
            if let Ok(mut s) = self.inner.state.write() {
                s.wipe = false;
                s.pending_migration = None;
            }
        }

        let mut failed_threads = Vec::new();
        for key in dirty_keys {
            let json_opt = match self.inner.state.read() {
                Ok(s) => s.thread_json(&key),
                Err(_) => return false,
            };
            let file = paths.thread_file(&key);
            let res = match json_opt {
                None => {
                    remove_quietly(&file);
                    Ok(())
                }
                Some(json) => self.write_atomic(&file, &json),
            };
            match res {
                Ok(()) => {
                    if let Some(ref mut mig) = pending_mig {
                        mig.remove(&key);
                    }
                }
                Err(err) => {
                    fail(&err);
                    failed_threads.push(key);
                }
            }
        }

        if !failed_threads.is_empty() {
            if let Ok(mut s) = self.inner.state.write() {
                for key in failed_threads {
                    s.dirty_threads.insert(key);
                }
                s.pending_migration = pending_mig.clone();
            }
        } else if pending_mig.is_some() {
            if let Ok(mut s) = self.inner.state.write() {
                s.pending_migration = pending_mig.clone();
            }
        }

        let migration_complete = pending_mig.as_ref().is_none_or(HashSet::is_empty);
        if want_meta && migration_complete {
            let meta_json = match self.inner.state.read() {
                Ok(s) => s.meta_json(),
                Err(_) => return false,
            };
            match self.write_atomic(&paths.meta, &meta_json) {
                Ok(()) => {
                    if pending_mig.is_some() {
                        if let Ok(mut s) = self.inner.state.write() {
                            s.pending_migration = None;
                        }
                        if paths.legacy_file.exists() {
                            if let Err(err) =
                                fs::rename(&paths.legacy_file, paths.migrated_legacy_file())
                            {
                                tracing::warn!(
                                    "Could not rename the old message cache {}: {err}",
                                    paths.legacy_file.display()
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    fail(&err);
                    if let Ok(mut s) = self.inner.state.write() {
                        s.meta_dirty = true;
                    }
                }
            }
        } else if want_meta {
            // Migration still pending failed threads; keep meta_dirty set until they succeed.
            if let Ok(mut s) = self.inner.state.write() {
                s.meta_dirty = true;
            }
            ok = false;
        }

        ok
    }

    fn write_atomic(&self, file: &Path, data: &str) -> std::io::Result<()> {
        let seq = self.next_seq();
        let mut tmp_os = OsString::from(file.as_os_str());
        tmp_os.push(format!(".{seq}.tmp"));
        let tmp = PathBuf::from(tmp_os);

        let write_res = (|| -> std::io::Result<()> {
            let mut opts = OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(data.as_bytes())?;
            f.flush()?;
            drop(f);
            fs::rename(&tmp, file)?;
            Ok(())
        })();

        if write_res.is_err() {
            remove_quietly(&tmp);
        }
        write_res
    }
}

fn build_search_folded(m: &StoredMessage) -> String {
    let title = m.attachment.as_ref().and_then(|a| a.title.as_deref());
    let desc = m.attachment.as_ref().and_then(|a| a.description.as_deref());
    let mut out = fold_for_search(&m.text);
    if let Some(t) = title {
        out.push('\0');
        out.push_str(&fold_for_search(t));
    }
    if let Some(d) = desc {
        out.push('\0');
        out.push_str(&fold_for_search(d));
    }
    out
}

fn has_missing_items(existing: Option<&[String]>, extra: &[String]) -> bool {
    if extra.is_empty() {
        return false;
    }
    let slice = existing.unwrap_or(&[]);
    extra.iter().any(|item| !slice.contains(item))
}

fn extend_unique(target: &mut Option<Vec<String>>, extra: &[String]) {
    if extra.is_empty() {
        return;
    }
    let vec = target.get_or_insert_with(Vec::new);
    for item in extra {
        if !vec.contains(item) {
            vec.push(item.clone());
        }
    }
}

fn merge_receipts(m: &mut StoredMessage, delivered_to: &[String], seen_by: &[String]) -> bool {
    let need_deliv = has_missing_items(m.delivered_to.as_deref(), delivered_to);
    let need_seen = has_missing_items(m.seen_by.as_deref(), seen_by);
    if !need_deliv && !need_seen {
        return false;
    }
    if need_deliv {
        extend_unique(&mut m.delivered_to, delivered_to);
    }
    if need_seen {
        extend_unique(&mut m.seen_by, seen_by);
    }
    true
}

fn is_hex_thread_file(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    !stem.is_empty()
        && stem.len() % 2 == 0
        && stem
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn list_dir_names(dir: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect()
}

fn remove_quietly(file: &Path) {
    let _ = fs::remove_file(file);
}

fn backup_corrupt(file: &Path, err: &str) {
    let mut backup_os = OsString::from(file.as_os_str());
    backup_os.push(format!(".corrupt.{}", now_ms()));
    let backup = PathBuf::from(backup_os);
    match fs::rename(file, &backup) {
        Ok(()) => tracing::warn!(
            "Backed up unreadable message cache {} to {}: {err}",
            file.display(),
            backup.display()
        ),
        Err(_) => tracing::warn!("Ignoring unreadable message cache {}: {err}", file.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(prefix: &str) -> PathBuf {
        let n: u64 = rand::random();
        let dir = std::env::temp_dir().join(format!("{prefix}-{n:016x}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_raw(overrides: Value) -> Value {
        let mut base = json!({
            "msgId": "1",
            "cliMsgId": "c1",
            "msgType": "webchat",
            "uidFrom": "100",
            "idTo": "me",
            "dName": "Alice",
            "ts": "1700000000000",
            "content": "hello",
        });
        if let (Some(b), Some(o)) = (base.as_object_mut(), overrides.as_object()) {
            for (k, v) in o {
                b.insert(k.clone(), v.clone());
            }
        }
        base
    }

    fn make_msg(id: &str, ts: i64, thread_id: &str, text: &str, is_self: bool) -> StoredMessage {
        let data = make_raw(json!({
            "msgId": id,
            "cliMsgId": format!("c{id}"),
            "uidFrom": if is_self { "me" } else { thread_id },
            "ts": ts.to_string(),
            "content": text,
        }));
        normalize_message(&data, ThreadKind::User, thread_id, is_self, Some("me"))
    }

    // The same vectors are in android/.../ui/MediaTest.kt (`extractOtpCode...`). Keep them in step.
    const OTP_POSITIVE: &[(&str, &str)] = &[
        ("Mã xác thực OTP của bạn là: 482910 (hiệu lực 2 phút)", "482910"),
        ("Your verification code is 9912", "9912"),
        ("739201 là mã xác nhận giao dịch", "739201"),
        (
            "Mã OTP xác thực giao dịch chuyển khoản 5.000.000đ đến TK 0123456789 là 482910",
            "482910",
        ),
        ("Mã OTP của bạn là 482 910", "482910"),
        ("Mã OTP: 482-910 (không chia sẻ)", "482910"),
        ("Your code is 123456", "123456"),
        ("code: 123456", "123456"),
        ("Your OTP: 5566. Do not share.", "5566"),
        ("mã: 123456", "123456"),
        ("Mã của bạn là 1234", "1234"),
        ("123456 is your verification code", "123456"),
        ("G-123456 is your Google verification code", "123456"),
        ("123 456 is your code", "123456"),
        ("Mã OTP giao dịch ngày 12/05/2024 là 482910", "482910"),
        ("Mã OTP xác thực giao dịch năm 2024 là 482910", "482910"),
        ("Mã xác thực OTP\n849201", "849201"),
        ("Mã OTP là 2024", "2024"),
    ];

    const OTP_NEGATIVE: &[&str] = &[
        "Hẹn gặp lúc 14:30 nhé",
        "Chiều nay 17:30 đi ăn nhé",
        "gọi 0901234567",
        "Mã OTP là 0901234567",
        "Chuyển cho mình 5.000.000 nhé",
        "Mã giao dịch 5.000.000đ",
        "Mã OTP 123456789",
        "Mã OTP giao dịch ngày 12/05/2024",
        "Mã OTP hết hạn năm 2025",
        "Sinh nhật mình 12/05/2024",
        "Tôi đi mà 1234 người đến",
        "Đây là mã 123456",
        "Số tài khoản 12345678 của mình",
        "OTP hotline 1900",
        "Mã OTP của bạn đã hết hạn. Gọi 1900 để được hỗ trợ",
        "Bạn đang yêu cầu cài đặt lại mã PIN tài khoản Giang với mã tài khoản 0378795621 tại The 1.",
        "zip code is 70000",
        "Mã OTP 5 000 000đ",
    ];

    #[test]
    fn extract_otp_code_positive_vectors() {
        for (text, code) in OTP_POSITIVE {
            assert_eq!(extract_otp_code(text).as_deref(), Some(*code), "{text}");
        }
    }

    #[test]
    fn extract_otp_code_negative_vectors() {
        for text in OTP_NEGATIVE {
            assert_eq!(extract_otp_code(text), None, "{text}");
        }
    }

    #[test]
    fn plain_text_otp_is_on_the_public_message_but_not_for_own_or_recalled() {
        let msg = make_msg("o1", 1000, "u1", "Mã OTP của bạn là 482910", false);
        let public = to_public_message(&msg, Some("me"));
        assert_eq!(public.otp.as_deref(), Some("482910"));
        let own = make_msg("o2", 1001, "u1", "Mã OTP của bạn là 482910", true);
        assert_eq!(to_public_message(&own, Some("me")).otp, None);
        let plain = make_msg("o3", 1002, "u1", "Hẹn gặp lúc 14:30", false);
        assert_eq!(to_public_message(&plain, Some("me")).otp, None);
        let mut recalled = msg.clone();
        recalled.recalled = true;
        assert_eq!(to_public_message(&recalled, Some("me")).otp, None);
    }

    #[test]
    fn parse_attachment_photos_local_uri_stripping_and_fallbacks() {
        let att = parse_attachment(
            "chat.photo",
            &json!({
                "title": "content://media/external/images/media/42",
                "href": "/storage/emulated/0/DCIM/img.jpg",
                "thumb": "file:///sdcard/thumb.jpg",
                "params": json!({
                    "oriUrl": "https://cdn.zaloapp.com/orig.jpg",
                    "thumbUrl": "https://cdn.zaloapp.com/thumb.jpg",
                    "width": 1280,
                    "height": "720",
                }).to_string(),
            }),
            false,
        )
        .unwrap();
        assert_eq!(att.kind, Some(AttachmentKind::Photo));
        assert_eq!(att.title, None, "local URI caption must be stripped");
        assert_eq!(
            att.href.as_deref(),
            Some("https://cdn.zaloapp.com/orig.jpg")
        );
        assert_eq!(
            att.thumb.as_deref(),
            Some("https://cdn.zaloapp.com/thumb.jpg")
        );
        assert_eq!(att.width, Some(1280));
        assert_eq!(att.height, Some(720));

        // HD param wins over content.href when valid remote URL
        let hd = parse_attachment(
            "chat.photo",
            &json!({
                "href": "https://x/normal.jpg",
                "thumb": "https://x/t.jpg",
                "params": json!({ "hd": "https://x/hd.jpg" }).to_string(),
            }),
            false,
        )
        .unwrap();
        assert_eq!(hd.href.as_deref(), Some("https://x/hd.jpg"));

        // HD upload echo: no content.href/thumb, only hdUrl/rawUrl/thumbUrl in params
        let echo = parse_attachment(
            "chat.photo",
            &json!({
                "params": json!({
                    "hdUrl": "https://x/hd2.jpg",
                    "rawUrl": "https://x/raw.jpg",
                    "thumbUrl": "https://x/t2.jpg",
                    "width": 1080,
                    "height": 5400,
                }).to_string(),
            }),
            true,
        )
        .unwrap();
        assert_eq!(echo.href.as_deref(), Some("https://x/hd2.jpg"));
        assert_eq!(echo.thumb.as_deref(), Some("https://x/t2.jpg"));
        let raw_only = parse_attachment(
            "chat.photo",
            &json!({ "params": json!({ "rawUrl": "https://x/raw.jpg" }).to_string() }),
            true,
        )
        .unwrap();
        assert_eq!(raw_only.href.as_deref(), Some("https://x/raw.jpg"));
    }

    #[test]
    fn parse_attachment_zns_pin_reset_and_otp_cards_and_list_action() {
        // PIN-reset card with 10-digit account ID (must NOT be treated as OTP)
        let pin_raw = make_raw(json!({
            "msgId": "200",
            "msgType": "chat.webcontent.v2",
            "uidFrom": "oa_the1",
            "dName": "The 1",
            "content": {
                "title": "Đổi mã PIN",
                "description": "Bạn đang yêu cầu cài đặt lại mã PIN tài khoản Giang với mã tài khoản 0378795621 tại The 1.",
                "href": "https://the1.vn/reset-pin",
                "params": json!({
                    "actions": [{ "actionTitle": "ĐẶT LẠI MÃ PIN", "actionUrl": "https://the1.vn/reset-pin" }],
                }).to_string(),
            }
        }));
        let norm_pin = normalize_message(&pin_raw, ThreadKind::User, "oa_the1", false, Some("me"));
        assert_eq!(norm_pin.text, "Đổi mã PIN");
        let att = norm_pin.attachment.unwrap();
        assert_eq!(att.kind, Some(AttachmentKind::Link));
        assert_eq!(att.title.as_deref(), Some("Đổi mã PIN"));
        assert_eq!(att.action_label.as_deref(), Some("ĐẶT LẠI MÃ PIN"));
        assert_eq!(att.href.as_deref(), Some("https://the1.vn/reset-pin"));
        assert_eq!(att.otp, None);

        // ZNS OTP card with zinstant_msg + highLightsV2 + actionData code
        let zns = parse_attachment(
            "chat.webcontent.v2",
            &json!({
                "thumb": "https://cdn.example/logo.png",
                "zinstant_msg": json!({
                    "customMsg": {
                        "msg": { "vi": "Mã xác thực OTP\nMã OTP của bạn tại $0$ là 849201." },
                        "highLightsV2": [{ "text": "ZaloPay" }],
                    },
                    "actions": [{ "actionTitle": "Sao chép mã", "actionData": json!({ "code": "849201" }).to_string() }],
                }).to_string(),
            }),
            false,
        )
        .unwrap();
        assert_eq!(zns.kind, Some(AttachmentKind::Link));
        assert_eq!(zns.title.as_deref(), Some("Mã xác thực OTP"));
        assert_eq!(
            zns.description.as_deref(),
            Some("Mã OTP của bạn tại ZaloPay là 849201.")
        );
        assert_eq!(zns.otp.as_deref(), Some("849201"));
        assert_eq!(zns.action_label.as_deref(), Some("Sao chép mã"));

        // OA chat.list.action with array content
        let list_raw = make_raw(json!({
            "msgId": "300",
            "msgType": "chat.list.action",
            "uidFrom": "oa_shop",
            "dName": "Shop OA",
            "content": [
                { "type": "l.a.header.full", "title": "Xác nhận đơn hàng", "description": "Đơn #1234 đang giao", "thumb": "https://cdn.example/box.jpg" },
                { "type": "l.a.child.full", "title": "Theo dõi đơn", "href": "https://shop.example/track/1234" },
            ],
        }));
        let norm_list = normalize_message(&list_raw, ThreadKind::User, "oa_shop", false, Some("me"));
        assert_eq!(norm_list.text, "Xác nhận đơn hàng");
        let latt = norm_list.attachment.unwrap();
        assert_eq!(latt.kind, Some(AttachmentKind::Link));
        assert_eq!(latt.action_label.as_deref(), Some("Theo dõi đơn"));
        assert_eq!(
            latt.href.as_deref(),
            Some("https://shop.example/track/1234")
        );
    }

    #[test]
    fn parse_attachment_stickers_voice_call_logs_and_polls() {
        let sticker = parse_attachment(
            "chat.sticker",
            &json!({ "id": 46991, "catId": 10425, "type": 7 }),
            false,
        )
        .unwrap();
        assert_eq!(sticker.kind, Some(AttachmentKind::Sticker));
        assert_eq!(
            sticker.thumb.as_deref(),
            Some("https://api.zaloapp.com/api/emoticon/sticker/webpc?size=130&eid=46991")
        );
        assert_eq!(
            sticker.sticker,
            Some(StickerRef {
                id: 46991,
                cat_id: 10425,
                sticker_type: 7
            })
        );

        let voice = parse_attachment(
            "chat.voice",
            &json!({ "href": "https://x/a.m4a", "params": "{\"duration\":\"4200\"}" }),
            false,
        )
        .unwrap();
        assert_eq!(voice.kind, Some(AttachmentKind::Voice));
        assert_eq!(voice.duration, Some(4200));

        let call_raw = make_raw(json!({
            "msgType": "chat.recommended",
            "content": {
                "action": "recommened.calltime",
                "title": "",
                "params": json!({ "calltype": 0, "isCaller": 0, "duration": 125, "reason": 0 }).to_string(),
            }
        }));
        let call_msg = normalize_message(&call_raw, ThreadKind::User, "100", false, Some("me"));
        assert_eq!(call_msg.text, "[Incoming voice call]");
        let catt = call_msg.attachment.unwrap();
        assert_eq!(catt.duration, Some(125_000));
        assert_eq!(
            catt.call,
            Some(CallInfo {
                video: false,
                missed: false,
                outgoing: false
            })
        );

        let missed = parse_attachment(
            "chat.recommended",
            &json!({
                "action": "recommened.misscall",
                "params": json!({ "calltype": 1, "isCaller": 0, "reason": 2, "duration": 0 }).to_string(),
            }),
            false,
        )
        .unwrap();
        assert_eq!(missed.title.as_deref(), Some("Missed video call"));
        assert_eq!(missed.duration, None);

        let poll_raw = make_raw(json!({
            "msgType": "group.poll",
            "content": { "poll_id": 999, "question": "Ăn trưa gì?", "action": "create" },
        }));
        let poll_msg = normalize_message(&poll_raw, ThreadKind::Group, "g1", false, Some("me"));
        assert_eq!(poll_msg.text, "[Poll] Ăn trưa gì?");
        assert_eq!(
            poll_msg.attachment.unwrap().poll,
            Some(PollRef {
                poll_id: 999,
                question: "Ăn trưa gì?".into(),
                event: None,
            })
        );
    }

    #[test]
    fn vietnamese_diacritic_insensitive_search() {
        let store = ChatStore::in_memory();
        store.set_own_id(Some("me".into()));
        store.upsert_thread(ThreadInfo {
            id: "100".into(),
            kind: ThreadKind::User,
            name: "Minh".into(),
            avatar: "https://a/100.jpg".into(),
        });
        store.add_message(make_msg("1", 1000, "100", "Chào đệp nhé!", false));
        store.add_message(make_msg("2", 2000, "100", "Đường Điện Biên Phủ", false));
        store.add_message(make_msg("3", 3000, "100", "Chào buổi sáng đã thu hồi", false));
        store.apply_undo("3");

        let hits = store.search_messages("chao", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message.id, "1");
        assert_eq!(hits[0].thread_name, "Minh");

        let dep_hits = store.search_messages("dep", None);
        assert_eq!(dep_hits.len(), 1);
        assert_eq!(dep_hits[0].message.id, "1");

        let duong_hits = store.search_messages("DUONG DIEN BIEN", None);
        assert_eq!(duong_hits.len(), 1);
        assert_eq!(duong_hits[0].message.id, "2");

        // NFD decomposed input also matches
        let nfd_hits = store.search_messages("Cha\u{0300}o", None);
        assert_eq!(nfd_hits.len(), 1);
        assert_eq!(nfd_hits[0].message.id, "1");
    }

    #[test]
    fn thread_sorting_unread_reactions_recalls_and_plain_migration() {
        let store = ChatStore::in_memory();
        store.set_own_id(Some("me".into()));

        store.upsert_thread(ThreadInfo {
            id: "a".into(),
            kind: ThreadKind::User,
            name: "Alice".into(),
            avatar: "".into(),
        });
        store.upsert_thread(ThreadInfo {
            id: "b".into(),
            kind: ThreadKind::User,
            name: "Bob".into(),
            avatar: "".into(),
        });
        store.add_message(make_msg("1", 5000, "a", "hi", false));
        store.add_message(make_msg("2", 1000, "b", "yo", false));

        // Unread counts
        assert_eq!(
            store
                .list_threads()
                .iter()
                .map(|t| (t.id.as_str(), t.unread))
                .collect::<Vec<_>>(),
            vec![("a", 1), ("b", 1)]
        );

        // Pin "b" so it sorts ahead of newer "a"
        store.set_pinned(ThreadKind::User, "b", true);
        assert_eq!(
            store
                .list_threads()
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );

        // Reactions + own flag
        store.apply_reaction("1", "a", "/-heart");
        store.apply_reaction("1", "me", "/-heart");
        store.apply_reaction("1", "c", "/-strong");
        let msgs = store.get_messages(ThreadKind::User, "a", None, None);
        assert_eq!(msgs[0].reactions.len(), 2);
        assert_eq!(msgs[0].reactions[0].key, "heart");
        assert_eq!(msgs[0].reactions[0].count, 2);
        assert!(msgs[0].reactions[0].mine);
        assert_eq!(msgs[0].reactions[1].key, "like");
        assert_eq!(msgs[0].reactions[1].count, 1);
        assert!(!msgs[0].reactions[1].mine);

        // Recall clears reactions & sets text
        store.apply_undo("1");
        let msgs = store.get_messages(ThreadKind::User, "a", None, None);
        assert!(msgs[0].recalled);
        assert_eq!(msgs[0].text, "Message recalled");
        assert!(msgs[0].reactions.is_empty());

        // Plain -> noised migration
        let mut hist = make_msg("h1", 500, "999", "old history", false);
        hist.from_name.clear();
        hist.raw = None;
        store.add_history_message(hist);
        let mut imported = HashSet::new();
        imported.insert("h1".to_string());
        store.settle_history(ThreadKind::User, "999", &imported);
        assert!(store.migrate_plain_thread(ThreadKind::User, "999", "a"));
        assert!(store.get_thread(ThreadKind::User, "999").is_none());
        let a_msgs = store.get_messages(ThreadKind::User, "a", None, None);
        assert_eq!(a_msgs.len(), 2);
        assert_eq!(a_msgs[0].id, "h1");
        assert_eq!(a_msgs[0].thread_id, "a");
        assert_eq!(a_msgs[0].from_name, "Alice");
    }

    #[test]
    fn official_account_flag_persists_and_serializes() {
        let dir = temp_dir("zalo-oa");
        {
            let store = ChatStore::open(&dir);
            store.add_message(make_msg("1", 1000, "oa1", "ad", false));
            store.add_message(make_msg("2", 1001, "friend", "hi", false));
            assert!(store.mark_official_account("oa1"));
            assert!(!store.mark_official_account("oa1"), "second mark is a no-op");
            assert!(!store.mark_official_account(""));
            assert!(store.is_official_account("oa1"));
            let list = store.list_threads();
            let oa = list.iter().find(|t| t.id == "oa1").unwrap();
            let fr = list.iter().find(|t| t.id == "friend").unwrap();
            assert!(oa.is_official_account && !fr.is_official_account);
            let v = serde_json::to_value(oa).unwrap();
            assert_eq!(v["isOfficialAccount"], json!(true));
            store.flush();
        }
        let store = ChatStore::open(&dir);
        assert!(store.is_official_account("oa1"));
        assert!(!store.is_official_account("friend"));
        store.delete_thread(ThreadKind::User, "oa1");
        assert!(!store.is_official_account("oa1"));
        // Old meta files without the field load with the flag off.
        let meta: MetaFile = serde_json::from_str(r#"{"version":1,"threads":{}}"#).unwrap();
        assert!(meta.official_accounts.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persistence_roundtrip_and_legacy_migration() {
        let dir = temp_dir("zalo-store-test");
        let legacy = dir.join("messages.json");

        // 1. Fresh write to messages/meta.json + messages/threads/<hex>.json
        {
            let store = ChatStore::open(&dir);
            store.set_own_id(Some("me".into()));
            store.add_message(make_msg("1", 1000, "100", "Xin chào", false));
            store.apply_reaction("1", "100", "/-heart");
            store.set_pinned(ThreadKind::User, "100", true);
            store.flush();

            assert!(!legacy.exists());
            let cache_dir = dir.join("messages");
            assert!(cache_dir.join("meta.json").exists());
            let hex_file = cache_dir.join("threads").join("757365723a313030.json");
            assert!(hex_file.exists());

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(fs::metadata(&cache_dir).unwrap().permissions().mode() & 0o777, 0o700);
                assert_eq!(
                    fs::metadata(cache_dir.join("meta.json")).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                assert_eq!(fs::metadata(&hex_file).unwrap().permissions().mode() & 0o777, 0o600);
            }
        }

        // Reload and verify
        {
            let store = ChatStore::open(&dir);
            store.set_own_id(Some("me".into()));
            let msgs = store.get_messages(ThreadKind::User, "100", None, None);
            assert_eq!(msgs.len(), 1);
            assert_eq!(msgs[0].text, "Xin chào");
            assert_eq!(msgs[0].reactions[0].key, "heart");
            // Ensure raw is preserved on disk / find_message but stripped from ChatMessage JSON
            assert!(store.find_message("1").unwrap().raw.is_some());
            let pub_json = serde_json::to_value(&msgs[0]).unwrap();
            assert!(pub_json.get("raw").is_none());
            assert!(store.list_threads()[0].pinned);
        }

        let _ = fs::remove_dir_all(&dir);

        // 2. Legacy messages.json migration -> messages/ + messages.json.migrated
        let mig_dir = temp_dir("zalo-store-migrate");
        let legacy_path = mig_dir.join("messages.json");
        let snapshot = json!({
            "threads": {
                "user:100": { "id": "100", "kind": "user", "name": "Alice", "avatar": "https://a/1.jpg" }
            },
            "messages": {
                "user:100": [
                    {
                        "id": "m1",
                        "cliMsgId": "cm1",
                        "threadId": "100",
                        "threadKind": "user",
                        "fromId": "100",
                        "fromName": "Alice",
                        "isSelf": false,
                        "ts": 1700000000000i64,
                        "msgType": "webchat",
                        "text": "Legacy hello"
                    }
                ]
            },
            "readUpTo": { "user:100": 1700000000000i64 },
            "pinned": ["user:100"],
            "mutedUntil": { "user:100": -1 },
            "markedUnread": []
        });
        fs::write(&legacy_path, serde_json::to_string(&snapshot).unwrap()).unwrap();

        let migrated = ChatStore::open(&mig_dir);
        assert!(!legacy_path.exists());
        assert!(mig_dir.join("messages.json.migrated").exists());
        assert!(mig_dir.join("messages").join("meta.json").exists());
        assert!(mig_dir
            .join("messages")
            .join("threads")
            .join("757365723a313030.json")
            .exists());
        let threads = migrated.list_threads_at(1000);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].name, "Alice");
        assert!(threads[0].pinned);
        assert_eq!(threads[0].muted_until, Some(-1));
        assert_eq!(threads[0].unread, 0);

        // Second clean load removes the .migrated backup
        let reloaded = ChatStore::open(&mig_dir);
        assert!(!mig_dir.join("messages.json.migrated").exists());
        assert_eq!(
            reloaded.get_messages(ThreadKind::User, "100", None, None)[0].text,
            "Legacy hello"
        );

        // Logout clear removes everything
        reloaded.clear();
        reloaded.flush();
        assert!(ChatStore::open(&mig_dir).list_threads().is_empty());

        let _ = fs::remove_dir_all(&mig_dir);
    }

    #[test]
    fn corrupt_files_are_quarantined() {
        let dir = temp_dir("zalo-store-corrupt");
        let legacy = dir.join("messages.json");
        fs::write(&legacy, "{not valid json").unwrap();

        let store = ChatStore::open(&dir);
        assert!(store.list_threads().is_empty());
        let backups: Vec<String> = list_dir_names(&dir)
            .into_iter()
            .filter(|n| n.starts_with("messages.json.corrupt."))
            .collect();
        assert_eq!(backups.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn receipts_pagination_quotes_mentions_and_mark_unread() {
        let store = ChatStore::in_memory();
        store.set_own_id(Some("me".into()));

        // Receipts: 1:1 cascades to earlier self messages; groups track seenCount per message
        store.add_message(make_msg("1", 1000, "100", "a", true));
        store.add_message(make_msg("2", 2000, "100", "b", true));
        store.add_message(make_msg("3", 3000, "100", "c", true));

        let d = store.apply_delivery(ThreadKind::User, "100", "2", &["100".into()]);
        assert_eq!(d.len(), 2);
        let s = store.apply_seen(ThreadKind::User, "100", "1", &["100".into()]);
        assert_eq!(s.len(), 1);
        assert!(store
            .apply_seen(ThreadKind::User, "100", "1", &["100".into()])
            .is_empty());

        let statuses: Vec<_> = store
            .get_messages(ThreadKind::User, "100", None, None)
            .into_iter()
            .map(|m| m.status)
            .collect();
        assert_eq!(
            statuses,
            vec![
                Some(MessageStatus::Seen),
                Some(MessageStatus::Delivered),
                Some(MessageStatus::Sent),
            ]
        );

        let g_raw = make_raw(json!({ "msgId": "g1m", "uidFrom": "me", "ts": "4000", "content": "hi group" }));
        let g_msg = normalize_message(&g_raw, ThreadKind::Group, "g1", true, Some("me"));
        store.add_message(g_msg);
        store.apply_seen(ThreadKind::Group, "g1", "g1m", &["a".into(), "b".into()]);
        store.apply_seen(ThreadKind::Group, "g1", "g1m", &["b".into(), "c".into()]);
        let gm = &store.get_messages(ThreadKind::Group, "g1", None, None)[0];
        assert_eq!(gm.status, Some(MessageStatus::Seen));
        assert_eq!(gm.seen_count, Some(3));

        // Pagination with before & limit
        for i in 4..=10 {
            store.add_message(make_msg(&i.to_string(), i * 1000, "100", "x", false));
        }
        let page: Vec<String> = store
            .get_messages(ThreadKind::User, "100", Some(6000), Some(2))
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(page, vec!["4", "5"]);

        // Quotes, mentions, mentionsMe (direct and @All "-1"), and recalled_message_id
        let q_raw = make_raw(json!({
            "msgId": "q1",
            "uidFrom": "200",
            "ts": "9000",
            "content": "@Me look",
            "quote": { "ownerId": 200, "globalMsgId": 42, "fromD": "Bob", "msg": "" },
            "mentions": [{ "uid": "me", "pos": 0, "len": 3 }],
        }));
        let qm = normalize_message(&q_raw, ThreadKind::Group, "g1", false, Some("me"));
        assert_eq!(
            qm.quote,
            Some(QuoteInfo {
                msg_id: "42".into(),
                from_id: "200".into(),
                from_name: "Bob".into(),
                text: "[Attachment]".into(),
            })
        );
        assert!(store.to_public(&qm).mentions_me);

        let undo_raw = make_raw(json!({ "content": { "deleteMsg": 1, "globalMsgId": 77 } }));
        assert_eq!(recalled_message_id(&undo_raw).as_deref(), Some("77"));
        assert_eq!(recalled_message_id(&make_raw(json!({}))), None);

        // Mark unread / read & replaying older self message
        store.mark_read(ThreadKind::User, "100", Some(20_000));
        store.mark_unread(ThreadKind::User, "100");
        assert_eq!(
            store.list_threads().iter().find(|t| t.id == "100").unwrap().unread,
            1
        );
        store.add_message(make_msg("self-old", 15_000, "100", "old", true));
        assert_eq!(
            store.list_threads().iter().find(|t| t.id == "100").unwrap().unread,
            1
        );
        store.add_message(make_msg("self-new", 25_000, "100", "new", true));
        assert_eq!(
            store.list_threads().iter().find(|t| t.id == "100").unwrap().unread,
            0
        );
    }

    #[test]
    fn parse_attachment_video_file_contact_ecard_and_direct_call_types() {
        let video = parse_attachment(
            "chat.video.msg",
            &json!({
                "href": "https://x/v.mp4",
                "thumb": "https://x/v.jpg",
                "params": json!({ "duration": 12500, "video_width": 720, "video_height": 1280 }).to_string(),
            }),
            false,
        )
        .unwrap();
        assert_eq!(video.kind, Some(AttachmentKind::Video));
        assert_eq!(video.width, Some(720));
        assert_eq!(video.height, Some(1280));
        assert_eq!(video.duration, Some(12500));

        let file = parse_attachment(
            "share.file",
            &json!({
                "title": "Báo cáo.PDF",
                "href": "https://x/f",
                "params": json!({ "fileSize": "20480" }).to_string(),
            }),
            false,
        )
        .unwrap();
        assert_eq!(file.kind, Some(AttachmentKind::File));
        assert_eq!(file.size, Some(20480));
        assert_eq!(file.file_ext.as_deref(), Some("pdf"));

        let contact = parse_attachment(
            "chat.recommended",
            &json!({ "action": "recommened.user", "title": "An", "thumb": "https://x/a.jpg", "params": "123456" }),
            false,
        )
        .unwrap();
        assert_eq!(contact.kind, Some(AttachmentKind::Contact));
        assert_eq!(contact.user_id.as_deref(), Some("123456"));

        let ecard = parse_attachment(
            "chat.ecard",
            &json!({
                "title": "Chúc mừng sinh nhật",
                "description": "Gửi lời chúc tốt đẹp",
                "thumb": "https://x/card.png",
                "href": "https://x/card",
                "params": json!({ "actions": [{ "name": "Mở thiệp" }] }).to_string(),
            }),
            false,
        )
        .unwrap();
        assert_eq!(ecard.kind, Some(AttachmentKind::Link));
        assert_eq!(ecard.action_label.as_deref(), Some("Mở thiệp"));

        let direct_call = parse_attachment(
            "call.video",
            &json!({ "duration": 30 }),
            true,
        )
        .unwrap();
        assert_eq!(direct_call.kind, Some(AttachmentKind::Call));
        assert_eq!(direct_call.title.as_deref(), Some("Outgoing video call"));
        assert_eq!(direct_call.duration, Some(30_000));
    }

    fn norm(over: Value) -> StoredMessage {
        normalize_message(&make_raw(over), ThreadKind::User, "100", false, Some("me"))
    }

    #[test]
    fn deleted_and_recalled_payload_shapes_all_become_recalled_messages() {
        let shapes = [
            // zca-js undo record delivered as a history message with its own id
            json!({ "content": { "deleteMsg": 1, "globalMsgId": 77 } }),
            json!({ "content": { "deleteMsg": 0 } }),
        ];
        for over in shapes {
            let m = norm(json!({
                "quote": { "msg": "x", "globalMsgId": 1, "ownerId": "1", "fromD": "A" },
                "mentions": [{ "uid": "1", "pos": 0, "len": 1 }]
            }));
            assert!(!m.recalled, "control: plain message stays plain");
            let mut full = over.clone();
            full["quote"] = json!({ "msg": "x", "globalMsgId": 1, "ownerId": "1", "fromD": "A" });
            let m = norm(full);
            assert!(m.recalled, "{over}");
            assert_eq!(m.text, "Message recalled");
            assert!(m.attachment.is_none() && m.quote.is_none() && m.mentions.is_empty());
            assert!(m.raw.as_ref().unwrap().content.is_none());
        }
        // guessed msgTypes are no longer treated as deletions (zca-js has no such evidence)
        for t in ["chat.undo", "chat.delete", "chat.recalled", "chat.deleted", "undo"] {
            assert!(!norm(json!({ "msgType": t, "content": "x" })).recalled, "{t}");
        }
        // ordinary text is untouched, even when it reads like a deletion notice
        assert!(!norm(json!({ "content": "tin nhắn" })).recalled);
        assert!(!norm(json!({ "content": "Tin nhắn đã bị xóa" })).recalled);
        assert!(!norm(json!({ "content": "Tin nhắn đã được thu hồi" })).recalled);
    }

    #[test]
    fn delete_for_me_tombstones_persist_and_block_re_adding() {
        let store = ChatStore::in_memory();
        store.add_message(make_msg("1", 1000, "100", "Dd", true));
        store.add_message(make_msg("2", 2000, "100", "keep", true));
        store.apply_undo("1");
        assert!(store.delete_for_me(ThreadKind::User, "100", "1").unwrap().recalled);
        assert!(store.get_message("1").is_none());
        assert!(!store.add_message(make_msg("1", 1000, "100", "Dd", true)));
        assert!(!store.add_history_message(make_msg("1", 1000, "100", "Dd", true)));
        assert!(store.get_message("1").is_none());
        // unknown ids are tombstoned too (nothing removed)
        assert!(store.delete_for_me(ThreadKind::User, "100", "9").is_none());
        assert!(store.is_deleted_for_me(ThreadKind::User, "100", "9"));
        assert!(!store.is_deleted_for_me(ThreadKind::Group, "100", "9"));
        // survives serialization
        let meta = store.inner.state.read().unwrap().meta_json();
        let parsed: MetaFile = serde_json::from_str(&meta).unwrap();
        assert_eq!(parsed.deleted_ids["user:100"], vec!["1".to_string(), "9".to_string()]);
        // old meta without the field still loads
        let old: MetaFile = serde_json::from_str("{\"version\":1}").unwrap();
        assert!(old.deleted_ids.is_empty());
    }

    #[test]
    fn undo_without_target_is_detected() {
        assert!(raw_is_undo_without_target(&make_raw(json!({ "content": { "deleteMsg": 1 } }))));
        assert!(!raw_is_undo_without_target(&make_raw(
            json!({ "content": { "deleteMsg": 1, "globalMsgId": 5 } })
        )));
        assert!(!raw_is_undo_without_target(&make_raw(json!({ "content": "hi" }))));
    }

    #[test]
    fn embed_shapes_and_unsupported_fallback() {
        // recommened.link with params.mediaTitle / domain
        let m = norm(json!({
            "msgType": "chat.recommended",
            "content": {
                "action": "recommened.link",
                "href": "https://news.example.com/a/1",
                "thumb": "https://img.example.com/t.jpg",
                "params": json!({ "mediaTitle": "Headline", "src": "news.example.com", "desc": "Summary" }).to_string(),
            }
        }));
        let a = m.attachment.unwrap();
        assert_eq!(a.kind, Some(AttachmentKind::Link));
        assert_eq!(a.title.as_deref(), Some("Headline"));
        assert_eq!(a.description.as_deref(), Some("Summary"));
        assert_eq!(a.domain.as_deref(), Some("news.example.com"));
        assert_eq!(m.text, "[Link] Headline");

        // unknown msgType with title/href/thumb in params -> embed, not "[Unsupported message]"
        let m = norm(json!({
            "msgType": "chat.mini.app",
            "content": { "params": { "title": "Mini", "url": "https://www.app.example.org/x", "image": "https://i.example.org/p.png" } }
        }));
        let a = m.attachment.unwrap();
        assert_eq!(a.kind, Some(AttachmentKind::Embed));
        assert_eq!(a.href.as_deref(), Some("https://www.app.example.org/x"));
        assert_eq!(a.thumb.as_deref(), Some("https://i.example.org/p.png"));
        assert_eq!(a.domain.as_deref(), Some("app.example.org"));
        assert_eq!(m.text, "Mini");

        // thumb only, unknown type: still an embed, labelled [Link]
        let m = norm(json!({
            "msgType": "chat.zzz",
            "content": { "thumb": "https://i.example.org/p.png" }
        }));
        assert_eq!(m.attachment.unwrap().kind, Some(AttachmentKind::Embed));
        assert_eq!(m.text, "[Link]");

        // unknown recommended action
        let a = parse_attachment(
            "chat.recommended",
            &json!({ "action": "recommened.zzz", "title": "T", "href": "https://x.example/y" }),
            false,
        )
        .unwrap();
        assert_eq!(a.kind, Some(AttachmentKind::Embed));

        // todo
        let a = parse_attachment(
            "chat.todo",
            &json!({ "params": json!({ "title": "Buy milk", "note": "2 litres" }).to_string() }),
            false,
        )
        .unwrap();
        assert_eq!(a.kind, Some(AttachmentKind::Embed));
        assert_eq!(a.title.as_deref(), Some("Buy milk"));
        assert_eq!(a.description.as_deref(), Some("2 litres"));

        // location without href gets a maps link from coordinates
        let a = parse_attachment(
            "chat.location.new",
            &json!({ "title": "Here", "params": json!({ "latitude": "10.5", "longitude": 106.25 }).to_string() }),
            false,
        )
        .unwrap();
        assert_eq!(a.kind, Some(AttachmentKind::Location));
        assert_eq!(a.href.as_deref(), Some("https://maps.google.com/?q=10.5,106.25"));

        // non-http hrefs are not turned into embed links
        let a = parse_attachment("chat.zzz", &json!({ "title": "T", "params": { "url": "javascript:alert(1)" } }), false).unwrap();
        assert_eq!(a.href, None);

        // truly nothing to show
        let m = norm(json!({ "msgType": "chat.zzz", "content": { "foo": 1 } }));
        assert!(m.attachment.is_none());
        assert_eq!(m.text, "[Unsupported message]");
    }

    #[test]
    fn contact_card_phone_and_user_id_shapes() {
        let a = parse_attachment(
            "chat.recommended",
            &json!({ "action": "recommened.user", "title": "An", "description": "+84 901 234 567", "params": "123456" }),
            false,
        )
        .unwrap();
        assert_eq!(a.user_id.as_deref(), Some("123456"));
        assert_eq!(a.phone.as_deref(), Some("+84 901 234 567"));

        let a = parse_attachment(
            "chat.recommended",
            &json!({ "action": "recommened.user", "title": "An", "href": "https://zalo.me/84901234567",
                     "params": json!({ "user_id": 99, "mobile": "0901234567" }).to_string() }),
            false,
        )
        .unwrap();
        assert_eq!(a.kind, Some(AttachmentKind::Contact));
        assert_eq!(a.user_id.as_deref(), Some("99"));
        assert_eq!(a.phone.as_deref(), Some("0901234567"));

        let a = parse_attachment(
            "chat.recommended",
            &json!({ "action": "recommened.user", "title": "An", "href": "https://zalo.me/84901234567", "params": "5" }),
            false,
        )
        .unwrap();
        assert_eq!(a.phone.as_deref(), Some("84901234567"));
    }

    #[test]
    fn corrupt_meta_and_corrupt_thread_recovery() {
        let dir = temp_dir("zalo-store-corrupt-meta");
        {
            let a = ChatStore::open(&dir);
            a.allow_messages(HISTORY_MESSAGES_PER_THREAD);
            for i in 0..600 {
                a.add_message(make_msg(&format!("a{i}"), 1000 + i, "a", "msg", false));
            }
            a.add_message(make_msg("b1", 2000, "b", "other", false));
            a.flush();
        }
        // Corrupt meta.json and thread b
        fs::write(dir.join("messages").join("meta.json"), "garbage").unwrap();
        fs::write(
            dir.join("messages")
                .join("threads")
                .join(format!("{}.json", hex::encode("user:b"))),
            "{broken",
        )
        .unwrap();

        let b = ChatStore::open(&dir);
        assert!(b.find_message("a0").is_some(), "all 600 messages in a recovered");
        assert_eq!(b.get_messages(ThreadKind::User, "a", None, Some(500)).len(), 500);
        // History recovered without meta.json is marked read so it doesn't flood unread
        let a_summary = b.list_threads().into_iter().find(|t| t.id == "a").unwrap();
        assert_eq!(a_summary.unread, 0);
        // Raised message_cap preserved on recovery
        b.add_message(make_msg("a_new", 9000, "a", "new", false));
        assert!(b.find_message("a0").is_some());

        let meta_backups: Vec<_> = list_dir_names(&dir.join("messages"))
            .into_iter()
            .filter(|n| n.starts_with("meta.json.corrupt."))
            .collect();
        assert_eq!(meta_backups.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn slim_raw_ref_deduplicates_and_loads_legacy_files() {
        // 1. Plain-text message omits duplicate content, msgType, uidFrom, ts, and empty propertyExt.
        let text_msg = normalize_message(
            &json!({
                "msgId": "t1",
                "cliMsgId": "c1",
                "msgType": "webchat",
                "uidFrom": "100",
                "idTo": "me",
                "dName": "Alice",
                "ts": "1700000000000",
                "content": "hello world",
                "propertyExt": {},
                "ttl": 0,
            }),
            ThreadKind::User,
            "100",
            false,
            Some("me"),
        );
        let raw = text_msg.raw.as_deref().unwrap();
        assert!(raw.content.is_none(), "plain text should not duplicate content");
        assert!(raw.property_ext.is_none(), "empty propertyExt should be dropped");
        assert!(raw.msg_type.is_none());
        assert!(raw.uid_from.is_none());
        assert_eq!(raw.id_to.as_deref(), Some("me"));
        assert!(raw.ts.is_none());
        assert_eq!(raw.content_for_quote(&text_msg), json!("hello world"));
        assert_eq!(raw.plain_text(&text_msg), Some("hello world"));
        assert_eq!(raw.resolved_uid_from(&text_msg), "100");
        assert_eq!(raw.resolved_id_to(&text_msg), "me");
        assert_eq!(raw.resolved_msg_type(&text_msg), "webchat");
        assert_eq!(raw.resolved_ts(&text_msg), "1700000000000");

        // 2. Non-text attachment strips bulky non-quote keys & drops propertyExt.
        let photo_msg = normalize_message(
            &json!({
                "msgId": "p1",
                "cliMsgId": "cp1",
                "msgType": "chat.photo",
                "uidFrom": "100",
                "idTo": "me",
                "ts": "1700000001000",
                "propertyExt": { "huge": [1, 2, 3] },
                "content": {
                    "href": "https://cdn.example/img.jpg",
                    "thumb": "https://cdn.example/thumb.jpg",
                    "title": "pic",
                    "tracking": { "bulky": "x".repeat(500) },
                    "extra": "drop me"
                }
            }),
            ThreadKind::User,
            "100",
            false,
            Some("me"),
        );
        let praw = photo_msg.raw.as_deref().unwrap();
        assert!(praw.property_ext.is_none());
        let pcontent = praw.content.as_deref().unwrap();
        assert_eq!(pcontent["href"], "https://cdn.example/img.jpg");
        assert_eq!(pcontent["thumb"], "https://cdn.example/thumb.jpg");
        assert!(pcontent.get("tracking").is_none());
        assert!(pcontent.get("extra").is_none());

        // 3. Legacy on-disk thread file with full unslimmed RawRef loads & slims cleanly.
        let dir = temp_dir("zalo-store-legacy-raw");
        let threads_dir = dir.join("messages").join("threads");
        fs::create_dir_all(&threads_dir).unwrap();
        let legacy_thread = json!({
            "version": 1,
            "messages": [
                {
                    "id": "leg1",
                    "cliMsgId": "cleg1",
                    "threadId": "100",
                    "threadKind": "user",
                    "fromId": "100",
                    "fromName": "Alice",
                    "isSelf": false,
                    "ts": 1000,
                    "msgType": "webchat",
                    "text": "legacy hello",
                    "raw": {
                        "msgType": "webchat",
                        "content": "legacy hello",
                        "propertyExt": {},
                        "uidFrom": "100",
                        "idTo": "me",
                        "ts": "1000",
                        "ttl": 0,
                        "st": 1,
                        "at": 2,
                        "cmd": 501
                    }
                }
            ]
        });
        fs::write(
            threads_dir.join(format!("{}.json", hex::encode("user:100"))),
            serde_json::to_string(&legacy_thread).unwrap(),
        )
        .unwrap();

        let store = ChatStore::open(&dir);
        let loaded = store.find_message("leg1").unwrap();
        let lraw = loaded.raw.as_deref().unwrap();
        assert!(lraw.content.is_none());
        assert!(lraw.property_ext.is_none());
        assert_eq!(lraw.content_for_quote(&loaded), json!("legacy hello"));
        assert_eq!(lraw.plain_text(&loaded), Some("legacy hello"));
        assert_eq!(lraw.resolved_uid_from(&loaded), "100");
        assert_eq!(lraw.resolved_id_to(&loaded), "me");
        assert_eq!(lraw.st, 1);
        assert_eq!(lraw.at, 2);
        assert_eq!(lraw.cmd, 501);
        let _ = fs::remove_dir_all(&dir);
    }
}

