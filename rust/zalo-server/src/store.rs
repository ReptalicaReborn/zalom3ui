//! Message normalization, attachment parsing, OTP detection, Vietnamese diacritic-insensitive
//! search, and crash-safe per-thread persistence (`ChatStore`), matching `src/store.ts`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::log::create_dir_private;
use crate::types::{
    is_local_device_uri, reaction_by_code, sanitize_remote_url, thread_key, Attachment,
    AttachmentKind, CallInfo, ChatMessage, MentionInfo, MessageSearchHit, MessageStatus, PollRef,
    QuoteInfo, RawRef, ReactionInfo, StickerRef, StoredMessage, ThreadInfo, ThreadKind,
    ThreadSummary, STICKER_IMAGE_URL,
};

pub const MAX_MESSAGES_PER_THREAD: usize = 500;
/// Per-chat limit once a history import has run: the phone's older messages would otherwise be evicted at once.
pub const HISTORY_MESSAGES_PER_THREAD: usize = 2000;

const META_FILE: &str = "meta.json";
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

/// Folds text for diacritic-insensitive, case-insensitive Vietnamese search.
pub fn fold_for_search(s: &str) -> String {
    fold_diacritics(s).to_lowercase()
}

// ---- OTP extraction ----

fn otp_keyword_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich|pin)|verification\s*code|security\s*code|auth(?:entication)?\s*code|confirm(?:ation)?\s*code|passcode|one[- ]time(?:\s*(?:code|password|pin))?)\b",
        )
        .expect("valid OTP keyword regex")
    })
}

fn otp_before_keyword_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:la\s+)?(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich)|verification\s*code|security\s*code|passcode)\b",
        )
        .expect("valid OTP before-keyword regex")
    })
}

/// Extracts a 4–8 digit OTP / verification code when accompanied by an OTP keyword (Vietnamese or
/// English). Ignores 9+ digit phone numbers / account IDs (e.g. `0378795621`).
pub fn extract_otp_code(text: &str) -> Option<String> {
    let folded = fold_diacritics(text);
    let kw_re = otp_keyword_re();
    if !kw_re.is_match(&folded) {
        return None;
    }

    // 1. Keyword followed by up to 40 non-digit, non-newline chars, then a standalone 4–8 digit run.
    for m in kw_re.find_iter(&folded) {
        let rest = &folded[m.end()..];
        let mut non_digits = 0usize;
        for (byte_idx, ch) in rest.char_indices() {
            if ch == '\n' {
                break;
            }
            if ch.is_ascii_digit() {
                let digits_slice = &rest[byte_idx..];
                let digit_len = digits_slice
                    .bytes()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                if (4..=8).contains(&digit_len) {
                    return Some(digits_slice[..digit_len].to_string());
                }
                // `[^\d\n]` cannot cross a digit run, so stop scanning after this keyword.
                break;
            }
            non_digits += 1;
            if non_digits > 40 {
                break;
            }
        }
    }

    // 2. Standalone 4–8 digit run followed by up to 24 non-digit, non-newline chars and keyword.
    for m in otp_before_keyword_re().find_iter(&folded) {
        let prefix = &folded[..m.start()];
        let mut non_digits = 0usize;
        for (byte_idx, ch) in prefix.char_indices().rev() {
            if ch == '\n' {
                break;
            }
            if ch.is_ascii_digit() {
                let end = byte_idx + ch.len_utf8();
                let mut start = byte_idx;
                while start > 0 && prefix.as_bytes()[start - 1].is_ascii_digit() {
                    start -= 1;
                }
                let digit_len = end - start;
                if (4..=8).contains(&digit_len) {
                    return Some(prefix[start..end].to_string());
                }
                break;
            }
            non_digits += 1;
            if non_digits > 24 {
                break;
            }
        }
    }

    None
}

// ---- Message normalization & attachment parsing ----

fn call_action_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^recommened\.(misscall|calling|receivecall|calltime)$")
            .expect("valid call action regex")
    })
}

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
    let is_call_msg_type = msg_type == "call.voice" || msg_type == "call.video";
    if call_action_re().is_match(&action) || is_call_msg_type {
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
    let raw_title = non_empty_str(content_map.get("title"));
    let title = raw_title.filter(|t| !(is_media && is_local_device_uri(t)));
    let description = non_empty_str(content_map.get("description"));
    let href = if is_media {
        (if kind == AttachmentKind::Photo {
            remote_str(params.get("hd"))
        } else {
            None
        })
        .or_else(|| remote_str(content_map.get("href")))
        .or_else(|| remote_str(params.get("oriUrl")))
        .or_else(|| remote_str(params.get("origUrl")))
        .or_else(|| remote_str(params.get("normalUrl")))
        .or_else(|| remote_str(params.get("url")))
    } else {
        non_empty_str(content_map.get("href"))
    };
    let thumb = if is_media {
        remote_str(content_map.get("thumb"))
            .or_else(|| remote_str(params.get("thumbUrl")))
            .or_else(|| remote_str(params.get("thumb")))
    } else {
        non_empty_str(content_map.get("thumb"))
    };
    let width = first_pos_u32(&[params.get("width"), params.get("video_width"), params.get("w")]);
    let height =
        first_pos_u32(&[params.get("height"), params.get("video_height"), params.get("h")]);
    let duration = first_pos_u64(&[params.get("duration")]);
    let size = first_pos_u64(&[params.get("fileSize"), params.get("size")]);
    let mut file_ext = non_empty_str(params.get("fileExt")).map(|s| s.to_ascii_lowercase());
    let mut user_id = None;

    if kind == AttachmentKind::Contact {
        if let Some(Value::String(p)) = content_map.get("params") {
            if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                user_id = Some(p.clone());
            }
        }
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

    let shows =
        title.is_some() || href.is_some() || thumb.is_some() || description.is_some();
    if shows || kind != AttachmentKind::Other {
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
            ..Default::default()
        })
    } else {
        None
    }
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
    let property_ext = raw_data
        .get("propertyExt")
        .filter(|v| !v.is_null())
        .cloned();

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

    let mut msg = StoredMessage {
        id: msg_id,
        cli_msg_id,
        thread_id: thread_id.to_string(),
        thread_kind,
        from_id: uid_from.clone(),
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
        raw: Some(RawRef {
            msg_type: msg_type.clone(),
            content: content_val.clone(),
            property_ext,
            uid_from,
            id_to,
            ts: ts_str,
            ttl: val_to_i64(raw_data.get("ttl")),
            st: val_to_i64(raw_data.get("st")),
            at: val_to_i64(raw_data.get("at")),
            cmd: val_to_i64(raw_data.get("cmd")),
        }),
    };

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

    let label = known.unwrap_or("[Unsupported message]");
    let att_kind = attachment.as_ref().and_then(|a| a.kind);
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetaFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message_cap: Option<usize>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadFile {
    #[serde(default)]
    key: String,
    messages: Vec<StoredMessage>,
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
    /// message id -> thread key
    index: HashMap<String, String>,
    own_id: Option<String>,
    message_cap: usize,
    /// Cached unread incoming count per thread at the read marker it was computed for.
    unread_counts: HashMap<String, (i64, usize)>,
    /// Dirty tracking for incremental persistence.
    dirty_threads: HashSet<String>,
    meta_dirty: bool,
    wipe: bool,
    pending_migration: Option<HashSet<String>>,
}

impl StoreState {
    fn new() -> Self {
        Self {
            message_cap: MAX_MESSAGES_PER_THREAD,
            ..Default::default()
        }
    }

    fn thread_changed(&mut self, key: &str, has_paths: bool) {
        self.unread_counts.remove(key);
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

    fn unread_count(&mut self, key: &str) -> usize {
        let read_ts = self.read_up_to.get(key).copied().unwrap_or(0);
        if let Some(&(cached_ts, count)) = self.unread_counts.get(key) {
            if cached_ts == read_ts {
                return count;
            }
        }
        let count = self
            .messages
            .get(key)
            .map(|list| list.iter().filter(|m| !m.is_self && m.ts > read_ts).count())
            .unwrap_or(0);
        self.unread_counts.insert(key.to_string(), (read_ts, count));
        count
    }

    fn meta_json(&self) -> String {
        let mut pinned: Vec<String> = self.pinned.iter().cloned().collect();
        pinned.sort();
        let mut marked_unread: Vec<String> = self.marked_unread.iter().cloned().collect();
        marked_unread.sort();
        let meta = MetaFile {
            version: Some(1),
            threads: self.threads.clone(),
            read_up_to: self.read_up_to.clone(),
            pinned,
            muted_until: self.muted_until.clone(),
            marked_unread,
            message_cap: if self.message_cap > MAX_MESSAGES_PER_THREAD {
                Some(self.message_cap)
            } else {
                None
            },
        };
        serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string())
    }

    fn thread_json(&self, key: &str) -> Option<String> {
        let list = self.messages.get(key)?;
        if list.is_empty() {
            return None;
        }
        let tf = ThreadFile {
            key: key.to_string(),
            messages: list.clone(),
        };
        serde_json::to_string(&tf).ok()
    }

    fn apply_meta(&mut self, meta: MetaFile) {
        self.threads = meta.threads;
        self.read_up_to = meta.read_up_to;
        self.pinned = meta.pinned.into_iter().collect();
        self.muted_until = meta.muted_until;
        self.marked_unread = meta.marked_unread.into_iter().collect();
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
        let filtered: Vec<&StoredMessage> = match before {
            Some(ts) => list.iter().filter(|m| m.ts < ts).collect(),
            None => list.iter().collect(),
        };
        let start = filtered.len().saturating_sub(cap);
        filtered[start..]
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
        let cap = limit.unwrap_or(50).clamp(1, 100);
        let Ok(s) = self.inner.state.read() else {
            return Vec::new();
        };
        let own = s.own_id.as_deref();
        let mut matches: Vec<(Option<&ThreadInfo>, &StoredMessage)> = Vec::new();
        for (key, list) in &s.messages {
            let thread = s.threads.get(key);
            for m in list {
                if m.recalled {
                    continue;
                }
                let hit = fold_for_search(&m.text).contains(&needle)
                    || m.attachment
                        .as_ref()
                        .and_then(|a| a.title.as_deref())
                        .is_some_and(|t| fold_for_search(t).contains(&needle))
                    || m.attachment
                        .as_ref()
                        .and_then(|a| a.description.as_deref())
                        .is_some_and(|d| fold_for_search(d).contains(&needle));
                if hit {
                    matches.push((thread, m));
                }
            }
        }
        matches.sort_by(|a, b| b.1.ts.cmp(&a.1.ts));
        matches
            .into_iter()
            .take(cap)
            .map(|(thread, m)| MessageSearchHit {
                thread_id: m.thread_id.clone(),
                thread_kind: m.thread_kind,
                thread_name: thread
                    .filter(|t| !t.name.is_empty())
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| m.thread_id.clone()),
                thread_avatar: thread.map(|t| t.avatar.clone()).unwrap_or_default(),
                message: to_public_message(m, own),
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
            let cloned = msg.clone();
            s.mark_dirty(&key, has_paths);
            cloned
        };
        self.schedule_save();
        Some(updated)
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
        let has_paths = self.inner.paths.is_some();
        let changed = {
            let Ok(mut s) = self.inner.state.write() else {
                return Vec::new();
            };
            let key = thread_key(kind, thread_id);
            let Some(list) = s.messages.get_mut(&key) else {
                return Vec::new();
            };
            let Some(target_ts) = list.iter().find(|m| m.id == msg_id).map(|m| m.ts) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for m in list.iter_mut() {
                let affected = if kind == ThreadKind::User {
                    m.is_self && m.ts <= target_ts
                } else {
                    m.id == msg_id
                };
                if !affected {
                    continue;
                }
                let prev_deliv = m.delivered_to.as_ref().map_or(0, Vec::len);
                let prev_seen = m.seen_by.as_ref().map_or(0, Vec::len);
                let new_deliv = union_strings(m.delivered_to.as_deref(), delivered_to);
                let new_seen = union_strings(m.seen_by.as_deref(), seen_by);
                if new_deliv.len() == prev_deliv && new_seen.len() == prev_seen {
                    continue;
                }
                m.delivered_to = Some(new_deliv);
                m.seen_by = Some(new_seen);
                out.push(m.clone());
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
            s.meta_changed(has_paths);
        }
        self.schedule_save();
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
        let Ok(mut s) = self.inner.state.write() else {
            return Vec::new();
        };
        let keys: Vec<String> = s.threads.keys().cloned().collect();
        let own = s.own_id.clone();
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let unread = s.unread_count(&key);
            let info = s.threads[&key].clone();
            let last_message = s
                .messages
                .get(&key)
                .and_then(|list| list.last())
                .map(|m| to_public_message(m, own.as_deref()));
            let marked = if s.marked_unread.contains(&key) { 1 } else { 0 };
            let pinned = s.pinned.contains(&key);
            let muted_until = s
                .muted_until
                .get(&key)
                .copied()
                .filter(|&u| u == -1 || u > now);
            out.push(ThreadSummary {
                id: info.id,
                kind: info.kind,
                name: info.name,
                avatar: info.avatar,
                last_message,
                unread: unread.max(marked),
                pinned,
                muted_until,
            });
        }
        out.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| {
                    let b_ts = b.last_message.as_ref().map_or(0, |m| m.ts);
                    let a_ts = a.last_message.as_ref().map_or(0, |m| m.ts);
                    b_ts.cmp(&a_ts)
                })
                .then_with(|| {
                    a.name
                        .to_lowercase()
                        .cmp(&b.name.to_lowercase())
                        .then_with(|| a.name.cmp(&b.name))
                })
        });
        out
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
            s.index.clear();
            s.unread_counts.clear();
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
                Ok(tf) => {
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
        let snapshot = match parsed {
            Ok(snap) => snap,
            Err(err) => {
                if let Ok(mut s) = self.inner.state.write() {
                    *s = StoreState::new();
                }
                backup_corrupt(legacy_file, &err);
                return;
            }
        };

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

        // Snapshot everything dirty under a brief write lock, then release the state lock before I/O.
        let (do_wipe, thread_jobs, want_meta, mut pending_mig) = {
            let Ok(mut s) = self.inner.state.write() else {
                return false;
            };
            let do_wipe = s.wipe;
            let dirty_keys: Vec<String> = s.dirty_threads.drain().collect();
            let mut jobs = Vec::with_capacity(dirty_keys.len());
            for key in dirty_keys {
                let json = s.thread_json(&key);
                jobs.push((key, json));
            }
            let want_meta = s.meta_dirty;
            if want_meta {
                s.meta_dirty = false;
            }
            let pending_mig = s.pending_migration.clone();
            (do_wipe, jobs, want_meta, pending_mig)
        };

        if !do_wipe && thread_jobs.is_empty() && !want_meta {
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
                for (key, _) in thread_jobs {
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
        for (key, json_opt) in thread_jobs {
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

fn union_strings(existing: Option<&[String]>, extra: &[String]) -> Vec<String> {
    let mut out: Vec<String> = existing.unwrap_or(&[]).to_vec();
    for item in extra {
        if !out.contains(item) {
            out.push(item.clone());
        }
    }
    out
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

    #[test]
    fn extract_otp_code_vietnamese_english_and_ignores_phone_numbers() {
        assert_eq!(
            extract_otp_code("Mã xác thực OTP của bạn là: 482910 (hiệu lực 2 phút)").as_deref(),
            Some("482910")
        );
        assert_eq!(
            extract_otp_code("Your verification code is 9912").as_deref(),
            Some("9912")
        );
        assert_eq!(
            extract_otp_code("739201 là mã xác nhận giao dịch").as_deref(),
            Some("739201")
        );
        assert_eq!(
            extract_otp_code(
                "Bạn đang yêu cầu cài đặt lại mã PIN tài khoản Giang với mã tài khoản 0378795621 tại The 1."
            ),
            None
        );
        assert_eq!(extract_otp_code("Hẹn gặp lúc 14:30 nhé"), None);
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
}

