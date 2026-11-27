use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::future::join_all;
use reqwest::Method;
use serde_json::{json, Map, Value};
use tokio::sync::oneshot;

use crate::api::ZaloClient;
use crate::crypto::{encode_aes, md5_hex};
use crate::error::{ProtoError, Result};
use crate::http::ZaloHttp;
use crate::types::{
    DeleteMessageTarget, DestType, ForwardMessagePayload, MentionInput, QuoteInput,
    ReactionTarget, SeenMessageInput, SendMessageOptions, SendMessageResponse, StickerParam,
    ThreadType, UndoTarget, UploadProgress, UploadProgressSink, UploadResult, UploadSource,
};

const UPLOAD_FILE_DONE_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_MESSAGES_PER_SEEN: usize = 50;

/// Reports one file's upload progress to an optional sink: a `sent = 0` report on creation,
/// monotonic `advance` reports, and exactly one final `done` report (on [`Self::finish`], or on
/// drop when the upload failed or was cancelled part-way).
pub(crate) struct UploadTracker {
    sink: Option<UploadProgressSink>,
    index: usize,
    filename: String,
    total: u64,
    sent: std::sync::atomic::AtomicU64,
    finished: std::sync::atomic::AtomicBool,
}

impl UploadTracker {
    pub(crate) fn start(
        sink: Option<UploadProgressSink>,
        index: usize,
        filename: &str,
        total: u64,
    ) -> Arc<Self> {
        let t = Arc::new(Self {
            sink,
            index,
            filename: filename.to_string(),
            total,
            sent: Default::default(),
            finished: Default::default(),
        });
        t.report(false);
        t
    }

    fn report(&self, done: bool) {
        if let Some(sink) = &self.sink {
            sink.report(UploadProgress {
                index: self.index,
                filename: self.filename.clone(),
                sent: self.sent.load(std::sync::atomic::Ordering::SeqCst),
                total: self.total,
                done,
            });
        }
    }

    /// Adds `n` sent bytes (clamped to `total`) and reports.
    pub(crate) fn advance(&self, n: u64) {
        use std::sync::atomic::Ordering::SeqCst;
        let prev = self.sent.load(SeqCst);
        let next = prev.saturating_add(n).min(self.total);
        self.sent.store(next, SeqCst);
        if next != prev {
            self.report(false);
        }
    }

    /// Final report; on success `sent` is set to `total` first.
    pub(crate) fn finish(&self, success: bool) {
        use std::sync::atomic::Ordering::SeqCst;
        if self.finished.swap(true, SeqCst) {
            return;
        }
        if success {
            self.sent.store(self.total, SeqCst);
        }
        self.report(true);
    }
}

impl Drop for UploadTracker {
    fn drop(&mut self) {
        self.finish(false);
    }
}

/// Maps a Zalo reaction icon string (all `zca-js` `Reactions` codes + `""` to clear) to its numeric `rType`.
pub fn reaction_type_id(icon: &str) -> i32 {
    match icon {
        ":>" => 0,          // HAHA
        "--b" => 1,         // SAD
        ":-((" => 2,        // CRY
        "/-strong" => 3,    // LIKE
        "/-weak" => 4,      // DISLIKE
        "/-heart" => 5,     // HEART
        ":')" => 7,         // TEARS_OF_JOY
        ":-*" => 8,         // KISS
        ":((" => 16,        // VERY_SAD
        ":-h" => 20,        // ANGRY
        "x-)" => 21,        // COOL
        "8-)" => 22,        // NERD
        ";-d" => 23,        // BIG_SMILE
        "b-)" => 26,        // SUNGLASSES
        ";xx" => 29,        // LOVE
        ":--|" => 30,       // NEUTRAL
        ":o" => 32,         // WOW
        "p-(" => 35,        // SAD_FACE
        ":-bye" => 36,      // BYE
        "|-)" => 38,        // SLEEPY
        ":wipe" => 39,      // WIPE
        ":-dig" => 42,      // DIG
        "&-(" => 44,        // ANGUISH
        ";-)" => 45,        // WINK
        ":handclap" => 46,  // HANDCLAP
        ">-|" => 47,        // ANGRY_FACE
        ":-f" => 48,        // F_CHAIR
        ":-l" => 49,        // L_CHAIR
        ":-r" => 50,        // R_CHAIR
        ";-/" => 51,        // CONFUSED
        ";-x" => 52,        // SILENT
        ":-o" => 53,        // SURPRISE
        ";-s" => 54,        // EMBARRASSED
        ";-a" => 60,        // AFRAID
        ":-<" => 61,        // SAD2
        ":))" => 62,        // BIG_LAUGH
        "$-)" => 63,        // RICH
        "/-break" => 65,    // BROKEN_HEART
        "/-shit" => 66,     // SHIT
        "/-li" => 67,       // SUN
        "/-ok" => 68,       // OK
        "/-v" => 69,        // PEACE
        "/-thanks" => 70,   // THANKS
        "/-punch" => 71,    // PUNCH
        "/-share" => 72,    // SHARE
        "_()_" => 73,       // PRAY
        "/-beer" => 99,     // BEER
        "/-rose" => 120,    // ROSE
        "/-fade" => 121,    // FADE
        "/-bd" => 126,      // BIRTHDAY
        "/-bome" => 127,    // BOMB
        "/-no" => 131,      // NO
        "/-bad" => 132,     // BAD
        "/-loveu" => 133,   // LOVE_YOU
        "" => -1,           // NONE (remove reaction)
        _ => -1,
    }
}

fn client_message_type(msg_type: &str) -> i32 {
    match msg_type {
        "webchat" => 1,
        "chat.voice" => 31,
        "chat.photo" => 32,
        "chat.sticker" => 36,
        "chat.doodle" => 37,
        "chat.recommended" | "chat.link" => 38,
        "chat.location.new" => 43,
        "chat.video.msg" => 44,
        "share.file" => 46,
        "chat.gif" => 49,
        _ => 1,
    }
}

fn file_extension(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

fn prepare_qmsg_attach(quote: &QuoteInput) -> Value {
    if quote.content.is_string() {
        return quote.property_ext.clone().unwrap_or(Value::Null);
    }
    if quote.msg_type == "chat.todo" {
        return json!({
            "properties": {
                "color": 0,
                "size": 0,
                "type": 0,
                "subType": 0,
                "ext": "{\"shouldParseLinkOrContact\":0}"
            }
        });
    }
    let mut obj = quote
        .content
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(thumb) = obj.get("thumb").cloned() {
        obj.insert("thumbUrl".to_string(), thumb);
    }
    if let Some(href) = obj.get("href").cloned() {
        obj.insert("oriUrl".to_string(), href.clone());
        obj.insert("normalUrl".to_string(), href);
    }
    Value::Object(obj)
}

fn prepare_qmsg(quote: &QuoteInput) -> String {
    if let Some(s) = quote.content.as_str() {
        return s.to_string();
    }
    if quote.msg_type == "chat.todo" {
        if let Some(params_s) = quote.content.get("params").and_then(|v| v.as_str()) {
            if let Ok(parsed) = serde_json::from_str::<Value>(params_s) {
                if let Some(c) = parsed
                    .get("item")
                    .and_then(|i| i.get("content"))
                    .and_then(|v| v.as_str())
                {
                    return c.to_string();
                }
            }
        }
    }
    String::new()
}

fn build_mentions(
    thread_type: ThreadType,
    msg: &str,
    mentions: &[MentionInput],
) -> Result<Vec<Value>> {
    if !thread_type.is_group() || mentions.is_empty() {
        return Ok(Vec::new());
    }
    let mut total_len = 0usize;
    let mut out = Vec::new();
    for m in mentions {
        if !m.uid.is_empty() && m.len > 0 {
            total_len += m.len;
            out.push(json!({
                "pos": m.pos,
                "uid": m.uid,
                "len": m.len,
                "type": if m.uid == "-1" { 1 } else { 0 },
            }));
        }
    }
    if total_len > msg.encode_utf16().count() && total_len > msg.len() {
        return Err(ProtoError::InvalidArg(
            "Invalid mentions: total mention characters exceed message length".into(),
        ));
    }
    Ok(out)
}

impl ZaloClient {
    fn sharefile_limits(&self) -> (usize, usize, usize, Vec<String>) {
        let sf = self
            .ctx
            .settings
            .get("features")
            .and_then(|f| f.get("sharefile"));
        let max_file = sf
            .and_then(|s| s.get("max_file"))
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;
        let max_size_mb = sf
            .and_then(|s| s.get("max_size_share_file_v3"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1024) as usize;
        let chunk_size = sf
            .and_then(|s| s.get("chunk_size_file"))
            .and_then(|v| v.as_u64())
            .filter(|&n| n > 0)
            .unwrap_or(1_048_576) as usize;
        let restricted = sf
            .and_then(|s| s.get("restricted_ext_file"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
                    .collect()
            })
            .unwrap_or_default();
        (max_file, max_size_mb, chunk_size, restricted)
    }

    /// Uploads one or more attachments (chunked multipart upload + MD5 + WebSocket `file_done` wait for video/files).
    pub async fn upload_attachment(
        &self,
        sources: &[UploadSource],
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Vec<UploadResult>> {
        self.upload_attachment_with_progress(sources, thread_id, thread_type, None)
            .await
    }

    /// [`Self::upload_attachment`] reporting per-file byte progress to `progress`: a `sent = 0`
    /// report when a file starts, reports as its body is streamed out, and a final `done` report
    /// (also on failure). Files not reached because an earlier one failed report nothing.
    pub async fn upload_attachment_with_progress(
        &self,
        sources: &[UploadSource],
        thread_id: &str,
        thread_type: ThreadType,
        progress: Option<UploadProgressSink>,
    ) -> Result<Vec<UploadResult>> {
        if sources.is_empty() {
            return Err(ProtoError::InvalidArg("Missing sources".into()));
        }
        if thread_id.is_empty() {
            return Err(ProtoError::InvalidArg("Missing threadId".into()));
        }
        let (max_file, max_size_mb, chunk_size, restricted) = self.sharefile_limits();
        if sources.len() > max_file {
            return Err(ProtoError::InvalidArg(format!(
                "Exceed maximum file of {max_file}"
            )));
        }

        let file_base = self.require_service("file")?;
        let is_group = thread_type.is_group();
        let url_prefix = format!(
            "{file_base}/api/{}/",
            if is_group { "group" } else { "message" }
        );
        let type_param = if is_group { "11" } else { "2" };
        let client_id = Utc::now().timestamp_millis();
        let mut results = Vec::with_capacity(sources.len());

        for (src_index, src) in sources.iter().enumerate() {
            if src.filename.is_empty() {
                return Err(ProtoError::InvalidArg("Missing filename".into()));
            }
            let ext = file_extension(&src.filename);
            if restricted.iter().any(|r| r == &ext) {
                return Err(ProtoError::InvalidArg(format!(
                    "File extension \"{ext}\" is not allowed"
                )));
            }
            let total_size = src.data.len();
            if total_size > max_size_mb * 1024 * 1024 {
                return Err(ProtoError::InvalidArg(format!(
                    "File {} size exceeds maximum size of {}MB",
                    src.filename, max_size_mb
                )));
            }

            let file_kind = match ext.as_str() {
                "jpg" | "jpeg" | "png" | "webp" => "image",
                "mp4" => "video",
                _ => "others",
            };
            let endpoint_suffix = if file_kind == "image" {
                "photo_original/upload"
            } else {
                "asyncfile/upload"
            };
            let total_chunk = total_size.div_ceil(chunk_size).max(1);
            let cur_client_id = client_id + src_index as i64;

            let mut final_res_data: Option<Value> = None;
            let tracker = UploadTracker::start(
                progress.clone(),
                src_index,
                &src.filename,
                total_size as u64,
            );

            for chunk_idx in 0..total_chunk {
                let start = chunk_idx * chunk_size;
                let end = (start + chunk_size).min(total_size);
                let slice = src.data[start..end].to_vec();

                let mut params = Map::new();
                if is_group {
                    params.insert("grid".to_string(), Value::String(thread_id.to_string()));
                } else {
                    params.insert("toid".to_string(), Value::String(thread_id.to_string()));
                }
                params.insert("totalChunk".to_string(), json!(total_chunk));
                params.insert("fileName".to_string(), json!(src.filename));
                params.insert("clientId".to_string(), json!(cur_client_id));
                params.insert("totalSize".to_string(), json!(total_size));
                params.insert("imei".to_string(), json!(self.ctx.imei));
                params.insert("isE2EE".to_string(), json!(0));
                params.insert("jxl".to_string(), json!(0));
                params.insert("chunkId".to_string(), json!(chunk_idx + 1));

                let encrypted = encode_aes(&self.ctx.secret_key, &Value::Object(params).to_string())?;
                let upload_url = ZaloHttp::make_ctx_url(
                    &self.ctx,
                    &format!("{url_prefix}{endpoint_suffix}"),
                    &[("type", type_param), ("params", &encrypted)],
                    true,
                )?;
                let on_bytes: Option<Arc<dyn Fn(u64) + Send + Sync>> = progress.as_ref().map(|_| {
                    let t = Arc::clone(&tracker);
                    Arc::new(move |n| t.advance(n)) as Arc<dyn Fn(u64) + Send + Sync>
                });
                let res_data = self
                    .http
                    .post_multipart_chunk_counted(
                        &self.ctx,
                        &upload_url,
                        "chunkContent",
                        &src.filename,
                        "application/octet-stream",
                        slice,
                        on_bytes,
                    )
                    .await?;

                let file_id_str = res_data
                    .get("fileId")
                    .map(val_to_str)
                    .unwrap_or_default();
                let photo_id_str = res_data
                    .get("photoId")
                    .map(val_to_str)
                    .unwrap_or_default();
                if file_id_str != "-1" && photo_id_str != "-1" {
                    final_res_data = Some(res_data);
                }
            }

            let res_data = final_res_data.ok_or_else(|| {
                ProtoError::Api {
                    code: -1,
                    message: "Upload did not complete with valid fileId/photoId".into(),
                }
            })?;

            tracker.finish(true);

            if file_kind == "image" {
                results.push(UploadResult::Image {
                    width: src.width.unwrap_or(0),
                    height: src.height.unwrap_or(0),
                    total_size,
                    hd_size: total_size,
                    finished: res_data.get("finished").cloned().unwrap_or(Value::Null),
                    normal_url: res_data
                        .get("normalUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    hd_url: res_data
                        .get("hdUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    thumb_url: res_data
                        .get("thumbUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    chunk_id: res_data.get("chunkId").cloned().unwrap_or(Value::Null),
                    photo_id: res_data.get("photoId").cloned().unwrap_or(Value::Null),
                    client_file_id: res_data.get("clientFileId").cloned().unwrap_or(Value::Null),
                });
            } else {
                let file_id = res_data
                    .get("fileId")
                    .map(val_to_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| ProtoError::Api {
                        code: -1,
                        message: "Upload response missing fileId".into(),
                    })?;

                let (tx, rx) = oneshot::channel();
                {
                    if let Ok(mut map) = self.ctx.upload_callbacks.lock() {
                        map.insert(file_id.clone(), tx);
                    }
                }
                let ws_done = match tokio::time::timeout(UPLOAD_FILE_DONE_TIMEOUT, rx).await {
                    Ok(Ok(payload)) => payload,
                    _ => {
                        if let Ok(mut map) = self.ctx.upload_callbacks.lock() {
                            map.remove(&file_id);
                        }
                        return Err(ProtoError::UploadTimeout);
                    }
                };
                let checksum = md5_hex(&src.data);
                let client_file_id = res_data.get("clientFileId").cloned().unwrap_or(Value::Null);
                if file_kind == "video" {
                    results.push(UploadResult::Video {
                        file_id: ws_done.file_id,
                        file_url: ws_done.file_url,
                        file_name: src.filename.clone(),
                        total_size,
                        checksum,
                        client_file_id,
                    });
                } else {
                    results.push(UploadResult::Others {
                        file_id: ws_done.file_id,
                        file_url: ws_done.file_url,
                        file_name: src.filename.clone(),
                        total_size,
                        checksum,
                        client_file_id,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Sends a text message and/or attachments (with optional quote, mentions, urgency, TTL).
    pub async fn send_message(
        &self,
        mut options: SendMessageOptions,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<SendMessageResponse> {
        if thread_id.is_empty() {
            return Err(ProtoError::InvalidArg("Missing threadId".into()));
        }
        if options.msg.is_empty() && options.attachments.is_empty() {
            return Err(ProtoError::InvalidArg("Missing message content".into()));
        }

        let (max_file, _, _, _) = self.sharefile_limits();
        if options.attachments.len() > max_file {
            return Err(ProtoError::InvalidArg(format!(
                "Exceed maximum file of {max_file}"
            )));
        }

        let mut response = SendMessageResponse {
            message: None,
            attachment: Vec::new(),
        };

        if !options.attachments.is_empty() {
            let first_ext = file_extension(&options.attachments[0].filename);
            let is_single = options.attachments.len() == 1;
            let can_be_desc =
                is_single && matches!(first_ext.as_str(), "jpg" | "jpeg" | "png" | "webp");
            if (!can_be_desc && !options.msg.is_empty())
                || (!options.msg.is_empty() && options.quote.is_some())
            {
                let text_res = self
                    .send_text_part(&options, thread_id, thread_type)
                    .await?;
                response.message = Some(text_res);
                options.msg.clear();
                options.mentions.clear();
            }

            let att_res = self
                .send_attachments_part(&options, thread_id, thread_type)
                .await?;
            response.attachment = att_res;
            options.msg.clear();
        }

        if !options.msg.is_empty() {
            let text_res = self
                .send_text_part(&options, thread_id, thread_type)
                .await?;
            response.message = Some(text_res);
        }

        Ok(response)
    }

    async fn send_text_part(
        &self,
        options: &SendMessageOptions,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Value> {
        if options.msg.is_empty() {
            return Err(ProtoError::InvalidArg("Missing message content".into()));
        }
        let is_group = thread_type.is_group();
        let mentions = build_mentions(thread_type, &options.msg, &options.mentions)?;
        let has_mentions = !mentions.is_empty() && is_group;

        if let Some(ref q) = options.quote {
            if !q.content.is_string() && q.msg_type == "webchat" {
                return Err(ProtoError::InvalidArg(
                    "This kind of `webchat` quote type is not available".into(),
                ));
            }
            if q.msg_type == "group.poll" {
                return Err(ProtoError::InvalidArg(
                    "The `group.poll` quote type is not available".into(),
                ));
            }
        }

        let mut params = Map::new();
        params.insert("message".to_string(), json!(options.msg));
        params.insert("clientId".to_string(), json!(Utc::now().timestamp_millis()));
        params.insert("ttl".to_string(), json!(options.ttl.unwrap_or(0)));
        if has_mentions {
            params.insert(
                "mentionInfo".to_string(),
                json!(Value::Array(mentions).to_string()),
            );
        }
        if is_group {
            params.insert("grid".to_string(), json!(thread_id));
            params.insert("visibility".to_string(), json!(0));
        } else {
            params.insert("toid".to_string(), json!(thread_id));
            params.insert("imei".to_string(), json!(self.ctx.imei));
        }

        if let Some(ref q) = options.quote {
            params.insert("qmsgOwner".to_string(), json!(q.uid_from));
            params.insert("qmsgId".to_string(), json!(q.msg_id));
            params.insert("qmsgCliId".to_string(), json!(q.cli_msg_id));
            params.insert("qmsgType".to_string(), json!(client_message_type(&q.msg_type)));
            params.insert("qmsgTs".to_string(), q.ts.clone());
            params.insert("qmsg".to_string(), json!(prepare_qmsg(q)));
            if is_group {
                params.insert(
                    "qmsgAttach".to_string(),
                    json!(prepare_qmsg_attach(q).to_string()),
                );
            }
            if let Some(qttl) = q.ttl {
                params.insert("qmsgTTL".to_string(), json!(qttl));
            }
        }
        if let Some(u) = options.urgency {
            if u == 1 || u == 2 {
                params.insert("metaData".to_string(), json!({ "urgency": u }));
            }
        }

        let base = if is_group {
            format!("{}/api/group", self.require_service("group")?)
        } else {
            format!("{}/api/message", self.require_service("chat")?)
        };
        let suffix = if options.quote.is_some() {
            "quote"
        } else if is_group {
            if has_mentions { "mention" } else { "sendmsg" }
        } else {
            "sms"
        };
        let url = format!("{base}/{suffix}");
        self.http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[("nretry", "0")])
            .await
    }

    async fn send_attachments_part(
        &self,
        options: &SendMessageOptions,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Vec<Value>> {
        let is_group = thread_type.is_group();
        let file_base = self.require_service("file")?;
        let att_base = format!(
            "{file_base}/api/{}/",
            if is_group { "group" } else { "message" }
        );

        let mut non_gifs: Vec<UploadSource> = Vec::new();
        let mut gifs: Vec<UploadSource> = Vec::new();
        for att in &options.attachments {
            if file_extension(&att.filename) == "gif" {
                gifs.push(att.clone());
            } else {
                non_gifs.push(att.clone());
            }
        }

        let uploaded = if non_gifs.is_empty() {
            Vec::new()
        } else {
            self.upload_attachment_with_progress(
                &non_gifs,
                thread_id,
                thread_type,
                options.progress.clone(),
            )
            .await?
        };

        let is_single = options.attachments.len() == 1;
        let first_ext = file_extension(&options.attachments[0].filename);
        let can_be_desc =
            is_single && matches!(first_ext.as_str(), "jpg" | "jpeg" | "png" | "webp");
        let mentions = build_mentions(thread_type, &options.msg, &options.mentions)?;
        let is_mentions_valid = !mentions.is_empty() && is_group && non_gifs.len() == 1;
        let is_multi = non_gifs.len() > 1;
        let group_layout_id = Utc::now().timestamp_millis().to_string();
        let mut client_id = Utc::now().timestamp_millis();
        let mut idx_in_group = 0usize;
        let mut out = Vec::new();

        for item in &uploaded {
            let (endpoint, mut params) = match item {
                UploadResult::Image {
                    width,
                    height,
                    total_size,
                    normal_url,
                    hd_url,
                    thumb_url,
                    photo_id,
                    ..
                } => {
                    let mut m = Map::new();
                    m.insert("photoId".to_string(), photo_id.clone());
                    m.insert("clientId".to_string(), json!(client_id.to_string()));
                    client_id += 1;
                    m.insert("desc".to_string(), json!(options.msg));
                    m.insert("width".to_string(), json!(width));
                    m.insert("height".to_string(), json!(height));
                    if is_group {
                        m.insert("grid".to_string(), json!(thread_id));
                        m.insert("oriUrl".to_string(), json!(normal_url));
                    } else {
                        m.insert("toid".to_string(), json!(thread_id));
                        m.insert("normalUrl".to_string(), json!(normal_url));
                    }
                    m.insert("rawUrl".to_string(), json!(normal_url));
                    m.insert("hdUrl".to_string(), json!(hd_url));
                    m.insert("thumbUrl".to_string(), json!(thumb_url));
                    m.insert("hdSize".to_string(), json!(total_size.to_string()));
                    m.insert("zsource".to_string(), json!(-1));
                    m.insert("ttl".to_string(), json!(options.ttl.unwrap_or(0)));
                    // No `jcp: {"convertible":"jxl"}` (Zalo Web sends it; it can show JPEG XL). It lets
                    // Zalo serve the photo as JPEG XL, which Android cannot decode: a high-quality JPG we
                    // sent came back as "Media unavailable" while ordinary photos showed.
                    if is_multi {
                        m.insert("groupLayoutId".to_string(), json!(group_layout_id));
                        m.insert("isGroupLayout".to_string(), json!(1));
                        m.insert("idInGroup".to_string(), json!(idx_in_group));
                        idx_in_group += 1;
                        m.insert("totalItemInGroup".to_string(), json!(uploaded.len()));
                        m.insert(
                            "extMsgProp".to_string(),
                            json!(format!(
                                "{{\"groupMediaMsg\":{{\"groupLayoutId\":\"{group_layout_id}\"}}}}"
                            )),
                        );
                    }
                    if is_mentions_valid && can_be_desc && options.quote.is_none() {
                        m.insert(
                            "mentionInfo".to_string(),
                            json!(Value::Array(mentions.clone()).to_string()),
                        );
                    }
                    ("photo_original/send", m)
                }
                UploadResult::Video {
                    file_id,
                    file_url,
                    file_name,
                    total_size,
                    checksum,
                    client_file_id,
                }
                | UploadResult::Others {
                    file_id,
                    file_url,
                    file_name,
                    total_size,
                    checksum,
                    client_file_id,
                } => {
                    let mut m = Map::new();
                    m.insert("fileId".to_string(), json!(file_id));
                    m.insert("checksum".to_string(), json!(checksum));
                    m.insert("checksumSha".to_string(), json!(""));
                    m.insert("extention".to_string(), json!(file_extension(file_name)));
                    m.insert("totalSize".to_string(), json!(total_size));
                    m.insert("fileName".to_string(), json!(file_name));
                    m.insert("clientId".to_string(), client_file_id.clone());
                    m.insert("fType".to_string(), json!(1));
                    m.insert("fileCount".to_string(), json!(0));
                    m.insert("fdata".to_string(), json!("{}"));
                    if is_group {
                        m.insert("grid".to_string(), json!(thread_id));
                    } else {
                        m.insert("toid".to_string(), json!(thread_id));
                    }
                    m.insert("fileUrl".to_string(), json!(file_url));
                    m.insert("zsource".to_string(), json!(-1));
                    m.insert("ttl".to_string(), json!(options.ttl.unwrap_or(0)));
                    ("asyncfile/msg", m)
                }
            };

            if let Some(u) = options.urgency {
                if u == 1 || u == 2 {
                    params.insert("metaData".to_string(), json!({ "urgency": u }));
                }
            }
            let url = format!("{att_base}{endpoint}");
            let res = self
                .http
                .post_encrypted(&self.ctx, &url, &Value::Object(params), &[("nretry", "0")])
                .await?;
            out.push(res);
        }

        // Handle GIFs (upthumb + gif multipart)
        for gif in gifs {
            let upthumb_params = json!({
                "clientId": Utc::now().timestamp_millis(),
                "imei": self.ctx.imei,
            });
            let enc_up = encode_aes(&self.ctx.secret_key, &upthumb_params.to_string())?;
            let upthumb_url = ZaloHttp::make_ctx_url(
                &self.ctx,
                &format!("{file_base}/api/message/upthumb"),
                &[("params", &enc_up)],
                true,
            )?;
            let thumb_res = self
                .http
                .post_multipart_chunk(
                    &self.ctx,
                    &upthumb_url,
                    "fileContent",
                    "blob",
                    "image/png",
                    gif.data.clone(),
                )
                .await?;
            let thumb_url = thumb_res
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let mut m = Map::new();
            m.insert("clientId".to_string(), json!(Utc::now().timestamp_millis().to_string()));
            m.insert("fileName".to_string(), json!(gif.filename));
            m.insert("totalSize".to_string(), json!(gif.data.len()));
            m.insert("width".to_string(), json!(gif.width.unwrap_or(0)));
            m.insert("height".to_string(), json!(gif.height.unwrap_or(0)));
            m.insert("msg".to_string(), json!(options.msg));
            m.insert("type".to_string(), json!(1));
            m.insert("ttl".to_string(), json!(options.ttl.unwrap_or(0)));
            if is_group {
                m.insert("visibility".to_string(), json!(0));
                m.insert("grid".to_string(), json!(thread_id));
            } else {
                m.insert("toid".to_string(), json!(thread_id));
            }
            m.insert("thumb".to_string(), json!(thumb_url));
            m.insert("checksum".to_string(), json!(md5_hex(&gif.data)));
            m.insert("totalChunk".to_string(), json!(1));
            m.insert("chunkId".to_string(), json!(1));
            if let Some(u) = options.urgency {
                if u == 1 || u == 2 {
                    m.insert("metaData".to_string(), json!({ "urgency": u }));
                }
            }
            let enc_gif = encode_aes(&self.ctx.secret_key, &Value::Object(m).to_string())?;
            let gif_url = ZaloHttp::make_ctx_url(
                &self.ctx,
                &format!("{att_base}gif"),
                &[("nretry", "0"), ("params", &enc_gif), ("type", "1")],
                true,
            )?;
            let res = self
                .http
                .post_multipart_chunk(
                    &self.ctx,
                    &gif_url,
                    "chunkContent",
                    &gif.filename,
                    "application/octet-stream",
                    gif.data,
                )
                .await?;
            out.push(res);
        }

        Ok(out)
    }

    /// Sends a voice message (`POST /api/message/forward` or `/api/group/forward`).
    pub async fn send_voice(
        &self,
        voice_url: &str,
        ttl: Option<i64>,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Value> {
        let base = self.require_service("file")?;
        let head = self
            .http
            .request_raw(Method::HEAD, voice_url, None, None)
            .await?;
        let file_size: u64 = if head.status().is_success() {
            head.headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        } else {
            0
        };

        let client_id = Utc::now().timestamp_millis().to_string();
        let msg_info = json!({
            "voiceUrl": voice_url,
            "m4aUrl": voice_url,
            "fileSize": file_size,
        })
        .to_string();

        let (url, params) = match thread_type {
            ThreadType::User => (
                format!("{base}/api/message/forward"),
                json!({
                    "toId": thread_id,
                    "ttl": ttl.unwrap_or(0),
                    "zsource": -1,
                    "msgType": 3,
                    "clientId": client_id,
                    "msgInfo": msg_info,
                    "imei": self.ctx.imei,
                }),
            ),
            ThreadType::Group => (
                format!("{base}/api/group/forward"),
                json!({
                    "grid": thread_id,
                    "visibility": 0,
                    "ttl": ttl.unwrap_or(0),
                    "zsource": -1,
                    "msgType": 3,
                    "clientId": client_id,
                    "msgInfo": msg_info,
                    "imei": self.ctx.imei,
                }),
            ),
        };
        self.http.post_encrypted(&self.ctx, &url, &params, &[]).await
    }

    /// Sends a sticker (`POST /api/message/sticker` or `/api/group/sticker`).
    pub async fn send_sticker(
        &self,
        sticker: &StickerParam,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Value> {
        if thread_id.is_empty() {
            return Err(ProtoError::InvalidArg("Missing threadId".into()));
        }
        if sticker.id == 0 {
            return Err(ProtoError::InvalidArg("Missing sticker id".into()));
        }
        if sticker.sticker_type == 0 {
            return Err(ProtoError::InvalidArg("Missing sticker type".into()));
        }
        let is_group = thread_type.is_group();
        let url = if is_group {
            format!("{}/api/group/sticker", self.require_service("group")?)
        } else {
            format!("{}/api/message/sticker", self.require_service("chat")?)
        };
        let mut params = Map::new();
        params.insert("stickerId".to_string(), json!(sticker.id));
        params.insert("cateId".to_string(), json!(sticker.cate_id));
        params.insert("type".to_string(), json!(sticker.sticker_type));
        params.insert("clientId".to_string(), json!(Utc::now().timestamp_millis()));
        params.insert("imei".to_string(), json!(self.ctx.imei));
        params.insert("zsource".to_string(), json!(101));
        if is_group {
            params.insert("grid".to_string(), json!(thread_id));
        } else {
            params.insert("toid".to_string(), json!(thread_id));
        }
        self.http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[("nretry", "0")])
            .await
    }

    /// Searches stickers by keyword, returning sticker IDs (`GET /api/message/sticker/suggest/stickers`).
    pub async fn get_stickers(&self, keyword: &str) -> Result<Vec<i64>> {
        if keyword.is_empty() {
            return Err(ProtoError::InvalidArg("Missing keyword".into()));
        }
        let base = self.require_service("sticker")?;
        let params = json!({
            "keyword": keyword,
            "gif": 1,
            "guggy": 0,
            "imei": self.ctx.imei,
        });
        let data = self
            .http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/message/sticker/suggest/stickers"),
                &params,
                &[],
            )
            .await?;
        let ids = data
            .get("sugg_sticker")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("sticker_id").and_then(|v| v.as_i64()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(ids)
    }

    /// Fetches sticker metadata for a list of sticker IDs (`GET /api/message/sticker/sticker_detail`).
    pub async fn get_stickers_detail(&self, sticker_ids: &[i64]) -> Result<Vec<Value>> {
        if sticker_ids.is_empty() {
            return Err(ProtoError::InvalidArg("Missing sticker id".into()));
        }
        let base = self.require_service("sticker")?;
        let url = format!("{base}/api/message/sticker/sticker_detail");
        let futs = sticker_ids.iter().map(|&sid| {
            let url = url.clone();
            async move {
                let params = json!({ "sid": sid });
                self.http.get_encrypted(&self.ctx, &url, &params, &[]).await
            }
        });
        let results = join_all(futs).await;
        Ok(results.into_iter().filter_map(|r| r.ok()).collect())
    }

    /// Adds or removes a reaction on a message (`POST /api/message/reaction` or `/api/group/reaction`).
    pub async fn add_reaction(&self, icon: &str, target: &ReactionTarget) -> Result<Value> {
        let base = self.require_service("reaction")?;
        let r_type = reaction_type_id(icon);
        let g_msg_id: Value = target
            .msg_id
            .parse::<i64>()
            .map(|n| json!(n))
            .unwrap_or_else(|_| json!(target.msg_id));
        let c_msg_id: Value = target
            .cli_msg_id
            .parse::<i64>()
            .map(|n| json!(n))
            .unwrap_or_else(|_| json!(target.cli_msg_id));

        let msg_json = json!({
            "rMsg": [{
                "gMsgID": g_msg_id,
                "cMsgID": c_msg_id,
                "msgType": 1,
            }],
            "rIcon": icon,
            "rType": r_type,
            "source": 6,
        })
        .to_string();

        let mut params = Map::new();
        params.insert(
            "react_list".to_string(),
            json!([{
                "message": msg_json,
                "clientId": Utc::now().timestamp_millis(),
            }]),
        );

        let url = match target.thread_type {
            ThreadType::User => {
                params.insert("toid".to_string(), json!(target.thread_id));
                format!("{base}/api/message/reaction")
            }
            ThreadType::Group => {
                params.insert("grid".to_string(), json!(target.thread_id));
                params.insert("imei".to_string(), json!(self.ctx.imei));
                format!("{base}/api/group/reaction")
            }
        };

        let data = self
            .http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[])
            .await?;
        if let Some(msg_ids_str) = data.get("msgIds").and_then(|v| v.as_str()) {
            if let Ok(parsed) = serde_json::from_str::<Value>(msg_ids_str) {
                return Ok(json!({ "msgIds": parsed }));
            }
        }
        Ok(data)
    }

    /// Recalls/undoes a sent message (`POST /api/message/undo` or `/api/group/undomsg`).
    pub async fn undo(
        &self,
        target: &UndoTarget,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Value> {
        let mut params = Map::new();
        params.insert("msgId".to_string(), json!(target.msg_id));
        params.insert("clientId".to_string(), json!(Utc::now().timestamp_millis()));
        params.insert("cliMsgIdUndo".to_string(), json!(target.cli_msg_id));

        let url = match thread_type {
            ThreadType::User => {
                params.insert("toid".to_string(), json!(thread_id));
                format!("{}/api/message/undo", self.require_service("chat")?)
            }
            ThreadType::Group => {
                params.insert("grid".to_string(), json!(thread_id));
                params.insert("visibility".to_string(), json!(0));
                params.insert("imei".to_string(), json!(self.ctx.imei));
                format!("{}/api/group/undomsg", self.require_service("group")?)
            }
        };
        self.http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[])
            .await
    }

    /// Deletes a message (`POST /api/message/delete` or `/api/group/deletemsg`).
    pub async fn delete_message(&self, target: &DeleteMessageTarget, only_me: bool) -> Result<Value> {
        let is_group = target.thread_type.is_group();
        let is_self = self.ctx.uid == target.uid_from;
        if is_self && !only_me {
            return Err(ProtoError::InvalidArg(
                "To delete your message for everyone, use undo api instead".into(),
            ));
        }
        if !is_group && !only_me {
            return Err(ProtoError::InvalidArg(
                "Can't delete message for everyone in a private chat".into(),
            ));
        }
        let mut params = Map::new();
        if is_group {
            params.insert("grid".to_string(), json!(target.thread_id));
        } else {
            params.insert("toid".to_string(), json!(target.thread_id));
            params.insert("imei".to_string(), json!(self.ctx.imei));
        }
        params.insert("cliMsgId".to_string(), json!(Utc::now().timestamp_millis()));
        params.insert(
            "msgs".to_string(),
            json!([{
                "cliMsgId": target.cli_msg_id,
                "globalMsgId": target.msg_id,
                "ownerId": target.uid_from,
                "destId": target.thread_id,
            }]),
        );
        params.insert("onlyMe".to_string(), json!(if only_me { 1 } else { 0 }));

        let url = if is_group {
            format!("{}/api/group/deletemsg", self.require_service("group")?)
        } else {
            format!("{}/api/message/delete", self.require_service("chat")?)
        };
        self.http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[])
            .await
    }

    /// Forwards a message to one or more threads (`POST /api/message/mforward` or `/api/group/mforward`).
    pub async fn forward_message(
        &self,
        payload: &ForwardMessagePayload,
        thread_ids: &[String],
        thread_type: ThreadType,
    ) -> Result<Value> {
        if payload.message.is_empty() {
            return Err(ProtoError::InvalidArg("Missing message content".into()));
        }
        if thread_ids.is_empty() {
            return Err(ProtoError::InvalidArg("Missing thread IDs".into()));
        }
        let base = self.require_service("file")?;
        let client_id = Utc::now().timestamp_millis().to_string();
        let ttl = payload.ttl.unwrap_or(0);

        let mut msg_info = Map::new();
        msg_info.insert("message".to_string(), json!(payload.message));
        if let Some(ref r) = payload.reference {
            let ref_data = serde_json::to_string(r)?;
            msg_info.insert(
                "reference".to_string(),
                json!(json!({ "type": 3, "data": ref_data }).to_string()),
            );
        }

        let decor_log = match &payload.reference {
            Some(r) => json!({
                "fw": {
                    "pmsg": { "st": 1, "ts": r.ts, "id": r.id },
                    "rmsg": { "st": 1, "ts": r.ts, "id": r.id },
                    "fwLvl": r.fw_lvl,
                }
            }),
            None => Value::Null,
        };

        let (url, params) = match thread_type {
            ThreadType::User => {
                let to_ids: Vec<Value> = thread_ids
                    .iter()
                    .map(|tid| {
                        json!({
                            "clientId": client_id,
                            "toUid": tid,
                            "ttl": ttl,
                        })
                    })
                    .collect();
                (
                    format!("{base}/api/message/mforward"),
                    json!({
                        "toIds": to_ids,
                        "imei": self.ctx.imei,
                        "ttl": ttl,
                        "msgType": "1",
                        "totalIds": thread_ids.len(),
                        "msgInfo": Value::Object(msg_info).to_string(),
                        "decorLog": decor_log.to_string(),
                    }),
                )
            }
            ThreadType::Group => {
                let grids: Vec<Value> = thread_ids
                    .iter()
                    .map(|tid| {
                        json!({
                            "clientId": client_id,
                            "grid": tid,
                            "ttl": ttl,
                        })
                    })
                    .collect();
                (
                    format!("{base}/api/group/mforward"),
                    json!({
                        "grids": grids,
                        "ttl": ttl,
                        "msgType": "1",
                        "totalIds": thread_ids.len(),
                        "msgInfo": Value::Object(msg_info).to_string(),
                        "decorLog": decor_log.to_string(),
                    }),
                )
            }
        };
        self.http.post_encrypted(&self.ctx, &url, &params, &[]).await
    }

    /// Parses link preview metadata (`GET /api/message/parselink` on `file` service).
    pub async fn parse_link(&self, link: &str) -> Result<Value> {
        if link.is_empty() {
            return Err(ProtoError::InvalidArg("Missing link".into()));
        }
        let base = self.require_service("file")?;
        let params = json!({
            "link": link,
            "version": 1,
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/message/parselink"),
                &params,
                &[],
            )
            .await
    }

    /// Sends a contact card (`POST /api/message/forward` or `/api/group/forward` on `file` service).
    pub async fn send_card(
        &self,
        user_id: &str,
        phone_number: Option<&str>,
        thread_id: &str,
        thread_type: ThreadType,
        ttl: Option<u64>,
    ) -> Result<Value> {
        if user_id.is_empty() {
            return Err(ProtoError::InvalidArg("Missing userId".into()));
        }
        if thread_id.is_empty() {
            return Err(ProtoError::InvalidArg("Missing threadId".into()));
        }
        let qr_data = self.get_qr(&[user_id.to_string()]).await?;
        let base = self.require_service("file")?;
        let client_id = Utc::now().timestamp_millis().to_string();

        let mut msg_info = Map::new();
        msg_info.insert("contactUid".to_string(), json!(user_id));
        if let Some(qr_url) = qr_data.get(user_id).filter(|v| !v.is_null()) {
            msg_info.insert("qrCodeUrl".to_string(), qr_url.clone());
        }
        if let Some(phone) = phone_number.filter(|p| !p.is_empty()) {
            msg_info.insert("phone".to_string(), json!(phone));
        }

        let mut params = Map::new();
        params.insert("ttl".to_string(), json!(ttl.unwrap_or(0)));
        params.insert("msgType".to_string(), json!(6));
        params.insert("clientId".to_string(), json!(client_id));
        params.insert("msgInfo".to_string(), json!(Value::Object(msg_info).to_string()));

        let url = match thread_type {
            ThreadType::Group => {
                params.insert("visibility".to_string(), json!(0));
                params.insert("grid".to_string(), json!(thread_id));
                format!("{base}/api/group/forward")
            }
            ThreadType::User => {
                params.insert("toId".to_string(), json!(thread_id));
                params.insert("imei".to_string(), json!(self.ctx.imei));
                format!("{base}/api/message/forward")
            }
        };
        self.http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[])
            .await
    }

    /// Sends a rich link preview message (`POST /api/message/link` or `/api/group/sendlink`).
    pub async fn send_link(
        &self,
        link: &str,
        msg: Option<&str>,
        thread_id: &str,
        thread_type: ThreadType,
        ttl: Option<u64>,
    ) -> Result<Value> {
        if link.is_empty() {
            return Err(ProtoError::InvalidArg("Missing link".into()));
        }
        if thread_id.is_empty() {
            return Err(ProtoError::InvalidArg("Missing threadId".into()));
        }
        let parsed = self.parse_link(link).await?;
        let meta = parsed.get("data").unwrap_or(&parsed);

        let full_msg = match msg.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) if m.contains(link) => m.to_string(),
            Some(m) => format!("{m} {link}"),
            None => link.to_string(),
        };

        let media_str = meta
            .get("media")
            .map(|m| m.to_string())
            .unwrap_or_else(|| "{}".to_string());

        let mut params = Map::new();
        params.insert("msg".to_string(), json!(full_msg));
        params.insert(
            "href".to_string(),
            meta.get("href").cloned().unwrap_or_else(|| json!(link)),
        );
        params.insert(
            "src".to_string(),
            meta.get("src").cloned().unwrap_or_else(|| json!("")),
        );
        params.insert(
            "title".to_string(),
            meta.get("title").cloned().unwrap_or_else(|| json!("")),
        );
        params.insert(
            "desc".to_string(),
            meta.get("desc").cloned().unwrap_or_else(|| json!("")),
        );
        params.insert(
            "thumb".to_string(),
            meta.get("thumb").cloned().unwrap_or_else(|| json!("")),
        );
        params.insert("type".to_string(), json!(2));
        params.insert("media".to_string(), json!(media_str));
        params.insert("ttl".to_string(), json!(ttl.unwrap_or(0)));
        params.insert("clientId".to_string(), json!(Utc::now().timestamp_millis()));

        let url = match thread_type {
            ThreadType::Group => {
                params.insert("grid".to_string(), json!(thread_id));
                params.insert("imei".to_string(), json!(self.ctx.imei));
                format!("{}/api/group/sendlink", self.require_service("group")?)
            }
            ThreadType::User => {
                params.insert("toId".to_string(), json!(thread_id));
                params.insert("mentionInfo".to_string(), json!(""));
                format!("{}/api/message/link", self.require_service("chat")?)
            }
        };

        self.http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[("nretry", "0")])
            .await
    }

    /// Sends a typing indicator (`POST /api/message/typing` or `/api/group/typing`).
    pub async fn send_typing_event(
        &self,
        thread_id: &str,
        thread_type: ThreadType,
        dest_type: Option<DestType>,
    ) -> Result<Value> {
        if thread_id.is_empty() {
            return Err(ProtoError::InvalidArg("Missing threadId".into()));
        }
        let (url, params) = match thread_type {
            ThreadType::User => (
                format!("{}/api/message/typing", self.require_service("chat")?),
                json!({
                    "toid": thread_id,
                    "destType": dest_type.unwrap_or(DestType::User) as u8,
                    "imei": self.ctx.imei,
                }),
            ),
            ThreadType::Group => (
                format!("{}/api/group/typing", self.require_service("group")?),
                json!({
                    "grid": thread_id,
                    "imei": self.ctx.imei,
                }),
            ),
        };
        self.http.post_encrypted(&self.ctx, &url, &params, &[]).await
    }

    /// Sends read/seen receipts for 1..=50 messages of the same thread (`POST /api/message/seenv2` or `/api/group/seenv2`).
    pub async fn send_seen_event(
        &self,
        messages: &[SeenMessageInput],
        thread_type: ThreadType,
    ) -> Result<Value> {
        if messages.is_empty() || messages.len() > MAX_MESSAGES_PER_SEEN {
            return Err(ProtoError::InvalidArg(
                "messages must contain between 1 and 50 messages.".into(),
            ));
        }
        let is_group = thread_type.is_group();
        let thread_id = if is_group {
            &messages[0].id_to
        } else {
            &messages[0].uid_from
        };
        let mut items = Vec::with_capacity(messages.len());
        for m in messages {
            let cur_tid = if is_group { &m.id_to } else { &m.uid_from };
            if cur_tid != thread_id {
                return Err(ProtoError::InvalidArg(
                    "All messages must belong to the same thread.".into(),
                ));
            }
            let di = if m.id_to == self.ctx.uid {
                "0"
            } else {
                m.id_to.as_str()
            };
            items.push(json!({
                "cmi": m.cli_msg_id,
                "gmi": m.msg_id,
                "si": m.uid_from,
                "di": di,
                "mt": m.msg_type,
                "st": if m.st.is_some() { 0 } else { -1 },
                "at": if m.at.is_some() { 0 } else { -1 },
                "cmd": if m.cmd.is_some() { 0 } else { -1 },
                "ts": if m.ts.is_some() { 0 } else { -1 },
            }));
        }

        let mut msg_infos = Map::new();
        msg_infos.insert("data".to_string(), Value::Array(items));
        if is_group {
            msg_infos.insert("grid".to_string(), json!(thread_id));
        } else {
            msg_infos.insert("senderId".to_string(), json!(thread_id));
        }

        let mut params = Map::new();
        params.insert("msgInfos".to_string(), json!(Value::Object(msg_infos).to_string()));
        if is_group {
            params.insert("imei".to_string(), json!(self.ctx.imei));
        }

        let url = if is_group {
            format!("{}/api/group/seenv2", self.require_service("group")?)
        } else {
            format!("{}/api/message/seenv2", self.require_service("chat")?)
        };
        self.http
            .post_encrypted(&self.ctx, &url, &Value::Object(params), &[("nretry", "0")])
            .await
    }

    /// Fetches recent group chat history (`GET /api/group/history`).
    pub async fn get_group_chat_history(&self, group_id: &str, count: Option<u32>) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grid": group_id,
            "count": count.unwrap_or(50),
        });
        let raw = self
            .http
            .get_encrypted(&self.ctx, &format!("{base}/api/group/history"), &params, &[])
            .await?;
        let mut data = match raw {
            Value::String(s) => serde_json::from_str::<Value>(&s)?,
            other => other,
        };
        let own_id = self.ctx.uid.clone();
        if let Some(arr) = data.get_mut("groupMsgs").and_then(|v| v.as_array_mut()) {
            for item in arr.iter_mut() {
                if let Some(obj) = item.as_object_mut() {
                    let raw_from = obj.get("uidFrom").map(val_to_str).unwrap_or_default();
                    let is_self = raw_from == "0";
                    let thread_id = obj.get("idTo").map(val_to_str).unwrap_or_default();
                    if raw_from == "0" {
                        obj.insert("uidFrom".to_string(), Value::String(own_id.clone()));
                    }
                    if let Some(q) = obj.get_mut("quote").and_then(|v| v.as_object_mut()) {
                        if let Some(owner) = q.get("ownerId") {
                            q.insert("ownerId".to_string(), Value::String(val_to_str(owner)));
                        }
                    }
                    let wrapped = json!({
                        "type": ThreadType::Group as u8,
                        "threadId": thread_id,
                        "isSelf": is_self,
                        "data": Value::Object(obj.clone()),
                    });
                    *item = wrapped;
                }
            }
        }
        Ok(data)
    }
}

fn val_to_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn collect() -> (UploadProgressSink, Arc<Mutex<Vec<UploadProgress>>>) {
        let out: Arc<Mutex<Vec<UploadProgress>>> = Default::default();
        let o = out.clone();
        (UploadProgressSink::new(move |p| o.lock().unwrap().push(p)), out)
    }

    #[test]
    fn tracker_reports_start_monotonic_progress_and_done_on_success() {
        let (sink, out) = collect();
        let t = UploadTracker::start(Some(sink), 2, "a.png", 10);
        t.advance(4);
        t.advance(4);
        t.advance(100); // clamped
        t.advance(1); // no change, no report
        t.finish(true);
        t.finish(true); // idempotent
        drop(t);
        let ev = out.lock().unwrap().clone();
        let sent: Vec<u64> = ev.iter().map(|e| e.sent).collect();
        assert_eq!(sent, vec![0, 4, 8, 10, 10]);
        assert!(ev.iter().all(|e| e.total == 10 && e.index == 2 && e.filename == "a.png"));
        assert_eq!(ev.iter().filter(|e| e.done).count(), 1);
        assert!(ev.last().unwrap().done);
    }

    #[test]
    fn tracker_emits_done_with_partial_sent_when_dropped_after_failure() {
        let (sink, out) = collect();
        let t = UploadTracker::start(Some(sink), 0, "f.bin", 100);
        t.advance(30);
        drop(t);
        let ev = out.lock().unwrap().clone();
        let last = ev.last().unwrap();
        assert!(last.done);
        assert_eq!((last.sent, last.total), (30, 100));
        assert_eq!(ev.iter().filter(|e| e.done).count(), 1);
    }

    #[test]
    fn tracker_without_sink_is_silent() {
        let t = UploadTracker::start(None, 0, "x", 1);
        t.advance(1);
        t.finish(true);
    }

    #[test]
    fn reaction_type_ids_match_zca_js() {
        assert_eq!(reaction_type_id("/-heart"), 5);
        assert_eq!(reaction_type_id("/-strong"), 3);
        assert_eq!(reaction_type_id(":>"), 0);
        assert_eq!(reaction_type_id(":o"), 32);
        assert_eq!(reaction_type_id(":-(("), 2);
        assert_eq!(reaction_type_id(":-h"), 20);
        assert_eq!(reaction_type_id(":-*"), 8);
        assert_eq!(reaction_type_id(":')"), 7);
        assert_eq!(reaction_type_id("/-weak"), 4);
        assert_eq!(reaction_type_id("/-rose"), 120);
        assert_eq!(reaction_type_id("/-break"), 65);
        assert_eq!(reaction_type_id("/-ok"), 68);
        assert_eq!(reaction_type_id("_()_"), 73);
        assert_eq!(reaction_type_id("/-thanks"), 70);
        assert_eq!(reaction_type_id(":handclap"), 46);
        assert_eq!(reaction_type_id(""), -1);
    }
}
