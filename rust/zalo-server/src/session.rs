//! Thread-safe `ZaloSession` managing credentials, QR/cookie login, offline-resilient resume,
//! WebSocket listener ingestion, contact/group/poll/sticker/messaging operations, and live event
//! broadcasting (matching `src/session.ts`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone, Timelike};
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, mpsc, oneshot, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use zalo_proto::{
    build_poll_view, poll_id_of_group_event, AddPollOptionsInput, CloseReason, CreateGroupOptions,
    CreatePollInput, Credentials, DeleteMessageTarget, ForwardMessagePayload, ForwardReference,
    LastMessageInfo, LoginQrEvent, MentionInput, MuteAction, MuteDuration, PollViewContext,
    ProtoError, QuoteInput, RawWsMessage, ReactionTarget, SeenMessageInput, SendMessageOptions,
    StickerParam, ThreadType, UndoTarget, UploadResult, UploadSource, WsEvent, ZaloClient,
};

use crate::api::{ListenerControl, ListenerStart, ProtoLogin, ZaloApi, ZaloLogin};
use crate::errors::HttpError;
use crate::log::{create_dir_private, CallTrace};
use crate::store::{normalize_message, recalled_message_id, ChatStore};
use crate::types::{
    reaction_code, thread_key, ChatMessage, FoundUser, FriendRequest, GroupMember, MentionInfo,
    MessageSearchHit, Person, PollOptionView, PollView, PollVoter, SessionState, SessionStatus,
    StickerInfo, StoredMessage, ThreadInfo, ThreadKind, ThreadSummary, STICKER_IMAGE_URL,
};

/// JSON event payload broadcast to all connected `/ws` clients.
pub type ServerBroadcastEvent = Value;

const GROUP_INFO_BATCH: usize = 50;
const MEMBER_INFO_BATCH: usize = 100;
const MEMBERS_CACHE_MS: i64 = 10 * 60_000;
const SEEN_RECEIPT_LIMIT: usize = 50;
const RESUME_RETRY_MIN_MS: u64 = 5_000;
const RESUME_RETRY_MAX_MS: u64 = 60_000;
const RECONNECT_MIN_MS: u64 = 5_000;
const RECONNECT_MAX_MS: u64 = 5 * 60_000;
const SESSION_EXPIRED: &str = "Your Zalo session has ended. Log in again with the QR code.";
const STICKER_SEARCH_LIMIT: usize = 30;
const MAX_STICKERS_CACHE: usize = 500;
const MAX_GROUP_NAME: usize = 100;
const MAX_TEXT_LENGTH: usize = 4000;
const MAX_FORWARD_TARGETS: usize = 20;
const VIEWING_TTL_MS: i64 = 90_000;
const NOTIFY_MAX_AGE_MS: i64 = 10 * 60_000;
const PHOTO_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];
const AUDIO_EXTENSIONS: &[&str] = &["m4a", "aac", "mp3", "ogg", "opus", "wav", "amr", "3gp", "webm"];

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Backoff before the `attempt`-th (0-based) automatic listener reconnect, without jitter:
/// 5 s, doubling each time, capped at 5 min.
pub fn reconnect_base_delay(attempt: u32) -> Duration {
    let ms = RECONNECT_MIN_MS
        .saturating_mul(1u64 << attempt.min(16))
        .min(RECONNECT_MAX_MS);
    Duration::from_millis(ms)
}

/// [`reconnect_base_delay`] shortened by up to 10 % at random, so clients don't retry in lockstep
/// (and the cap is never exceeded).
fn reconnect_delay(attempt: u32) -> Duration {
    let base = reconnect_base_delay(attempt);
    base.mul_f64(1.0 - rand::random::<f64>() * 0.1)
}

fn to_thread_type(kind: ThreadKind) -> ThreadType {
    match kind {
        ThreadKind::User => ThreadType::User,
        ThreadKind::Group => ThreadType::Group,
    }
}

fn from_thread_type(tt: ThreadType) -> ThreadKind {
    match tt {
        ThreadType::User => ThreadKind::User,
        ThreadType::Group => ThreadKind::Group,
    }
}

/// Uploaded photo or file payload accepted by `send_attachment`.
#[derive(Debug, Clone)]
pub struct UploadFile {
    pub data: Vec<u8>,
    pub filename: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Supported mute durations matching `PUT /api/threads/:kind/:id/mute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteOption {
    OneHour,
    FourHours,
    Until8Am,
    Forever,
}

impl MuteOption {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "1h" => Some(Self::OneHour),
            "4h" => Some(Self::FourHours),
            "8am" => Some(Self::Until8Am),
            "forever" => Some(Self::Forever),
            _ => None,
        }
    }

    fn to_proto(self) -> MuteDuration {
        match self {
            Self::OneHour => MuteDuration::OneHour,
            Self::FourHours => MuteDuration::FourHours,
            Self::Until8Am => MuteDuration::Until8Am,
            Self::Forever => MuteDuration::Forever,
        }
    }
}

/// Computes when a mute set now with `option` expires in epoch milliseconds (`-1` = forever).
pub fn mute_until(option: MuteOption) -> i64 {
    match option {
        MuteOption::OneHour => now_ms() + 3_600_000,
        MuteOption::FourHours => now_ms() + 4 * 3_600_000,
        MuteOption::Forever => -1,
        MuteOption::Until8Am => {
            let now = Local::now();
            let mut target_date = now.date_naive();
            if now.hour() >= 8 {
                if let Some(next_day) = target_date.succ_opt() {
                    target_date = next_day;
                }
            }
            let naive_8am = target_date
                .and_hms_opt(8, 0, 0)
                .unwrap_or_else(|| now.naive_local());
            Local
                .from_local_datetime(&naive_8am)
                .earliest()
                .map(|dt| dt.timestamp_millis())
                .unwrap_or_else(|| {
                    // Fallback: compute next 8am offset in seconds from midnight
                    let secs = now.num_seconds_from_midnight() as i64;
                    let eight = 8 * 3600;
                    let delta = if secs < eight {
                        eight - secs
                    } else {
                        (24 * 3600 - secs) + eight
                    };
                    now_ms() + delta * 1000
                })
        }
    }
}

struct SessionInner {
    status: SessionStatus,
    client: Option<Arc<dyn ZaloApi>>,
    listener_handle: Option<Arc<dyn ListenerControl>>,
    listener_task: Option<JoinHandle<()>>,
    reconnect_task: Option<JoinHandle<()>>,
    /// Consecutive reconnect delays that have run out without a successful `Connected`.
    reconnect_attempts: u32,
    resume_task: Option<JoinHandle<()>>,
    qr_task: Option<JoinHandle<()>>,
    qr_abort_tx: Option<oneshot::Sender<()>>,
    generation: u64,
    resuming: bool,
    members: HashMap<String, (i64, Vec<GroupMember>)>,
    poll_groups: HashMap<i64, String>,
    friend_list: HashMap<String, Person>,
    stickers: HashMap<i64, StickerInfo>,
    sticker_order: VecDeque<i64>,
    viewing: Option<(String, i64)>,
    looked_up_users: HashSet<String>,
}

impl Default for SessionInner {
    fn default() -> Self {
        Self {
            status: SessionStatus::default(),
            client: None,
            listener_handle: None,
            listener_task: None,
            reconnect_task: None,
            reconnect_attempts: 0,
            resume_task: None,
            qr_task: None,
            qr_abort_tx: None,
            generation: 0,
            resuming: false,
            members: HashMap::new(),
            poll_groups: HashMap::new(),
            friend_list: HashMap::new(),
            stickers: HashMap::new(),
            sticker_order: VecDeque::new(),
            viewing: None,
            looked_up_users: HashSet::new(),
        }
    }
}

/// Manages the Zalo login lifecycle, real-time WebSocket listener, message/contact caching,
/// and all REST chat/social/group/poll operations.
pub struct ZaloSession {
    store: ChatStore,
    call_trace: CallTrace,
    data_dir: PathBuf,
    credentials_file: PathBuf,
    send_read_receipts: bool,
    inner: RwLock<SessionInner>,
    events_tx: broadcast::Sender<ServerBroadcastEvent>,
    voip_tx: broadcast::Sender<Value>,
    history_sync_tx: broadcast::Sender<Value>,
    retry_notify: Arc<Notify>,
    /// Wakes a pending listener reconnect (`notify_waiters`, so no stale permit is left behind).
    reconnect_wake: Notify,
    login: Arc<dyn ZaloLogin>,
}

impl ZaloSession {
    pub fn new(store: ChatStore, data_dir: PathBuf, send_read_receipts: bool) -> Self {
        Self::with_login(store, data_dir, send_read_receipts, Arc::new(ProtoLogin))
    }

    /// Like [`Self::new`], with a custom login backend (tests use the fake in `crate::testing`).
    pub fn with_login(
        store: ChatStore,
        data_dir: PathBuf,
        send_read_receipts: bool,
        login: Arc<dyn ZaloLogin>,
    ) -> Self {
        let credentials_file = data_dir.join("credentials.json");
        let call_trace = CallTrace::new(&data_dir);
        let (events_tx, _) = broadcast::channel(512);
        let (voip_tx, _) = broadcast::channel(128);
        let (history_sync_tx, _) = broadcast::channel(128);
        Self {
            store,
            call_trace,
            data_dir,
            credentials_file,
            send_read_receipts,
            inner: RwLock::new(SessionInner::default()),
            events_tx,
            voip_tx,
            history_sync_tx,
            retry_notify: Arc::new(Notify::new()),
            reconnect_wake: Notify::new(),
            login,
        }
    }

    pub fn store(&self) -> &ChatStore {
        &self.store
    }

    pub fn call_trace(&self) -> &CallTrace {
        &self.call_trace
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn get_status(&self) -> SessionStatus {
        self.inner
            .read()
            .map(|s| s.status.clone())
            .unwrap_or_default()
    }

    pub fn own_id(&self) -> Option<String> {
        self.inner
            .read()
            .ok()
            .and_then(|s| s.status.own_id.clone())
    }

    pub fn friend_name(&self, user_id: &str) -> Option<String> {
        self.inner
            .read()
            .ok()
            .and_then(|s| s.friend_list.get(user_id).map(|p| p.name.clone()))
    }

    /// Returns the active [`ZaloApi`], or `409 Conflict` when not logged in.
    pub fn api(&self) -> Result<Arc<dyn ZaloApi>, HttpError> {
        let s = self
            .inner
            .read()
            .map_err(|_| HttpError::internal("Session lock poisoned"))?;
        match (&s.client, s.status.state) {
            (Some(c), SessionState::Ready | SessionState::Disconnected) => Ok(Arc::clone(c)),
            _ => Err(HttpError::conflict("Not logged in")),
        }
    }

    /// Returns the active concrete [`ZaloClient`] (for the call and history-sync managers), or
    /// `409 Conflict` when not logged in or when the session runs on a non-protocol API (tests).
    pub fn client(&self) -> Result<Arc<ZaloClient>, HttpError> {
        self.api()?
            .concrete()
            .ok_or_else(|| HttpError::conflict("No protocol client"))
    }

    /// Broadcasts a JSON event payload to all connected `/ws` clients.
    pub fn broadcast_event(&self, event: Value) {
        let _ = self.events_tx.send(event);
    }

    /// Subscribes to live server broadcast events for `/ws`.
    pub fn subscribe_events(&self) -> broadcast::Receiver<ServerBroadcastEvent> {
        self.events_tx.subscribe()
    }

    /// Subscribes to raw `voip` WebSocket controls (hook for Track 1 calls).
    pub fn subscribe_voip_controls(&self) -> broadcast::Receiver<Value> {
        self.voip_tx.subscribe()
    }

    /// Subscribes to raw history-sync WebSocket controls (hook for Track 2 history sync).
    pub fn subscribe_history_sync_controls(&self) -> broadcast::Receiver<Value> {
        self.history_sync_tx.subscribe()
    }

    /// Publishes one history-sync control (`{ actType, act, data, raw }`) to subscribers of
    /// [`Self::subscribe_history_sync_controls`]. Used for controls that arrive over the control
    /// poller rather than the chat socket.
    pub fn publish_history_sync_control(&self, control: Value) {
        let _ = self.history_sync_tx.send(control);
    }

    // ---- Lifecycle: start / resume / QR login / retry / reconnect / logout / shutdown ----

    /// Attempts to resume from saved `<data_dir>/credentials.json` with offline-resilient backoff
    /// (`5s -> 10s -> 20s -> 40s -> 60s`). If no credentials exist, stays `logged_out`.
    pub fn start(self: &Arc<Self>) {
        let Some(creds) = self.read_credentials() else {
            return;
        };
        let generation = {
            let Ok(mut s) = self.inner.write() else {
                return;
            };
            if s.resuming || matches!(s.status.state, SessionState::Ready | SessionState::Disconnected) {
                return;
            }
            s.resuming = true;
            s.generation
        };
        let session = Arc::clone(self);
        let handle = tokio::spawn(async move {
            session.resume_loop(creds, generation).await;
        });
        if let Ok(mut s) = self.inner.write() {
            if s.generation == generation {
                s.resume_task = Some(handle);
            } else {
                handle.abort();
            }
        }
    }

    async fn resume_loop(self: &Arc<Self>, creds: Credentials, generation: u64) {
        let mut delay_ms = RESUME_RETRY_MIN_MS;
        loop {
            if !self.is_generation(generation) {
                break;
            }
            self.patch_status(|st| {
                st.state = SessionState::LoggingIn;
                st.qr_image = None;
                st.scanned_by = None;
                st.error = None;
                st.retrying = false;
            });

            match self.login.login_cookie(creds.clone()).await {
                Ok(client) => {
                    if self.is_generation(generation) {
                        self.on_logged_in(client, generation).await;
                    }
                    break;
                }
                Err(err) => {
                    if !self.is_generation(generation) {
                        break;
                    }
                    if !is_transient_proto_error(&err) {
                        warn!("Saved Zalo session rejected: {err}");
                        self.patch_status(|st| {
                            st.state = SessionState::LoggedOut;
                            st.qr_image = None;
                            st.scanned_by = None;
                            st.retrying = false;
                            st.error = Some(SESSION_EXPIRED.to_string());
                        });
                        break;
                    }
                    let secs = (delay_ms + 500) / 1000;
                    let desc = describe_proto_error(&err);
                    warn!("Can't reach Zalo ({desc}); retrying in {secs}s");
                    self.patch_status(|st| {
                        st.state = SessionState::LoggingIn;
                        st.retrying = true;
                        st.error = Some(format!("Can't reach Zalo ({desc}). Retrying in {secs} s…"));
                    });
                    let woken = tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => false,
                        _ = self.retry_notify.notified() => true,
                    };
                    if !self.is_generation(generation) {
                        break;
                    }
                    if !woken {
                        delay_ms = (delay_ms * 2).min(RESUME_RETRY_MAX_MS);
                    }
                }
            }
        }

        if let Ok(mut s) = self.inner.write() {
            if s.generation == generation {
                s.resuming = false;
                s.resume_task = None;
                if s.status.retrying {
                    s.status.retrying = false;
                    let snapshot = s.status.clone();
                    drop(s);
                    self.broadcast_status(&snapshot);
                }
            }
        }
    }

    /// Wakes a waiting offline-resume loop and a pending listener reconnect immediately, so they
    /// retry without waiting for backoff (the app calls this when the network comes back).
    pub fn retry_now(&self) {
        self.retry_notify.notify_one();
        self.reconnect_wake.notify_waiters();
    }

    /// Alias for [`Self::retry_now`].
    pub fn retry(&self) {
        self.retry_now();
    }

    /// Starts a new QR code login, aborting any prior QR login or offline resume attempt.
    pub fn start_qr_login(self: &Arc<Self>) {
        let generation = {
            let Ok(mut s) = self.inner.write() else {
                return;
            };
            if matches!(s.status.state, SessionState::Ready | SessionState::Disconnected) {
                return;
            }
            // Abort any prior QR attempt or offline resume loop
            s.generation = s.generation.wrapping_add(1);
            s.resuming = false;
            if let Some(tx) = s.qr_abort_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = s.qr_task.take() {
                task.abort();
            }
            if let Some(task) = s.resume_task.take() {
                task.abort();
            }
            if let Some(task) = s.reconnect_task.take() {
                task.abort();
            }
            s.reconnect_attempts = 0;
            s.status.state = SessionState::LoggingIn;
            s.status.qr_image = None;
            s.status.scanned_by = None;
            s.status.error = None;
            s.status.retrying = false;
            let snapshot = s.status.clone();
            let gen = s.generation;
            drop(s);
            self.retry_notify.notify_one();
            self.broadcast_status(&snapshot);
            gen
        };

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<LoginQrEvent>();
        let (abort_tx, abort_rx) = oneshot::channel::<()>();

        let session_events = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(ev) = event_rx.recv().await {
                if !session_events.is_generation(generation) {
                    break;
                }
                match ev {
                    LoginQrEvent::QrCodeGenerated { image_base64, .. } => {
                        let data_url = if image_base64.starts_with("data:") {
                            image_base64
                        } else {
                            format!("data:image/png;base64,{image_base64}")
                        };
                        session_events.patch_status(|st| {
                            st.state = SessionState::AwaitingQrScan;
                            st.qr_image = Some(data_url);
                            st.scanned_by = None;
                            st.error = None;
                        });
                    }
                    LoginQrEvent::QrCodeScanned { display_name, .. } => {
                        session_events.patch_status(|st| {
                            st.scanned_by = Some(display_name);
                        });
                    }
                    LoginQrEvent::QrCodeExpired => {
                        debug!("QR code expired; awaiting fresh code from login_qr");
                    }
                    LoginQrEvent::QrCodeDeclined => {
                        session_events.patch_status(|st| {
                            st.state = SessionState::LoggedOut;
                            st.qr_image = None;
                            st.scanned_by = None;
                            st.error = Some("Login was declined on the phone. Try again.".to_string());
                        });
                    }
                    LoginQrEvent::GotLoginInfo(creds) => {
                        if let Err(e) = session_events.write_credentials(&creds) {
                            warn!("Failed to persist credentials.json: {e}");
                        }
                        session_events.patch_status(|st| {
                            st.state = SessionState::LoggingIn;
                            st.qr_image = None;
                        });
                    }
                }
            }
        });

        let session = Arc::clone(self);
        let task = tokio::spawn(async move {
            match session.login.login_qr(event_tx, abort_rx).await {
                Ok((creds, client)) => {
                    if !session.is_generation(generation) {
                        return;
                    }
                    if let Err(e) = session.write_credentials(&creds) {
                        warn!("Failed to persist credentials.json: {e}");
                    }
                    session.on_logged_in(client, generation).await;
                }
                Err(ProtoError::QrDeclined) => {
                    if session.is_generation(generation) {
                        session.patch_status(|st| {
                            st.state = SessionState::LoggedOut;
                            st.qr_image = None;
                            st.scanned_by = None;
                            st.error = Some("Login was declined on the phone. Try again.".to_string());
                        });
                    }
                }
                Err(ProtoError::QrAborted) => {
                    if session.is_generation(generation) {
                        session.patch_status(|st| {
                            st.state = SessionState::LoggedOut;
                            st.qr_image = None;
                            st.scanned_by = None;
                            st.error = Some("Login was cancelled.".to_string());
                        });
                    }
                }
                Err(e) => {
                    if session.is_generation(generation) {
                        session.patch_status(|st| {
                            st.state = SessionState::LoggedOut;
                            st.qr_image = None;
                            st.scanned_by = None;
                            st.error = Some(describe_proto_error(&e));
                        });
                    }
                }
            }
            if let Ok(mut s) = session.inner.write() {
                if s.generation == generation {
                    s.qr_abort_tx = None;
                    s.qr_task = None;
                }
            }
        });

        if let Ok(mut s) = self.inner.write() {
            if s.generation == generation {
                s.qr_abort_tx = Some(abort_tx);
                s.qr_task = Some(task);
            } else {
                let _ = abort_tx.send(());
                task.abort();
            }
        }
    }

    /// Logs out: stops background tasks/listener, deletes `credentials.json`, clears store & call trace.
    pub async fn logout(&self) {
        {
            if let Ok(mut s) = self.inner.write() {
                s.generation = s.generation.wrapping_add(1);
                s.resuming = false;
                if let Some(tx) = s.qr_abort_tx.take() {
                    let _ = tx.send(());
                }
                if let Some(t) = s.qr_task.take() {
                    t.abort();
                }
                if let Some(t) = s.resume_task.take() {
                    t.abort();
                }
                if let Some(h) = s.listener_handle.take() {
                    h.stop();
                }
                if let Some(t) = s.listener_task.take() {
                    t.abort();
                }
                if let Some(t) = s.reconnect_task.take() {
                    t.abort();
                }
                s.reconnect_attempts = 0;
                s.client = None;
                s.members.clear();
                s.poll_groups.clear();
                s.friend_list.clear();
                s.stickers.clear();
                s.sticker_order.clear();
                s.viewing = None;
                s.looked_up_users.clear();
                s.status = SessionStatus::default();
            }
        }
        self.retry_notify.notify_one();
        self.reconnect_wake.notify_waiters();
        self.call_trace.clear();
        self.store.set_own_id(None);
        let _ = fs::remove_file(&self.credentials_file);
        self.store.clear();
        self.store.flush();
        self.broadcast_status(&SessionStatus::default());
    }

    /// Stops background tasks and WebSocket listener without deleting credentials or message cache.
    pub fn shutdown(&self) {
        if let Ok(mut s) = self.inner.write() {
            s.generation = s.generation.wrapping_add(1);
            s.resuming = false;
            if let Some(tx) = s.qr_abort_tx.take() {
                let _ = tx.send(());
            }
            if let Some(t) = s.qr_task.take() {
                t.abort();
            }
            if let Some(t) = s.resume_task.take() {
                t.abort();
            }
            if let Some(h) = s.listener_handle.take() {
                h.stop();
            }
            if let Some(t) = s.listener_task.take() {
                t.abort();
            }
            if let Some(t) = s.reconnect_task.take() {
                t.abort();
            }
            s.reconnect_attempts = 0;
            s.client = None;
        }
        self.retry_notify.notify_one();
        self.reconnect_wake.notify_waiters();
    }

    /// Restarts the real-time WebSocket listener now (returns `409 Conflict` if not logged in).
    /// Cancels any pending automatic reconnect and resets its backoff.
    pub fn reconnect(self: &Arc<Self>) -> Result<(), HttpError> {
        let (client, generation) = {
            let mut s = self
                .inner
                .write()
                .map_err(|_| HttpError::internal("Session lock poisoned"))?;
            let Some(c) = s.client.as_ref() else {
                return Err(HttpError::conflict("Not logged in"));
            };
            let client = Arc::clone(c);
            if let Some(t) = s.reconnect_task.take() {
                t.abort();
            }
            s.reconnect_attempts = 0;
            (client, s.generation)
        };
        self.start_listener(client, generation)
            .map_err(proto_to_http_error)
    }

    // ---- Internal login completion & listener ----

    async fn on_logged_in(self: &Arc<Self>, client: Arc<dyn ZaloApi>, generation: u64) {
        if !self.is_generation(generation) {
            return;
        }
        let own_id = client.own_id();
        self.store.set_own_id(Some(own_id.clone()));

        if let Ok(mut s) = self.inner.write() {
            if s.generation != generation {
                return;
            }
            s.client = Some(Arc::clone(&client));
        }

        let mut own_name: Option<String> = None;
        let mut own_avatar: Option<String> = None;
        if let Ok(acc) = client.fetch_account_info().await {
            let profile = acc.get("profile").unwrap_or(&acc);
            own_name = non_empty_str(profile.get("displayName"))
                .or_else(|| non_empty_str(profile.get("zaloName")));
            own_avatar = non_empty_str(profile.get("avatar"));
        }

        if !self.is_generation_and_client(generation, &client) {
            return;
        }

        self.patch_status(|st| {
            st.state = SessionState::Ready;
            st.own_id = Some(own_id.clone());
            st.own_name = own_name;
            st.own_avatar = own_avatar;
            st.qr_image = None;
            st.scanned_by = None;
            st.error = None;
            st.retrying = false;
        });

        if let Err(err) = self.start_listener(Arc::clone(&client), generation) {
            warn!("Failed to start Zalo WebSocket listener: {err}");
            self.patch_status(|st| {
                st.state = SessionState::Disconnected;
                st.error = Some(describe_proto_error(&err));
            });
        }

        let session_contacts = Arc::clone(self);
        let client_contacts = Arc::clone(&client);
        tokio::spawn(async move {
            session_contacts.load_contacts(client_contacts, generation).await;
        });
    }

    fn start_listener(
        self: &Arc<Self>,
        client: Arc<dyn ZaloApi>,
        generation: u64,
    ) -> zalo_proto::Result<()> {
        let ListenerStart {
            events: mut event_rx,
            control: handle,
        } = client.start_listener()?;

        // Stop any previous listener before installing the new one
        if let Ok(mut s) = self.inner.write() {
            if s.generation != generation {
                handle.stop();
                return Ok(());
            }
            if let Some(old_h) = s.listener_handle.take() {
                old_h.stop();
            }
            if let Some(old_t) = s.listener_task.take() {
                old_t.abort();
            }
            s.listener_handle = Some(Arc::clone(&handle));
        }

        let session = Arc::clone(self);
        let client_loop = Arc::clone(&client);
        let handle_loop = Arc::clone(&handle);
        let task = tokio::spawn(async move {
            while let Some(ev) = event_rx.recv().await {
                if !session.is_generation_and_client(generation, &client_loop) {
                    break;
                }
                session
                    .handle_ws_event(ev, &client_loop, &handle_loop, generation)
                    .await;
            }
        });

        if let Ok(mut s) = self.inner.write() {
            if s.generation == generation {
                s.listener_task = Some(task);
            } else {
                task.abort();
            }
        }
        Ok(())
    }

    async fn handle_ws_event(
        self: &Arc<Self>,
        event: WsEvent,
        client: &Arc<dyn ZaloApi>,
        handle: &Arc<dyn ListenerControl>,
        generation: u64,
    ) {
        match event {
            WsEvent::Connected => {
                info!("Zalo WebSocket listener connected");
                self.call_trace.record(&json!({ "kind": "listening" }));
                if let Ok(mut s) = self.inner.write() {
                    s.reconnect_attempts = 0;
                }
                self.patch_status(|st| {
                    st.state = SessionState::Ready;
                    st.error = None;
                });
                handle.request_old_messages(ThreadType::User);
                handle.request_old_messages(ThreadType::Group);
                handle.request_old_reactions(ThreadType::User);
                handle.request_old_reactions(ThreadType::Group);
            }
            WsEvent::CipherKey(_) => {
                debug!("Received Zalo WebSocket cipher key");
            }
            WsEvent::Messages(msgs) => {
                for raw in msgs {
                    self.ingest_raw(raw, false, generation);
                }
            }
            WsEvent::OldMessages { messages, .. } => {
                for raw in messages {
                    self.ingest_raw(raw, true, generation);
                }
            }
            WsEvent::Undo(undo) => {
                let global_id = undo
                    .data
                    .get("content")
                    .and_then(|c| c.get("globalMsgId"))
                    .map(val_to_string)
                    .unwrap_or_default();
                if !global_id.is_empty() {
                    if let Some(updated) = self.store.apply_undo(&global_id) {
                        self.broadcast_message_updated(&updated);
                    }
                }
            }
            WsEvent::Reactions { reactions, .. } => {
                for r in reactions {
                    self.handle_reaction_event(&r);
                }
            }
            WsEvent::Typing {
                thread_type,
                thread_id,
                uid,
                ..
            } => {
                let kind = from_thread_type(thread_type);
                let name = match kind {
                    ThreadKind::User => self
                        .store
                        .get_thread(ThreadKind::User, &uid)
                        .map(|t| t.name)
                        .unwrap_or_default(),
                    ThreadKind::Group => self
                        .inner
                        .read()
                        .ok()
                        .and_then(|s| {
                            s.members
                                .get(&thread_id)
                                .and_then(|(_, list)| list.iter().find(|m| m.id == uid))
                                .map(|m| m.name.clone())
                        })
                        .or_else(|| self.store.get_thread(ThreadKind::User, &uid).map(|t| t.name))
                        .unwrap_or_default(),
                };
                self.broadcast_event(json!({
                    "type": "typing",
                    "threadKind": kind,
                    "threadId": thread_id,
                    "uid": uid,
                    "name": name,
                }));
            }
            WsEvent::Delivered { thread_type, data } => {
                self.handle_delivered_event(thread_type, &data);
            }
            WsEvent::Seen { thread_type, data } => {
                self.handle_seen_event(thread_type, &data);
            }
            WsEvent::GroupEvent(ev) => {
                let thread_id = ev
                    .get("threadId")
                    .map(val_to_string)
                    .unwrap_or_default();
                if thread_id.is_empty() {
                    return;
                }
                if let Some(poll_id) = poll_id_of_group_event(&ev) {
                    if let Ok(mut s) = self.inner.write() {
                        s.poll_groups.insert(poll_id, thread_id.clone());
                    }
                    self.broadcast_event(json!({
                        "type": "poll",
                        "pollId": poll_id,
                    }));
                }
                if let Ok(mut s) = self.inner.write() {
                    s.members.remove(&thread_id);
                }
                let session = Arc::clone(self);
                let c = Arc::clone(client);
                tokio::spawn(async move {
                    session.load_groups(&c, &[thread_id], generation).await;
                    if session.is_generation_and_client(generation, &c) {
                        session.broadcast_event(json!({ "type": "threads" }));
                    }
                });
            }
            WsEvent::FriendEvent(_) => {
                debug!("Received Zalo friend event");
            }
            WsEvent::VoipControl {
                cmd,
                sub_cmd,
                control,
            } => {
                let act = control
                    .get("act")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.call_trace.record(&json!({
                    "kind": "control",
                    "source": "socket",
                    "cmd": cmd,
                    "subCmd": sub_cmd,
                    "act": act,
                    "control": control,
                }));
                let _ = self.voip_tx.send(control);
            }
            WsEvent::HistorySyncControl {
                cmd,
                sub_cmd,
                control,
            } => {
                debug!(cmd, sub_cmd, act = ?control.get("act"), "Received history sync control");
                let _ = self.history_sync_tx.send(control);
            }
            WsEvent::RawFrame { cmd, sub_cmd, .. } => {
                debug!(cmd, sub_cmd, "Unhandled raw WebSocket frame");
            }
            WsEvent::Closed { code, reason } => {
                if code == CloseReason::ManualClosure as u16 {
                    return;
                }
                let kicked = code == CloseReason::DuplicateConnection as u16
                    || code == CloseReason::KickConnection as u16;
                let error = if kicked {
                    "Zalo Web was opened somewhere else. Only one web session can listen at a time."
                        .to_string()
                } else if reason.is_empty() {
                    format!("Connection closed ({code})")
                } else {
                    format!("Connection closed ({code}: {reason})")
                };
                warn!(code, %reason, kicked, "Zalo WebSocket listener closed");
                self.patch_status(|st| {
                    st.state = SessionState::Disconnected;
                    st.error = Some(error);
                });
                if !kicked {
                    self.schedule_reconnect(Arc::clone(client), generation);
                }
            }
            WsEvent::Error(err) => {
                warn!("Zalo listener error: {err}");
            }
        }
    }

    /// Schedules the one automatic listener reconnect that follows an unexpected close: waits the
    /// current backoff delay (or until [`Self::retry_now`]), then restarts the listener. Replaces
    /// any reconnect already pending, so at most one such task exists; logout, a new login and a
    /// manual [`Self::reconnect`] cancel it. The backoff resets on the next `Connected`.
    fn schedule_reconnect(self: &Arc<Self>, client: Arc<dyn ZaloApi>, generation: u64) {
        let session = Arc::clone(self);
        let task_client = Arc::clone(&client);
        let task = tokio::spawn(async move {
            session.reconnect_loop(task_client, generation).await;
        });
        if let Ok(mut s) = self.inner.write() {
            if s.generation == generation {
                if let Some(old) = s.reconnect_task.replace(task) {
                    old.abort();
                }
                return;
            }
        }
        task.abort();
    }

    async fn reconnect_loop(self: &Arc<Self>, client: Arc<dyn ZaloApi>, generation: u64) {
        loop {
            let attempt = self
                .inner
                .read()
                .map(|s| s.reconnect_attempts)
                .unwrap_or(0);
            let delay = reconnect_delay(attempt);
            // Created before sleeping so a wake in between is not lost.
            let wake = self.reconnect_wake.notified();
            let woken = tokio::select! {
                _ = tokio::time::sleep(delay) => false,
                _ = wake => true,
            };
            if !self.is_generation_and_client(generation, &client)
                || self.get_status().state != SessionState::Disconnected
            {
                return;
            }
            if !woken {
                if let Ok(mut s) = self.inner.write() {
                    s.reconnect_attempts = s.reconnect_attempts.saturating_add(1);
                }
            }
            info!(attempt, woken, "Reconnecting Zalo WebSocket listener");
            match self.start_listener(Arc::clone(&client), generation) {
                Ok(()) => {
                    if let Ok(mut s) = self.inner.write() {
                        if s.generation == generation {
                            s.reconnect_task = None;
                        }
                    }
                    return;
                }
                Err(err) => warn!("Could not restart the Zalo WebSocket listener: {err}"),
            }
        }
    }

    fn ingest_raw(self: &Arc<Self>, raw: RawWsMessage, is_old: bool, generation: u64) {
        if let Some(recalled_id) = recalled_message_id(&raw.data) {
            if let Some(updated) = self.store.apply_undo(&recalled_id) {
                self.broadcast_message_updated(&updated);
            }
            return;
        }
        let kind = from_thread_type(raw.thread_type);
        let own_id = self.own_id();
        let msg = normalize_message(&raw.data, kind, &raw.thread_id, raw.is_self, own_id.as_deref());

        if let Some(ref poll) = msg.attachment.as_ref().and_then(|a| a.poll.as_ref()) {
            if msg.thread_kind == ThreadKind::Group {
                if let Ok(mut s) = self.inner.write() {
                    s.poll_groups.insert(poll.poll_id, msg.thread_id.clone());
                }
            }
            if poll.event.is_some() {
                self.broadcast_event(json!({
                    "type": "poll",
                    "pollId": poll.poll_id,
                }));
            }
        }

        if self.store.add_message(msg.clone()) {
            let public_msg = self.store.to_public(&msg);
            let notify = self.should_notify(&public_msg, is_old);
            let mut payload = json!({
                "type": "message",
                "message": public_msg,
                "notify": notify,
            });
            if notify {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(
                        "threadName".to_string(),
                        Value::String(self.thread_name(msg.thread_kind, &msg.thread_id)),
                    );
                }
            }
            self.broadcast_event(payload);

            if msg.thread_kind == ThreadKind::User {
                let thread = self.store.get_thread(ThreadKind::User, &msg.thread_id);
                if thread.as_ref().is_none_or(|t| t.name.is_empty() || t.name == t.id) {
                    self.resolve_user_thread(msg.thread_id, generation);
                }
            }
        }
    }

    fn resolve_user_thread(self: &Arc<Self>, user_id: String, generation: u64) {
        if user_id.is_empty() || user_id == "0" {
            return;
        }
        let client = {
            let Ok(mut s) = self.inner.write() else {
                return;
            };
            if s.generation != generation || !s.looked_up_users.insert(user_id.clone()) {
                return;
            }
            match s.client.as_ref() {
                Some(c) => Arc::clone(c),
                None => return,
            }
        };

        let session = Arc::clone(self);
        tokio::spawn(async move {
            let Ok(res) = client.get_user_info(&[user_id], None).await else {
                return;
            };
            if !session.is_generation_and_client(generation, &client) {
                return;
            }
            let mut changed = false;
            for key in ["changed_profiles", "unchanged_profiles"] {
                if let Some(map) = res.get(key).and_then(|v| v.as_object()) {
                    for (raw_id, prof) in map {
                        let name = non_empty_str(prof.get("displayName"))
                            .or_else(|| non_empty_str(prof.get("zaloName")));
                        let avatar = non_empty_str(prof.get("avatar")).unwrap_or_default();
                        if name.is_none() && avatar.is_empty() {
                            continue;
                        }
                        let clean_id = raw_id.split('_').next().unwrap_or(raw_id).to_string();
                        session.store.upsert_thread(ThreadInfo {
                            id: clean_id.clone(),
                            kind: ThreadKind::User,
                            name: name.unwrap_or(clean_id),
                            avatar,
                        });
                        changed = true;
                    }
                }
            }
            if changed {
                session.broadcast_event(json!({ "type": "threads" }));
            }
        });
    }

    fn handle_reaction_event(&self, r: &Value) {
        let Some(data) = r.get("data") else {
            return;
        };
        let uid_from = data.get("uidFrom").map(val_to_string).unwrap_or_default();
        let Some(content) = data.get("content") else {
            return;
        };
        let r_icon = content
            .get("rIcon")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Some(arr) = content.get("rMsg").and_then(|v| v.as_array()) {
            for target in arr {
                let g_msg_id = target
                    .get("gMsgID")
                    .or_else(|| target.get("gMsgId"))
                    .map(val_to_string)
                    .unwrap_or_default();
                if !g_msg_id.is_empty() {
                    if let Some(updated) = self.store.apply_reaction(&g_msg_id, &uid_from, r_icon) {
                        self.broadcast_message_updated(&updated);
                    }
                }
            }
        }
    }

    fn handle_delivered_event(&self, thread_type: ThreadType, data: &Value) {
        let Some(arr) = data.as_array() else {
            return;
        };
        let kind = from_thread_type(thread_type);
        let own_id = self.own_id();
        for item in arr {
            let msg_id = item
                .get("msgId")
                .or_else(|| item.get("realMsgId"))
                .map(val_to_string)
                .unwrap_or_default();
            if msg_id.is_empty() {
                continue;
            }
            let delivered = extract_uids_excluding(item.get("deliveredUids"), own_id.as_deref());
            let seen = extract_uids_excluding(item.get("seenUids"), own_id.as_deref());
            let thread_id = match kind {
                ThreadKind::Group => item
                    .get("groupId")
                    .or_else(|| item.get("grid"))
                    .or_else(|| item.get("idTo"))
                    .map(val_to_string)
                    .filter(|s| !s.is_empty()),
                ThreadKind::User => item
                    .get("idTo")
                    .or_else(|| item.get("uidFrom"))
                    .map(val_to_string)
                    .filter(|s| !s.is_empty() && s != "0" && own_id.as_deref() != Some(s.as_str()))
                    .or_else(|| delivered.first().cloned()),
            }
            .or_else(|| self.store.find_message(&msg_id).map(|m| m.thread_id));
            if let Some(tid) = thread_id {
                for m in self.store.apply_receipt(kind, &tid, &msg_id, &delivered, &seen) {
                    self.broadcast_message_updated(&m);
                }
            }
        }
    }

    fn handle_seen_event(&self, thread_type: ThreadType, data: &Value) {
        let Some(arr) = data.as_array() else {
            return;
        };
        let own_id = self.own_id();
        for item in arr {
            let msg_id = item
                .get("msgId")
                .or_else(|| item.get("realMsgId"))
                .map(val_to_string)
                .unwrap_or_default();
            match thread_type {
                ThreadType::Group => {
                    let thread_id = item
                        .get("groupId")
                        .or_else(|| item.get("grid"))
                        .or_else(|| item.get("idTo"))
                        .map(val_to_string)
                        .filter(|s| !s.is_empty())
                        .or_else(|| self.store.find_message(&msg_id).map(|m| m.thread_id));
                    let Some(tid) = thread_id else {
                        continue;
                    };
                    let raw_seen: Vec<String> = item
                        .get("seenUids")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().map(val_to_string).collect())
                        .unwrap_or_default();
                    let is_self = item
                        .get("isSelf")
                        .and_then(|v| v.as_bool())
                        .unwrap_or_else(|| {
                            own_id
                                .as_deref()
                                .is_some_and(|oid| raw_seen.iter().any(|u| u == oid))
                        });
                    let seen_by: Vec<String> = raw_seen
                        .into_iter()
                        .filter(|u| own_id.as_deref() != Some(u.as_str()))
                        .collect();
                    if !msg_id.is_empty() && !seen_by.is_empty() {
                        for m in self.store.apply_seen(ThreadKind::Group, &tid, &msg_id, &seen_by) {
                            self.broadcast_message_updated(&m);
                        }
                    }
                    if is_self {
                        if let Some(msg) = self.store.find_message(&msg_id) {
                            self.store.mark_read(ThreadKind::Group, &tid, Some(msg.ts));
                            self.broadcast_event(json!({
                                "type": "read",
                                "threadKind": ThreadKind::Group,
                                "threadId": tid,
                            }));
                            self.broadcast_event(json!({ "type": "threads" }));
                        }
                    }
                }
                ThreadType::User => {
                    let thread_id = item
                        .get("idTo")
                        .or_else(|| item.get("uidFrom"))
                        .map(val_to_string)
                        .filter(|s| !s.is_empty() && s != "0")
                        .or_else(|| self.store.find_message(&msg_id).map(|m| m.thread_id));
                    let Some(tid) = thread_id else {
                        continue;
                    };
                    let is_self = item
                        .get("isSelf")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if is_self {
                        let ts = self
                            .store
                            .find_message(&msg_id)
                            .map(|m| m.ts)
                            .unwrap_or_else(now_ms);
                        self.store.mark_read(ThreadKind::User, &tid, Some(ts));
                        self.broadcast_event(json!({
                            "type": "read",
                            "threadKind": ThreadKind::User,
                            "threadId": tid,
                        }));
                        self.broadcast_event(json!({ "type": "threads" }));
                    } else if !msg_id.is_empty() {
                        for m in self
                            .store
                            .apply_seen(ThreadKind::User, &tid, &msg_id, std::slice::from_ref(&tid))
                        {
                            self.broadcast_message_updated(&m);
                        }
                    }
                }
            }
        }
    }

    async fn load_contacts(&self, client: Arc<dyn ZaloApi>, generation: u64) {
        if let Err(err) = self.load_friends(&client, generation).await {
            warn!("Could not load friends: {err}");
        }
        if !self.is_generation_and_client(generation, &client) {
            return;
        }
        if let Ok(all_groups) = client.get_all_groups().await {
            if !self.is_generation_and_client(generation, &client) {
                return;
            }
            if let Some(map) = all_groups.get("gridVerMap").and_then(|v| v.as_object()) {
                let ids: Vec<String> = map.keys().cloned().collect();
                self.load_groups(&client, &ids, generation).await;
            }
        }
        if !self.is_generation_and_client(generation, &client) {
            return;
        }
        let _ = self.resolve_unnamed_threads().await;
        if !self.is_generation_and_client(generation, &client) {
            return;
        }

        // Pinned conversations
        match client.get_pin_conversations().await {
            Ok(pins) => {
                if !self.is_generation_and_client(generation, &client) {
                    return;
                }
                let keys: Vec<String> = pins
                    .get("conversations")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_str())
                            .filter(|s| s.len() > 1)
                            .map(|c| {
                                let kind = if c.starts_with('g') {
                                    ThreadKind::Group
                                } else {
                                    ThreadKind::User
                                };
                                thread_key(kind, &c[1..])
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.store.set_pinned_set(keys);
            }
            Err(err) => warn!("Could not load pinned chats: {err}"),
        }

        // Muted conversations
        match client.get_mute().await {
            Ok(mute) => {
                if !self.is_generation_and_client(generation, &client) {
                    return;
                }
                let mut entries: Vec<(String, i64)> = Vec::new();
                for (kind, field) in [
                    (ThreadKind::User, "chatEntries"),
                    (ThreadKind::Group, "groupChatEntries"),
                ] {
                    if let Some(list) = mute.get(field).and_then(|v| v.as_array()) {
                        for e in list {
                            let id = e.get("id").map(val_to_string).unwrap_or_default();
                            if id.is_empty() {
                                continue;
                            }
                            let duration = val_to_i64(e.get("duration"));
                            let start_time = val_to_i64(e.get("startTime"));
                            let until = if duration == -1 {
                                -1
                            } else {
                                (start_time + duration) * 1000
                            };
                            entries.push((thread_key(kind, &id), until));
                        }
                    }
                }
                self.store.set_mutes(entries);
            }
            Err(err) => warn!("Could not load muted chats: {err}"),
        }

        if self.is_generation_and_client(generation, &client) {
            self.broadcast_event(json!({ "type": "threads" }));
        }
    }

    async fn load_friends(&self, client: &Arc<dyn ZaloApi>, generation: u64) -> Result<(), ProtoError> {
        let raw = client.get_all_friends(None, None, None).await?;
        if !self.is_generation_and_client(generation, client) {
            return Ok(());
        }
        let arr = match raw {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        let mut map = HashMap::new();
        for f in arr {
            let id = f.get("userId").map(val_to_string).unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let name = non_empty_str(f.get("displayName"))
                .or_else(|| non_empty_str(f.get("zaloName")))
                .unwrap_or_else(|| id.clone());
            let avatar = non_empty_str(f.get("avatar")).unwrap_or_default();
            self.store.upsert_thread(ThreadInfo {
                id: id.clone(),
                kind: ThreadKind::User,
                name: name.clone(),
                avatar: avatar.clone(),
            });
            map.insert(id.clone(), Person { id, name, avatar });
        }
        if let Ok(mut s) = self.inner.write() {
            if s.generation == generation {
                s.friend_list = map;
            }
        }
        Ok(())
    }

    async fn load_groups(&self, client: &Arc<dyn ZaloApi>, group_ids: &[String], generation: u64) {
        for chunk in group_ids.chunks(GROUP_INFO_BATCH) {
            let Ok(info) = client.get_group_info(chunk).await else {
                continue;
            };
            if !self.is_generation_and_client(generation, client) {
                return;
            }
            if let Some(grid_map) = info.get("gridInfoMap").and_then(|v| v.as_object()) {
                for (id, g) in grid_map {
                    let name = non_empty_str(g.get("name")).unwrap_or_default();
                    let avatar = non_empty_str(g.get("avt")).unwrap_or_default();
                    self.store.upsert_thread(ThreadInfo {
                        id: id.clone(),
                        kind: ThreadKind::Group,
                        name,
                        avatar,
                    });
                }
            }
        }
    }

    /// Resolves unnamed threads (migrating plain mobile IDs via `/api/znoise` and looking up profiles).
    pub async fn resolve_unnamed_threads(&self) -> bool {
        let Ok(client) = self.api() else {
            return false;
        };
        let generation = self.inner.read().map(|s| s.generation).unwrap_or(0);
        let mut changed = false;

        let initial_unnamed = self.store.unnamed_threads();
        let plain_users: Vec<String> = initial_unnamed
            .iter()
            .filter(|t| {
                t.kind == ThreadKind::User
                    && is_plain_numeric_id(&t.id)
                    && self.store.has_only_history(ThreadKind::User, &t.id)
            })
            .map(|t| t.id.clone())
            .collect();
        let plain_groups: Vec<String> = initial_unnamed
            .iter()
            .filter(|t| {
                t.kind == ThreadKind::Group
                    && is_plain_numeric_id(&t.id)
                    && self.store.has_only_history(ThreadKind::Group, &t.id)
            })
            .map(|t| t.id.clone())
            .collect();

        if !plain_users.is_empty() || !plain_groups.is_empty() {
            let max_len = plain_users.len().max(plain_groups.len());
            for i in (0..max_len).step_by(100) {
                let u = if i < plain_users.len() {
                    &plain_users[i..(i + 100).min(plain_users.len())]
                } else {
                    &[]
                };
                let g = if i < plain_groups.len() {
                    &plain_groups[i..(i + 100).min(plain_groups.len())]
                } else {
                    &[]
                };
                if u.is_empty() && g.is_empty() {
                    continue;
                }
                if let Ok(res) = client.get_znoise(u, g).await {
                    if !self.is_generation_and_client(generation, &client) {
                        return changed;
                    }
                    for (plain, noised) in res.users {
                        if self.store.migrate_plain_thread(ThreadKind::User, &plain, &noised) {
                            changed = true;
                        }
                    }
                    for (plain, noised) in res.groups {
                        if self.store.migrate_plain_thread(ThreadKind::Group, &plain, &noised) {
                            changed = true;
                        }
                    }
                }
            }
        }

        let unnamed = self.store.unnamed_threads();
        let users: Vec<String> = unnamed
            .iter()
            .filter(|t| t.kind == ThreadKind::User)
            .map(|t| t.id.clone())
            .collect();
        for chunk in users.chunks(GROUP_INFO_BATCH) {
            if let Ok(res) = client.get_user_info(chunk, None).await {
                if !self.is_generation_and_client(generation, &client) {
                    return changed;
                }
                for key in ["changed_profiles", "unchanged_profiles"] {
                    if let Some(map) = res.get(key).and_then(|v| v.as_object()) {
                        for (raw_id, p) in map {
                            let Some(name) = non_empty_str(p.get("displayName"))
                                .or_else(|| non_empty_str(p.get("zaloName")))
                            else {
                                continue;
                            };
                            let clean_id = raw_id.split('_').next().unwrap_or(raw_id).to_string();
                            let avatar = non_empty_str(p.get("avatar")).unwrap_or_default();
                            self.store.upsert_thread(ThreadInfo {
                                id: clean_id,
                                kind: ThreadKind::User,
                                name,
                                avatar,
                            });
                            changed = true;
                        }
                    }
                }
            }
        }

        let groups: Vec<String> = unnamed
            .iter()
            .filter(|t| t.kind == ThreadKind::Group)
            .map(|t| t.id.clone())
            .collect();
        for chunk in groups.chunks(GROUP_INFO_BATCH) {
            if let Ok(info) = client.get_group_info(chunk).await {
                if !self.is_generation_and_client(generation, &client) {
                    return changed;
                }
                if let Some(grid_map) = info.get("gridInfoMap").and_then(|v| v.as_object()) {
                    for (id, g) in grid_map {
                        let Some(name) = non_empty_str(g.get("name")) else {
                            continue;
                        };
                        let avatar = non_empty_str(g.get("avt")).unwrap_or_default();
                        self.store.upsert_thread(ThreadInfo {
                            id: id.clone(),
                            kind: ThreadKind::Group,
                            name,
                            avatar,
                        });
                        changed = true;
                    }
                }
            }
        }

        changed
    }

    // ---- Conversations & Presence ----

    pub fn list_threads(&self) -> Vec<ThreadSummary> {
        self.store.list_threads()
    }

    pub async fn thread_info(&self, kind: ThreadKind, id: &str) -> Result<Value, HttpError> {
        match kind {
            ThreadKind::User => {
                let last_online = match self.last_online(id).await {
                    Ok(v) => v,
                    Err(e) if e.status == 409 => return Err(e),
                    Err(_) => None,
                };
                Ok(json!({ "lastOnline": last_online }))
            }
            ThreadKind::Group => {
                let members = self.group_members(id).await?;
                Ok(json!({ "members": members }))
            }
        }
    }

    pub async fn last_online(&self, user_id: &str) -> Result<Option<i64>, HttpError> {
        let client = self.api()?;
        let res = client.last_online(user_id).await.map_err(proto_to_http_error)?;
        let show = res
            .get("settings")
            .and_then(|s| s.get("show_online_status"))
            .map(js_truthy)
            .unwrap_or(false);
        let ts = res.get("lastOnline").map(|v| val_to_i64(Some(v))).unwrap_or(0);
        if show && ts > 0 {
            Ok(Some(ts))
        } else {
            Ok(None)
        }
    }

    pub async fn group_members(&self, group_id: &str) -> Result<Vec<GroupMember>, HttpError> {
        let now = now_ms();
        if let Ok(s) = self.inner.read() {
            if let Some((at, list)) = s.members.get(group_id) {
                if now - *at < MEMBERS_CACHE_MS {
                    return Ok(list.clone());
                }
            }
        }

        let client = self.api()?;
        let res = client
            .get_group_info(&[group_id.to_string()])
            .await
            .map_err(proto_to_http_error)?;
        let info = res
            .get("gridInfoMap")
            .and_then(|m| m.get(group_id))
            .ok_or_else(|| HttpError::not_found("Group not found"))?;

        let mut list: Vec<GroupMember> = info
            .get("currentMems")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|m| GroupMember {
                        id: m.get("id").map(val_to_string).unwrap_or_default(),
                        name: non_empty_str(m.get("dName"))
                            .or_else(|| non_empty_str(m.get("zaloName")))
                            .unwrap_or_default(),
                    })
                    .filter(|m| !m.id.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        if list.is_empty() {
            let ids: Vec<String> = info
                .get("memVerList")
                .or_else(|| info.get("memberIds"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|v| {
                            let s = val_to_string(v);
                            s.split('_').next().unwrap_or(&s).to_string()
                        })
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            for chunk in ids.chunks(MEMBER_INFO_BATCH) {
                let profiles_res = client
                    .get_group_members_info(chunk)
                    .await
                    .map_err(proto_to_http_error)?;
                if let Some(profiles) = profiles_res.get("profiles").and_then(|v| v.as_object()) {
                    for (id, p) in profiles {
                        let name = non_empty_str(p.get("displayName"))
                            .or_else(|| non_empty_str(p.get("zaloName")))
                            .unwrap_or_default();
                        list.push(GroupMember {
                            id: id.clone(),
                            name,
                        });
                    }
                }
            }
        }

        let own_id = self.own_id();
        list.retain(|m| own_id.as_deref() != Some(m.id.as_str()));
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));

        if let Ok(mut s) = self.inner.write() {
            s.members.insert(group_id.to_string(), (now_ms(), list.clone()));
        }
        Ok(list)
    }

    pub async fn get_messages(
        self: &Arc<Self>,
        kind: ThreadKind,
        id: &str,
        before: Option<i64>,
        limit: Option<usize>,
    ) -> Result<(Option<ThreadInfo>, Vec<ChatMessage>), HttpError> {
        if kind == ThreadKind::Group
            && before.is_none()
            && self.store.get_messages(kind, id, None, None).len() < 20
        {
            if let Err(err) = self.fetch_group_history(id, Some(50)).await {
                warn!("Group history failed for {id}: {err}");
            }
        }
        Ok((
            self.store.get_thread(kind, id),
            self.store.get_messages(kind, id, before, limit),
        ))
    }

    pub async fn fetch_group_history(
        self: &Arc<Self>,
        group_id: &str,
        count: Option<u32>,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        let generation = self.inner.read().map(|s| s.generation).unwrap_or(0);
        let res = client
            .get_group_chat_history(group_id, count)
            .await
            .map_err(proto_to_http_error)?;
        let arr = res
            .get("groupMsgs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let had_msgs = !arr.is_empty();
        for item in arr {
            let thread_id = item
                .get("threadId")
                .map(val_to_string)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| group_id.to_string());
            let is_self = item.get("isSelf").and_then(|v| v.as_bool()).unwrap_or(false);
            let data = item.get("data").cloned().unwrap_or(item);
            let raw = RawWsMessage {
                thread_type: ThreadType::Group,
                thread_id,
                is_self,
                data,
            };
            self.ingest_raw(raw, true, generation);
        }
        if had_msgs {
            self.broadcast_event(json!({ "type": "threads" }));
        }
        Ok(())
    }

    pub fn search_messages(&self, query: &str, limit: Option<usize>) -> Vec<MessageSearchHit> {
        self.store.search_messages(query, limit)
    }

    pub async fn mark_read(&self, kind: ThreadKind, thread_id: &str) -> Result<(), HttpError> {
        let unread = self.store.unread_messages(kind, thread_id);
        let was_marked_unread = self.store.mark_read(kind, thread_id, None);
        self.broadcast_event(json!({
            "type": "read",
            "threadKind": kind,
            "threadId": thread_id,
        }));

        let Ok(client) = self.api() else {
            return Ok(());
        };

        if self.send_read_receipts {
            let with_raw: Vec<&StoredMessage> =
                unread.iter().filter(|m| m.raw.is_some()).collect();
            if !with_raw.is_empty() {
                let start = with_raw.len().saturating_sub(SEEN_RECEIPT_LIMIT);
                let params: Vec<SeenMessageInput> = with_raw[start..]
                    .iter()
                    .filter_map(|m| {
                        let raw = m.raw.as_ref()?;
                        Some(SeenMessageInput {
                            msg_id: m.id.clone(),
                            cli_msg_id: m.cli_msg_id.clone(),
                            uid_from: raw.uid_from.clone(),
                            id_to: raw.id_to.clone(),
                            msg_type: raw.msg_type.clone(),
                            st: Some(raw.st),
                            at: Some(raw.at),
                            cmd: Some(raw.cmd),
                            ts: raw.ts.parse::<i64>().ok(),
                        })
                    })
                    .collect();
                if !params.is_empty() {
                    if let Err(err) = client.send_seen_event(&params, to_thread_type(kind)).await {
                        warn!("sendSeenEvent failed: {err}");
                    }
                }
            }
        }
        if was_marked_unread {
            let _ = client
                .remove_unread_mark(thread_id, to_thread_type(kind))
                .await;
        }
        Ok(())
    }

    pub async fn mark_unread(&self, kind: ThreadKind, thread_id: &str) -> Result<(), HttpError> {
        let client = self.api()?;
        client
            .add_unread_mark(thread_id, to_thread_type(kind))
            .await
            .map_err(proto_to_http_error)?;
        self.store.mark_unread(kind, thread_id);
        self.broadcast_event(json!({ "type": "threads" }));
        Ok(())
    }

    pub async fn set_pinned(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        pinned: bool,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        client
            .set_pinned_conversations(pinned, &[thread_id.to_string()], to_thread_type(kind))
            .await
            .map_err(proto_to_http_error)?;
        self.store.set_pinned(kind, thread_id, pinned);
        self.broadcast_event(json!({ "type": "threads" }));
        Ok(())
    }

    pub async fn set_muted(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        option: Option<MuteOption>,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        match option {
            None => {
                client
                    .set_mute(
                        MuteAction::Unmute,
                        MuteDuration::Forever,
                        thread_id,
                        to_thread_type(kind),
                    )
                    .await
                    .map_err(proto_to_http_error)?;
                self.store.set_muted(kind, thread_id, None);
            }
            Some(opt) => {
                client
                    .set_mute(
                        MuteAction::Mute,
                        opt.to_proto(),
                        thread_id,
                        to_thread_type(kind),
                    )
                    .await
                    .map_err(proto_to_http_error)?;
                self.store.set_muted(kind, thread_id, Some(mute_until(opt)));
            }
        }
        self.broadcast_event(json!({ "type": "threads" }));
        Ok(())
    }

    pub async fn send_typing(&self, kind: ThreadKind, thread_id: &str) -> Result<(), HttpError> {
        let client = self.api()?;
        client
            .send_typing_event(thread_id, to_thread_type(kind), None)
            .await
            .map_err(proto_to_http_error)?;
        Ok(())
    }

    pub async fn delete_thread(&self, kind: ThreadKind, thread_id: &str) -> Result<(), HttpError> {
        let client = self.api()?;
        if let Some(last) = self.store.last_message(kind, thread_id) {
            if let Some(raw) = last.raw {
                let info = LastMessageInfo {
                    owner_id: raw.uid_from,
                    cli_msg_id: last.cli_msg_id,
                    global_msg_id: last.id,
                };
                client
                    .delete_chat(&info, thread_id, to_thread_type(kind))
                    .await
                    .map_err(proto_to_http_error)?;
            }
        }
        self.store.clear_thread_messages(kind, thread_id);
        self.broadcast_event(json!({ "type": "threads" }));
        Ok(())
    }

    /// Sets which chat the user is currently viewing (90s TTL), or clears it when `None`.
    pub fn set_presence(&self, kind: Option<ThreadKind>, thread_id: Option<&str>) {
        if let Ok(mut s) = self.inner.write() {
            s.viewing = match (kind, thread_id) {
                (Some(k), Some(id)) if !id.is_empty() => {
                    Some((thread_key(k, id), now_ms() + VIEWING_TTL_MS))
                }
                _ => None,
            };
        }
    }

    pub fn should_notify(&self, msg: &ChatMessage, is_old: bool) -> bool {
        if is_old || msg.is_self || msg.recalled {
            return false;
        }
        let now = now_ms();
        if now - msg.ts > NOTIFY_MAX_AGE_MS {
            return false;
        }
        if self.store.is_muted_at(msg.thread_kind, &msg.thread_id, now) {
            return false;
        }
        let key = thread_key(msg.thread_kind, &msg.thread_id);
        let viewing_this = self
            .inner
            .read()
            .ok()
            .and_then(|s| s.viewing.clone())
            .is_some_and(|(vkey, until)| vkey == key && until > now);
        !viewing_this
    }

    pub fn thread_name(&self, kind: ThreadKind, thread_id: &str) -> String {
        self.store
            .get_thread(kind, thread_id)
            .map(|t| t.name)
            .unwrap_or_else(|| thread_id.to_string())
    }

    // ---- Messages, attachments, voice, stickers, reactions ----

    pub async fn send_text(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        text: &str,
        quote_id: Option<&str>,
        mentions: Option<Vec<MentionInfo>>,
    ) -> Result<Option<String>, HttpError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(HttpError::bad_request("text is required"));
        }
        if trimmed.chars().count() > MAX_TEXT_LENGTH {
            return Err(HttpError::bad_request(format!(
                "text is longer than {MAX_TEXT_LENGTH} characters"
            )));
        }
        let client = self.api()?;
        let quote = if let Some(qid) = quote_id.filter(|s| !s.is_empty()) {
            let (q, raw) = self.require_message(kind, thread_id, qid)?;
            if q.recalled {
                return Err(HttpError::bad_request("Can't reply to a recalled message"));
            }
            Some(QuoteInput {
                uid_from: raw.uid_from,
                msg_id: q.id,
                cli_msg_id: q.cli_msg_id,
                msg_type: raw.msg_type,
                ts: Value::String(raw.ts),
                content: raw.content,
                property_ext: raw.property_ext,
                ttl: Some(raw.ttl),
            })
        } else {
            None
        };
        let proto_mentions: Vec<MentionInput> = if kind == ThreadKind::Group {
            mentions
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m.pos >= 0 && m.len > 0)
                .map(|m| MentionInput {
                    pos: m.pos as usize,
                    uid: m.uid,
                    len: m.len as usize,
                })
                .collect()
        } else {
            Vec::new()
        };

        let opts = SendMessageOptions {
            msg: trimmed.to_string(),
            attachments: Vec::new(),
            mentions: proto_mentions,
            quote,
            ttl: None,
            urgency: None,
        };
        let res = client
            .send_message(opts, thread_id, to_thread_type(kind))
            .await
            .map_err(proto_to_http_error)?;
        let msg_id = res
            .message
            .as_ref()
            .and_then(|m| m.get("msgId"))
            .map(val_to_string)
            .filter(|s| !s.is_empty());
        Ok(msg_id)
    }

    pub async fn send_attachment(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        file: UploadFile,
    ) -> Result<Vec<String>, HttpError> {
        let client = self.api()?;
        let ext = file
            .filename
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext.is_empty() {
            return Err(HttpError::bad_request("The file needs a name with an extension"));
        }
        if ext == "gif" {
            return Err(HttpError::bad_request("GIFs can't be sent yet"));
        }
        if file.data.is_empty() {
            return Err(HttpError::bad_request("The file is empty"));
        }
        let is_photo = PHOTO_EXTENSIONS.contains(&ext.as_str());
        if is_photo && !(file.width.is_some_and(|w| w > 0) && file.height.is_some_and(|h| h > 0)) {
            return Err(HttpError::bad_request("Photos need their width and height"));
        }
        let opts = SendMessageOptions {
            msg: String::new(),
            attachments: vec![UploadSource {
                filename: file.filename,
                data: file.data,
                width: if is_photo { file.width } else { None },
                height: if is_photo { file.height } else { None },
            }],
            ..Default::default()
        };
        let res = client
            .send_message(opts, thread_id, to_thread_type(kind))
            .await
            .map_err(proto_to_http_error)?;
        let msg_ids = res
            .attachment
            .iter()
            .filter_map(|a| a.get("msgId").map(val_to_string))
            .filter(|s| !s.is_empty())
            .collect();
        Ok(msg_ids)
    }

    pub async fn send_voice(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        data: Vec<u8>,
        original_name: &str,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        if data.is_empty() {
            return Err(HttpError::bad_request("The voice recording is empty"));
        }
        let raw_name = Path::new(original_name.trim())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let base_name = if !raw_name.is_empty()
            && raw_name != "."
            && raw_name != ".."
            && !raw_name.starts_with('.')
        {
            raw_name
        } else {
            "voice.m4a"
        };
        let ext = base_name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        let filename = if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            base_name.to_string()
        } else {
            format!("{base_name}.m4a")
        };

        let uploaded = client
            .upload_attachment(
                &[UploadSource {
                    filename,
                    data,
                    width: None,
                    height: None,
                }],
                thread_id,
                to_thread_type(kind),
            )
            .await
            .map_err(proto_to_http_error)?;

        let voice_url = uploaded
            .first()
            .and_then(|u| match u {
                UploadResult::Video { file_url, .. } | UploadResult::Others { file_url, .. } => {
                    Some(file_url.clone())
                }
                UploadResult::Image { .. } => None,
            })
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HttpError::new(502, "Failed to upload voice recording"))?;

        client
            .send_voice(&voice_url, None, thread_id, to_thread_type(kind))
            .await
            .map_err(proto_to_http_error)?;
        Ok(())
    }

    pub async fn search_stickers(&self, keyword: &str) -> Result<Vec<StickerInfo>, HttpError> {
        let client = self.api()?;
        let mut ids = client
            .get_stickers(keyword)
            .await
            .map_err(proto_to_http_error)?;
        ids.truncate(STICKER_SEARCH_LIMIT);

        let missing: Vec<i64> = {
            let s = self
                .inner
                .read()
                .map_err(|_| HttpError::internal("Session lock poisoned"))?;
            ids.iter()
                .copied()
                .filter(|id| !s.stickers.contains_key(id))
                .collect()
        };
        if !missing.is_empty() {
            let details = client
                .get_stickers_detail(&missing)
                .await
                .map_err(proto_to_http_error)?;
            for d in details {
                self.remember_sticker(&d);
            }
        }
        let s = self
            .inner
            .read()
            .map_err(|_| HttpError::internal("Session lock poisoned"))?;
        Ok(ids
            .iter()
            .filter_map(|id| s.stickers.get(id).cloned())
            .collect())
    }

    pub async fn send_sticker(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        sticker_id: i64,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        let cached = self
            .inner
            .read()
            .ok()
            .and_then(|s| s.stickers.get(&sticker_id).cloned());
        let sticker = match cached {
            Some(s) => s,
            None => {
                let details = client
                    .get_stickers_detail(&[sticker_id])
                    .await
                    .unwrap_or_default();
                for d in details {
                    self.remember_sticker(&d);
                }
                self.inner
                    .read()
                    .ok()
                    .and_then(|s| s.stickers.get(&sticker_id).cloned())
                    .ok_or_else(|| HttpError::not_found("Sticker not found"))?
            }
        };
        client
            .send_sticker(
                &StickerParam {
                    id: sticker.id,
                    cate_id: sticker.cat_id,
                    sticker_type: sticker.sticker_type as i64,
                },
                thread_id,
                to_thread_type(kind),
            )
            .await
            .map_err(proto_to_http_error)?;
        Ok(())
    }

    pub async fn set_reaction(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
        reaction: Option<&str>,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        let (msg, _raw) = self.require_message(kind, thread_id, msg_id)?;
        let code: &str = match reaction {
            None | Some("") => "",
            Some(key) => reaction_code(key)
                .ok_or_else(|| HttpError::bad_request(format!("Unknown reaction: {key}")))?,
        };
        client
            .add_reaction(
                code,
                &ReactionTarget {
                    thread_id: thread_id.to_string(),
                    thread_type: to_thread_type(kind),
                    msg_id: msg.id.clone(),
                    cli_msg_id: msg.cli_msg_id.clone(),
                },
            )
            .await
            .map_err(proto_to_http_error)?;
        let own = self.own_id().unwrap_or_default();
        if let Some(updated) = self.store.apply_reaction(&msg.id, &own, code) {
            self.broadcast_message_updated(&updated);
        }
        Ok(())
    }

    pub async fn recall_message(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        let (msg, _raw) = self.require_message(kind, thread_id, msg_id)?;
        if !msg.is_self {
            return Err(HttpError::forbidden("You can only recall your own messages"));
        }
        client
            .undo(
                &UndoTarget {
                    msg_id: msg.id.clone(),
                    cli_msg_id: msg.cli_msg_id.clone(),
                },
                thread_id,
                to_thread_type(kind),
            )
            .await
            .map_err(proto_to_http_error)?;
        if let Some(updated) = self.store.apply_undo(&msg.id) {
            self.broadcast_message_updated(&updated);
        }
        Ok(())
    }

    pub async fn delete_message(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        let (msg, raw) = self.require_message(kind, thread_id, msg_id)?;
        client
            .delete_message(
                &DeleteMessageTarget {
                    thread_id: thread_id.to_string(),
                    thread_type: to_thread_type(kind),
                    msg_id: msg.id.clone(),
                    cli_msg_id: msg.cli_msg_id.clone(),
                    uid_from: raw.uid_from,
                },
                true,
            )
            .await
            .map_err(proto_to_http_error)?;
        if self.store.delete_message(&msg.id).is_some() {
            self.broadcast_event(json!({
                "type": "message_deleted",
                "threadKind": kind,
                "threadId": thread_id,
                "id": msg.id,
            }));
        }
        Ok(())
    }

    pub async fn forward_message(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
        targets: &[(ThreadKind, String)],
    ) -> Result<(), HttpError> {
        if targets.is_empty() {
            return Err(HttpError::bad_request("to must be a non-empty array"));
        }
        let mut seen = HashSet::new();
        let mut unique: Vec<(ThreadKind, String)> = Vec::new();
        for (k, id) in targets {
            if id.is_empty() {
                return Err(HttpError::bad_request("each target needs an id"));
            }
            let key = thread_key(*k, id);
            if seen.insert(key) {
                unique.push((*k, id.clone()));
                if unique.len() > MAX_FORWARD_TARGETS {
                    return Err(HttpError::bad_request(format!(
                        "Cannot forward to more than {MAX_FORWARD_TARGETS} chats at once"
                    )));
                }
            }
        }
        let client = self.api()?;
        let (msg, raw) = self.require_message(kind, thread_id, msg_id)?;
        let Some(text) = raw.content.as_str() else {
            return Err(HttpError::bad_request("Only text messages can be forwarded"));
        };
        if msg.recalled {
            return Err(HttpError::bad_request("Only text messages can be forwarded"));
        }
        let payload = ForwardMessagePayload {
            message: text.to_string(),
            ttl: None,
            reference: Some(ForwardReference {
                id: msg.id,
                ts: msg.ts,
                fw_lvl: 1,
            }),
        };
        for target_kind in [ThreadKind::User, ThreadKind::Group] {
            let ids: Vec<String> = unique
                .iter()
                .filter(|(k, _)| *k == target_kind)
                .map(|(_, id)| id.clone())
                .collect();
            if ids.is_empty() {
                continue;
            }
            let res = client
                .forward_message(&payload, &ids, to_thread_type(target_kind))
                .await
                .map_err(proto_to_http_error)?;
            let fail_len = res
                .get("fail")
                .and_then(|v| v.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            if fail_len > 0 {
                return Err(HttpError::new(
                    502,
                    format!("Forwarding failed for {fail_len} chat(s)"),
                ));
            }
        }
        Ok(())
    }

    // ---- Friends ----

    pub fn friends(&self) -> Vec<Person> {
        let Ok(s) = self.inner.read() else {
            return Vec::new();
        };
        if s.client.is_none() {
            return Vec::new();
        }
        let mut out: Vec<Person> = s.friend_list.values().cloned().collect();
        out.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }

    pub async fn friend_requests(&self) -> Result<Vec<FriendRequest>, HttpError> {
        let client = self.api()?;
        let res = client
            .get_friend_recommendations()
            .await
            .map_err(proto_to_http_error)?;
        let out = res
            .get("recommItems")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("dataInfo"))
                    .filter(|d| val_to_i64(d.get("recommType")) == 2)
                    .map(|d| {
                        let id = d.get("userId").map(val_to_string).unwrap_or_default();
                        let name = non_empty_str(d.get("displayName"))
                            .or_else(|| non_empty_str(d.get("zaloName")))
                            .unwrap_or_else(|| id.clone());
                        FriendRequest {
                            id,
                            name,
                            avatar: non_empty_str(d.get("avatar")).unwrap_or_default(),
                            message: d
                                .get("recommInfo")
                                .and_then(|ri| non_empty_str(ri.get("message")))
                                .unwrap_or_default(),
                            time: val_to_i64(d.get("recommTime")),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(out)
    }

    pub async fn sent_friend_requests(&self) -> Result<Vec<FriendRequest>, HttpError> {
        let client = self.api()?;
        let sent = client
            .get_sent_friend_request()
            .await
            .unwrap_or_else(|_| Value::Object(Map::new()));
        let mut out: Vec<FriendRequest> = sent
            .as_object()
            .map(|map| {
                map.iter()
                    .map(|(key, d)| {
                        let id = d
                            .get("userId")
                            .map(val_to_string)
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| key.clone());
                        let name = non_empty_str(d.get("displayName"))
                            .or_else(|| non_empty_str(d.get("zaloName")))
                            .unwrap_or_else(|| id.clone());
                        let f_req = d.get("fReqInfo");
                        FriendRequest {
                            id,
                            name,
                            avatar: non_empty_str(d.get("avatar")).unwrap_or_default(),
                            message: f_req
                                .and_then(|fr| non_empty_str(fr.get("message")))
                                .unwrap_or_default(),
                            time: val_to_i64(f_req.and_then(|fr| fr.get("time"))),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b.time.cmp(&a.time));
        Ok(out)
    }

    pub async fn accept_friend_request(&self, user_id: &str) -> Result<(), HttpError> {
        let client = self.api()?;
        let generation = self.inner.read().map(|s| s.generation).unwrap_or(0);
        client
            .accept_friend_request(user_id)
            .await
            .map_err(proto_to_http_error)?;
        let _ = self.load_friends(&client, generation).await;
        self.broadcast_event(json!({ "type": "threads" }));
        Ok(())
    }

    pub async fn reject_friend_request(&self, user_id: &str) -> Result<(), HttpError> {
        let client = self.api()?;
        client
            .reject_friend_request(user_id)
            .await
            .map_err(proto_to_http_error)?;
        Ok(())
    }

    pub async fn send_friend_request(
        &self,
        user_id: &str,
        message: &str,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        client
            .send_friend_request(message, user_id)
            .await
            .map_err(proto_to_http_error)?;
        Ok(())
    }

    pub async fn undo_friend_request(&self, user_id: &str) -> Result<(), HttpError> {
        let client = self.api()?;
        client
            .undo_friend_request(user_id)
            .await
            .map_err(proto_to_http_error)?;
        Ok(())
    }

    pub async fn find_user(&self, phone: &str) -> Result<FoundUser, HttpError> {
        let client = self.api()?;
        let user = client.find_user(phone, None).await.ok();
        let uid = user
            .as_ref()
            .and_then(|u| u.get("uid"))
            .map(val_to_string)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HttpError::not_found("No Zalo account uses this phone number"))?;
        let user_val = user.unwrap_or(Value::Null);
        let requested = client
            .get_sent_friend_request()
            .await
            .ok()
            .and_then(|m| m.as_object().map(|o| o.contains_key(&uid)))
            .unwrap_or(false);
        let is_friend = self
            .inner
            .read()
            .ok()
            .is_some_and(|s| s.friend_list.contains_key(&uid));
        let name = non_empty_str(user_val.get("display_name"))
            .or_else(|| non_empty_str(user_val.get("zalo_name")))
            .unwrap_or_else(|| uid.clone());
        let avatar = non_empty_str(user_val.get("avatar")).unwrap_or_default();
        Ok(FoundUser {
            id: uid,
            name,
            avatar,
            is_friend,
            requested,
        })
    }

    // ---- Groups ----

    pub async fn create_group(
        &self,
        name: &str,
        member_ids: &[String],
    ) -> Result<(String, Vec<String>), HttpError> {
        let client = self.api()?;
        let own_id = self.own_id();
        let generation = self.inner.read().map(|s| s.generation).unwrap_or(0);
        let mut seen = HashSet::new();
        let members: Vec<String> = member_ids
            .iter()
            .filter(|id| own_id.as_deref() != Some(id.as_str()))
            .filter(|id| seen.insert((*id).clone()))
            .cloned()
            .collect();
        if members.len() < 2 {
            return Err(HttpError::bad_request(
                "A group needs at least 2 other people",
            ));
        }
        let trimmed: String = name.trim().chars().take(MAX_GROUP_NAME).collect();
        let res = client
            .create_group(&CreateGroupOptions {
                name: if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.clone())
                },
                members,
            })
            .await
            .map_err(proto_to_http_error)?;
        let group_id = res
            .get("groupId")
            .map(val_to_string)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HttpError::new(502, "Zalo did not return a groupId"))?;

        self.load_groups(&client, std::slice::from_ref(&group_id), generation)
            .await;
        if self.store.get_thread(ThreadKind::Group, &group_id).is_none() {
            self.store.upsert_thread(ThreadInfo {
                id: group_id.clone(),
                kind: ThreadKind::Group,
                name: if trimmed.is_empty() {
                    "New group".to_string()
                } else {
                    trimmed
                },
                avatar: String::new(),
            });
        }
        self.broadcast_event(json!({ "type": "threads" }));
        let failed = res
            .get("errorMembers")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(val_to_string).collect())
            .unwrap_or_default();
        Ok((group_id, failed))
    }

    pub async fn add_group_members(
        &self,
        group_id: &str,
        member_ids: &[String],
    ) -> Result<Vec<String>, HttpError> {
        let client = self.api()?;
        let own_id = self.own_id();
        let mut seen = HashSet::new();
        let ids: Vec<String> = member_ids
            .iter()
            .filter(|id| own_id.as_deref() != Some(id.as_str()))
            .filter(|id| seen.insert((*id).clone()))
            .cloned()
            .collect();
        if ids.is_empty() {
            return Err(HttpError::bad_request("Pick someone to add"));
        }
        let res = client
            .add_user_to_group(&ids, group_id)
            .await
            .map_err(proto_to_http_error)?;
        if let Ok(mut s) = self.inner.write() {
            s.members.remove(group_id);
        }
        let failed = res
            .get("errorMembers")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(val_to_string).collect())
            .unwrap_or_default();
        Ok(failed)
    }

    pub async fn remove_group_member(
        &self,
        group_id: &str,
        member_id: &str,
    ) -> Result<(), HttpError> {
        let client = self.api()?;
        let res = client
            .remove_user_from_group(&[member_id.to_string()], group_id)
            .await
            .map_err(proto_to_http_error)?;
        if let Ok(mut s) = self.inner.write() {
            s.members.remove(group_id);
        }
        let has_err = res
            .get("errorMembers")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_err {
            return Err(HttpError::forbidden(
                "Only the group's owner and deputies can remove members",
            ));
        }
        Ok(())
    }

    pub async fn rename_group(&self, group_id: &str, name: &str) -> Result<(), HttpError> {
        let trimmed: String = name.trim().chars().take(MAX_GROUP_NAME).collect();
        if trimmed.is_empty() {
            return Err(HttpError::bad_request("The group needs a name"));
        }
        let client = self.api()?;
        client
            .change_group_name(&trimmed, group_id)
            .await
            .map_err(proto_to_http_error)?;
        let avatar = self
            .store
            .get_thread(ThreadKind::Group, group_id)
            .map(|t| t.avatar)
            .unwrap_or_default();
        self.store.upsert_thread(ThreadInfo {
            id: group_id.to_string(),
            kind: ThreadKind::Group,
            name: trimmed,
            avatar,
        });
        self.broadcast_event(json!({ "type": "threads" }));
        Ok(())
    }

    pub async fn leave_group(&self, group_id: &str, silent: bool) -> Result<(), HttpError> {
        let client = self.api()?;
        client
            .leave_group(group_id, silent)
            .await
            .map_err(proto_to_http_error)?;
        if let Ok(mut s) = self.inner.write() {
            s.members.remove(group_id);
        }
        self.store.delete_thread(ThreadKind::Group, group_id);
        self.broadcast_event(json!({ "type": "threads" }));
        Ok(())
    }

    // ---- Polls ----

    pub async fn create_poll(
        &self,
        group_id: &str,
        input: CreatePollInput,
    ) -> Result<PollView, HttpError> {
        let client = self.api()?;
        let detail = client
            .create_poll(&input, group_id)
            .await
            .map_err(proto_to_http_error)?;
        let poll_id = val_to_i64(detail.get("poll_id"));
        if poll_id > 0 {
            if let Ok(mut s) = self.inner.write() {
                s.poll_groups.insert(poll_id, group_id.to_string());
            }
            self.broadcast_event(json!({ "type": "poll", "pollId": poll_id }));
        }
        self.poll_view_with_names(&detail).await
    }

    pub async fn get_poll(&self, poll_id: i64) -> Result<PollView, HttpError> {
        let client = self.api()?;
        let detail = client
            .get_poll_detail(poll_id)
            .await
            .map_err(proto_to_http_error)?;
        self.poll_view_with_names(&detail).await
    }

    pub async fn vote_poll(
        &self,
        poll_id: i64,
        option_ids: &[i64],
    ) -> Result<PollView, HttpError> {
        let client = self.api()?;
        let detail = client
            .get_poll_detail(poll_id)
            .await
            .map_err(proto_to_http_error)?;
        let expired_time = val_to_i64(detail.get("expired_time"));
        let closed = detail
            .get("closed")
            .map(js_truthy)
            .unwrap_or(false)
            || (expired_time > 0 && expired_time <= now_ms());
        if closed {
            return Err(HttpError::conflict("This poll is closed"));
        }
        let known: HashSet<i64> = detail
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(|o| val_to_i64(o.get("option_id"))).collect())
            .unwrap_or_default();
        if option_ids.iter().any(|id| !known.contains(id)) {
            return Err(HttpError::bad_request("Unknown option"));
        }
        let allow_multi = detail
            .get("allow_multi_choices")
            .map(js_truthy)
            .unwrap_or(false);
        if !allow_multi && option_ids.len() > 1 {
            return Err(HttpError::bad_request("This poll takes one choice"));
        }
        client
            .vote_poll(poll_id, option_ids)
            .await
            .map_err(proto_to_http_error)?;
        self.refreshed_poll(poll_id).await
    }

    pub async fn add_poll_options(
        &self,
        poll_id: i64,
        options: &[String],
    ) -> Result<PollView, HttpError> {
        let client = self.api()?;
        let detail = client
            .get_poll_detail(poll_id)
            .await
            .map_err(proto_to_http_error)?;
        if detail.get("closed").map(js_truthy).unwrap_or(false) {
            return Err(HttpError::conflict("This poll is closed"));
        }
        let allow_add = detail
            .get("allow_add_new_option")
            .map(js_truthy)
            .unwrap_or(false);
        let creator = detail.get("creator").map(val_to_string).unwrap_or_default();
        let own = self.own_id().unwrap_or_default();
        if !allow_add && creator != own {
            return Err(HttpError::forbidden("This poll doesn't take new options"));
        }
        let voted_option_ids: Vec<i64> = detail
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|o| o.get("voted").map(js_truthy).unwrap_or(false))
                    .map(|o| val_to_i64(o.get("option_id")))
                    .collect()
            })
            .unwrap_or_default();
        client
            .add_poll_options(&AddPollOptionsInput {
                poll_id,
                options: options.to_vec(),
                voted_option_ids,
            })
            .await
            .map_err(proto_to_http_error)?;
        self.refreshed_poll(poll_id).await
    }

    pub async fn lock_poll(&self, poll_id: i64) -> Result<PollView, HttpError> {
        let client = self.api()?;
        let detail = client
            .get_poll_detail(poll_id)
            .await
            .map_err(proto_to_http_error)?;
        let view = self.poll_view_with_names(&detail).await?;
        if !view.can_close {
            return Err(HttpError::forbidden(
                "Only the creator or a group admin can close a poll",
            ));
        }
        client
            .lock_poll(poll_id)
            .await
            .map_err(proto_to_http_error)?;
        self.refreshed_poll(poll_id).await
    }

    async fn refreshed_poll(&self, poll_id: i64) -> Result<PollView, HttpError> {
        let view = self.get_poll(poll_id).await?;
        self.broadcast_event(json!({ "type": "poll", "pollId": poll_id }));
        Ok(view)
    }

    async fn is_group_admin(&self, group_id: Option<&str>) -> bool {
        let (Some(gid), Some(own_id)) = (group_id, self.own_id()) else {
            return false;
        };
        let Ok(client) = self.api() else {
            return false;
        };
        let Ok(res) = client.get_group_info(&[gid.to_string()]).await else {
            return false;
        };
        let Some(info) = res.get("gridInfoMap").and_then(|m| m.get(gid)) else {
            return false;
        };
        if info.get("creatorId").map(val_to_string).as_deref() == Some(own_id.as_str()) {
            return true;
        }
        info.get("adminIds")
            .and_then(|v| v.as_array())
            .is_some_and(|arr| arr.iter().any(|x| val_to_string(x) == own_id))
    }

    async fn poll_view_with_names(&self, detail: &Value) -> Result<PollView, HttpError> {
        let poll_id = val_to_i64(detail.get("poll_id"));
        let group_id = self
            .inner
            .read()
            .ok()
            .and_then(|s| s.poll_groups.get(&poll_id).cloned());
        let mut names = HashMap::new();
        if let Some(ref gid) = group_id {
            if let Ok(members) = self.group_members(gid).await {
                for m in members {
                    names.insert(m.id, m.name);
                }
            }
        }
        let status = self.get_status();
        if let Some(own_id) = status.own_id.as_deref() {
            names.insert(
                own_id.to_string(),
                status.own_name.unwrap_or_else(|| "You".to_string()),
            );
        }
        let is_admin = self.is_group_admin(group_id.as_deref()).await;
        let proto_view = build_poll_view(
            detail,
            &PollViewContext {
                own_id: status.own_id.as_deref(),
                is_admin,
                now: now_ms(),
                names: Some(&names),
            },
        );
        Ok(convert_poll_view(proto_view))
    }

    // ---- Internal helpers ----

    fn require_message(
        &self,
        kind: ThreadKind,
        thread_id: &str,
        msg_id: &str,
    ) -> Result<(StoredMessage, crate::types::RawRef), HttpError> {
        let msg = self
            .store
            .find_message(msg_id)
            .filter(|m| m.thread_kind == kind && m.thread_id == thread_id)
            .ok_or_else(|| HttpError::not_found("Message not found"))?;
        let raw = msg
            .raw
            .clone()
            .ok_or_else(|| HttpError::bad_request("This message is too old to act on"))?;
        Ok((msg, raw))
    }

    fn remember_sticker(&self, d: &Value) -> Option<StickerInfo> {
        let id = val_to_i64(d.get("id"));
        if id <= 0 {
            return None;
        }
        let cat_id = val_to_i64(d.get("cateId").or_else(|| d.get("catId")));
        let sticker_type = val_to_i64(d.get("type")) as i32;
        let info = StickerInfo {
            id,
            cat_id,
            sticker_type,
            url: format!("{STICKER_IMAGE_URL}{id}"),
        };
        if let Ok(mut s) = self.inner.write() {
            if s.stickers.contains_key(&id) {
                s.sticker_order.retain(|&x| x != id);
            }
            s.stickers.insert(id, info.clone());
            s.sticker_order.push_back(id);
            while s.stickers.len() > MAX_STICKERS_CACHE {
                if let Some(oldest) = s.sticker_order.pop_front() {
                    s.stickers.remove(&oldest);
                } else {
                    break;
                }
            }
        }
        Some(info)
    }

    fn is_generation(&self, generation: u64) -> bool {
        self.inner
            .read()
            .map(|s| s.generation == generation)
            .unwrap_or(false)
    }

    fn is_generation_and_client(&self, generation: u64, client: &Arc<dyn ZaloApi>) -> bool {
        self.inner
            .read()
            .map(|s| {
                s.generation == generation
                    && s.client.as_ref().is_some_and(|c| Arc::ptr_eq(c, client))
            })
            .unwrap_or(false)
    }

    fn patch_status(&self, f: impl FnOnce(&mut SessionStatus)) {
        let snapshot = {
            let Ok(mut s) = self.inner.write() else {
                return;
            };
            f(&mut s.status);
            s.status.clone()
        };
        self.broadcast_status(&snapshot);
    }

    fn broadcast_status(&self, status: &SessionStatus) {
        self.broadcast_event(json!({
            "type": "status",
            "status": status,
        }));
    }

    fn broadcast_message_updated(&self, msg: &StoredMessage) {
        self.broadcast_event(json!({
            "type": "message_updated",
            "message": self.store.to_public(msg),
        }));
    }

    fn read_credentials(&self) -> Option<Credentials> {
        let text = fs::read_to_string(&self.credentials_file).ok()?;
        serde_json::from_str::<Credentials>(&text).ok()
    }

    fn write_credentials(&self, creds: &Credentials) -> std::io::Result<()> {
        if let Some(parent) = self.credentials_file.parent() {
            create_dir_private(parent)?;
        }
        let tmp = self.credentials_file.with_extension("json.tmp");
        let payload = serde_json::to_vec(creds)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut opts = OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&payload)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &self.credentials_file)
    }
}

/// Hooks for tests (`testing` feature): skip the login and drive the listener loop directly.
#[cfg(any(test, feature = "testing"))]
impl ZaloSession {
    /// Makes `api` the signed-in client without logging in: state `ready`, own id from the API.
    /// Starts no listener and loads no contacts.
    pub fn testing_attach(&self, api: Arc<dyn ZaloApi>) {
        let own_id = api.own_id();
        self.store.set_own_id(Some(own_id.clone()));
        if let Ok(mut s) = self.inner.write() {
            s.client = Some(api);
        }
        self.patch_status(|st| {
            st.state = SessionState::Ready;
            st.own_id = Some(own_id);
        });
    }

    /// Starts the listener on the attached API, as a completed login does.
    pub fn testing_start_listener(self: &Arc<Self>) -> Result<(), HttpError> {
        let (client, generation) = {
            let s = self
                .inner
                .read()
                .map_err(|_| HttpError::internal("Session lock poisoned"))?;
            let client = s
                .client
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| HttpError::conflict("Not logged in"))?;
            (client, s.generation)
        };
        self.start_listener(client, generation)
            .map_err(proto_to_http_error)
    }

    /// Runs one event through the handler the listener loop uses, and waits for it.
    pub async fn testing_dispatch(self: &Arc<Self>, event: WsEvent) {
        struct Noop;
        impl ListenerControl for Noop {
            fn request_old_messages(&self, _: ThreadType) {}
            fn request_old_reactions(&self, _: ThreadType) {}
            fn stop(&self) {}
        }
        let (client, handle, generation) = {
            let Ok(s) = self.inner.read() else { return };
            let Some(client) = s.client.as_ref().map(Arc::clone) else {
                return;
            };
            let handle = s
                .listener_handle
                .as_ref()
                .map(Arc::clone)
                .unwrap_or_else(|| Arc::new(Noop) as Arc<dyn ListenerControl>);
            (client, handle, s.generation)
        };
        self.handle_ws_event(event, &client, &handle, generation).await;
    }
}

fn convert_poll_view(v: zalo_proto::PollView) -> PollView {
    PollView {
        poll_id: v.poll_id,
        question: v.question,
        creator_id: v.creator_id,
        creator_name: v.creator_name,
        closed: v.closed,
        expired: v.expired,
        expires_at: v.expires_at,
        created_at: v.created_at,
        allow_multiple: v.allow_multiple,
        allow_add_options: v.allow_add_options,
        anonymous: v.anonymous,
        results_hidden: v.results_hidden,
        total_votes: v.total_votes as usize,
        voter_count: v.voter_count,
        my_votes: v.my_votes,
        can_vote: v.can_vote,
        can_add_options: v.can_add_options,
        can_close: v.can_close,
        options: v
            .options
            .into_iter()
            .map(|o| PollOptionView {
                id: o.id,
                text: o.text,
                votes: o.votes as usize,
                voted: o.voted,
                voters: o
                    .voters
                    .into_iter()
                    .map(|vt| PollVoter {
                        id: vt.id,
                        name: vt.name,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn is_transient_proto_error(err: &ProtoError) -> bool {
    match err {
        ProtoError::Http(_) | ProtoError::Io(_) | ProtoError::WebSocket(_) => true,
        ProtoError::HttpStatus { status, .. } => *status >= 500 || *status == 408 || *status == 429,
        _ => false,
    }
}

fn describe_proto_error(err: &ProtoError) -> String {
    match err {
        ProtoError::Http(e) => {
            if e.is_timeout() {
                "connection timed out".to_string()
            } else if e.is_connect() {
                "no internet connection".to_string()
            } else {
                e.to_string()
            }
        }
        other => other.to_string(),
    }
}

fn proto_to_http_error(err: ProtoError) -> HttpError {
    match err {
        ProtoError::InvalidArg(m) => HttpError::bad_request(m),
        ProtoError::Api { message, .. } => HttpError::new(502, message),
        other => HttpError::new(502, other.to_string()),
    }
}

fn is_plain_numeric_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 11 && id.bytes().all(|b| b.is_ascii_digit())
}

fn non_empty_str(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn val_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
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

fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn extract_uids_excluding(v: Option<&Value>, exclude: Option<&str>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .map(val_to_string)
                .filter(|s| !s.is_empty() && exclude != Some(s.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_until_values_are_sensible() {
        let now = now_ms();
        let one_h = mute_until(MuteOption::OneHour);
        assert!(one_h >= now + 3_599_000 && one_h <= now + 3_601_000);
        assert_eq!(mute_until(MuteOption::Forever), -1);
        let eight_am = mute_until(MuteOption::Until8Am);
        assert!(eight_am > now && eight_am <= now + 24 * 3_600_000 + 60_000);
    }

    #[test]
    fn presence_suppresses_notifications_for_open_thread() {
        let dir = std::env::temp_dir().join(format!("zalo-sess-test-{:016x}", rand::random::<u64>()));
        let store = ChatStore::in_memory();
        let session = ZaloSession::new(store, dir.clone(), true);

        let msg = ChatMessage {
            id: "1".into(),
            cli_msg_id: "1".into(),
            thread_id: "u1".into(),
            thread_kind: ThreadKind::User,
            from_id: "u1".into(),
            from_name: "Alice".into(),
            is_self: false,
            ts: now_ms(),
            msg_type: "webchat".into(),
            text: "hi".into(),
            attachment: None,
            quote: None,
            mentions: vec![],
            recalled: false,
            reactions: vec![],
            mentions_me: false,
            status: None,
            seen_count: None,
        };

        assert!(session.should_notify(&msg, false));
        assert!(!session.should_notify(&msg, true));

        session.set_presence(Some(ThreadKind::User), Some("u1"));
        assert!(!session.should_notify(&msg, false));

        session.set_presence(None, None);
        assert!(session.should_notify(&msg, false));

        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod reconnect_tests;
