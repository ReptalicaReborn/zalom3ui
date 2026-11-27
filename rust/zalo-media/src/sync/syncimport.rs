//! BinNet TLV parser and SQLite `ChatContent` row converter (`src/syncimport.ts` port).
//!
//! Converts phone history-sync SQLite conversation tables into normalized Zalo raw message
//! JSON records ready to feed directly into `zalo_server::store::normalize_message`.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::sqliteread::{read_table_rows, SqlValue, SqliteError};

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

/// Zero-allocation TLV header walk that validates framing and computes the endianness score.
fn walk_and_score_tlv(buf: &[u8], little: bool) -> Option<i64> {
    let mut off = 0usize;
    let mut seen_low: u128 = 0; // distinct keys 0..=127
    let mut seen_0x80 = false;
    let mut score = 0i64;
    let mut large_keys = 0i64;

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
        off += len;

        if key < 128 {
            let bit = 1u128 << key;
            if (seen_low & bit) == 0 {
                seen_low |= bit;
                score += 1;
                if (0x05..=0x0b).contains(&key) {
                    score += 6;
                } else if (0x28..=0x37).contains(&key) || (0x40..=0x44).contains(&key) {
                    score += 4;
                }
            }
        } else if key == 0x80 {
            if !seen_0x80 {
                seen_0x80 = true;
                score += 1;
            }
        } else if key > 0x10000 {
            large_keys += 1;
        }
    }
    if off != buf.len() && buf.len() - off > 8 {
        return None;
    }
    score -= large_keys * 3;
    Some(score)
}

/// Validates a TLV buffer under a known endianness without allocating.
fn is_valid_tlv(buf: &[u8], little: bool) -> bool {
    let mut off = 0usize;
    while off + 8 <= buf.len() {
        let len_bytes: [u8; 4] = buf[off + 4..off + 8].try_into().unwrap();
        let len = if little {
            u32::from_le_bytes(len_bytes) as usize
        } else {
            u32::from_be_bytes(len_bytes) as usize
        };
        off += 8;
        if len > buf.len() - off {
            return false;
        }
        off += len;
    }
    off == buf.len() || buf.len() - off <= 8
}

/// Detects TLV endianness (`Some(true)` = LE, `Some(false)` = BE) without allocating.
fn detect_tlv_endian(buf: &[u8]) -> Option<bool> {
    let le = walk_and_score_tlv(buf, true);
    let be = walk_and_score_tlv(buf, false);
    match (le, be) {
        (None, None) => None,
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (Some(le_score), Some(be_score)) => Some(be_score <= le_score),
    }
}

/// Iterates `(key, value_slice)` entries of a validated TLV buffer without allocating.
fn for_each_tlv<'a>(buf: &'a [u8], little: bool, mut f: impl FnMut(u32, &'a [u8])) {
    let mut off = 0usize;
    while off + 8 <= buf.len() {
        let key_bytes: [u8; 4] = buf[off..off + 4].try_into().unwrap();
        let len_bytes: [u8; 4] = buf[off + 4..off + 8].try_into().unwrap();
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
            break;
        }
        f(key, &buf[off..off + len]);
        off += len;
    }
}

fn decode_int32(v: &[u8], little: bool) -> Option<i32> {
    if v.len() < 4 {
        return None;
    }
    let bytes: [u8; 4] = v[..4].try_into().ok()?;
    Some(if little {
        i32::from_le_bytes(bytes)
    } else {
        i32::from_be_bytes(bytes)
    })
}

fn decode_int64_text(v: &[u8], little: bool) -> Option<String> {
    if v.len() >= 8 {
        let bytes: [u8; 8] = v[..8].try_into().ok()?;
        let n = if little {
            i64::from_le_bytes(bytes)
        } else {
            i64::from_be_bytes(bytes)
        };
        return Some(n.to_string());
    }
    if v.len() >= 4 {
        let bytes: [u8; 4] = v[..4].try_into().ok()?;
        let n = if little {
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

fn sub_tlv_endian(buf: &[u8], inherited_little: bool) -> Option<bool> {
    if is_valid_tlv(buf, inherited_little) {
        Some(inherited_little)
    } else {
        detect_tlv_endian(buf)
    }
}

fn parse_attachment(buf: &[u8], inherited_little: bool) -> BinNetAttachment {
    let mut out = BinNetAttachment::default();
    let Some(little) = sub_tlv_endian(buf, inherited_little) else {
        return out;
    };
    let mut seen: u16 = 0;
    for_each_tlv(buf, little, |key, v| {
        let bit = match key {
            0x28 => 1u16 << 0,
            0x29 => 1u16 << 1,
            0x2a => 1u16 << 2,
            0x2b => 1u16 << 3,
            0x2d => 1u16 << 4,
            0x2e => 1u16 << 5,
            0x2f => 1u16 << 6,
            0x30 => 1u16 << 7,
            0x31 => 1u16 << 8,
            0x32 => 1u16 << 9,
            0x33 => 1u16 << 10,
            0x34 => 1u16 << 11,
            _ => return,
        };
        if (seen & bit) != 0 {
            return;
        }
        seen |= bit;
        match key {
            0x28 => out.r#type = decode_tlv_text(v).unwrap_or_default(),
            0x29 => out.cat_id = decode_int32(v, little).unwrap_or(0),
            0x2a => out.id = decode_int32(v, little).unwrap_or(0),
            0x2b => out.ext_info = decode_tlv_text(v).unwrap_or_default(),
            0x2d => out.action = decode_tlv_text(v).unwrap_or_default(),
            0x2e => out.params = decode_tlv_text(v).unwrap_or_default(),
            0x2f => out.title = decode_tlv_text(v).unwrap_or_default(),
            0x30 => out.href = decode_tlv_text(v).unwrap_or_default(),
            0x31 => out.thumb = decode_tlv_text(v).unwrap_or_default(),
            0x32 => out.description = decode_tlv_text(v).unwrap_or_default(),
            0x33 => out.size = decode_tlv_text(v).unwrap_or_default(),
            0x34 => out.ext = decode_tlv_text(v).unwrap_or_default(),
            _ => {}
        }
    });
    out
}

fn parse_quote(buf: &[u8], inherited_little: bool) -> Option<BinNetQuote> {
    let little = sub_tlv_endian(buf, inherited_little)?;
    let mut q = BinNetQuote {
        cli_msg_id: "0".into(),
        global_msg_id: "0".into(),
        ts: "0".into(),
        ..Default::default()
    };
    let mut seen: u16 = 0;
    for_each_tlv(buf, little, |key, v| {
        let bit = match key {
            0x50 => 1u16 << 0,
            0x51 => 1u16 << 1,
            0x52 => 1u16 << 2,
            0x53 => 1u16 << 3,
            0x54 => 1u16 << 4,
            0x56 => 1u16 << 5,
            0x57 => 1u16 << 6,
            0x5a => 1u16 << 7,
            _ => return,
        };
        if (seen & bit) != 0 {
            return;
        }
        seen |= bit;
        match key {
            0x50 => q.owner_id = decode_int64_text(v, little).unwrap_or_default(),
            0x51 => {
                if let Some(s) = decode_int64_text(v, little) {
                    q.cli_msg_id = s;
                }
            }
            0x52 => {
                if let Some(s) = decode_int64_text(v, little) {
                    q.global_msg_id = s;
                }
            }
            0x53 => q.cli_msg_type = decode_int32(v, little).unwrap_or(0),
            0x54 => {
                if let Some(s) = decode_int64_text(v, little) {
                    q.ts = s;
                }
            }
            0x56 => q.msg = decode_tlv_text(v).unwrap_or_default(),
            0x57 => q.attach = decode_tlv_text(v).unwrap_or_default(),
            0x5a => q.from_d = decode_tlv_text(v).unwrap_or_default(),
            _ => {}
        }
    });
    Some(q)
}

fn parse_mention(buf: &[u8], inherited_little: bool) -> Option<BinNetMention> {
    let little = sub_tlv_endian(buf, inherited_little)?;
    let mut m = BinNetMention {
        uid: "0".into(),
        ..Default::default()
    };
    let mut seen: u8 = 0;
    for_each_tlv(buf, little, |key, v| {
        let bit = match key {
            0x65 => 1u8 << 0,
            0x66 => 1u8 << 1,
            0x67 => 1u8 << 2,
            0x68 => 1u8 << 3,
            _ => return,
        };
        if (seen & bit) != 0 {
            return;
        }
        seen |= bit;
        match key {
            0x65 => {
                if let Some(s) = decode_int64_text(v, little) {
                    m.uid = s;
                }
            }
            0x66 => m.pos = decode_int32(v, little).unwrap_or(0),
            0x67 => m.len = decode_int32(v, little).unwrap_or(0),
            0x68 => m.r#type = decode_int32(v, little).unwrap_or(0),
            _ => {}
        }
    });
    Some(m)
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
    let Some(little) = detect_tlv_endian(bytes) else {
        return out;
    };
    let mut seen_single: u8 = 0;
    for_each_tlv(bytes, little, |key, v| match key {
        0x06 => {
            if v.len() >= 8 {
                out.attachs.push(parse_attachment(v, little));
            }
        }
        0x07 => {
            if (seen_single & 1) == 0 {
                seen_single |= 1;
                out.quote = parse_quote(v, little);
            }
        }
        0x08 => {
            if let Some(m) = parse_mention(v, little) {
                out.mentions.push(m);
            }
        }
        0x09 => {
            if (seen_single & 2) == 0 {
                seen_single |= 2;
                out.quote_status = decode_int32(v, little).unwrap_or(0);
            }
        }
        0x0a => {
            if (seen_single & 4) == 0 {
                seen_single |= 4;
                out.data_source = decode_tlv_text(v).unwrap_or_default();
            }
        }
        0x0b => {
            if (seen_single & 8) == 0 {
                seen_single |= 8;
                out.text = decode_tlv_text(v).unwrap_or_default();
            }
        }
        _ => {}
    });
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

fn take_sql_str(v: Option<&mut SqlValue>) -> String {
    match v {
        None | Some(SqlValue::Null) => String::new(),
        Some(SqlValue::Int(n)) => n.to_string(),
        Some(SqlValue::Float(f)) => f.to_string(),
        Some(SqlValue::Text(s)) => std::mem::take(s),
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

fn outgoing_flag_indexed(row: &[SqlValue], outgoing_idxs: &[usize]) -> Option<bool> {
    for &idx in outgoing_idxs {
        match row.get(idx) {
            None | Some(SqlValue::Null) => continue,
            Some(SqlValue::Int(n)) => return Some(*n != 0),
            Some(SqlValue::Float(f)) if f.is_finite() => return Some(*f != 0.0),
            Some(SqlValue::Text(s)) => {
                if let Ok(f) = s.trim().parse::<f64>() {
                    if f.is_finite() {
                        return Some(f != 0.0);
                    }
                }
            }
            Some(SqlValue::Blob(b)) => {
                let s = String::from_utf8_lossy(b);
                if let Ok(f) = s.trim().parse::<f64>() {
                    if f.is_finite() {
                        return Some(f != 0.0);
                    }
                }
            }
            Some(SqlValue::Float(_)) => {}
        }
    }
    None
}

fn find_col(columns: &[String], name: &str) -> Option<usize> {
    columns.iter().position(|c| c == name)
}

fn find_col_indices(columns: &[String], names: &[&str]) -> Vec<usize> {
    names
        .iter()
        .filter_map(|&n| find_col(columns, n))
        .collect()
}

fn first_non_null_idx(row: &[SqlValue], idxs: &[usize]) -> Option<usize> {
    for &idx in idxs {
        if let Some(v) = row.get(idx) {
            if !matches!(v, SqlValue::Null) {
                return Some(idx);
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
    let Some(table) = read_table_rows(db, "ChatContent")? else {
        return Ok(None);
    };
    let status_idx = find_col(&table.columns, "MsgStatus");
    let cli_idxs = find_col_indices(&table.columns, &["CliMsgId", "cliMsgId"]);
    let glb_idxs = find_col_indices(&table.columns, &["GlbMsgId", "GlobalMsgId", "MsgId"]);
    let bin_idx = find_col(&table.columns, "BinNet");
    let msg_type_idx = find_col(&table.columns, "MsgType");
    let ts_idxs = find_col_indices(&table.columns, &["TimeStamp", "Ts"]);
    let sender_idxs = find_col_indices(&table.columns, &["SenderId", "FromId", "FromUid"]);
    let outgoing_idxs = find_col_indices(&table.columns, OUTGOING_COLUMNS);
    let content_idxs = find_col_indices(&table.columns, &["MsgContent", "Message"]);
    let ttl_idx = find_col(&table.columns, "TTL");

    let source_rows = table.rows.len();
    let mut rows = Vec::with_capacity(source_rows);

    for mut r in table.rows {
        if let Some(si) = status_idx {
            if sql_num(r.get(si)) <= 0 {
                continue;
            }
        }
        let cli_i = first_non_null_idx(&r, &cli_idxs);
        let cli = take_sql_str(cli_i.and_then(|i| r.get_mut(i)));
        if cli.is_empty() {
            continue;
        }
        let glb_i = first_non_null_idx(&r, &glb_idxs);
        let glb = take_sql_str(glb_i.and_then(|i| r.get_mut(i)));
        let bin = match bin_idx.and_then(|i| r.get(i)) {
            Some(SqlValue::Blob(b)) => parse_bin_net(Some(b.as_slice())),
            _ => BinNet::default(),
        };
        let msg_type = sql_num(msg_type_idx.and_then(|i| r.get(i)));
        let ts = to_millis(sql_num(
            first_non_null_idx(&r, &ts_idxs).and_then(|i| r.get(i)),
        ));
        let outgoing = outgoing_flag_indexed(&r, &outgoing_idxs);
        let ttl = sql_num(ttl_idx.and_then(|i| r.get(i)));
        let sender_i = first_non_null_idx(&r, &sender_idxs);
        let sender_id = take_sql_str(sender_i.and_then(|i| r.get_mut(i)));
        let content_i = first_non_null_idx(&r, &content_idxs);
        let text = take_sql_str(content_i.and_then(|i| r.get_mut(i)));

        rows.push(BackupRow {
            global_msg_id: glb,
            cli_msg_id: cli,
            msg_type,
            ts,
            sender_id,
            outgoing,
            text,
            ttl,
            bin,
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

#[inline]
fn starts_with_ascii_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
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
    starts_with_ascii_ci(trimmed, "content://")
        || starts_with_ascii_ci(trimmed, "file://")
        || starts_with_ascii_ci(trimmed, "/storage/")
        || starts_with_ascii_ci(trimmed, "/sdcard/")
        || starts_with_ascii_ci(trimmed, "/data/")
        || starts_with_ascii_ci(trimmed, "/mnt/")
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
