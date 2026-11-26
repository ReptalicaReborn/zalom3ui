//! Track 2 (`feat/rust-history-sync`): phone history-sync state machine & REST endpoints.
//!
//! Direct Rust port of `src/historysync.ts` and the history-sync routes in `src/server.ts` /
//! `src/session.ts`. Coordinates with the official Zalo mobile app to request, download,
//! decrypt (RSA-2048 + ZDB4.0 AES-256-CBC + LZMA/XZ), resolve plain→noised IDs (`/api/znoise`),
//! and import historical SQLite conversations into [`ChatStore`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, TimeZone, Utc};
use futures_util::future::BoxFuture;
use serde_json::{json, Map, Value};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::warn;
use url::Url;

use zalo_media::sync::{
    backup_key_text, conversation_files, convert_conversation, decrypt_backup_key,
    describe_senders, generate_sync_keypair, read_conversation, unpack_backup, BackupConversation,
    BackupError, BackupIdMap, SyncKeyPair, DEFAULT_MAX_OUTPUT,
};
use zalo_proto::{parse_json_keeping_bigints, NoisedIdsReply, ProtoError};

use crate::errors::HttpError;
use crate::routes::AppState;
use crate::session::ZaloSession;
use crate::store::{normalize_message, ChatStore, HISTORY_MESSAGES_PER_THREAD};
use crate::types::{HistorySyncCounts, HistorySyncStatus, SyncState, ThreadKind};

/// Control `act_type` values belonging to mobile history sync.
pub const SYNC_ACT_TYPES: &[&str] = &[
    "syncmsgmb",
    "transfer_msg",
    "transfer_sync2",
    "transfer_after_login",
];

const MAX_DOWNLOAD_ATTEMPTS: usize = 5;
const RETRY_BASE_MS: u64 = 1_000;
const RETRY_MAX_MS: u64 = 8_000;
const RETRY_AFTER_MAX_MS: u64 = 30_000;

const CONFIRM_TIMEOUT_MS: u64 = 3 * 60_000;
const BACKUP_TIMEOUT_MS: u64 = 30 * 60_000;
const DOWNLOAD_STALL_MS: u64 = 2 * 60_000;
const NOISE_BATCH: usize = 100;
/// Zalo's answer when the phone uses sync v2 (iPhone), which isn't supported here.
const CODE_CLIENT_NOT_SUPPORTED: i64 = 211;
const LOG_LINES: usize = 200;
const DEFAULT_PC_NAME: &str = "Zalo-Android";

// ---- Control parsing & pure helpers ----

#[derive(Debug, Clone, PartialEq)]
pub struct SyncControl {
    pub act_type: String,
    pub act: String,
    pub data: Map<String, Value>,
}

/// Parses a history-sync control from either:
/// - `ZaloListener`'s emitted `{ actType, act, data }` object,
/// - a raw `{ act_type, act, data }` object, or
/// - a wrapped `{ content: { act_type, act, data } }` control entry.
pub fn sync_control_of(value: &Value) -> Option<SyncControl> {
    let obj = value.as_object()?;
    let c = match obj.get("content").and_then(|v| v.as_object()) {
        Some(inner) if inner.contains_key("act_type") || inner.contains_key("actType") => inner,
        _ => obj,
    };
    let act_type = c
        .get("act_type")
        .or_else(|| c.get("actType"))
        .and_then(|v| v.as_str())?;
    if !SYNC_ACT_TYPES.contains(&act_type) {
        return None;
    }
    let act = c
        .get("act")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let data = parse_json_object(c.get("data"));
    Some(SyncControl {
        act_type: act_type.to_string(),
        act,
        data,
    })
}

/// Extracts all history-sync controls from a slice of control entries.
pub fn sync_controls_in(controls: &[Value]) -> Vec<SyncControl> {
    controls.iter().filter_map(sync_control_of).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferError {
    PhoneIdle,
    PhoneActive,
    Failed,
    Ignore,
}

/// Classifies the phone's `transfer_error` event (`status`: 2=idle, 1=active; else `error_code`).
pub fn classify_transfer_error(data: &Map<String, Value>) -> TransferError {
    let status = val_i64(data.get("status"), -1);
    if status == 2 {
        return TransferError::PhoneIdle;
    }
    if status == 1 {
        return TransferError::PhoneActive;
    }
    if val_i64(data.get("error_code"), 0) != 0 {
        TransferError::Failed
    } else {
        TransferError::Ignore
    }
}

/// Computes the retry delay in milliseconds before the next backup download attempt,
/// or `None` to give up. Honours `Retry-After` (delta-seconds or HTTP-date) for 429/503,
/// capped at 30 seconds.
pub fn download_retry_delay_ms(
    status: u16,
    attempt: usize,
    retry_after: Option<&str>,
    now_ms: i64,
) -> Option<u64> {
    if attempt < 1 || attempt >= MAX_DOWNLOAD_ATTEMPTS {
        return None;
    }
    let retryable = matches!(status, 0 | 400 | 404 | 408 | 429) || status >= 500;
    if !retryable {
        return None;
    }
    let shift = (attempt - 1).min(10) as u32;
    let backoff = RETRY_BASE_MS.saturating_mul(1u64 << shift).min(RETRY_MAX_MS);
    let hint = retry_after.unwrap_or("").trim();
    if (status != 429 && status != 503) || hint.is_empty() {
        return Some(backoff);
    }

    let wait_ms: Option<i64> = if hint.bytes().all(|b| b.is_ascii_digit()) {
        hint.parse::<u64>().ok().map(|secs| {
            let capped = secs.min(RETRY_AFTER_MAX_MS / 1000);
            (capped * 1000) as i64
        })
    } else {
        DateTime::parse_from_rfc2822(hint)
            .or_else(|_| DateTime::parse_from_rfc3339(hint))
            .ok()
            .map(|dt| dt.timestamp_millis() - now_ms)
    };

    match wait_ms {
        Some(w) if w >= 0 => Some((w as u64).max(backoff).min(RETRY_AFTER_MAX_MS)),
        _ => Some(backoff),
    }
}

/// Short single-line excerpt of an HTTP error body for the sync diagnostic log.
pub fn answer_excerpt(text: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in text.chars() {
        let printable = (ch >= '\x20' && ch <= '\x7e') || ch >= '\u{00a0}';
        if printable {
            out.push(ch);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
        if out.chars().count() >= 200 {
            break;
        }
    }
    out.trim().chars().take(200).collect()
}

fn parse_json_object(value: Option<&Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(m)) => m.clone(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.starts_with('{') {
                if let Ok(Value::Object(m)) = parse_json_keeping_bigints(trimmed) {
                    return m;
                }
            }
            Map::new()
        }
        _ => Map::new(),
    }
}

fn val_i64(v: Option<&Value>, default: i64) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| u as i64))
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
            .unwrap_or(default),
        Some(Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(|f| f as i64)
            .unwrap_or(default),
        _ => default,
    }
}

fn val_u64(v: Option<&Value>) -> u64 {
    let n = val_i64(v, 0);
    if n > 0 {
        n as u64
    } else {
        0
    }
}

fn host_of(raw_url: &str) -> String {
    Url::parse(raw_url)
        .ok()
        .and_then(|u| {
            let host = u.host_str()?.to_string();
            match u.port() {
                Some(p) => Some(format!("{host}:{p}")),
                None => Some(host),
            }
        })
        .unwrap_or_default()
}

fn param_names_of(raw_url: &str) -> String {
    Url::parse(raw_url)
        .ok()
        .map(|u| {
            u.query_pairs()
                .map(|(k, _)| k.into_owned())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}

fn sorted_keys(map: &Map<String, Value>) -> String {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys.join(",")
}

fn is_https_url(raw: &str) -> bool {
    raw.len() >= 8 && raw[..8].eq_ignore_ascii_case("https://")
}

fn is_active_state(state: SyncState) -> bool {
    matches!(
        state,
        SyncState::WaitingForPhone | SyncState::Downloading | SyncState::Importing
    )
}

// ---- Dependencies abstraction (live ZaloSession + mockable in unit tests) ----

#[derive(Debug, Clone)]
pub struct FileCallError {
    pub code: Option<i64>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct DownloadError {
    pub status: u16,
    pub retry_after: Option<String>,
    pub answer: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub struct SyncTimeouts {
    pub confirm_ms: u64,
    pub backup_ms: u64,
    pub download_stall_ms: u64,
}

impl Default for SyncTimeouts {
    fn default() -> Self {
        Self {
            confirm_ms: CONFIRM_TIMEOUT_MS,
            backup_ms: BACKUP_TIMEOUT_MS,
            download_stall_ms: DOWNLOAD_STALL_MS,
        }
    }
}

pub type ProgressCallback = Arc<dyn Fn(usize, usize) + Send + Sync>;

pub trait HistorySyncDeps: Send + Sync + 'static {
    fn own_id(&self) -> Option<String>;
    fn imei(&self) -> String;
    fn pc_name(&self) -> &str {
        DEFAULT_PC_NAME
    }
    fn store(&self) -> &ChatStore;

    fn call_file_service(
        &self,
        endpoint: String,
        params: Value,
    ) -> BoxFuture<'static, Result<Value, FileCallError>>;

    fn download_backup(
        &self,
        url: String,
        on_progress: ProgressCallback,
    ) -> BoxFuture<'static, Result<Vec<u8>, DownloadError>>;

    fn has_znoise(&self) -> bool {
        true
    }

    fn resolve_noised_ids(
        &self,
        user_ids: Vec<String>,
        group_ids: Vec<String>,
    ) -> BoxFuture<'static, Result<NoisedIdsReply, String>>;

    fn name_of(&self, user_id: &str) -> String;

    fn prepare(&self, _conversation_id: String, _is_group: bool) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }

    fn on_status_change(&self, _status: &HistorySyncStatus) {}

    fn generate_keypair(&self) -> BoxFuture<'static, Result<SyncKeyPair, BackupError>> {
        Box::pin(async {
            tokio::task::spawn_blocking(generate_sync_keypair)
                .await
                .map_err(|e| BackupError(format!("RSA keygen task failed: {e}")))?
        })
    }

    fn sleep(&self, ms: u64) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        })
    }

    fn timeouts(&self) -> SyncTimeouts {
        SyncTimeouts::default()
    }

    fn now_ms(&self) -> i64 {
        Utc::now().timestamp_millis()
    }
}

/// Live production implementation backed by [`ZaloSession`] and [`ChatStore`].
struct SessionSyncDeps {
    session: Arc<ZaloSession>,
    store: ChatStore,
    member_names: Arc<RwLock<HashMap<String, String>>>,
}

impl SessionSyncDeps {
    fn new(state: AppState) -> Self {
        Self {
            session: state.session,
            store: state.store,
            member_names: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl HistorySyncDeps for SessionSyncDeps {
    fn own_id(&self) -> Option<String> {
        self.session.own_id()
    }

    fn imei(&self) -> String {
        self.session
            .client()
            .map(|c| c.ctx().imei.clone())
            .unwrap_or_default()
    }

    fn store(&self) -> &ChatStore {
        &self.store
    }

    fn call_file_service(
        &self,
        endpoint: String,
        params: Value,
    ) -> BoxFuture<'static, Result<Value, FileCallError>> {
        let session = Arc::clone(&self.session);
        Box::pin(async move {
            let client = session.client().map_err(|e| FileCallError {
                code: None,
                message: e.message,
            })?;
            client
                .get_file_service(&endpoint, params)
                .await
                .map_err(|err| match err {
                    ProtoError::Api { code, message } => FileCallError {
                        code: Some(code),
                        message,
                    },
                    ProtoError::HttpStatus { status, message } => FileCallError {
                        code: Some(status as i64),
                        message,
                    },
                    other => FileCallError {
                        code: None,
                        message: other.to_string(),
                    },
                })
        })
    }

    fn download_backup(
        &self,
        url: String,
        on_progress: ProgressCallback,
    ) -> BoxFuture<'static, Result<Vec<u8>, DownloadError>> {
        let session = Arc::clone(&self.session);
        Box::pin(async move {
            let client = session.client().map_err(|e| DownloadError {
                status: 0,
                retry_after: None,
                answer: None,
                message: e.message,
            })?;
            match client.download_backup(&url).await {
                Ok(bytes) => {
                    on_progress(bytes.len(), bytes.len());
                    Ok(bytes)
                }
                Err(ProtoError::HttpStatus { status, message }) => Err(DownloadError {
                    status,
                    retry_after: None,
                    answer: {
                        let ex = answer_excerpt(&message);
                        if ex.is_empty() {
                            None
                        } else {
                            Some(ex)
                        }
                    },
                    message: format!("HTTP {status}"),
                }),
                Err(other) => Err(DownloadError {
                    status: 0,
                    retry_after: None,
                    answer: None,
                    message: other.to_string(),
                }),
            }
        })
    }

    fn resolve_noised_ids(
        &self,
        user_ids: Vec<String>,
        group_ids: Vec<String>,
    ) -> BoxFuture<'static, Result<NoisedIdsReply, String>> {
        let session = Arc::clone(&self.session);
        Box::pin(async move {
            let client = session.client().map_err(|e| e.message)?;
            client
                .get_znoise(&user_ids, &group_ids)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn name_of(&self, user_id: &str) -> String {
        if let Some(name) = self.session.friend_name(user_id) {
            if !name.is_empty() && name != user_id {
                return name;
            }
        }
        if let Some(thread) = self.store.get_thread(ThreadKind::User, user_id) {
            if !thread.name.is_empty() && thread.name != user_id {
                return thread.name;
            }
        }
        if let Ok(cache) = self.member_names.read() {
            if let Some(name) = cache.get(user_id) {
                if !name.is_empty() && name != user_id {
                    return name.clone();
                }
            }
        }
        String::new()
    }

    fn prepare(&self, conversation_id: String, is_group: bool) -> BoxFuture<'static, ()> {
        if !is_group {
            return Box::pin(async {});
        }
        let session = Arc::clone(&self.session);
        let cache = Arc::clone(&self.member_names);
        Box::pin(async move {
            if let Ok(members) = session.group_members(&conversation_id).await {
                if let Ok(mut map) = cache.write() {
                    for m in members {
                        if !m.name.is_empty() && m.name != m.id {
                            map.insert(m.id, m.name);
                        }
                    }
                }
            }
        })
    }

    fn on_status_change(&self, status: &HistorySyncStatus) {
        let mut ev = serde_json::to_value(status).unwrap_or_else(|_| json!({}));
        if let Some(obj) = ev.as_object_mut() {
            obj.insert("type".to_string(), Value::String("history_sync".into()));
        }
        self.session.broadcast_event(ev);

        if status.state == SyncState::Done {
            self.session.broadcast_event(json!({ "type": "threads" }));
            let session = Arc::clone(&self.session);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if session.resolve_unnamed_threads().await {
                        session.broadcast_event(json!({ "type": "threads" }));
                    }
                });
            }
        }
    }
}

// ---- HistorySyncManager state machine ----

struct SyncInner {
    status: HistorySyncStatus,
    keys: Option<SyncKeyPair>,
    generation: u64,
    starting: bool,
    timeout_task: Option<JoinHandle<()>>,
    worker_task: Option<JoinHandle<()>>,
    temp_key: String,
    lines: VecDeque<String>,
}

impl Default for SyncInner {
    fn default() -> Self {
        Self {
            status: HistorySyncStatus::default(),
            keys: None,
            generation: 0,
            starting: false,
            timeout_task: None,
            worker_task: None,
            temp_key: String::new(),
            lines: VecDeque::new(),
        }
    }
}

/// Manages the phone history-sync lifecycle, timeouts, download retries, decryption,
/// `/api/znoise` ID translation, and store import.
pub struct HistorySyncManager {
    deps: Arc<dyn HistorySyncDeps>,
    inner: Mutex<SyncInner>,
}

impl HistorySyncManager {
    pub fn new(deps: Arc<dyn HistorySyncDeps>) -> Arc<Self> {
        Arc::new(Self {
            deps,
            inner: Mutex::new(SyncInner::default()),
        })
    }

    pub fn for_session(state: AppState) -> Arc<Self> {
        Self::new(Arc::new(SessionSyncDeps::new(state)))
    }

    pub fn get_status(&self) -> HistorySyncStatus {
        self.inner
            .lock()
            .map(|s| s.status.clone())
            .unwrap_or_default()
    }

    pub fn get_log(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|s| s.lines.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn is_active(&self) -> bool {
        self.inner
            .lock()
            .map(|s| s.starting || is_active_state(s.status.state))
            .unwrap_or(false)
    }

    fn log(&self, line: impl AsRef<str>) {
        let ts_ms = self.deps.now_ms();
        let ts = Utc
            .timestamp_millis_opt(ts_ms)
            .single()
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_else(|| ts_ms.to_string());
        let entry = format!("{ts} {}", line.as_ref());
        if let Ok(mut s) = self.inner.lock() {
            s.lines.push_back(entry);
            while s.lines.len() > LOG_LINES {
                s.lines.pop_front();
            }
        }
    }

    fn update_status<F>(&self, f: F) -> HistorySyncStatus
    where
        F: FnOnce(&mut HistorySyncStatus),
    {
        let snapshot = {
            let Ok(mut s) = self.inner.lock() else {
                return HistorySyncStatus::default();
            };
            f(&mut s.status);
            s.status.clone()
        };
        self.deps.on_status_change(&snapshot);
        snapshot
    }

    fn is_generation(&self, gen: u64) -> bool {
        self.inner
            .lock()
            .map(|s| s.generation == gen)
            .unwrap_or(false)
    }

    fn arm_timeout(self: &Arc<Self>, gen: u64, ms: u64, error_msg: &'static str) {
        let mgr = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            mgr.fail(gen, error_msg.to_string());
        });
        if let Ok(mut s) = self.inner.lock() {
            if let Some(old) = s.timeout_task.take() {
                old.abort();
            }
            if s.generation == gen {
                s.timeout_task = Some(handle);
            } else {
                handle.abort();
            }
        }
    }

    fn disarm_timeout(&self) {
        if let Ok(mut s) = self.inner.lock() {
            if let Some(t) = s.timeout_task.take() {
                t.abort();
            }
        }
    }

    /// Starts a new phone history sync. Returns `(was_already_active, status)`.
    pub async fn start(self: &Arc<Self>) -> Result<(bool, HistorySyncStatus), HttpError> {
        {
            let Ok(mut s) = self.inner.lock() else {
                return Err(HttpError::internal("History sync lock poisoned"));
            };
            if s.starting || is_active_state(s.status.state) {
                return Ok((true, s.status.clone()));
            }
            if self.deps.own_id().is_none() {
                return Err(HttpError::conflict("Not logged in"));
            }
            s.starting = true;
        }

        let keys = match self.deps.generate_keypair().await {
            Ok(k) => k,
            Err(err) => {
                if let Ok(mut s) = self.inner.lock() {
                    s.starting = false;
                }
                return Err(HttpError::internal(err.to_string()));
            }
        };

        let (gen, temp_key, public_key, snapshot) = {
            let Ok(mut s) = self.inner.lock() else {
                return Err(HttpError::internal("History sync lock poisoned"));
            };
            s.starting = false;
            if let Some(t) = s.timeout_task.take() {
                t.abort();
            }
            if let Some(w) = s.worker_task.take() {
                w.abort();
            }
            s.generation = s.generation.wrapping_add(1);
            let gen = s.generation;
            let public_key = keys.public_key_spki_b64.clone();
            s.keys = Some(keys);
            s.status = HistorySyncStatus {
                state: SyncState::WaitingForPhone,
                progress: 5,
                error: None,
                detail: "Request sent. Open Zalo on your phone and tap Allow to transfer your messages."
                    .to_string(),
                expected_messages: None,
                counts: None,
                log: Vec::new(),
            };
            (gen, s.temp_key.clone(), public_key, s.status.clone())
        };
        self.deps.on_status_change(&snapshot);

        self.arm_timeout(
            gen,
            self.deps.timeouts().confirm_ms,
            "The phone did not confirm the request. Open Zalo on the phone and try again.",
        );

        let params = json!({
            "pc_name": self.deps.pc_name(),
            "public_key": public_key,
            "from_seq_id": 0,
            "is_retry": 0,
            "min_seq_id": 0,
            "temp_key": temp_key,
            "imei": self.deps.imei(),
        });

        match self
            .deps
            .call_file_service("pull_mobile_msg".to_string(), params)
            .await
        {
            Ok(_) => {
                self.log("pull_mobile_msg ok");
            }
            Err(err) => {
                let code_str = err
                    .code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".to_string());
                self.log(format!("pull_mobile_msg failed code={code_str}"));
                let msg = if err.code == Some(CODE_CLIENT_NOT_SUPPORTED) {
                    "This phone uses a newer way of syncing history (sync v2, used by Zalo on iPhone), which isn't supported yet."
                        .to_string()
                } else if let Some(c) = err.code {
                    format!("Zalo refused the sync request (code {c}).")
                } else {
                    "Zalo refused the sync request.".to_string()
                };
                self.fail(gen, msg);
            }
        }

        Ok((false, self.get_status()))
    }

    /// Cancels an active sync, notifies Zalo best-effort, and resets status to `idle`.
    pub fn cancel(self: &Arc<Self>) {
        let (was_active, keys, should_emit) = {
            let Ok(mut s) = self.inner.lock() else {
                return;
            };
            let was_active = s.starting || is_active_state(s.status.state);
            let keys = s.keys.take();
            s.starting = false;
            if let Some(t) = s.timeout_task.take() {
                t.abort();
            }
            if let Some(w) = s.worker_task.take() {
                w.abort();
            }
            s.generation = s.generation.wrapping_add(1);
            let should_emit = s.status.state != SyncState::Idle;
            if should_emit {
                s.status = HistorySyncStatus::default();
            }
            (was_active, keys, should_emit)
        };

        if was_active {
            if let Some(k) = keys {
                let deps = Arc::clone(&self.deps);
                let params = json!({
                    "pc_name": deps.pc_name(),
                    "public_key": k.public_key_spki_b64,
                    "imei": deps.imei(),
                });
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        let _ = deps
                            .call_file_service("cancel_pull_mobile_msg".to_string(), params)
                            .await;
                    });
                }
            }
        }

        if should_emit {
            self.deps.on_status_change(&HistorySyncStatus::default());
        }
    }

    /// Processes a parsed history-sync control from WebSocket or control poll.
    pub fn handle_control(self: &Arc<Self>, control: SyncControl) {
        let SyncControl {
            act_type,
            act,
            data,
        } = control;

        let (state, gen, ours) = {
            let Ok(s) = self.inner.lock() else {
                return;
            };
            let ours = s.keys.as_ref().is_some_and(|k| {
                data.get("public_key").and_then(|v| v.as_str()) == Some(&k.public_key_spki_b64)
            });
            (s.status.state, s.generation, ours)
        };

        let state_str = serde_json::to_value(state)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".into());
        self.log(format!(
            "control act_type={act_type} act={act} state={state_str} ours={ours} keys=[{}]",
            sorted_keys(&data)
        ));

        if act_type == "transfer_after_login" {
            let key = data
                .get("tempKey")
                .or_else(|| data.get("temp_key"))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.trim().to_string()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if !key.is_empty() {
                if let Ok(mut s) = self.inner.lock() {
                    s.temp_key = key;
                }
            }
            return;
        }

        if act_type == "transfer_sync2" {
            if state == SyncState::WaitingForPhone {
                self.log("phone speaks sync v2 (not supported)");
            }
            return;
        }

        if act_type != "syncmsgmb" || state != SyncState::WaitingForPhone || !ours {
            return;
        }

        let same_pc = match data.get("pc_name") {
            None | Some(Value::Null) => true,
            Some(v) => v.as_str() == Some(self.deps.pc_name()),
        };

        if act == "user_confirm" && same_pc {
            let action = val_i64(data.get("user_action"), -1);
            match action {
                1 | 3 => {
                    self.update_status(|st| {
                        st.progress = 15;
                        st.detail =
                            "The phone agreed. Waiting for it to pack your messages...".to_string();
                    });
                    self.arm_timeout(
                        gen,
                        self.deps.timeouts().backup_ms,
                        "The phone did not send the backup within 30 minutes.",
                    );
                }
                0 => self.fail(gen, "The sync request was declined on the phone.".into()),
                2 => self.fail(
                    gen,
                    "The phone is restoring data and can't send messages right now. Try again later."
                        .into(),
                ),
                _ => {}
            }
        } else if act == "transfer_error" && same_pc {
            match classify_transfer_error(&data) {
                TransferError::PhoneIdle => {
                    self.update_status(|st| {
                        st.detail = "Keep Zalo open on your phone, with the screen on, so the backup can continue."
                            .to_string();
                    });
                }
                TransferError::PhoneActive => {
                    self.update_status(|st| {
                        st.progress = 20;
                        st.detail = "The phone is backing up your messages...".to_string();
                    });
                    self.arm_timeout(
                        gen,
                        self.deps.timeouts().backup_ms,
                        "The phone did not send the backup within 30 minutes.",
                    );
                }
                TransferError::Failed => {
                    let code = val_i64(data.get("error_code"), 0);
                    self.fail(
                        gen,
                        format!("The phone could not make the backup (code {code})."),
                    );
                }
                TransferError::Ignore => {}
            }
        } else if act == "syncmsg_info" {
            self.disarm_timeout();
            let mgr = Arc::clone(self);
            let handle = tokio::spawn(async move {
                if let Err(err_msg) = mgr.receive_backup(gen, data).await {
                    mgr.fail(gen, err_msg);
                }
            });
            if let Ok(mut s) = self.inner.lock() {
                if let Some(old) = s.worker_task.take() {
                    old.abort();
                }
                if s.generation == gen {
                    s.worker_task = Some(handle);
                } else {
                    handle.abort();
                }
            }
        }
    }

    async fn receive_backup(
        self: &Arc<Self>,
        gen: u64,
        data: Map<String, Value>,
    ) -> Result<(), String> {
        let info = parse_json_object(data.get("db_info"));
        let counts = info
            .get("origin_db")
            .or_else(|| info.get("backup_db"))
            .and_then(|v| v.as_object());
        let expected = counts
            .map(|c| val_u64(c.get("msg_total")) as usize)
            .unwrap_or(0);
        let format = val_u64(info.get("db_format")) as u32;
        let url = data
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let encrypted_key = data
            .get("encrypted_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let file_size = val_u64(data.get("file_size"));

        self.log(format!(
            "backup info: format={format} msgs={expected} hasUrl={} host={} urlParams=[{}] hasKey={} size={file_size} dataKeys=[{}] infoKeys=[{}]",
            !url.is_empty(),
            host_of(&url),
            param_names_of(&url),
            !encrypted_key.is_empty(),
            sorted_keys(&data),
            sorted_keys(&info),
        ));

        if info.is_empty() {
            return Err("The backup could not be read: the backup description is missing.".into());
        }
        if expected == 0 {
            self.finish(
                gen,
                HistorySyncCounts::default(),
                "Your phone had no messages to transfer.".to_string(),
            );
            return Ok(());
        }
        if format != 1 {
            return Err(
                "The backup could not be read: the backup uses an old format (0) that isn't supported."
                    .into(),
            );
        }
        if !is_https_url(&url) || encrypted_key.is_empty() {
            return Err(
                "The backup could not be read: the backup has no download link or key.".into(),
            );
        }

        self.update_status(|st| {
            st.state = SyncState::Downloading;
            st.progress = 20;
            st.detail = "Downloading the backup from Zalo...".to_string();
            st.expected_messages = Some(expected);
        });

        let bytes = self.download_with_retries(gen, &url).await?;
        if !self.is_generation(gen) {
            return Ok(());
        }
        self.log(format!(
            "downloaded bytes={} expected={file_size}",
            bytes.len()
        ));

        self.update_status(|st| {
            st.state = SyncState::Importing;
            st.progress = 55;
            st.detail = "Decrypting the backup...".to_string();
        });
        tokio::task::yield_now().await;

        let keys = {
            let Ok(s) = self.inner.lock() else {
                return Ok(());
            };
            if s.generation != gen {
                return Ok(());
            }
            match s.keys.clone() {
                Some(k) => k,
                None => return Ok(()),
            }
        };

        let public_key_for_cleanup = keys.public_key_spki_b64.clone();
        let unpack_result = tokio::task::spawn_blocking(move || {
            let plain_key = decrypt_backup_key(&keys.private_key, &encrypted_key)
                .ok_or_else(|| BackupError("the backup key could not be decrypted".into()))?;
            let key_text = backup_key_text(&plain_key, format);
            let files = unpack_backup(&bytes, &key_text, DEFAULT_MAX_OUTPUT)?;
            let total_files = files.len();
            let conversations = conversation_files(&files);
            Ok::<_, BackupError>((total_files, conversations))
        })
        .await
        .map_err(|e| format!("The backup could not be read: {e}."))?;

        let (file_count, conversations) = unpack_result
            .map_err(|e| format!("The backup could not be read: {e}."))?;

        if !self.is_generation(gen) {
            return Ok(());
        }
        self.log(format!(
            "unpacked files={file_count} conversations={}",
            conversations.len()
        ));

        // Best-effort snapshot cleanup on Zalo's servers.
        {
            let deps = Arc::clone(&self.deps);
            let params = json!({
                "public_key": public_key_for_cleanup,
                "imei": deps.imei(),
            });
            tokio::spawn(async move {
                let _ = deps
                    .call_file_service("delete_snapshot_mobile_msg".to_string(), params)
                    .await;
            });
        }

        if conversations.is_empty() {
            return Err("The backup could not be read: it holds no conversations.".into());
        }

        let store = self.deps.store();
        store.allow_messages(HISTORY_MESSAGES_PER_THREAD);
        let own_id = self.deps.own_id().unwrap_or_default();
        let my_plain_uid = match data.get("uid") {
            Some(Value::String(s)) => s.trim().to_string(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };

        // Parse every SQLite conversation DB first so we can batch-resolve plain→noised IDs.
        let mut parsed: Vec<BackupConversation> = Vec::new();
        let mut unreadable = 0usize;
        for file in &conversations {
            if !self.is_generation(gen) {
                return Ok(());
            }
            match read_conversation(&file.data, &file.id, file.is_group) {
                Ok(Some(conv)) if !conv.rows.is_empty() => parsed.push(conv),
                Ok(_) => {}
                Err(_) => {
                    unreadable += 1;
                    self.log("conversation unreadable: SqliteError");
                }
            }
        }
        self.log(format!(
            "parsed conversations={} unreadable={unreadable} hasMyPlainUid={}",
            parsed.len(),
            !my_plain_uid.is_empty()
        ));

        let mut id_map: Option<BackupIdMap> = if !my_plain_uid.is_empty() {
            Some(BackupIdMap {
                users: None,
                groups: None,
                my_plain_uid: my_plain_uid.clone(),
            })
        } else {
            None
        };

        if self.deps.has_znoise() && !parsed.is_empty() {
            let mut plain_users: Vec<String> = Vec::new();
            let mut plain_user_set: HashSet<String> = HashSet::new();
            let mut plain_groups: Vec<String> = Vec::new();
            let mut plain_group_set: HashSet<String> = HashSet::new();

            let mut add_plain_user = |id: &str| {
                if !id.is_empty()
                    && id != "0"
                    && id != my_plain_uid
                    && id.bytes().all(|b| b.is_ascii_digit())
                    && plain_user_set.insert(id.to_string())
                {
                    plain_users.push(id.to_string());
                }
            };

            for conv in &parsed {
                if conv.is_group {
                    if plain_group_set.insert(conv.id.clone()) {
                        plain_groups.push(conv.id.clone());
                    }
                } else {
                    add_plain_user(&conv.id);
                }
                for row in &conv.rows {
                    add_plain_user(&row.sender_id);
                    if let Some(ref q) = row.bin.quote {
                        add_plain_user(&q.owner_id);
                    }
                    for m in &row.bin.mentions {
                        add_plain_user(&m.uid);
                    }
                }
            }

            let mut mapped_users: HashMap<String, String> = HashMap::new();
            let mut mapped_groups: HashMap<String, String> = HashMap::new();
            self.update_status(|st| {
                st.state = SyncState::Importing;
                st.progress = 58;
                st.detail = "Matching contacts with Zalo...".to_string();
            });

            let max_len = plain_users.len().max(plain_groups.len());
            for i in (0..max_len).step_by(NOISE_BATCH) {
                if !self.is_generation(gen) {
                    return Ok(());
                }
                let u: Vec<String> = if i < plain_users.len() {
                    plain_users[i..(i + NOISE_BATCH).min(plain_users.len())].to_vec()
                } else {
                    Vec::new()
                };
                let g: Vec<String> = if i < plain_groups.len() {
                    plain_groups[i..(i + NOISE_BATCH).min(plain_groups.len())].to_vec()
                } else {
                    Vec::new()
                };
                if u.is_empty() && g.is_empty() {
                    continue;
                }
                let u_len = u.len();
                let g_len = g.len();
                match self.deps.resolve_noised_ids(u, g).await {
                    Ok(res) => {
                        if !self.is_generation(gen) {
                            return Ok(());
                        }
                        let mu = res.users.len();
                        let mg = res.groups.len();
                        mapped_users.extend(res.users);
                        mapped_groups.extend(res.groups);
                        let shape_part = if res.reply_shape.is_empty() {
                            String::new()
                        } else {
                            format!(" reply={}", res.reply_shape)
                        };
                        self.log(format!(
                            "znoise asked users={u_len} groups={g_len} mapped users={mu} groups={mg}{shape_part}"
                        ));
                    }
                    Err(err) => {
                        self.log(format!("znoise failed: {err}"));
                    }
                }
            }

            if mapped_users.is_empty()
                && mapped_groups.is_empty()
                && (plain_users.len() + plain_groups.len() > 0)
            {
                return Err(
                    "The backup could not be read: could not match chats with Zalo.".into(),
                );
            }
            id_map = Some(BackupIdMap {
                users: Some(mapped_users),
                groups: Some(mapped_groups),
                my_plain_uid: my_plain_uid.clone(),
            });
        }

        let mut total = HistorySyncCounts::default();
        let mut done = 0usize;
        let mut imported_chats = 0usize;
        let mut unmapped_chats = 0usize;
        let total_convs = parsed.len().max(1);

        for conv in &parsed {
            if !self.is_generation(gen) {
                return Ok(());
            }
            let target_id = if let Some(ref map) = id_map {
                if map.users.is_some() || map.groups.is_some() {
                    if conv.is_group {
                        map.groups
                            .as_ref()
                            .and_then(|g| g.get(&conv.id))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        map.users
                            .as_ref()
                            .and_then(|u| u.get(&conv.id))
                            .cloned()
                            .unwrap_or_default()
                    }
                } else {
                    conv.id.clone()
                }
            } else {
                conv.id.clone()
            };

            if target_id.is_empty() {
                unmapped_chats += 1;
                total.skipped += conv.source_rows;
                done += 1;
                continue;
            }

            if done == 0 && !conv.columns.is_empty() {
                self.log(format!("schema columns={}", conv.columns.join(",")));
            }
            if done < 3 {
                self.log(format!(
                    "senders {}",
                    describe_senders(conv, &own_id, &my_plain_uid)
                ));
            }

            let kind = if conv.is_group {
                ThreadKind::Group
            } else {
                ThreadKind::User
            };
            if target_id != conv.id {
                store.drop_plain_thread(kind, &conv.id);
            }

            self.deps.prepare(target_id.clone(), conv.is_group).await;
            if !self.is_generation(gen) {
                return Ok(());
            }

            let conv_counts = import_conversation_into_store(
                conv,
                &own_id,
                store,
                |uid| self.deps.name_of(uid),
                id_map.as_ref(),
            );
            total.imported += conv_counts.imported;
            total.duplicates += conv_counts.duplicates;
            total.skipped += conv_counts.skipped + (conv.source_rows.saturating_sub(conv.rows.len()));
            total.recalled += conv_counts.recalled;
            if conv_counts.imported > 0 || conv_counts.duplicates > 0 {
                imported_chats += 1;
            }
            done += 1;
            let progress = (60 + (39 * done) / total_convs).min(99) as u8;
            let parsed_len = parsed.len();
            self.update_status(|st| {
                st.state = SyncState::Importing;
                st.progress = progress;
                st.detail = format!("Importing messages ({done}/{parsed_len} chats)...");
            });
            tokio::task::yield_now().await;
        }

        let chat_count = if imported_chats > 0 {
            imported_chats
        } else {
            parsed.len().saturating_sub(unmapped_chats)
        };
        total.conversations = chat_count;
        self.log(format!(
            "import done imported={} duplicates={} skipped={} recalled={} chats={chat_count} unmapped={unmapped_chats}",
            total.imported, total.duplicates, total.skipped, total.recalled
        ));

        let dup_note = if total.duplicates > 0 {
            format!("; {} were already here", total.duplicates)
        } else {
            String::new()
        };
        let detail = format!(
            "Added {} messages from {} chats{dup_note}.",
            total.imported, chat_count
        );
        self.finish(gen, total, detail);
        Ok(())
    }

    async fn download_with_retries(
        self: &Arc<Self>,
        gen: u64,
        url: &str,
    ) -> Result<Vec<u8>, String> {
        let stall_ms = self.deps.timeouts().download_stall_ms;
        for attempt in 1usize.. {
            if !self.is_generation(gen) {
                return Err("cancelled".into());
            }
            let ping = Arc::new(Notify::new());
            let ping_cb = Arc::clone(&ping);
            let mgr_cb = Arc::clone(self);
            let on_progress: ProgressCallback = Arc::new(move |got, total| {
                ping_cb.notify_one();
                if total > 0 && mgr_cb.is_generation(gen) {
                    let pct = (20 + ((30 * got) / total).min(30)) as u8;
                    mgr_cb.update_status(|st| {
                        st.progress = pct;
                    });
                }
            });

            let dl_fut = self.deps.download_backup(url.to_string(), on_progress);
            let stall_fut = wait_for_stall(stall_ms, ping);

            let outcome = tokio::select! {
                res = dl_fut => Ok(res),
                _ = stall_fut => Err(()),
            };

            if !self.is_generation(gen) {
                return Err("cancelled".into());
            }

            match outcome {
                Ok(Ok(bytes)) => return Ok(bytes),
                Err(()) => {
                    self.log(format!("download attempt={attempt} status=0 retryIn=-1"));
                    return Err("The backup could not be read: the download stalled.".into());
                }
                Ok(Err(err)) => {
                    let wait = download_retry_delay_ms(
                        err.status,
                        attempt,
                        err.retry_after.as_deref(),
                        self.deps.now_ms(),
                    );
                    let wait_log = wait.map(|w| w as i64).unwrap_or(-1);
                    let answer_part = err
                        .answer
                        .as_deref()
                        .map(|a| format!(" answer={}", serde_json::to_string(a).unwrap_or_default()))
                        .unwrap_or_default();
                    self.log(format!(
                        "download attempt={attempt} status={} retryIn={wait_log}{answer_part}",
                        err.status
                    ));
                    let Some(wait_ms) = wait else {
                        let reason = if err.status != 0 {
                            format!("the download failed (HTTP {})", err.status)
                        } else {
                            "the download failed (network error)".to_string()
                        };
                        return Err(format!("The backup could not be read: {reason}."));
                    };
                    self.update_status(|st| {
                        st.progress = 20;
                        st.detail =
                            "Waiting for Zalo's server to prepare the backup...".to_string();
                    });
                    self.deps.sleep(wait_ms).await;
                    if !self.is_generation(gen) {
                        return Err("cancelled".into());
                    }
                }
            }
        }
        unreachable!()
    }

    fn finish(&self, gen: u64, counts: HistorySyncCounts, detail: String) {
        let snapshot = {
            let Ok(mut s) = self.inner.lock() else {
                return;
            };
            if s.generation != gen {
                return;
            }
            if let Some(t) = s.timeout_task.take() {
                t.abort();
            }
            s.keys = None;
            s.status.state = SyncState::Done;
            s.status.progress = 100;
            s.status.error = None;
            s.status.detail = detail;
            s.status.counts = Some(counts);
            s.status.clone()
        };
        self.deps.on_status_change(&snapshot);
    }

    fn fail(&self, gen: u64, error: String) {
        let (keys, snapshot) = {
            let Ok(mut s) = self.inner.lock() else {
                return;
            };
            if s.generation != gen {
                return;
            }
            let keys = s.keys.take();
            if let Some(t) = s.timeout_task.take() {
                t.abort();
            }
            s.generation = s.generation.wrapping_add(1);
            s.status.state = SyncState::Failed;
            s.status.progress = 0;
            s.status.error = Some(error.clone());
            s.status.detail.clear();
            (keys, s.status.clone())
        };

        if let Some(k) = keys {
            let deps = Arc::clone(&self.deps);
            let params = json!({
                "pc_name": deps.pc_name(),
                "public_key": k.public_key_spki_b64,
                "imei": deps.imei(),
            });
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = deps
                        .call_file_service("cancel_pull_mobile_msg".to_string(), params)
                        .await;
                });
            }
        }

        self.log(format!("failed: {error}"));
        self.deps.on_status_change(&snapshot);
    }
}

async fn wait_for_stall(stall_ms: u64, ping: Arc<Notify>) {
    loop {
        tokio::select! {
            _ = ping.notified() => continue,
            _ = tokio::time::sleep(Duration::from_millis(stall_ms)) => return,
        }
    }
}

/// Converts and imports a single backup conversation into [`ChatStore`], deduplicating by ID,
/// stripping `raw` copies, applying recalls, and settling read state so history is never unread.
pub fn import_conversation_into_store<F>(
    conv: &BackupConversation,
    own_id: &str,
    store: &ChatStore,
    name_of: F,
    id_map: Option<&BackupIdMap>,
) -> HistorySyncCounts
where
    F: Fn(&str) -> String,
{
    let converted = convert_conversation(conv, own_id, name_of, id_map);
    let skipped_sum: usize = converted.skipped.values().sum();
    let Some(thread_id) = converted.thread_id else {
        return HistorySyncCounts {
            skipped: skipped_sum,
            ..Default::default()
        };
    };

    let kind = if conv.is_group {
        ThreadKind::Group
    } else {
        ThreadKind::User
    };
    let mut imported_ids = HashSet::new();
    let mut imported = 0usize;
    let mut duplicates = 0usize;

    for raw in &converted.messages {
        let mut msg = normalize_message(&raw.data, kind, &raw.thread_id, raw.is_self, Some(own_id));
        // Imported history is never quoted/reacted to, so omit raw copy to save memory & mark as history-only.
        msg.raw = None;
        let msg_id = msg.id.clone();
        if store.add_history_message(msg) {
            imported += 1;
            imported_ids.insert(msg_id);
        } else {
            duplicates += 1;
        }
    }

    let mut recalled = 0usize;
    for id in &converted.recalled {
        if store.apply_undo(id).is_some() {
            recalled += 1;
        }
    }

    store.settle_history(kind, &thread_id, &imported_ids);

    HistorySyncCounts {
        imported,
        duplicates,
        skipped: skipped_sum,
        recalled,
        conversations: 0,
    }
}

// ---- Axum Router & HTTP Handlers ----

/// Builds the Track 2 history-sync sub-router and wires the WebSocket history-sync control listener.
pub fn router(state: AppState) -> Router<AppState> {
    let manager = HistorySyncManager::for_session(state.clone());
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let mut rx = state.session.subscribe_history_sync_controls();
        let mgr_bg = Arc::clone(&manager);
        handle.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(val) => {
                        if let Some(ctrl) = sync_control_of(&val) {
                            mgr_bg.handle_control(ctrl);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "History sync control receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    Router::new()
        .route("/api/history-sync/status", get(get_status))
        .route("/api/history-sync/start", post(post_start))
        .route("/api/history-sync/cancel", post(post_cancel))
        .layer(Extension(manager))
}

async fn get_status(Extension(mgr): Extension<Arc<HistorySyncManager>>) -> impl IntoResponse {
    let status = mgr.get_status();
    let log = mgr.get_log();
    let mut val = serde_json::to_value(&status).unwrap_or_else(|_| json!({}));
    if let Some(obj) = val.as_object_mut() {
        obj.insert("log".to_string(), json!(log));
    }
    (StatusCode::OK, Json(val))
}

async fn post_start(
    Extension(mgr): Extension<Arc<HistorySyncManager>>,
) -> Result<impl IntoResponse, HttpError> {
    let (was_active, status) = mgr.start().await?;
    let code = if was_active {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((code, Json(status)))
}

async fn post_cancel(Extension(mgr): Extension<Arc<HistorySyncManager>>) -> impl IntoResponse {
    mgr.cancel();
    (StatusCode::OK, Json(mgr.get_status()))
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    use zalo_media::sync::{encrypt_backup_key_pkcs1, pack_zdb_backup};

    use crate::config::Config;
    use crate::types::StoredMessage;

    const OWN: &str = "555";
    const KEY_TEXT: &str = "00112233445566778899aabbccddeeff";

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/fixtures")
            .join(name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
    }

    fn build_fixture_backup(key_text: &str) -> Vec<u8> {
        let files_json: Vec<(String, usize)> =
            serde_json::from_slice(&fixture("sync_files.json")).unwrap();
        let specs: Vec<(&str, usize)> = files_json.iter().map(|(n, s)| (n.as_str(), *s)).collect();
        let body = fixture("sync_body.xz");
        pack_zdb_backup(&specs, &body, key_text, 0, 0).unwrap()
    }

    struct MockDeps {
        own_id: Option<String>,
        store: ChatStore,
        key_pair: SyncKeyPair,
        calls: Mutex<Vec<(String, Value)>>,
        call_error: Option<FileCallError>,
        downloads: Mutex<Vec<String>>,
        download_bytes: Vec<u8>,
        download_fail_times: AtomicUsize,
        download_fail_status: u16,
        download_always_fail: Option<u16>,
        waits: Mutex<Vec<u64>>,
        states: Mutex<Vec<HistorySyncStatus>>,
        timeouts: SyncTimeouts,
        znoise_reply: Option<NoisedIdsReply>,
    }

    impl MockDeps {
        fn new(key_text: &str) -> Arc<Self> {
            let store = ChatStore::in_memory();
            store.set_own_id(Some(OWN.into()));
            Arc::new(Self {
                own_id: Some(OWN.into()),
                store,
                key_pair: generate_sync_keypair().unwrap(),
                calls: Mutex::new(Vec::new()),
                call_error: None,
                downloads: Mutex::new(Vec::new()),
                download_bytes: build_fixture_backup(key_text),
                download_fail_times: AtomicUsize::new(0),
                download_fail_status: 404,
                download_always_fail: None,
                waits: Mutex::new(Vec::new()),
                states: Mutex::new(Vec::new()),
                timeouts: SyncTimeouts::default(),
                znoise_reply: None,
            })
        }

        fn encrypted_key_b64(&self) -> String {
            let raw_key = hex::decode(KEY_TEXT).unwrap();
            encrypt_backup_key_pkcs1(&self.key_pair, &raw_key).unwrap()
        }

        fn syncmsg_info_data(&self, extra: Map<String, Value>) -> Map<String, Value> {
            let mut m = Map::new();
            m.insert(
                "public_key".into(),
                Value::String(self.key_pair.public_key_spki_b64.clone()),
            );
            m.insert("pc_name".into(), Value::String(DEFAULT_PC_NAME.into()));
            m.insert(
                "url".into(),
                Value::String("https://files.example.invalid/backup/abc".into()),
            );
            m.insert("encrypted_key".into(), Value::String(self.encrypted_key_b64()));
            m.insert("file_size".into(), json!(self.download_bytes.len()));
            m.insert(
                "db_info".into(),
                Value::String(
                    json!({ "db_format": 1, "origin_db": { "msg_total": 16 } }).to_string(),
                ),
            );
            for (k, v) in extra {
                m.insert(k, v);
            }
            m
        }
    }

    impl HistorySyncDeps for MockDeps {
        fn own_id(&self) -> Option<String> {
            self.own_id.clone()
        }

        fn imei(&self) -> String {
            "imei-1".into()
        }

        fn store(&self) -> &ChatStore {
            &self.store
        }

        fn call_file_service(
            &self,
            endpoint: String,
            params: Value,
        ) -> BoxFuture<'static, Result<Value, FileCallError>> {
            self.calls.lock().unwrap().push((endpoint, params));
            let err = self.call_error.clone();
            Box::pin(async move {
                if let Some(e) = err {
                    Err(e)
                } else {
                    Ok(json!({}))
                }
            })
        }

        fn download_backup(
            &self,
            url: String,
            on_progress: ProgressCallback,
        ) -> BoxFuture<'static, Result<Vec<u8>, DownloadError>> {
            self.downloads.lock().unwrap().push(url);
            if let Some(status) = self.download_always_fail {
                return Box::pin(async move {
                    Err(DownloadError {
                        status,
                        retry_after: None,
                        answer: None,
                        message: format!("HTTP {status}"),
                    })
                });
            }
            let prev = self.download_fail_times.load(Ordering::SeqCst);
            if prev > 0 {
                self.download_fail_times.fetch_sub(1, Ordering::SeqCst);
                let status = self.download_fail_status;
                return Box::pin(async move {
                    Err(DownloadError {
                        status,
                        retry_after: None,
                        answer: None,
                        message: format!("HTTP {status}"),
                    })
                });
            }
            let bytes = self.download_bytes.clone();
            Box::pin(async move {
                let len = bytes.len();
                on_progress(len / 2, len);
                on_progress(len, len);
                Ok(bytes)
            })
        }

        fn has_znoise(&self) -> bool {
            self.znoise_reply.is_some()
        }

        fn resolve_noised_ids(
            &self,
            _user_ids: Vec<String>,
            _group_ids: Vec<String>,
        ) -> BoxFuture<'static, Result<NoisedIdsReply, String>> {
            let reply = self.znoise_reply.clone().unwrap_or_default();
            Box::pin(async move { Ok(reply) })
        }

        fn name_of(&self, _user_id: &str) -> String {
            String::new()
        }

        fn on_status_change(&self, status: &HistorySyncStatus) {
            self.states.lock().unwrap().push(status.clone());
        }

        fn generate_keypair(&self) -> BoxFuture<'static, Result<SyncKeyPair, BackupError>> {
            let kp = self.key_pair.clone();
            Box::pin(async move { Ok(kp) })
        }

        fn sleep(&self, ms: u64) -> BoxFuture<'static, ()> {
            self.waits.lock().unwrap().push(ms);
            Box::pin(async {})
        }

        fn timeouts(&self) -> SyncTimeouts {
            self.timeouts
        }
    }

    async fn settle(mgr: &HistorySyncManager, want: SyncState) -> HistorySyncStatus {
        for _ in 0..400 {
            let st = mgr.get_status();
            if st.state == want {
                return st;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        mgr.get_status()
    }

    #[test]
    fn pure_helpers_control_parsing_and_retry_delays() {
        let ctrl = sync_control_of(&json!({
            "act_type": "syncmsgmb",
            "act": "user_confirm",
            "data": "{\"user_action\":1,\"uid\":123456789012345678}"
        }))
        .unwrap();
        assert_eq!(ctrl.act, "user_confirm");
        assert_eq!(ctrl.data["user_action"], 1);
        assert_eq!(ctrl.data["uid"], "123456789012345678");

        // Listener camelCase format
        let ctrl2 = sync_control_of(&json!({
            "actType": "transfer_after_login",
            "act": "x",
            "data": { "temp_key": "tk-9" }
        }))
        .unwrap();
        assert_eq!(ctrl2.act_type, "transfer_after_login");
        assert_eq!(ctrl2.data["temp_key"], "tk-9");

        // Non-sync act_type ignored
        assert!(sync_control_of(&json!({ "act_type": "voip", "act": "request" })).is_none());

        // transfer_error classification
        assert_eq!(
            classify_transfer_error(json!({ "status": 2 }).as_object().unwrap()),
            TransferError::PhoneIdle
        );
        assert_eq!(
            classify_transfer_error(json!({ "status": 1, "error_code": 5 }).as_object().unwrap()),
            TransferError::PhoneActive
        );
        assert_eq!(
            classify_transfer_error(json!({ "status": 0, "error_code": 5 }).as_object().unwrap()),
            TransferError::Failed
        );
        assert_eq!(
            classify_transfer_error(json!({}).as_object().unwrap()),
            TransferError::Ignore
        );

        // Retry backoff & Retry-After
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap().timestamp_millis();
        assert_eq!(download_retry_delay_ms(404, 1, None, now), Some(1000));
        assert_eq!(download_retry_delay_ms(404, 2, None, now), Some(2000));
        assert_eq!(download_retry_delay_ms(500, 4, None, now), Some(8000));
        assert_eq!(download_retry_delay_ms(0, 1, None, now), Some(1000));
        assert_eq!(download_retry_delay_ms(403, 1, None, now), None);
        assert_eq!(download_retry_delay_ms(404, 5, None, now), None);
        assert_eq!(download_retry_delay_ms(429, 1, Some("5"), now), Some(5000));
        assert_eq!(download_retry_delay_ms(429, 1, Some("3600"), now), Some(30_000));
        assert_eq!(
            download_retry_delay_ms(503, 1, Some("Thu, 01 Jan 2026 12:00:06 GMT"), now),
            Some(6000)
        );
        assert_eq!(
            download_retry_delay_ms(503, 1, Some("Thu, 01 Jan 2026 11:59:54 GMT"), now),
            Some(1000)
        );
        assert_eq!(download_retry_delay_ms(404, 1, Some("5"), now), Some(1000));
    }

    #[test]
    fn import_conversation_dedupes_recalls_and_preserves_live_unread() {
        let store = ChatStore::in_memory();
        store.set_own_id(Some(OWN.into()));
        store.add_message(StoredMessage {
            id: "9002".into(),
            cli_msg_id: "802".into(),
            thread_id: "1001".into(),
            thread_kind: ThreadKind::User,
            from_id: OWN.into(),
            from_name: String::new(),
            is_self: true,
            ts: 1_700_000_001_000,
            msg_type: "webchat".into(),
            text: "hi back (live copy)".into(),
            attachment: None,
            quote: None,
            mentions: Vec::new(),
            recalled: false,
            reactions: None,
            delivered_to: None,
            seen_by: None,
            raw: None,
        });

        let conv = read_conversation(&fixture("sync_1001.db"), "1001", false)
            .unwrap()
            .unwrap();
        let counts = import_conversation_into_store(&conv, OWN, &store, |_| String::new(), None);
        assert_eq!(counts.imported, 6);
        assert_eq!(counts.duplicates, 2);
        assert_eq!(counts.skipped, 1);
        assert_eq!(counts.recalled, 1);

        let msgs = store.get_messages(ThreadKind::User, "1001", None, Some(100));
        assert!(msgs.windows(2).all(|w| w[0].ts <= w[1].ts));
        assert_eq!(
            msgs.iter().find(|m| m.id == "9002").unwrap().text,
            "hi back (live copy)"
        );
        assert!(msgs.iter().find(|m| m.id == "9001").unwrap().recalled);
        assert_eq!(
            store
                .list_threads()
                .iter()
                .find(|t| t.id == "1001")
                .unwrap()
                .unread,
            0
        );
        assert!(store.get_message("9003").unwrap().raw.is_none());

        // Re-importing adds nothing
        let again = import_conversation_into_store(&conv, OWN, &store, |_| String::new(), None);
        assert_eq!(again.imported, 0);

        // Newer unread live message stays unread
        let store2 = ChatStore::in_memory();
        store2.add_message(StoredMessage {
            id: "live1".into(),
            cli_msg_id: "c".into(),
            thread_id: "1001".into(),
            thread_kind: ThreadKind::User,
            from_id: "1001".into(),
            from_name: "Alice".into(),
            is_self: false,
            ts: 1_800_000_000_000,
            msg_type: "webchat".into(),
            text: "fresh".into(),
            attachment: None,
            quote: None,
            mentions: Vec::new(),
            recalled: false,
            reactions: None,
            delivered_to: None,
            seen_by: None,
            raw: None,
        });
        import_conversation_into_store(&conv, OWN, &store2, |_| String::new(), None);
        assert_eq!(
            store2
                .list_threads()
                .iter()
                .find(|t| t.id == "1001")
                .unwrap()
                .unread,
            1
        );
    }

    #[tokio::test]
    async fn full_flow_request_to_imported_messages() {
        let deps = MockDeps::new(KEY_TEXT);
        let mgr = HistorySyncManager::new(deps.clone());

        let (was_active, started) = mgr.start().await.unwrap();
        assert!(!was_active);
        assert_eq!(started.state, SyncState::WaitingForPhone);
        {
            let calls = deps.calls.lock().unwrap();
            assert_eq!(calls[0].0, "pull_mobile_msg");
            assert_eq!(calls[0].1["pc_name"], DEFAULT_PC_NAME);
            assert_eq!(calls[0].1["public_key"], deps.key_pair.public_key_spki_b64);
            assert_eq!(calls[0].1["imei"], "imei-1");
        }

        // Someone else's public_key is ignored
        mgr.handle_control(SyncControl {
            act_type: "syncmsgmb".into(),
            act: "user_confirm".into(),
            data: json!({ "public_key": "other", "user_action": 0 })
                .as_object()
                .unwrap()
                .clone(),
        });
        assert_eq!(mgr.get_status().state, SyncState::WaitingForPhone);

        // Phone confirms
        mgr.handle_control(SyncControl {
            act_type: "syncmsgmb".into(),
            act: "user_confirm".into(),
            data: json!({
                "public_key": deps.key_pair.public_key_spki_b64,
                "pc_name": DEFAULT_PC_NAME,
                "user_action": 1
            })
            .as_object()
            .unwrap()
            .clone(),
        });
        assert_eq!(mgr.get_status().progress, 15);

        // Phone sends backup info
        mgr.handle_control(SyncControl {
            act_type: "syncmsgmb".into(),
            act: "syncmsg_info".into(),
            data: deps.syncmsg_info_data(Map::new()),
        });

        let done = settle(&mgr, SyncState::Done).await;
        assert_eq!(done.state, SyncState::Done, "error: {:?}", done.error);
        assert_eq!(done.progress, 100);
        let counts = done.counts.unwrap();
        assert_eq!(counts.conversations, 2);
        assert_eq!(counts.imported, 11);
        assert_eq!(
            deps.store.get_messages(ThreadKind::Group, "777", None, None).len(),
            4
        );
        assert!(
            deps.store.get_messages(ThreadKind::User, "1001", None, None).len() >= 7
        );
        assert_eq!(
            deps.store
                .list_threads()
                .iter()
                .map(|t| t.unread)
                .sum::<usize>(),
            0
        );

        // Snapshot delete called
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(deps
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(ep, p)| ep == "delete_snapshot_mobile_msg"
                && p["public_key"] == deps.key_pair.public_key_spki_b64));

        // Privacy check on logs
        let log = mgr.get_log().join("\n");
        assert!(!log.contains("hello"));
        assert!(!log.contains(&deps.key_pair.public_key_spki_b64));
        assert!(!log.contains("backup/abc"));
        assert!(log.contains("backup info: format=1 msgs=16"));
    }

    #[tokio::test]
    async fn download_retries_and_recovers() {
        let deps = MockDeps::new(KEY_TEXT);
        deps.download_fail_times.store(2, Ordering::SeqCst);
        let mgr = HistorySyncManager::new(deps.clone());

        mgr.start().await.unwrap();
        mgr.handle_control(SyncControl {
            act_type: "syncmsgmb".into(),
            act: "syncmsg_info".into(),
            data: deps.syncmsg_info_data(Map::new()),
        });

        let done = settle(&mgr, SyncState::Done).await;
        assert_eq!(done.state, SyncState::Done, "error: {:?}", done.error);
        assert_eq!(deps.downloads.lock().unwrap().len(), 3);
        assert_eq!(*deps.waits.lock().unwrap(), vec![1000, 2000]);
    }

    #[tokio::test]
    async fn znoise_maps_plain_to_noised_threads() {
        let mut deps_raw = Arc::try_unwrap(MockDeps::new(KEY_TEXT)).ok().unwrap();
        deps_raw.znoise_reply = Some(NoisedIdsReply {
            users: HashMap::from([("1001".into(), "9000000000000001001".into())]),
            groups: HashMap::from([("777".into(), "8000000000000000777".into())]),
            reply_shape: "{fids:obj(1),gids:obj(1)}".into(),
        });
        let deps = Arc::new(deps_raw);
        let mgr = HistorySyncManager::new(deps.clone());

        mgr.start().await.unwrap();
        mgr.handle_control(SyncControl {
            act_type: "syncmsgmb".into(),
            act: "syncmsg_info".into(),
            data: deps.syncmsg_info_data(Map::new()),
        });

        let done = settle(&mgr, SyncState::Done).await;
        assert_eq!(done.state, SyncState::Done, "error: {:?}", done.error);
        assert!(deps
            .store
            .get_thread(ThreadKind::User, "9000000000000001001")
            .is_some());
        assert!(deps
            .store
            .get_thread(ThreadKind::Group, "8000000000000000777")
            .is_some());
        assert!(deps.store.get_thread(ThreadKind::User, "1001").is_none());
        assert!(deps.store.get_thread(ThreadKind::Group, "777").is_none());
    }

    #[tokio::test]
    async fn cancel_aborts_and_ignores_subsequent_controls() {
        let deps = MockDeps::new(KEY_TEXT);
        let mgr = HistorySyncManager::new(deps.clone());

        mgr.start().await.unwrap();
        mgr.cancel();
        assert_eq!(mgr.get_status().state, SyncState::Idle);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(deps
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(ep, _)| ep == "cancel_pull_mobile_msg"));

        mgr.handle_control(SyncControl {
            act_type: "syncmsgmb".into(),
            act: "syncmsg_info".into(),
            data: deps.syncmsg_info_data(Map::new()),
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(mgr.get_status().state, SyncState::Idle);
        assert!(deps.downloads.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn timeout_and_error_cases() {
        // Confirm timeout
        let mut deps_raw = Arc::try_unwrap(MockDeps::new(KEY_TEXT)).ok().unwrap();
        deps_raw.timeouts.confirm_ms = 25;
        let mgr = HistorySyncManager::new(Arc::new(deps_raw));
        mgr.start().await.unwrap();
        let failed = settle(&mgr, SyncState::Failed).await;
        assert!(failed.error.unwrap_or_default().contains("did not confirm"));

        // Code 211 (sync v2)
        let mut deps_211 = Arc::try_unwrap(MockDeps::new(KEY_TEXT)).ok().unwrap();
        deps_211.call_error = Some(FileCallError {
            code: Some(211),
            message: "unsupported".into(),
        });
        let mgr_211 = HistorySyncManager::new(Arc::new(deps_211));
        let (_, st) = mgr_211.start().await.unwrap();
        assert_eq!(st.state, SyncState::Failed);
        assert!(st.error.unwrap_or_default().contains("sync v2"));

        // Wrong key fails without importing
        let deps_wk = MockDeps::new("ffeeddccbbaa99887766554433221100");
        let mgr_wk = HistorySyncManager::new(deps_wk.clone());
        mgr_wk.start().await.unwrap();
        mgr_wk.handle_control(SyncControl {
            act_type: "syncmsgmb".into(),
            act: "syncmsg_info".into(),
            data: deps_wk.syncmsg_info_data(Map::new()),
        });
        let wk_st = settle(&mgr_wk, SyncState::Failed).await;
        assert!(wk_st.error.unwrap_or_default().contains("Wrong key"));
        assert!(deps_wk.store.list_threads().is_empty());

        // Not logged in returns 409
        let mut deps_out = Arc::try_unwrap(MockDeps::new(KEY_TEXT)).ok().unwrap();
        deps_out.own_id = None;
        let mgr_out = HistorySyncManager::new(Arc::new(deps_out));
        let err = mgr_out.start().await.unwrap_err();
        assert_eq!(err.status, 409);

        // temp_key from transfer_after_login is sent on next pull_mobile_msg
        let deps_tk = MockDeps::new(KEY_TEXT);
        let mgr_tk = HistorySyncManager::new(deps_tk.clone());
        mgr_tk.handle_control(SyncControl {
            act_type: "transfer_after_login".into(),
            act: "x".into(),
            data: json!({ "temp_key": "tk-1" }).as_object().unwrap().clone(),
        });
        mgr_tk.start().await.unwrap();
        assert_eq!(deps_tk.calls.lock().unwrap()[0].1["temp_key"], "tk-1");
    }

    #[tokio::test]
    async fn axum_routes_status_start_409_and_cancel() {
        let store = ChatStore::in_memory();
        let session = Arc::new(ZaloSession::new(
            store.clone(),
            std::env::temp_dir().join("zaloo-test-hs"),
            true,
        ));
        let state = AppState {
            config: Arc::new(Config::from_lookup(|_| None).unwrap()),
            session,
            store,
        };
        let app = router(state.clone()).with_state(state);

        // GET /api/history-sync/status -> 200 idle with log array
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/history-sync/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["state"], "idle");
        assert!(body["log"].is_array());

        // POST /api/history-sync/start while logged out -> 409
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/history-sync/start")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);

        // POST /api/history-sync/cancel -> 200 idle
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/history-sync/cancel")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["state"], "idle");
    }
}
