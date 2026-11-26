use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::cookies::{deserialize_saved_cookies, SavedCookie, ZaloCookieJar};

/// Default Zalo Web API type and version matching `zca-js` v2.2.0.
pub const DEFAULT_API_TYPE: u32 = 30;
pub const DEFAULT_API_VERSION: u32 = 685;

/// Zalo thread type (`0 = User`, `1 = Group`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ThreadType {
    User = 0,
    Group = 1,
}

impl ThreadType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::User),
            1 => Some(Self::Group),
            _ => None,
        }
    }

    pub fn is_group(self) -> bool {
        matches!(self, Self::Group)
    }
}

/// Destination type for typing events (`zca-js` `DestType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DestType {
    Group = 1,
    User = 3,
    Page = 5,
}

/// Avatar size enum (`zca-js` `AvatarSize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum AvatarSize {
    Small = 120,
    Medium = 180,
    Large = 240,
    ExtraLarge = 360,
}

/// Serializable credentials compatible with `zca-js`'s `credentials.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub imei: String,
    #[serde(deserialize_with = "deserialize_saved_cookies")]
    pub cookie: Vec<SavedCookie>,
    #[serde(rename = "userAgent", alias = "user_agent")]
    pub user_agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Events emitted during QR code login.
#[derive(Debug, Clone)]
pub enum LoginQrEvent {
    QrCodeGenerated {
        image_base64: String,
        code: String,
    },
    QrCodeScanned {
        display_name: String,
        avatar: Option<String>,
    },
    QrCodeExpired,
    QrCodeDeclined,
    GotLoginInfo(Credentials),
}

/// Payload delivered over WebSocket `cmd=601` `act_type="file_done"` after an async file upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDonePayload {
    #[serde(rename = "fileUrl")]
    pub file_url: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
}

/// Active Zalo session context.
pub struct ZaloContext {
    pub uid: String,
    pub imei: String,
    pub user_agent: String,
    pub language: String,
    pub api_type: u32,
    pub api_version: u32,
    pub secret_key: String,
    pub service_map: HashMap<String, Vec<String>>,
    pub ws_urls: Vec<String>,
    pub ctrs_urls: Vec<String>,
    pub settings: Value,
    pub extra_ver: Value,
    pub login_info: Value,
    pub cookies: Arc<RwLock<ZaloCookieJar>>,
    pub upload_callbacks: Arc<Mutex<HashMap<String, oneshot::Sender<FileDonePayload>>>>,
}

impl std::fmt::Debug for ZaloContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZaloContext")
            .field("uid", &self.uid)
            .field("imei", &self.imei)
            .field("language", &self.language)
            .field("api_type", &self.api_type)
            .field("api_version", &self.api_version)
            .field("ws_urls", &self.ws_urls)
            .field("ctrs_urls", &self.ctrs_urls)
            .finish_non_exhaustive()
    }
}

impl ZaloContext {
    /// Returns the first URL for a service key in `zpw_service_map_v3`, trimming trailing slashes.
    pub fn service_url(&self, service: &str) -> Option<String> {
        self.service_map
            .get(service)
            .and_then(|list| list.first())
            .map(|u| u.trim_end_matches('/').to_string())
    }

    /// Exports current credentials from the active session.
    pub fn export_credentials(&self) -> Credentials {
        let cookie = self
            .cookies
            .read()
            .map(|jar| jar.export_cookies())
            .unwrap_or_default();
        Credentials {
            imei: self.imei.clone(),
            cookie,
            user_agent: self.user_agent.clone(),
            language: Some(self.language.clone()),
        }
    }
}

/// Normalized raw WebSocket message (covers `msgs`, `groupMsgs`, and `pageMsgs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawWsMessage {
    pub thread_type: ThreadType,
    pub thread_id: String,
    pub is_self: bool,
    pub data: Value,
}

/// Events emitted by [`crate::listener::ZaloListener`].
#[derive(Debug, Clone)]
pub enum WsEvent {
    Connected,
    CipherKey(String),
    Messages(Vec<RawWsMessage>),
    OldMessages {
        thread_type: ThreadType,
        messages: Vec<RawWsMessage>,
    },
    Undo(RawWsMessage),
    Reactions {
        thread_type: ThreadType,
        reactions: Vec<Value>,
        is_old: bool,
    },
    Typing {
        thread_type: ThreadType,
        thread_id: String,
        uid: String,
        is_pc: bool,
    },
    Delivered {
        thread_type: ThreadType,
        data: Value,
    },
    Seen {
        thread_type: ThreadType,
        data: Value,
    },
    GroupEvent(Value),
    FriendEvent(Value),
    VoipControl {
        cmd: u16,
        sub_cmd: u8,
        control: Value,
    },
    HistorySyncControl {
        cmd: u16,
        sub_cmd: u8,
        control: Value,
    },
    RawFrame {
        cmd: u16,
        sub_cmd: u8,
        payload: Value,
    },
    Closed {
        code: u16,
        reason: String,
    },
    Error(String),
}

// ---- Poll view types (matching src/polls.ts JSON serialization exactly) ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PollVoter {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PollOptionView {
    pub id: i64,
    pub text: String,
    pub votes: u64,
    pub voted: bool,
    pub voters: Vec<PollVoter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    pub total_votes: u64,
    pub voter_count: usize,
    pub my_votes: Vec<i64>,
    pub can_vote: bool,
    pub can_add_options: bool,
    pub can_close: bool,
    pub options: Vec<PollOptionView>,
}

#[derive(Debug, Clone, Default)]
pub struct PollViewContext<'a> {
    pub own_id: Option<&'a str>,
    pub is_admin: bool,
    pub now: i64,
    pub names: Option<&'a HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePollInput {
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub allow_multiple: bool,
    #[serde(default)]
    pub allow_add_options: bool,
    #[serde(default)]
    pub anonymous: bool,
    #[serde(default)]
    pub hide_vote_preview: bool,
    #[serde(default)]
    pub expires_at: i64,
}

// ---- API request/response helper types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionInput {
    pub pos: usize,
    pub uid: String,
    pub len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteInput {
    pub uid_from: String,
    pub msg_id: String,
    pub cli_msg_id: String,
    pub msg_type: String,
    pub ts: Value,
    pub content: Value,
    #[serde(default)]
    pub property_ext: Option<Value>,
    #[serde(default)]
    pub ttl: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct UploadSource {
    pub filename: String,
    pub data: Vec<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct SendMessageOptions {
    pub msg: String,
    pub attachments: Vec<UploadSource>,
    pub mentions: Vec<MentionInput>,
    pub quote: Option<QuoteInput>,
    pub ttl: Option<i64>,
    pub urgency: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "fileType")]
pub enum UploadResult {
    #[serde(rename = "image")]
    Image {
        width: u32,
        height: u32,
        #[serde(rename = "totalSize")]
        total_size: usize,
        #[serde(rename = "hdSize")]
        hd_size: usize,
        finished: Value,
        #[serde(rename = "normalUrl")]
        normal_url: String,
        #[serde(rename = "hdUrl")]
        hd_url: String,
        #[serde(rename = "thumbUrl")]
        thumb_url: String,
        #[serde(rename = "chunkId")]
        chunk_id: Value,
        #[serde(rename = "photoId")]
        photo_id: Value,
        #[serde(rename = "clientFileId")]
        client_file_id: Value,
    },
    #[serde(rename = "video")]
    Video {
        #[serde(rename = "fileId")]
        file_id: String,
        #[serde(rename = "fileUrl")]
        file_url: String,
        #[serde(rename = "fileName")]
        file_name: String,
        #[serde(rename = "totalSize")]
        total_size: usize,
        checksum: String,
        #[serde(rename = "clientFileId")]
        client_file_id: Value,
    },
    #[serde(rename = "others")]
    Others {
        #[serde(rename = "fileId")]
        file_id: String,
        #[serde(rename = "fileUrl")]
        file_url: String,
        #[serde(rename = "fileName")]
        file_name: String,
        #[serde(rename = "totalSize")]
        total_size: usize,
        checksum: String,
        #[serde(rename = "clientFileId")]
        client_file_id: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResponse {
    pub message: Option<Value>,
    pub attachment: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerParam {
    pub id: i64,
    #[serde(rename = "cateId", alias = "catId")]
    pub cate_id: i64,
    #[serde(rename = "type")]
    pub sticker_type: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionTarget {
    pub thread_id: String,
    pub thread_type: ThreadType,
    pub msg_id: String,
    pub cli_msg_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoTarget {
    pub msg_id: String,
    pub cli_msg_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMessageTarget {
    pub thread_id: String,
    pub thread_type: ThreadType,
    pub msg_id: String,
    pub cli_msg_id: String,
    pub uid_from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardReference {
    pub id: String,
    pub ts: i64,
    #[serde(rename = "fwLvl")]
    pub fw_lvl: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardMessagePayload {
    pub message: String,
    #[serde(default)]
    pub ttl: Option<i64>,
    #[serde(default)]
    pub reference: Option<ForwardReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeenMessageInput {
    pub msg_id: String,
    pub cli_msg_id: String,
    pub uid_from: String,
    pub id_to: String,
    pub msg_type: String,
    #[serde(default)]
    pub st: Option<i64>,
    #[serde(default)]
    pub at: Option<i64>,
    #[serde(default)]
    pub cmd: Option<i64>,
    #[serde(default)]
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteAction {
    Mute = 1,
    Unmute = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteDuration {
    OneHour,
    FourHours,
    Until8Am,
    Forever,
    Seconds(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastMessageInfo {
    #[serde(rename = "ownerId")]
    pub owner_id: String,
    #[serde(rename = "cliMsgId")]
    pub cli_msg_id: String,
    #[serde(rename = "globalMsgId")]
    pub global_msg_id: String,
}

#[derive(Debug, Clone)]
pub struct CreateGroupOptions {
    pub name: Option<String>,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddPollOptionsInput {
    pub poll_id: i64,
    pub options: Vec<String>,
    pub voted_option_ids: Vec<i64>,
}
