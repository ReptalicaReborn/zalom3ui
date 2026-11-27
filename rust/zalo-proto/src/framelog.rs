//! Structure-only description of a decoded WebSocket frame, for the on-device socket log.
//!
//! A [`FrameReport`] records the frame header, whether a handler arm consumed it, the top-level
//! keys, array names and lengths, and for message arrays one [`ItemReport`] per item (msgType,
//! the *shape* of `content`, who sent it, and whether it became a message or why it was dropped).
//!
//! **Nothing here ever copies message text, content values, names or phone numbers.** Only
//! field names, JSON types, counts, cmd numbers, short enum-like strings (`msgType`, `act`) and
//! ids are kept, and every string that comes from the wire is filtered through [`safe_token`].

use serde::Serialize;
use serde_json::Value;

/// Max items described per array (the length is always exact).
pub(crate) const MAX_ITEMS: usize = 20;
const MAX_KEYS: usize = 24;

/// Keeps `[A-Za-z0-9._:-]` and caps the length; anything else becomes `?`. Used for every
/// wire-supplied string that ends up in the log (keys, msgType, act, ids).
pub fn safe_token(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max + 1));
    let mut chars = s.chars();
    for c in chars.by_ref().take(max) {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-') {
            out.push(c);
        } else {
            out.push('?');
        }
    }
    if chars.next().is_some() {
        out.push('~');
    }
    out
}

fn sorted_keys(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = obj.keys().map(|k| safe_token(k, 32)).collect();
    keys.sort();
    keys.truncate(MAX_KEYS);
    keys
}

/// Shape of a `content` value: `{key,key}`, `string`, `string>{key,key}` (a JSON object inside
/// a string), `number`, `bool`, `null`, `array` or `missing`. Never the value itself.
pub fn describe_content(content: Option<&Value>) -> String {
    match content {
        None => "missing".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(_)) => "bool".into(),
        Some(Value::Number(_)) => "number".into(),
        Some(Value::Array(_)) => "array".into(),
        Some(Value::Object(o)) => format!("{{{}}}", sorted_keys(o).join(",")),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.starts_with('{') && t.ends_with('}') {
                if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(t) {
                    return format!("string>{{{}}}", sorted_keys(&o).join(","));
                }
            }
            "string".into()
        }
    }
}

/// `zero` (uidFrom 0 = own account), `own`, `other` or `missing`.
pub fn uid_class(uid_from: Option<&Value>, own_id: &str) -> &'static str {
    match uid_from {
        Some(Value::String(s)) => {
            if s.is_empty() {
                "missing"
            } else if s == "0" {
                "zero"
            } else if !own_id.is_empty() && s == own_id {
                "own"
            } else {
                "other"
            }
        }
        Some(Value::Number(n)) => {
            if n.as_u64() == Some(0) || n.as_i64() == Some(0) {
                "zero"
            } else if !own_id.is_empty() && n.to_string() == own_id {
                "own"
            } else {
                "other"
            }
        }
        _ => "missing",
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ItemReport {
    #[serde(rename = "msgType")]
    pub msg_type: Option<String>,
    pub content: String,
    #[serde(rename = "uidFrom")]
    pub uid_from: &'static str,
    /// `message`, `undo` or `dropped`.
    pub outcome: &'static str,
    /// Why it was dropped (outcome `dropped` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// Extra remark on a kept item, e.g. `e2ee-masked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
    #[serde(rename = "msgId", skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
}

impl ItemReport {
    pub fn describe(item: &Value, own_id: &str) -> Self {
        let msg_type = item.get("msgType").map(|v| match v {
            Value::String(s) => safe_token(s, 48),
            Value::Number(_) => "number".to_string(),
            _ => "other".to_string(),
        });
        let msg_id = item
            .get("msgId")
            .or_else(|| item.get("globalMsgId"))
            .or_else(|| item.get("cliMsgId"))
            .and_then(|v| match v {
                Value::String(s) if !s.is_empty() => Some(safe_token(s, 24)),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            });
        Self {
            msg_type,
            content: describe_content(item.get("content")),
            uid_from: uid_class(item.get("uidFrom"), own_id),
            outcome: "dropped",
            reason: Some("not processed"),
            note: None,
            msg_id,
        }
    }

    pub fn kept(&mut self) {
        self.outcome = "message";
        self.reason = None;
    }

    pub fn undo(&mut self) {
        self.outcome = "undo";
        self.reason = None;
    }

    pub fn dropped(&mut self, reason: &'static str) {
        self.outcome = "dropped";
        self.reason = Some(reason);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArrayReport {
    pub name: String,
    pub len: usize,
    /// Whether a handler arm reads this array for this cmd. `false` means it is ignored.
    pub read: bool,
    pub items: Vec<ItemReport>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub items_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FrameReport {
    /// Unix ms, filled in by the session when the report is recorded.
    pub ts: i64,
    pub version: u8,
    pub cmd: u16,
    #[serde(rename = "subCmd")]
    pub sub_cmd: u8,
    /// `true` if a handler arm consumed the frame, `false` if it fell through to `RawFrame`.
    pub consumed: bool,
    #[serde(rename = "payloadKeys")]
    pub payload_keys: Vec<String>,
    #[serde(rename = "dataKeys")]
    pub data_keys: Vec<String>,
    pub arrays: Vec<ArrayReport>,
    /// `act_type/act` of control frames (601, 602, ...).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub acts: Vec<String>,
    /// Decode problem class (no payload text), when the frame could not be decoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'static str>,
    /// Non-array fields of the payload and of `data` as `key=value`: numbers and booleans in full
    /// (flags, counts, ids such as `more`, `queueStatus`, `lastActionId`), strings only as their
    /// length, objects as their key names. Never message text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scalars: Vec<String>,
    /// For a frame that could not be decoded: the outer `{ encrypt, data, ... }` wrapper described the
    /// same way (key names, `encrypt` value, `data` type and length).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub envelope: Vec<String>,
    /// Why decoding failed, reduced to a safe token (our own error text, no payload).
    #[serde(rename = "errorDetail", default, skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
    /// What Zalỏ did because of this frame (from a fixed vocabulary), e.g. asking for missed messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

/// Describes one JSON value without revealing text: numbers and booleans in full, strings as
/// `str(N)` (their length in characters), objects as `{key,...}`, arrays as `[N]`.
pub fn describe_scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => safe_token(&n.to_string(), 24),
        Value::String(s) => format!("str({})", s.chars().count()),
        Value::Array(a) => format!("[{}]", a.len()),
        Value::Object(o) => format!("{{{}}}", sorted_keys(o).join(",")),
    }
}

fn push_scalars(out: &mut Vec<String>, obj: &serde_json::Map<String, Value>, skip_objects: bool) {
    let mut names: Vec<&String> = obj.keys().collect();
    names.sort();
    for name in names.into_iter().take(MAX_KEYS) {
        let v = &obj[name];
        if v.is_array() || (skip_objects && v.is_object()) {
            continue;
        }
        out.push(format!("{}={}", safe_token(name, 32), describe_scalar(v)));
    }
}

impl FrameReport {
    pub fn new(version: u8, cmd: u16, sub_cmd: u8) -> Self {
        Self {
            ts: 0,
            version,
            cmd,
            sub_cmd,
            consumed: true,
            payload_keys: Vec::new(),
            data_keys: Vec::new(),
            arrays: Vec::new(),
            acts: Vec::new(),
            error: None,
            scalars: Vec::new(),
            envelope: Vec::new(),
            error_detail: None,
            note: None,
        }
    }

    /// Describes the outer wrapper of a frame whose payload could not be decoded, plus why.
    pub fn describe_envelope(&mut self, parsed: &Value, why: &str) {
        if let Some(o) = parsed.as_object() {
            push_scalars(&mut self.envelope, o, false);
        } else {
            self.envelope.push(format!("body={}", describe_scalar(parsed)));
        }
        self.error_detail = Some(safe_token(why, 80));
    }

    /// Fills keys, array lengths and control acts from the decoded payload.
    pub fn describe_payload(&mut self, decoded: &Value, inner: &Value) {
        if let Some(o) = decoded.as_object() {
            self.payload_keys = sorted_keys(o);
            // `data` itself is described below; other payload fields (error_code, ...) here.
            push_scalars(&mut self.scalars, o, true);
        }
        if let Some(o) = inner.as_object() {
            if !std::ptr::eq(decoded, inner) {
                self.data_keys = sorted_keys(o);
                push_scalars(&mut self.scalars, o, false);
            }
            let mut names: Vec<&String> = o.keys().collect();
            names.sort();
            for name in names.into_iter().take(MAX_KEYS) {
                if let Some(list) = o[name].as_array() {
                    self.arrays.push(ArrayReport {
                        name: safe_token(name, 32),
                        len: list.len(),
                        read: false,
                        items: Vec::new(),
                        items_truncated: false,
                    });
                    if name == "controls" || name == "actions" {
                        for it in list.iter().take(MAX_ITEMS) {
                            let c = it.get("content").unwrap_or(it);
                            let at = c.get("act_type").and_then(|v| v.as_str()).unwrap_or("");
                            let a = c.get("act").and_then(|v| v.as_str()).unwrap_or("");
                            if !at.is_empty() || !a.is_empty() {
                                self.acts.push(format!("{}/{}", safe_token(at, 32), safe_token(a, 32)));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Attaches the per-item outcomes of an array a handler read.
    pub fn set_items(&mut self, name: &str, mut items: Vec<ItemReport>) {
        let name = safe_token(name, 32);
        if let Some(a) = self.arrays.iter_mut().find(|a| a.name == name) {
            a.read = true;
            if items.len() > MAX_ITEMS {
                items.truncate(MAX_ITEMS);
                a.items_truncated = true;
            } else if a.len > MAX_ITEMS {
                a.items_truncated = true;
            }
            a.items = items;
        }
    }
}
