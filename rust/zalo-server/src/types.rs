//! Wire and persistence types matching `src/store.ts`, `src/session.ts`, `src/polls.ts`,
//! `src/historysync.ts`, and Android `Models.kt` / `HistorySyncModels.kt`.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Where the official client loads a sticker's picture from (its default `stickerUrl` config) by sticker id.
pub const STICKER_IMAGE_URL: &str =
    "https://api.zaloapp.com/api/emoticon/sticker/webpc?size=130&eid=";

/// Definition of a supported Zalo reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReactionDef {
    pub key: &'static str,
    pub code: &'static str,
    pub emoji: &'static str,
}

/// Public view of a reaction option returned by `GET /api/reactions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReactionChoice {
    pub key: &'static str,
    pub emoji: &'static str,
}

/// Reactions offered by the API, keyed by a stable name, with Zalo's code and an emoji to display.
/// The first six are the quick reactions Zalo shows on every message.
pub const REACTIONS: &[ReactionDef] = &[
    ReactionDef {
        key: "heart",
        code: "/-heart",
        emoji: "❤️",
    },
    ReactionDef {
        key: "like",
        code: "/-strong",
        emoji: "👍",
    },
    ReactionDef {
        key: "haha",
        code: ":>",
        emoji: "😆",
    },
    ReactionDef {
        key: "wow",
        code: ":o",
        emoji: "😮",
    },
    ReactionDef {
        key: "cry",
        code: ":-((",
        emoji: "😭",
    },
    ReactionDef {
        key: "angry",
        code: ":-h",
        emoji: "😡",
    },
    ReactionDef {
        key: "kiss",
        code: ":-*",
        emoji: "😘",
    },
    ReactionDef {
        key: "tears_of_joy",
        code: ":')",
        emoji: "😂",
    },
    ReactionDef {
        key: "dislike",
        code: "/-weak",
        emoji: "👎",
    },
    ReactionDef {
        key: "rose",
        code: "/-rose",
        emoji: "🌹",
    },
    ReactionDef {
        key: "broken_heart",
        code: "/-break",
        emoji: "💔",
    },
    ReactionDef {
        key: "ok",
        code: "/-ok",
        emoji: "👌",
    },
    ReactionDef {
        key: "pray",
        code: "_()_",
        emoji: "🙏",
    },
    ReactionDef {
        key: "thanks",
        code: "/-thanks",
        emoji: "🙏",
    },
    ReactionDef {
        key: "handclap",
        code: ":handclap",
        emoji: "👏",
    },
];

/// Returns the display emoji for a reaction key (e.g. `"heart"`) or Zalo code (e.g. `"/-heart"`).
pub fn reaction_emoji(key_or_code: &str) -> Option<&'static str> {
    REACTIONS
        .iter()
        .find(|r| r.key == key_or_code || r.code == key_or_code)
        .map(|r| r.emoji)
}

/// Returns Zalo's reaction code for a stable API key (e.g. `"heart"` -> `"/-heart"`).
pub fn reaction_code(key: &str) -> Option<&'static str> {
    REACTIONS.iter().find(|r| r.key == key).map(|r| r.code)
}

/// Looks up a reaction definition by Zalo's wire code (e.g. `"/-heart"`).
pub fn reaction_by_code(code: &str) -> Option<&'static ReactionDef> {
    REACTIONS.iter().find(|r| r.code == code)
}

/// Returns `true` if `s` looks like a local Android/device URI or filesystem path that cannot
/// be fetched remotely (e.g. `content://...`, `file://...`, `/storage/...`, `/sdcard/...`).
pub fn is_local_device_uri(s: &str) -> bool {
    let trimmed = s.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("content://")
        || lower.starts_with("file://")
        || lower.starts_with("/storage/")
        || lower.starts_with("/sdcard/")
        || lower.starts_with("/data/")
        || lower.starts_with("/mnt/")
}

/// Returns a trimmed `http://` or `https://` URL with a non-empty host, stripping local URIs.
pub fn sanitize_remote_url(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() || is_local_device_uri(trimmed) {
        return None;
    }
    let rest = if trimmed.len() >= 8 && trimmed[..8].eq_ignore_ascii_case("https://") {
        &trimmed[8..]
    } else if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("http://") {
        &trimmed[7..]
    } else {
        return None;
    };
    let first = rest.chars().next()?;
    if first.is_whitespace() || first == '/' {
        return None;
    }
    Some(trimmed.to_string())
}

/// Strips empty strings, local phone URIs/paths, and `[Photo]`-style placeholder captions.
pub fn sanitize_caption(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() || is_local_device_uri(trimmed) {
        return None;
    }
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        let len = inner.chars().count();
        if (1..=24).contains(&len) && !inner.contains(']') {
            return None;
        }
    }
    Some(trimmed.to_string())
}

// ---- Flexible numeric deserializers for JS-generated cache files ----

fn is_false(b: &bool) -> bool {
    !*b
}

pub(crate) fn de_i64_flexible<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Value::deserialize(deserializer)?;
    match val {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i)
            } else if let Some(u) = n.as_u64() {
                Ok(u as i64)
            } else if let Some(f) = n.as_f64() {
                Ok(f.round() as i64)
            } else {
                Ok(0)
            }
        }
        Value::String(s) => Ok(s.trim().parse::<f64>().map(|f| f.round() as i64).unwrap_or(0)),
        _ => Ok(0),
    }
}

fn de_opt_u32_flexible<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<Value>::deserialize(deserializer)?;
    Ok(opt.and_then(|val| match val {
        Value::Number(n) => n
            .as_u64()
            .and_then(|u| u32::try_from(u).ok())
            .or_else(|| n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f.round() as u32)),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite() && *f >= 0.0)
            .map(|f| f.round() as u32),
        _ => None,
    }))
}

fn de_opt_u64_flexible<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<Value>::deserialize(deserializer)?;
    Ok(opt.and_then(|val| match val {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f.round() as u64)),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite() && *f >= 0.0)
            .map(|f| f.round() as u64),
        _ => None,
    }))
}

// ---- Core chat types ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadKind {
    User,
    Group,
}

impl ThreadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadKind::User => "user",
            ThreadKind::Group => "group",
        }
    }

    pub fn from_raw_type(raw_type: i64) -> Self {
        if raw_type == 1 {
            ThreadKind::Group
        } else {
            ThreadKind::User
        }
    }

    pub fn to_raw_type(self) -> u8 {
        match self {
            ThreadKind::User => 0,
            ThreadKind::Group => 1,
        }
    }
}

impl fmt::Display for ThreadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ThreadKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(ThreadKind::User),
            "group" => Ok(ThreadKind::Group),
            other => Err(format!("Invalid thread kind: {other}")),
        }
    }
}

/// Canonical cache key for a conversation (`"user:<id>"` or `"group:<id>"`).
pub fn thread_key(kind: ThreadKind, id: &str) -> String {
    format!("{}:{id}", kind.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfo {
    pub id: String,
    pub kind: ThreadKind,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub avatar: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Photo,
    Sticker,
    Gif,
    Video,
    Voice,
    File,
    Link,
    Contact,
    Location,
    Call,
    Poll,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerRef {
    pub id: i64,
    #[serde(default)]
    pub cat_id: i64,
    #[serde(default, rename = "type")]
    pub sticker_type: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallInfo {
    #[serde(default)]
    pub video: bool,
    #[serde(default)]
    pub missed: bool,
    #[serde(default)]
    pub outgoing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollRef {
    pub poll_id: i64,
    #[serde(default)]
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

/// Non-text message attachment matching `Attachment` in `src/store.ts` and `Models.kt`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AttachmentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_u32_flexible"
    )]
    pub width: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_u32_flexible"
    )]
    pub height: Option<u32>,
    /// Video and voice length in ms.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_u64_flexible"
    )]
    pub duration: Option<u64>,
    /// File size in bytes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_u64_flexible"
    )]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_ext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticker: Option<StickerRef>,
    /// Contact cards: the user id the card is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Call log entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<CallInfo>,
    /// Poll messages: details (options, votes) come from `GET /api/polls/:pollId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<PollRef>,
    /// Call-to-action button label for OA/ZNS/ecard link cards (e.g. `"ĐẶT LẠI MÃ PIN"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_label: Option<String>,
    /// Detected one-time verification / OTP code (4–8 digits) for quick copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otp: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteInfo {
    pub msg_id: String,
    #[serde(default)]
    pub from_id: String,
    #[serde(default)]
    pub from_name: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionInfo {
    pub uid: String,
    pub pos: i32,
    pub len: i32,
}

/// Aggregated reaction count shown on `ChatMessage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionInfo {
    pub key: String,
    pub emoji: String,
    pub count: usize,
    #[serde(default)]
    pub mine: bool,
}

/// Alias matching the TypeScript / Kotlin `ReactionSummary` name.
pub type ReactionSummary = ReactionInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    Sent,
    Delivered,
    Seen,
}

/// Raw Zalo message fields kept server-side only (needed to quote, react, recall, delete, or mark seen).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawRef {
    #[serde(default)]
    pub msg_type: String,
    #[serde(default)]
    pub content: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_ext: Option<Value>,
    #[serde(default)]
    pub uid_from: String,
    #[serde(default)]
    pub id_to: String,
    #[serde(default)]
    pub ts: String,
    #[serde(default, deserialize_with = "de_i64_flexible")]
    pub ttl: i64,
    #[serde(default, deserialize_with = "de_i64_flexible")]
    pub st: i64,
    #[serde(default, deserialize_with = "de_i64_flexible")]
    pub at: i64,
    #[serde(default, deserialize_with = "de_i64_flexible")]
    pub cmd: i64,
}

/// Public message view returned by HTTP/WebSocket APIs (`raw`, `deliveredTo`, `seenBy` stripped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    #[serde(default)]
    pub cli_msg_id: String,
    pub thread_id: String,
    pub thread_kind: ThreadKind,
    #[serde(default)]
    pub from_id: String,
    #[serde(default)]
    pub from_name: String,
    #[serde(default)]
    pub is_self: bool,
    #[serde(deserialize_with = "de_i64_flexible")]
    pub ts: i64,
    #[serde(default)]
    pub msg_type: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<QuoteInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<MentionInfo>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub recalled: bool,
    #[serde(default)]
    pub reactions: Vec<ReactionInfo>,
    #[serde(default)]
    pub mentions_me: bool,
    /// Only for messages sent by this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<MessageStatus>,
    /// Group chats only: how many members have seen it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_count: Option<usize>,
}

/// Persisted on-disk message inside `threads/<hex>.json` and legacy `messages.json`.
/// Never serialized directly to clients; convert via `ChatStore::to_public`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub id: String,
    #[serde(default)]
    pub cli_msg_id: String,
    pub thread_id: String,
    pub thread_kind: ThreadKind,
    #[serde(default)]
    pub from_id: String,
    #[serde(default)]
    pub from_name: String,
    #[serde(default)]
    pub is_self: bool,
    #[serde(deserialize_with = "de_i64_flexible")]
    pub ts: i64,
    #[serde(default)]
    pub msg_type: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<QuoteInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<MentionInfo>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub recalled: bool,
    /// Latest Zalo reaction code per user id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactions: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_to: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_by: Option<Vec<String>>,
    /// Server-side raw fields; omitted from API responses and absent for imported history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<RawRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub kind: ThreadKind,
    pub name: String,
    pub avatar: String,
    pub last_message: Option<ChatMessage>,
    pub unread: usize,
    pub pinned: bool,
    /// Epoch ms the mute ends, -1 for "until turned off", absent when not muted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_until: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSearchHit {
    pub thread_id: String,
    pub thread_kind: ThreadKind,
    pub thread_name: String,
    pub thread_avatar: String,
    pub message: ChatMessage,
}

// ---- Session, social, sticker, poll, call, and history-sync API types ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    LoggedOut,
    AwaitingQrScan,
    LoggingIn,
    Ready,
    Disconnected,
    /// The chat listener is paused (`POST /api/listener/pause`); logged in, not connected.
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub state: SessionState,
    pub qr_image: Option<String>,
    pub scanned_by: Option<String>,
    pub own_id: Option<String>,
    pub own_name: Option<String>,
    pub own_avatar: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub retrying: bool,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self {
            state: SessionState::LoggedOut,
            qr_image: None,
            scanned_by: None,
            own_id: None,
            own_name: None,
            own_avatar: None,
            error: None,
            retrying: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub avatar: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundUser {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub is_friend: bool,
    #[serde(default)]
    pub requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendRequest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, deserialize_with = "de_i64_flexible")]
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerInfo {
    pub id: i64,
    #[serde(default)]
    pub cat_id: i64,
    #[serde(default, rename = "type")]
    pub sticker_type: i32,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollVoter {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollOptionView {
    pub id: i64,
    pub text: String,
    #[serde(default)]
    pub votes: usize,
    #[serde(default)]
    pub voted: bool,
    #[serde(default)]
    pub voters: Vec<PollVoter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollView {
    pub poll_id: i64,
    pub question: String,
    pub creator_id: String,
    pub creator_name: String,
    pub closed: bool,
    pub expired: bool,
    pub expires_at: i64,
    pub created_at: i64,
    pub allow_multiple: bool,
    pub allow_add_options: bool,
    pub anonymous: bool,
    pub results_hidden: bool,
    pub total_votes: usize,
    pub voter_count: usize,
    pub my_votes: Vec<i64>,
    pub can_vote: bool,
    pub can_add_options: bool,
    pub can_close: bool,
    pub options: Vec<PollOptionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingCall {
    pub call_id: String,
    pub caller_id: String,
    #[serde(default)]
    pub video: bool,
    #[serde(default)]
    pub group: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, deserialize_with = "de_i64_flexible")]
    pub ts: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub caller_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallState {
    Connecting,
    Calling,
    Ringing,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCall {
    pub call_id: String,
    pub caller_id: String,
    pub caller_name: String,
    #[serde(default)]
    pub video: bool,
    pub state: CallState,
    pub connected_at: Option<i64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub outgoing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CallEvent {
    Incoming {
        call: IncomingCall,
    },
    Connected(CurrentCall),
    Outgoing(CurrentCall),
    Ringing(CurrentCall),
    Ended {
        #[serde(rename = "callId")]
        call_id: String,
        #[serde(default)]
        act: String,
    },
    Peer {
        #[serde(rename = "callId")]
        call_id: String,
        #[serde(rename = "cameraOn")]
        camera_on: bool,
        muted: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Idle,
    WaitingForPhone,
    Downloading,
    Importing,
    Done,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySyncCounts {
    #[serde(default)]
    pub imported: usize,
    #[serde(default)]
    pub duplicates: usize,
    #[serde(default)]
    pub skipped: usize,
    #[serde(default)]
    pub recalled: usize,
    #[serde(default)]
    pub conversations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySyncStatus {
    pub state: SyncState,
    pub progress: u8,
    pub error: Option<String>,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_messages: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<HistorySyncCounts>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<String>,
}

impl Default for HistorySyncStatus {
    fn default() -> Self {
        Self {
            state: SyncState::Idle,
            progress: 0,
            error: None,
            detail: String::new(),
            expected_messages: None,
            counts: None,
            log: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reactions_constant_has_all_15_entries() {
        assert_eq!(REACTIONS.len(), 15);
        assert_eq!(reaction_emoji("heart"), Some("❤️"));
        assert_eq!(reaction_emoji("/-heart"), Some("❤️"));
        assert_eq!(reaction_code("like"), Some("/-strong"));
        assert_eq!(reaction_by_code(":handclap").map(|r| r.key), Some("handclap"));
        assert_eq!(reaction_emoji("unknown"), None);
    }

    #[test]
    fn uri_and_caption_sanitizers() {
        assert!(is_local_device_uri("content://media/external/images/1"));
        assert!(is_local_device_uri("FILE:///sdcard/pic.jpg"));
        assert!(is_local_device_uri("/storage/emulated/0/DCIM/1.jpg"));
        assert!(is_local_device_uri("/sdcard/Download/a.png"));
        assert!(is_local_device_uri("/data/user/0/app/a.png"));
        assert!(is_local_device_uri("/mnt/media/1.png"));
        assert!(!is_local_device_uri("https://cdn.zaloapp.com/a.jpg"));

        assert_eq!(
            sanitize_remote_url("  https://cdn.zaloapp.com/a.jpg  "),
            Some("https://cdn.zaloapp.com/a.jpg".to_string())
        );
        assert_eq!(sanitize_remote_url("content://media/1"), None);
        assert_eq!(sanitize_remote_url("http:///bad"), None);
        assert_eq!(sanitize_remote_url("ftp://example.com/a"), None);

        assert_eq!(sanitize_caption("[Photo]"), None);
        assert_eq!(sanitize_caption("content://media/1"), None);
        assert_eq!(sanitize_caption("/storage/emulated/0/a.jpg"), None);
        assert_eq!(sanitize_caption("   "), None);
        assert_eq!(sanitize_caption("Báo cáo.PDF"), Some("Báo cáo.PDF".to_string()));
    }

    #[test]
    fn call_event_and_session_status_wire_json() {
        let ev = CallEvent::Connected(CurrentCall {
            call_id: "c1".into(),
            caller_id: "u1".into(),
            caller_name: "Alice".into(),
            video: true,
            state: CallState::Connected,
            connected_at: Some(1700000000000),
            outgoing: true,
        });
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            json!({
                "type": "connected",
                "callId": "c1",
                "callerId": "u1",
                "callerName": "Alice",
                "video": true,
                "state": "connected",
                "connectedAt": 1700000000000i64,
                "outgoing": true,
            })
        );
        let back: CallEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, ev);

        let status = SessionStatus {
            state: SessionState::AwaitingQrScan,
            qr_image: Some("data:image/png;base64,AAA".into()),
            ..Default::default()
        };
        let sv = serde_json::to_value(&status).unwrap();
        assert_eq!(sv["state"], "awaiting_qr_scan");
        assert_eq!(sv["qrImage"], "data:image/png;base64,AAA");
        assert_eq!(sv["retrying"], false);
        assert!(sv["ownId"].is_null());
    }
}
