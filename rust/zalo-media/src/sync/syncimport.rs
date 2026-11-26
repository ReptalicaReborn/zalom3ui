//! BinNet TLV parser and SQLite `ChatContent` row converter (`src/syncimport.ts` port).
//!
//! Converts phone history-sync SQLite conversation tables into normalized Zalo raw message
//! JSON records ready to feed directly into `zalo_server::store::normalize_message`.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::sqliteread::{read_table, SqlRow, SqlValue, SqliteError};

// ---- BinNet flat TLV parser ----

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinNetAttachment {
    pub r#type: String,
    pub cat_id: i32,
    pub id: i32,
    pub ext_info: String,
    pub action: String,
    pub params: String,
    pub title: String,
    pub href: String,
    pub thumb: String,
    pub description: String,
    pub size: String,
    pub ext: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinNetQuote {
    pub owner_id: String,
    pub cli_msg_type: i32,
    pub from_d: String,
    pub cli_msg_id: String,
    pub global_msg_id: String,
    pub ts: String,
    pub msg: String,
    pub attach: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinNetMention {
    pub uid: String,
    pub pos: i32,
    pub len: i32,
    pub r#type: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinNet {
    pub text: String,
    pub data_source: String,
    pub quote_status: i32,
    pub quote: Option<BinNetQuote>,
    pub attachs: Vec<BinNetAttachment>,
    pub mentions: Vec<BinNetMention>,
}

struct Tlv<'a> {
    values: HashMap<u32, Vec<&'a [u8]>>,
    little: bool,
}

fn parse_tlv_with(buf: &[u8], little: bool) -> Option<HashMap<u32, Vec<&[u8]>>> {
    let mut values: HashMap<u32, Vec<&[u8]>> = HashMap::new();
    let mut off = 0usize;
    while off + 8 <= buf.len() {
        let key_bytes: [u8; 4] = buf[off..off + 4].try_into().ok()?;
        let len_bytes: [u8; 4] = buf[off + 4..off + 8].try_into().ok()?;
        let key = if little {
            u32::from_le_bytes(key_bytes)
        } else {
            u32::from_be_bytes(key_bytes)
        };
        let len = if little {
            u32::from_le_bytes(len_bytes) as usize
        } else {
            u32::from_be_bytes(len_bytes) as usize
        };
        off += 8;
        if len > buf.len() - off {
            return None;
        }
        values.entry(key).or_default().push(&buf[off..off + len]);
        off += len;
    }
    if off != buf.len() && buf.len() - off > 8 {
        return None;
    }
    Some(values)
}

const TOP_KEYS: &[u32] = &[0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b];
const ATTACH_KEYS: &[u32] = &[
    0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36,
    0x37, 0x40, 0x41, 0x42, 0x43, 0x44,
];

fn score_tlv(values: &HashMap<u32, Vec<&[u8]>>) -> i64 {
    let mut s = 0i64;
    for k in TOP_KEYS {
        if values.contains_key(k) {
            s += 6;
        }
    }
    for k in ATTACH_KEYS {
        if values.contains_key(k) {
            s += 4;
        }
    }
    for &k in values.keys() {
        if k <= 0x80 {
            s += 1;
        }
        if k > 0x10000 {
            s -= 3;
        }
    }
    s
}

fn parse_tlv(buf: &[u8]) -> Option<Tlv<'_>> {
    let le = parse_tlv_with(buf, true);
    let be = parse_tlv_with(buf, false);
    match (le, be) {
        (None, None) => None,
        (Some(le_v), None) => Some(Tlv {
            values: le_v,
            little: true,
        }),
        (None, Some(be_v)) => Some(Tlv {
            values: be_v,
            little: false,
        }),
        (Some(le_v), Some(be_v)) => {
            if score_tlv(&be_v) > score_tlv(&le_v) {
                Some(Tlv {
                    values: be_v,
                    little: false,
                })
            } else {
                Some(Tlv {
                    values: le_v,
                    little: true,
                })
            }
        }
    }
}

fn tlv_int32(t: &Tlv<'_>, key: u32) -> Option<i32> {
    let v = *t.values.get(&key)?.first()?;
    if v.len() < 4 {
        return None;
    }
    let bytes: [u8; 4] = v[..4].try_into().ok()?;
    Some(if t.little {
        i32::from_le_bytes(bytes)
    } else {
        i32::from_be_bytes(bytes)
    })
}

fn tlv_int64_text(t: &Tlv<'_>, key: u32) -> Option<String> {
    let v = *t.values.get(&key)?.first()?;
    if v.len() >= 8 {
        let bytes: [u8; 8] = v[..8].try_into().ok()?;
        let n = if t.little {
            i64::from_le_bytes(bytes)
        } else {
            i64::from_be_bytes(bytes)
        };
        return Some(n.to_string());
    }
    if v.len() >= 4 {
        let bytes: [u8; 4] = v[..4].try_into().ok()?;
        let n = if t.little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        };
        return Some(n.to_string());
    }
    None
}

fn looks_like_utf8(v: &[u8]) -> Option<&str> {
    if v.is_empty() {
        return Some("");
    }
    if v.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(v).ok()?;
    for ch in text.chars() {
        let c = ch as u32;
        if c < 0x20 && c != 9 && c != 10 && c != 13 {
            return None;
        }
    }
    Some(text)
}

fn decode_tlv_text(v: &[u8]) -> Option<String> {
    if let Some(s) = looks_like_utf8(v) {
        return Some(s.to_string());
    }
    if v.len() >= 4 && v.len() % 2 == 0 {
        let zero = v.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
        if zero * 100 >= (v.len() / 2) * 60 {
            let units: Vec<u16> = v
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            return Some(String::from_utf16_lossy(&units));
        }
    }
    None
}

fn tlv_text(t: &Tlv<'_>, key: u32) -> Option<String> {
    let v = *t.values.get(&key)?.first()?;
    decode_tlv_text(v)
}

fn parse_attachment(buf: &[u8]) -> BinNetAttachment {
    let mut out = BinNetAttachment::default();
    let Some(t) = parse_tlv(buf) else {
        return out;
    };
    out.r#type = tlv_text(&t, 0x28).unwrap_or_default();
    out.cat_id = tlv_int32(&t, 0x29).unwrap_or(0);
    out.id = tlv_int32(&t, 0x2a).unwrap_or(0);
    out.ext_info = tlv_text(&t, 0x2b).unwrap_or_default();
    out.action = tlv_text(&t, 0x2d).unwrap_or_default();
    out.params = tlv_text(&t, 0x2e).unwrap_or_default();
    out.title = tlv_text(&t, 0x2f).unwrap_or_default();
    out.href = tlv_text(&t, 0x30).unwrap_or_default();
    out.thumb = tlv_text(&t, 0x31).unwrap_or_default();
    out.description = tlv_text(&t, 0x32).unwrap_or_default();
    out.size = tlv_text(&t, 0x33).unwrap_or_default();
    out.ext = tlv_text(&t, 0x34).unwrap_or_default();
    out
}

fn parse_quote(buf: &[u8]) -> Option<BinNetQuote> {
    let t = parse_tlv(buf)?;
    Some(BinNetQuote {
        owner_id: tlv_int64_text(&t, 0x50).unwrap_or_default(),
        cli_msg_type: tlv_int32(&t, 0x53).unwrap_or(0),
        cli_msg_id: tlv_int64_text(&t, 0x51).unwrap_or_else(|| "0".into()),
        global_msg_id: tlv_int64_text(&t, 0x52).unwrap_or_else(|| "0".into()),
        ts: tlv_int64_text(&t, 0x54).unwrap_or_else(|| "0".into()),
        msg: tlv_text(&t, 0x56).unwrap_or_default(),
        attach: tlv_text(&t, 0x57).unwrap_or_default(),
        from_d: tlv_text(&t, 0x5a).unwrap_or_default(),
    })
}

fn parse_mention(buf: &[u8]) -> Option<BinNetMention> {
    let t = parse_tlv(buf)?;
    Some(BinNetMention {
        uid: tlv_int64_text(&t, 0x65).unwrap_or_else(|| "0".into()),
        pos: tlv_int32(&t, 0x66).unwrap_or(0),
        len: tlv_int32(&t, 0x67).unwrap_or(0),
        r#type: tlv_int32(&t, 0x68).unwrap_or(0),
    })
}

/// Parses a message's `BinNet` TLV blob. Unreadable or empty blobs yield `BinNet::default()`.
pub fn parse_bin_net(buf: Option<&[u8]>) -> BinNet {
    let mut out = BinNet::default();
    let Some(bytes) = buf else {
        return out;
    };
    if bytes.is_empty() {
        return out;
    }
    let Some(t) = parse_tlv(bytes) else {
        return out;
    };
    if let Some(list) = t.values.get(&0x06) {
        for v in list {
            if v.len() >= 8 {
                out.attachs.push(parse_attachment(v));
            }
        }
    }
    if let Some(q) = t.values.get(&0x07).and_then(|l| l.first()) {
        out.quote = parse_quote(q);
    }
    if let Some(list) = t.values.get(&0x08) {
        for v in list {
            if let Some(m) = parse_mention(v) {
                out.mentions.push(m);
            }
        }
    }
    out.quote_status = tlv_int32(&t, 0x09).unwrap_or(0);
    out.data_source = tlv_text(&t, 0x0a).unwrap_or_default();
    out.text = tlv_text(&t, 0x0b).unwrap_or_default();
    out
}

// ---- Backup rows ----

#[derive(Debug, Clone)]
pub struct BackupRow {
    pub global_msg_id: String,
    pub cli_msg_id: String,
    pub msg_type: i64,
    pub ts: i64,
    pub sender_id: String,
    pub outgoing: Option<bool>,
    pub text: String,
    pub ttl: i64,
    pub bin: BinNet,
}

#[derive(Debug, Clone)]
pub struct BackupConversation {
    pub id: String,
    pub is_group: bool,
    pub rows: Vec<BackupRow>,
    pub source_rows: usize,
    pub columns: Vec<String>,
}

fn sql_str(v: Option<&SqlValue>) -> String {
    match v {
        None | Some(SqlValue::Null) => String::new(),
        Some(SqlValue::Int(n)) => n.to_string(),
        Some(SqlValue::Float(f)) => f.to_string(),
        Some(SqlValue::Text(s)) => s.clone(),
        Some(SqlValue::Blob(b)) => String::from_utf8_lossy(b).into_owned(),
    }
}

fn sql_num(v: Option<&SqlValue>) -> i64 {
    match v {
        Some(SqlValue::Int(n)) => *n,
        Some(SqlValue::Float(f)) if f.is_finite() => *f as i64,
        Some(SqlValue::Text(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(|f| f as i64)
            .unwrap_or(0),
        Some(SqlValue::Blob(b)) => String::from_utf8_lossy(b)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(|f| f as i64)
            .unwrap_or(0),
        _ => 0,
    }
}

const OUTGOING_COLUMNS: &[&str] = &[
    "IsSend",
    "IsSent",
    "IsOutgoing",
    "IsMine",
    "IsSelf",
    "FromMe",
    "IsFromMe",
    "Outgoing",
];

fn outgoing_flag(row: &SqlRow) -> Option<bool> {
    for &name in OUTGOING_COLUMNS {
        match row.get(name) {
            None | Some(SqlValue::Null) => continue,
            Some(SqlValue::Int(n)) => return Some(*n != 0),
            Some(SqlValue::Float(f)) if f.is_finite() => return Some(*f != 0.0),
            Some(v) => {
                let s = sql_str(Some(v));
                if let Ok(f) = s.trim().parse::<f64>() {
                    if f.is_finite() {
                        return Some(f != 0.0);
                    }
                }
            }
        }
    }
    None
}

fn first_col<'a>(row: &'a SqlRow, names: &[&str]) -> Option<&'a SqlValue> {
    for &n in names {
        if let Some(v) = row.get(n) {
            if !matches!(v, SqlValue::Null) {
                return Some(v);
            }
        }
    }
    None
}

/// Converts a timestamp to epoch milliseconds (scaling up values that look like seconds).
pub fn to_millis(ts: i64) -> i64 {
    if ts > 0 && ts < 100_000_000_000 {
        ts.saturating_mul(1000)
    } else {
        ts
    }
}

/// Reads one conversation SQLite database (`ChatContent` table), keeping rows with
/// `MsgStatus > 0` (when the column exists) and a non-empty `CliMsgId`, sorted oldest-first.
pub fn read_conversation(
    db: &[u8],
    id: &str,
    is_group: bool,
) -> Result<Option<BackupConversation>, SqliteError> {
    let Some(table) = read_table(db, "ChatContent")? else {
        return Ok(None);
    };
    let has_status = table.columns.iter().any(|c| c == "MsgStatus");
    let source_rows = table.rows.len();
    let mut rows = Vec::with_capacity(source_rows);

    for r in &table.rows {
        if has_status && sql_num(r.get("MsgStatus")) <= 0 {
            continue;
        }
        let cli = sql_str(first_col(r, &["CliMsgId", "cliMsgId"]));
        if cli.is_empty() {
            continue;
        }
        let glb = sql_str(first_col(r, &["GlbMsgId", "GlobalMsgId", "MsgId"]));
        let bin_bytes = match r.get("BinNet") {
            Some(SqlValue::Blob(b)) => Some(b.as_slice()),
            _ => None,
        };
        rows.push(BackupRow {
            global_msg_id: glb,
            cli_msg_id: cli,
            msg_type: sql_num(r.get("MsgType")),
            ts: to_millis(sql_num(first_col(r, &["TimeStamp", "Ts"]))),
            sender_id: sql_str(first_col(r, &["SenderId", "FromId", "FromUid"])),
            outgoing: outgoing_flag(r),
            text: sql_str(first_col(r, &["MsgContent", "Message"])),
            ttl: sql_num(r.get("TTL")),
            bin: parse_bin_net(bin_bytes),
        });
    }

    // Stable sort preserves the phone's row order for equal timestamps.
    rows.sort_by_key(|r| r.ts);
    Ok(Some(BackupConversation {
        id: id.to_string(),
        is_group,
        rows,
        source_rows,
        columns: table.columns,
    }))
}

// ---- Row conversion ----

const SKIPPED_TYPES: &[i64] = &[21, 25, 29, 32, 33, 34, 35, 45, 51, 52];

fn type_name(t: i64) -> &'static str {
    match t {
        2 => "chat.doodle",
        3 | 4 | 31 => "chat.photo",
        6 => "chat.voice",
        10 => "chat.sticker",
        12 => "chat.recommended",
        15 => "chat.list.action",
        18 => "chat.location.new",
        19 => "chat.video.msg",
        22 => "share.file",
        23 => "chat.gif",
        24 => "chat.webcontent",
        26 => "chat.poll",
        53 => "chat.webcontent.v2",
        _ => "webchat",
    }
}

/// Returns `true` for `[Photo]`-style placeholder captions or local Android device URIs/paths.
pub fn is_placeholder_caption(s: &str) -> bool {
    let trimmed = s.trim();
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        let len = inner.chars().count();
        if (1..=24).contains(&len) && !inner.contains(']') {
            return true;
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("content://")
        || lower.starts_with("file://")
        || lower.starts_with("/storage/")
        || lower.starts_with("/sdcard/")
        || lower.starts_with("/data/")
        || lower.starts_with("/mnt/")
}

fn json_object(raw: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn pick_id(obj: &Map<String, Value>, keys: &[&str]) -> String {
    for &k in keys {
        if let Some(v) = obj.get(k) {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            };
            if !s.is_empty() && s != "0" {
                return s;
            }
        }
    }
    String::new()
}

/// Plain-to-noised ID map returned by `/api/znoise`, plus our own plain UID from `syncmsg_info`.
#[derive(Debug, Clone, Default)]
pub struct BackupIdMap {
    pub users: Option<HashMap<String, String>>,
    pub groups: Option<HashMap<String, String>>,
    pub my_plain_uid: String,
}

/// Determines whether this account sent `row`.
pub fn is_outgoing(
    row: &BackupRow,
    conv_id: &str,
    is_group: bool,
    own_id: &str,
    my_plain_uid: &str,
) -> bool {
    if let Some(explicit) = row.outgoing {
        return explicit;
    }
    let sender = row.sender_id.as_str();
    if sender == "0"
        || (!my_plain_uid.is_empty() && sender == my_plain_uid)
        || (!own_id.is_empty() && sender == own_id)
    {
        return true;
    }
    if !is_group && !sender.is_empty() && sender != conv_id {
        return true;
    }
    false
}

/// Normalized raw message payload ready for `zalo_server::store::normalize_message`.
#[derive(Debug, Clone)]
pub struct ConvertedMessage {
    pub is_group: bool,
    pub thread_id: String,
    pub is_self: bool,
    pub data: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ConvertResult {
    pub messages: Vec<ConvertedMessage>,
    pub recalled: Vec<String>,
    pub skipped: HashMap<String, usize>,
    pub thread_id: Option<String>,
}

/// Converts a conversation's backup rows into raw Zalo message payloads.
pub fn convert_conversation<F>(
    conv: &BackupConversation,
    own_id: &str,
    name_of: F,
    id_map: Option<&BackupIdMap>,
) -> ConvertResult
where
    F: Fn(&str) -> String,
{
    let mut messages = Vec::new();
    let mut recalled = Vec::new();
    let mut skipped: HashMap<String, usize> = HashMap::new();
    let mut skip = |reason: String| {
        *skipped.entry(reason).or_insert(0) += 1;
    };

    let has_noise_map = id_map.is_some_and(|m| m.users.is_some() || m.groups.is_some());
    let thread_id = if has_noise_map {
        let map = id_map.unwrap();
        let looked_up = if conv.is_group {
            map.groups.as_ref().and_then(|g| g.get(&conv.id))
        } else {
            map.users.as_ref().and_then(|u| u.get(&conv.id))
        };
        match looked_up {
            Some(id) if !id.is_empty() => id.clone(),
            _ => {
                if !conv.rows.is_empty() {
                    skipped.insert("no_thread".into(), conv.rows.len());
                }
                return ConvertResult {
                    messages,
                    recalled,
                    skipped,
                    thread_id: None,
                };
            }
        }
    } else {
        conv.id.clone()
    };

    let my_plain_uid = id_map.map(|m| m.my_plain_uid.as_str()).unwrap_or("");
    let map_user = |plain: &str| -> String {
        if plain.is_empty() || plain == "0" {
            return String::new();
        }
        if !my_plain_uid.is_empty() && plain == my_plain_uid {
            return own_id.to_string();
        }
        id_map
            .and_then(|m| m.users.as_ref())
            .and_then(|u| u.get(plain))
            .cloned()
            .unwrap_or_else(|| plain.to_string())
    };

    for row in &conv.rows {
        if SKIPPED_TYPES.contains(&row.msg_type) {
            skip(format!("type_{}", row.msg_type));
            continue;
        }
        let row_id = if !row.global_msg_id.is_empty() && row.global_msg_id != "0" {
            row.global_msg_id.clone()
        } else {
            row.cli_msg_id.clone()
        };
        let mine = is_outgoing(row, &conv.id, conv.is_group, own_id, my_plain_uid);
        let bin = &row.bin;

        if row.msg_type == 36 {
            let src = json_object(&row.text);
            let target = pick_id(&src, &["globalMsgId", "msgId"]);
            if !target.is_empty() {
                recalled.push(target);
            } else {
                skip("undo_without_id".into());
            }
            continue;
        }
        if row.sender_id.is_empty() && !mine && conv.is_group {
            skip("no_sender".into());
            continue;
        }
        if row_id.is_empty() {
            skip("no_id".into());
            continue;
        }

        let msg_type = type_name(row.msg_type);
        let content: Value = if row.msg_type == 26 {
            Value::Object(json_object(&row.text))
        } else if msg_type == "webchat" || bin.attachs.is_empty() {
            let body = if !row.text.is_empty() {
                row.text.as_str()
            } else {
                bin.text.as_str()
            };
            if (body.is_empty() && bin.attachs.is_empty())
                || (msg_type != "webchat" && is_placeholder_caption(body))
            {
                skip("empty_or_placeholder".into());
                continue;
            }
            Value::String(body.to_string())
        } else {
            let a = &bin.attachs[0];
            let mut obj = Map::new();
            let title = if !a.title.is_empty() {
                a.title.clone()
            } else if !is_placeholder_caption(&row.text) {
                row.text.clone()
            } else {
                String::new()
            };
            obj.insert("title".into(), Value::String(title));
            obj.insert("href".into(), Value::String(a.href.clone()));
            obj.insert("thumb".into(), Value::String(a.thumb.clone()));
            obj.insert("description".into(), Value::String(a.description.clone()));
            obj.insert("action".into(), Value::String(a.action.clone()));
            obj.insert("params".into(), Value::String(a.params.clone()));
            obj.insert("type".into(), Value::String(a.r#type.clone()));
            if msg_type == "chat.sticker" {
                obj.insert("id".into(), json!(a.id));
                obj.insert("catId".into(), json!(a.cat_id));
            }
            Value::Object(obj)
        };

        if msg_type == "webchat" && content.as_str() == Some("") {
            skip("empty_or_placeholder".into());
            continue;
        }

        let uid_from = if conv.is_group {
            if mine {
                own_id.to_string()
            } else {
                map_user(&row.sender_id)
            }
        } else if mine {
            own_id.to_string()
        } else {
            thread_id.clone()
        };
        let id_to = if conv.is_group {
            thread_id.clone()
        } else if mine {
            thread_id.clone()
        } else {
            own_id.to_string()
        };

        let mut data = Map::new();
        data.insert("msgId".into(), Value::String(row_id));
        data.insert("cliMsgId".into(), Value::String(row.cli_msg_id.clone()));
        data.insert("msgType".into(), Value::String(msg_type.to_string()));
        data.insert("uidFrom".into(), Value::String(uid_from.clone()));
        data.insert("idTo".into(), Value::String(id_to));
        data.insert(
            "dName".into(),
            Value::String(if mine { String::new() } else { name_of(&uid_from) }),
        );
        data.insert("ts".into(), Value::String(row.ts.to_string()));
        data.insert("content".into(), content);
        data.insert("ttl".into(), json!(row.ttl));

        if let Some(ref q) = bin.quote {
            if q.global_msg_id != "0" || q.cli_msg_id != "0" || !q.msg.is_empty() {
                let quote_owner = q.owner_id.as_str();
                let quote_mine = quote_owner == "0"
                    || (!my_plain_uid.is_empty() && quote_owner == my_plain_uid)
                    || (!own_id.is_empty() && quote_owner == own_id);
                let mapped_owner = if quote_mine {
                    own_id.to_string()
                } else {
                    map_user(quote_owner)
                };
                let from_d = if !q.from_d.is_empty() {
                    q.from_d.clone()
                } else if quote_mine {
                    String::new()
                } else {
                    let n = name_of(&mapped_owner);
                    if !n.is_empty() {
                        n
                    } else if conv.is_group {
                        String::new()
                    } else {
                        name_of(&thread_id)
                    }
                };
                let owner_out = if mapped_owner.is_empty() {
                    quote_owner.to_string()
                } else {
                    mapped_owner
                };
                data.insert(
                    "quote".into(),
                    json!({
                        "ownerId": owner_out,
                        "globalMsgId": q.global_msg_id,
                        "fromD": from_d,
                        "msg": q.msg,
                        "attach": q.attach,
                    }),
                );
            }
        }

        let mentions: Vec<Value> = bin
            .mentions
            .iter()
            .filter_map(|m| {
                let uid = if m.uid.starts_with('-') {
                    "-1".to_string()
                } else {
                    map_user(&m.uid)
                };
                if uid == "0" || uid.is_empty() {
                    return None;
                }
                Some(json!({
                    "uid": uid,
                    "pos": m.pos,
                    "len": m.len,
                    "type": m.r#type,
                }))
            })
            .collect();
        if !mentions.is_empty() {
            data.insert("mentions".into(), Value::Array(mentions));
        }

        messages.push(ConvertedMessage {
            is_group: conv.is_group,
            thread_id: thread_id.clone(),
            is_self: mine,
            data: Value::Object(data),
        });
    }

    ConvertResult {
        messages,
        recalled,
        skipped,
        thread_id: Some(thread_id),
    }
}

/// Summarizes how a conversation's rows were classified (counts only, no IDs or message text).
pub fn describe_senders(conv: &BackupConversation, own_id: &str, my_plain_uid: &str) -> String {
    let mut zero = 0usize;
    let mut empty = 0usize;
    let mut peer = 0usize;
    let mut own = 0usize;
    let mut other = 0usize;
    let mut sent = 0usize;
    for r in &conv.rows {
        if r.sender_id == "0" {
            zero += 1;
        } else if r.sender_id.is_empty() {
            empty += 1;
        } else if r.sender_id == conv.id {
            peer += 1;
        } else if (!my_plain_uid.is_empty() && r.sender_id == my_plain_uid)
            || (!own_id.is_empty() && r.sender_id == own_id)
        {
            own += 1;
        } else {
            other += 1;
        }
        if is_outgoing(r, &conv.id, conv.is_group, own_id, my_plain_uid) {
            sent += 1;
        }
    }
    let has_direction = conv.rows.iter().any(|r| r.outgoing.is_some());
    format!(
        "{} rows={} sender0={} empty={} peer={} own={} other={} direction={} sent={}",
        if conv.is_group { "group" } else { "chat" },
        conv.rows.len(),
        zero,
        empty,
        peer,
        own,
        other,
        has_direction,
        sent
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
    }

    fn tlv(items: &[(u32, Vec<u8>)], little: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in items {
            if little {
                out.extend_from_slice(&k.to_le_bytes());
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
            } else {
                out.extend_from_slice(&k.to_be_bytes());
                out.extend_from_slice(&(v.len() as u32).to_be_bytes());
            }
            out.extend_from_slice(v);
        }
        out
    }

    fn b_i32(n: i32, little: bool) -> Vec<u8> {
        if little {
            n.to_le_bytes().to_vec()
        } else {
            n.to_be_bytes().to_vec()
        }
    }

    fn b_i64(n: i64, little: bool) -> Vec<u8> {
        if little {
            n.to_le_bytes().to_vec()
        } else {
            n.to_be_bytes().to_vec()
        }
    }

    #[test]
    fn parse_bin_net_both_endiannesses() {
        for little in [true, false] {
            let attach = tlv(
                &[
                    (0x28, b"photo".to_vec()),
                    (0x2a, b_i32(77, little)),
                    (0x2f, b"pic.jpg".to_vec()),
                    (0x30, b"https://x.invalid/p".to_vec()),
                    (0x2e, br#"{"width":1}"#.to_vec()),
                ],
                little,
            );
            let quote = tlv(
                &[
                    (0x50, b_i64(555, little)),
                    (0x51, b_i64(802, little)),
                    (0x52, b_i64(7_351_234_567_890_123_456, little)),
                    (0x54, b_i64(1_700_000_000_000, little)),
                    (0x56, b"old text".to_vec()),
                    (0x5a, b"Alice".to_vec()),
                ],
                little,
            );
            let mention = tlv(
                &[
                    (0x65, b_i64(1002, little)),
                    (0x66, b_i32(3, little)),
                    (0x67, b_i32(4, little)),
                    (0x68, b_i32(0, little)),
                ],
                little,
            );
            let blob = tlv(
                &[
                    (0x06, attach),
                    (0x07, quote),
                    (0x08, mention),
                    (0x0b, b"hello Bob".to_vec()),
                ],
                little,
            );
            let bin = parse_bin_net(Some(&blob));
            assert_eq!(bin.text, "hello Bob");
            assert_eq!(bin.attachs.len(), 1);
            assert_eq!(bin.attachs[0].r#type, "photo");
            assert_eq!(bin.attachs[0].id, 77);
            assert_eq!(bin.attachs[0].title, "pic.jpg");
            assert_eq!(bin.attachs[0].params, r#"{"width":1}"#);
            let q = bin.quote.unwrap();
            assert_eq!(q.global_msg_id, "7351234567890123456");
            assert_eq!(q.owner_id, "555");
            assert_eq!(q.msg, "old text");
            assert_eq!(q.from_d, "Alice");
            assert_eq!(
                bin.mentions,
                vec![BinNetMention {
                    uid: "1002".into(),
                    pos: 3,
                    len: 4,
                    r#type: 0
                }]
            );
        }
    }

    #[test]
    fn parse_bin_net_utf8_utf16le_and_junk() {
        let utf8_blob = tlv(&[(0x0b, "xin chào 😀".as_bytes().to_vec())], true);
        assert_eq!(parse_bin_net(Some(&utf8_blob)).text, "xin chào 😀");

        let utf16: Vec<u8> = "abc def"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let utf16_blob = tlv(&[(0x0b, utf16)], true);
        assert_eq!(parse_bin_net(Some(&utf16_blob)).text, "abc def");

        assert!(parse_bin_net(Some(b"junk")).attachs.is_empty());
        assert_eq!(parse_bin_net(None).text, "");

        let mut bad = Vec::new();
        bad.extend_from_slice(&0x0bu32.to_le_bytes());
        bad.extend_from_slice(&999u32.to_le_bytes());
        assert_eq!(parse_bin_net(Some(&bad)).text, "");
    }

    #[test]
    fn read_and_convert_sync_1001_fixture() {
        let conv = read_conversation(&fixture("sync_1001.db"), "1001", false)
            .unwrap()
            .unwrap();
        assert_eq!(conv.source_rows, 12);
        assert_eq!(conv.rows.len(), 10);
        assert!(conv.rows.windows(2).all(|w| w[0].ts <= w[1].ts));

        let res = convert_conversation(
            &conv,
            "555",
            |id| if id == "1001" { "Alice".into() } else { String::new() },
            None,
        );
        assert_eq!(res.recalled, vec!["9001"]);
        assert_eq!(res.skipped.get("type_33"), Some(&1));

        let by_id: HashMap<&str, &ConvertedMessage> = res
            .messages
            .iter()
            .map(|m| (m.data["msgId"].as_str().unwrap(), m))
            .collect();
        assert!(by_id.contains_key("808"));
        let mine = by_id["9002"];
        assert!(mine.is_self);
        assert_eq!(mine.data["uidFrom"], "555");
        assert_eq!(mine.data["idTo"], "1001");
        let theirs = by_id["9001"];
        assert!(!theirs.is_self);
        assert_eq!(theirs.data["uidFrom"], "1001");
        assert_eq!(theirs.data["idTo"], "555");
        assert_eq!(theirs.data["dName"], "Alice");
        assert_eq!(theirs.data["content"], "hello");
        let photo = by_id["9003"];
        assert_eq!(photo.data["msgType"], "chat.photo");
        assert_eq!(photo.data["content"]["title"], "pic.jpg");
        let quoted = by_id["9004"];
        assert_eq!(quoted.data["quote"]["globalMsgId"], "9002");
        assert_eq!(quoted.data["quote"]["msg"], "hi back");
        assert_eq!(
            by_id["7351234567890123456"].data["msgId"],
            "7351234567890123456"
        );
    }

    #[test]
    fn read_and_convert_group_777_fixture() {
        let conv = read_conversation(&fixture("sync_group_777.db"), "777", true)
            .unwrap()
            .unwrap();
        let res = convert_conversation(&conv, "555", |_| String::new(), None);
        assert_eq!(res.messages.len(), 4);
        assert!(res
            .messages
            .iter()
            .all(|m| m.is_group && m.thread_id == "777" && m.data["idTo"] == "777"));
        assert_eq!(res.messages[0].data["uidFrom"], "1001");
        assert_eq!(res.messages[1].data["mentions"][0]["uid"], "1002");
        assert!(res.messages[2].is_self);
        assert_eq!(res.messages[2].data["uidFrom"], "555");
        assert!(res.messages[3].is_self);
        assert_eq!(res.messages[3].data["mentions"][0]["uid"], "-1");
    }

    #[test]
    fn convert_maps_plain_ids_to_noised_ids() {
        let own_noised = "9000000000000000001";
        let peer_noised = "9000000000000000002";
        let group_noised = "8000000000000000003";
        let id_map = BackupIdMap {
            users: Some(HashMap::from([("324724890".into(), peer_noised.into())])),
            groups: Some(HashMap::from([("721897407".into(), group_noised.into())])),
            my_plain_uid: "111222333".into(),
        };

        let conv = BackupConversation {
            id: "721897407".into(),
            is_group: true,
            source_rows: 2,
            columns: vec![],
            rows: vec![
                BackupRow {
                    global_msg_id: "m1".into(),
                    cli_msg_id: "c1".into(),
                    msg_type: 1,
                    ts: 1000,
                    sender_id: "111222333".into(),
                    outgoing: None,
                    text: "my group msg".into(),
                    ttl: 0,
                    bin: BinNet::default(),
                },
                BackupRow {
                    global_msg_id: "m2".into(),
                    cli_msg_id: "c2".into(),
                    msg_type: 1,
                    ts: 2000,
                    sender_id: "324724890".into(),
                    outgoing: None,
                    text: "@Me hi".into(),
                    ttl: 0,
                    bin: BinNet {
                        quote_status: 1,
                        quote: Some(BinNetQuote {
                            owner_id: "324724890".into(),
                            cli_msg_id: "c1".into(),
                            global_msg_id: "m1".into(),
                            ts: "1000".into(),
                            msg: "my group msg".into(),
                            ..Default::default()
                        }),
                        mentions: vec![BinNetMention {
                            uid: "111222333".into(),
                            pos: 0,
                            len: 3,
                            r#type: 0,
                        }],
                        ..Default::default()
                    },
                },
            ],
        };

        let res = convert_conversation(
            &conv,
            own_noised,
            |id| if id == peer_noised { "Alice".into() } else { String::new() },
            Some(&id_map),
        );
        assert_eq!(res.thread_id.as_deref(), Some(group_noised));
        assert_eq!(res.messages.len(), 2);
        assert!(res.messages[0].is_self);
        assert_eq!(res.messages[0].data["uidFrom"], own_noised);
        assert!(!res.messages[1].is_self);
        assert_eq!(res.messages[1].data["uidFrom"], peer_noised);
        assert_eq!(res.messages[1].data["dName"], "Alice");
        assert_eq!(res.messages[1].data["quote"]["ownerId"], peer_noised);
        assert_eq!(res.messages[1].data["quote"]["fromD"], "Alice");
        assert_eq!(res.messages[1].data["mentions"][0]["uid"], own_noised);

        // Unmapped conversation is skipped
        let unmapped = BackupConversation {
            id: "99999999".into(),
            is_group: false,
            source_rows: 1,
            columns: vec![],
            rows: vec![BackupRow {
                global_msg_id: "1".into(),
                cli_msg_id: "1".into(),
                msg_type: 1,
                ts: 1,
                sender_id: "99999999".into(),
                outgoing: None,
                text: "x".into(),
                ttl: 0,
                bin: BinNet::default(),
            }],
        };
        let skipped = convert_conversation(&unmapped, own_noised, |_| String::new(), Some(&id_map));
        assert!(skipped.thread_id.is_none());
        assert!(skipped.messages.is_empty());
        assert_eq!(skipped.skipped.get("no_thread"), Some(&1));
    }

    #[test]
    fn placeholder_caption_and_outgoing_rules() {
        assert!(is_placeholder_caption("[Photo]"));
        assert!(is_placeholder_caption("content://media/external/images/1"));
        assert!(is_placeholder_caption("FILE:///sdcard/pic.jpg"));
        assert!(is_placeholder_caption("/storage/emulated/0/DCIM/1.jpg"));
        assert!(is_placeholder_caption("/sdcard/a.png"));
        assert!(is_placeholder_caption("/data/user/0/a.png"));
        assert!(is_placeholder_caption("/mnt/media/1.png"));
        assert!(!is_placeholder_caption("normal message"));

        let mk = |sender: &str, outgoing: Option<bool>| BackupRow {
            global_msg_id: "1".into(),
            cli_msg_id: "1".into(),
            msg_type: 1,
            ts: 1,
            sender_id: sender.into(),
            outgoing,
            text: "x".into(),
            ttl: 0,
            bin: BinNet::default(),
        };
        assert!(is_outgoing(&mk("0", None), "100", false, "9999", ""));
        assert!(is_outgoing(&mk("9999", None), "g1", true, "9999", ""));
        assert!(is_outgoing(&mk("4242", None), "777", true, "9999", "4242"));
        assert!(!is_outgoing(&mk("100", None), "100", false, "me", ""));
        assert!(is_outgoing(&mk("other", None), "100", false, "me", ""));
        assert!(!is_outgoing(&mk("", None), "100", false, "me", ""));
        assert!(!is_outgoing(&mk("555", None), "g1", true, "me", "4242"));
        assert!(is_outgoing(&mk("100", Some(true)), "100", false, "me", ""));
        assert!(!is_outgoing(&mk("0", Some(false)), "100", false, "me", ""));
    }
}
