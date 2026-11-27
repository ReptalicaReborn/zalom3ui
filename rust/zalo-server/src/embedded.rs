//! In-process core for embedders such as the Android app: runs every `/api/*` route directly
//! through [`crate::dispatch`] (no socket, port, token, `axum`, or `tower`), plus the event
//! stream and the call-audio link as plain handles.
//!
//! [`EmbeddedCore::start`] spawns tasks, so it must run inside a Tokio runtime. A core can be shut
//! down and a new one started in the same process.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use bytes::Bytes;
pub type Body = Bytes;

use serde_json::json;
use tokio::sync::watch;

use crate::config::Config;
use crate::dispatch;
use crate::routes::calls::{AudioLink, AudioWsOut, CallManager, NO_ACTIVE_CALL_CLOSE};
use crate::routes::history_sync::HistorySyncManager;
use crate::routes::AppState;
use crate::session::{ServerBroadcastEvent, ZaloSession};
use crate::store::ChatStore;

/// The event stream every client of the server sees: first `{"type":"status",...}` with the
/// current status, then every broadcast event, each as the JSON text sent over `/ws`. Shared by
/// the WebSocket handler and the in-process embedded subscription.
pub struct EventStream {
    rx: tokio::sync::broadcast::Receiver<ServerBroadcastEvent>,
    initial: Option<Arc<str>>,
}

impl EventStream {
    /// Subscribes first, then snapshots the status, so no event is missed in between.
    pub fn new(session: &ZaloSession) -> Self {
        let rx = session.subscribe_events();
        let initial: Arc<str> = json!({
            "type": "status",
            "status": session.get_status(),
        })
        .to_string()
        .into();
        Self {
            rx,
            initial: Some(initial),
        }
    }

    /// The next event as shared JSON text, or `None` once the server is gone. Cancel-safe.
    pub async fn next_shared(&mut self) -> Option<Arc<str>> {
        if let Some(initial) = self.initial.take() {
            return Some(initial);
        }
        loop {
            match self.rx.recv().await {
                Ok(payload) => return Some(payload),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// The next event as JSON text, or `None` once the server is gone. Cancel-safe.
    pub async fn next(&mut self) -> Option<String> {
        self.next_shared().await.map(|s| s.to_string())
    }
}

/// How the embedded core is set up.
#[derive(Debug, Clone)]
pub struct EmbeddedConfig {
    /// Where `credentials.json`, the message store and media live.
    pub data_dir: PathBuf,
    /// Send "seen" receipts to Zalo when a chat is read.
    pub read_receipts: bool,
}

/// What a request to the in-process dispatcher answered.
#[derive(Debug, Clone)]
pub struct EmbeddedResponse {
    pub status: u16,
    /// The `Content-Type` header, empty when there is none.
    pub content_type: String,
    pub body: Bytes,
}

/// The running core: session, store, call manager, and history-sync manager.
pub struct EmbeddedCore {
    state: AppState,
    calls: Arc<CallManager>,
    history_sync: Arc<HistorySyncManager>,
    session: Arc<ZaloSession>,
    store: ChatStore,
    shutdown_tx: watch::Sender<bool>,
}

impl EmbeddedCore {
    /// Opens the store, creates and starts the session (resuming saved credentials), and starts
    /// the call & history-sync managers. Must be called inside a Tokio runtime.
    pub fn start(config: EmbeddedConfig) -> Self {
        let config = Config {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: config.data_dir,
            access_token: None,
            read_receipts: config.read_receipts,
            log_file: None,
            public_dir: None,
        };
        let (config, store, session) = crate::open_core(config);
        let state = AppState {
            config,
            session: Arc::clone(&session),
            store: store.clone(),
        };
        let calls = crate::routes::calls::start_manager(&state);
        let history_sync = crate::routes::history_sync::start_manager(state.clone());
        session.start();
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            state,
            calls,
            history_sync,
            session,
            store,
            shutdown_tx,
        }
    }

    /// Runs one request directly through the API dispatcher. `path_and_query` is already
    /// percent-encoded, e.g. `/api/threads?limit=5`. `Err` only for a request that could not be
    /// built (bad method or path); HTTP errors come back as ordinary responses.
    pub async fn request(
        &self,
        method: &str,
        path_and_query: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> Result<EmbeddedResponse, String> {
        let parsed = dispatch::parse_request(method, path_and_query)?;
        let ct = content_type.filter(|c| !c.is_empty());
        Ok(dispatch::dispatch_parsed(
            &self.state,
            &self.calls,
            &self.history_sync,
            parsed,
            ct,
            body,
        )
        .await)
    }

    /// Like [`request`](Self::request), reading the body from a file in chunks up to the route's
    /// body limit so an oversized file is rejected with `413` without loading it into memory.
    pub async fn request_file(
        &self,
        method: &str,
        path_and_query: &str,
        content_type: Option<&str>,
        file_path: &std::path::Path,
    ) -> Result<EmbeddedResponse, String> {
        use tokio::io::AsyncReadExt;

        let mut file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| format!("Could not open {}: {e}", file_path.display()))?;
        let parsed = dispatch::parse_request(method, path_and_query)?;
        let limit = dispatch::body_limit_for(&parsed);
        let ct = content_type.filter(|c| !c.is_empty());

        if let Ok(meta) = file.metadata().await {
            if meta.len() > limit as u64 {
                return Ok(dispatch::payload_too_large());
            }
        }

        let mut data = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let n = file
                .read(&mut chunk)
                .await
                .map_err(|e| format!("Could not read {}: {e}", file_path.display()))?;
            if n == 0 {
                break;
            }
            if data.len().saturating_add(n) > limit {
                return Ok(dispatch::payload_too_large());
            }
            data.extend_from_slice(&chunk[..n]);
        }

        Ok(dispatch::dispatch_parsed(
            &self.state,
            &self.calls,
            &self.history_sync,
            parsed,
            ct,
            Bytes::from(data),
        )
        .await)
    }

    /// Subscribes to events: first `{"type":"status",...}`, then every broadcast event, as the
    /// same JSON text `/ws` sends.
    pub fn events(&self) -> EmbeddedEvents {
        EmbeddedEvents {
            stream: tokio::sync::Mutex::new(EventStream::new(&self.session)),
            core_closed: self.shutdown_tx.subscribe(),
            local_closed: watch::channel(false).0,
        }
    }

    /// Opens the call-audio link; `Err((1008, "No active call"))` when no call is in progress.
    pub fn open_audio(&self) -> Result<EmbeddedAudio, (u16, String)> {
        match self.calls.open_audio_link() {
            Some(link) => Ok(EmbeddedAudio::new(link, self.shutdown_tx.subscribe())),
            None => Err((
                NO_ACTIVE_CALL_CLOSE.0,
                NO_ACTIVE_CALL_CLOSE.1.to_string(),
            )),
        }
    }

    /// Stops the session and flushes the store, as the TCP server does on exit. Subscriptions
    /// report closed afterwards. The core must not be used again; start a new one.
    pub fn shutdown(&self) {
        self.shutdown_tx.send_replace(true);
        crate::stop_core(&self.session, &self.store);
    }
}

/// Result of waiting for the next event.
#[derive(Debug, PartialEq, Eq)]
pub enum EventPoll {
    Event(String),
    Timeout,
    Closed,
}

/// An event subscription (the embedded counterpart of one `/ws` connection).
pub struct EmbeddedEvents {
    stream: tokio::sync::Mutex<EventStream>,
    core_closed: watch::Receiver<bool>,
    local_closed: watch::Sender<bool>,
}

impl EmbeddedEvents {
    /// Waits up to `timeout` for the next event. One waiter at a time.
    pub async fn next(&self, timeout: Duration) -> EventPoll {
        let mut core = self.core_closed.clone();
        let mut local = self.local_closed.subscribe();
        if *core.borrow() || *local.borrow() {
            return EventPoll::Closed;
        }
        let mut stream = self.stream.lock().await;
        tokio::select! {
            ev = stream.next() => match ev {
                Some(text) => EventPoll::Event(text),
                None => EventPoll::Closed,
            },
            _ = core.changed() => EventPoll::Closed,
            _ = local.changed() => EventPoll::Closed,
            _ = tokio::time::sleep(timeout) => EventPoll::Timeout,
        }
    }

    /// Makes a waiting [`next`](Self::next) and every later one report closed.
    pub fn close(&self) {
        self.local_closed.send_replace(true);
    }
}

/// Result of waiting for the next audio message.
#[derive(Debug, PartialEq, Eq)]
pub enum AudioPoll {
    /// A binary frame: `0x01` audio, `0x03` video AU, `0x05` key-frame request.
    Frame(Vec<u8>),
    Timeout,
    /// The link is over; see [`EmbeddedAudio::close_info`].
    Closed,
}

/// The call-audio link as a handle (the embedded counterpart of one `/ws/call-audio` connection).
pub struct EmbeddedAudio {
    link: AudioLink,
    core_closed: watch::Receiver<bool>,
    local_closed: watch::Sender<bool>,
    closed: std::sync::atomic::AtomicBool,
    close: Mutex<Option<(u16, String)>>,
}

impl EmbeddedAudio {
    pub(crate) fn new(link: AudioLink, core_closed: watch::Receiver<bool>) -> Self {
        Self {
            link,
            core_closed,
            local_closed: watch::channel(false).0,
            closed: std::sync::atomic::AtomicBool::new(false),
            close: Mutex::new(None),
        }
    }

    /// Lock-free check whether the link has already closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Acquire)
    }

    fn set_close(&self, code: u16, reason: &str) {
        if let Ok(mut c) = self.close.lock() {
            c.get_or_insert((code, reason.to_string()));
        }
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Sends one frame from the app: `0x02` audio, `0x04` video, `0x06` key-frame request.
    pub fn send(&self, data: &[u8]) {
        if !self.is_closed() {
            self.link.handle_incoming(data.to_vec());
        }
    }

    /// Like [`send`](Self::send), taking an owned buffer to avoid copying large video frames.
    pub fn send_owned(&self, data: Vec<u8>) {
        if !self.is_closed() {
            self.link.handle_incoming(data);
        }
    }

    /// Waits up to `timeout` for the next message from the call.
    pub async fn next(&self, timeout: Duration) -> AudioPoll {
        if self.is_closed() {
            return AudioPoll::Closed;
        }
        if *self.core_closed.borrow() {
            self.set_close(1001, "shutting down");
            return AudioPoll::Closed;
        }
        if *self.local_closed.borrow() {
            self.set_close(1000, "closed");
            return AudioPoll::Closed;
        }
        // Fast path: return a queued frame immediately without cloning watch channels or
        // registering a Tokio sleep timer (saves 50–80 timer allocs/sec during active calls).
        match self.link.try_recv() {
            Ok(AudioWsOut::Binary(data)) => return AudioPoll::Frame(data),
            Ok(AudioWsOut::Close(code, reason)) => {
                self.set_close(code, reason);
                return AudioPoll::Closed;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                self.set_close(1000, "closed");
                return AudioPoll::Closed;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
        }

        let mut core = self.core_closed.clone();
        let mut local = self.local_closed.subscribe();
        if *core.borrow() {
            self.set_close(1001, "shutting down");
            return AudioPoll::Closed;
        }
        if *local.borrow() {
            self.set_close(1000, "closed");
            return AudioPoll::Closed;
        }
        tokio::select! {
            out = self.link.recv() => match out {
                Some(AudioWsOut::Binary(data)) => AudioPoll::Frame(data),
                Some(AudioWsOut::Close(code, reason)) => {
                    self.set_close(code, reason);
                    AudioPoll::Closed
                }
                None => {
                    self.set_close(1000, "closed");
                    AudioPoll::Closed
                }
            },
            _ = core.changed() => {
                self.set_close(1001, "shutting down");
                AudioPoll::Closed
            }
            _ = local.changed() => {
                self.set_close(1000, "closed");
                AudioPoll::Closed
            }
            _ = tokio::time::sleep(timeout) => AudioPoll::Timeout,
        }
    }

    /// The close code and reason once the link is over, else `None`.
    pub fn close_info(&self) -> Option<(u16, String)> {
        if !self.is_closed() {
            return None;
        }
        self.close.lock().ok().and_then(|c| c.clone())
    }

    /// Makes a waiting [`next`](Self::next) and every later one report closed.
    pub fn close(&self) {
        self.local_closed.send_replace(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn start(dir: &std::path::Path) -> EmbeddedCore {
        EmbeddedCore::start(EmbeddedConfig {
            data_dir: dir.to_path_buf(),
            read_receipts: true,
        })
    }

    async fn call(
        core: &EmbeddedCore,
        method: &str,
        path: &str,
        ct: Option<&str>,
        body: Vec<u8>,
    ) -> EmbeddedResponse {
        core.request(method, path, ct, Body::from(body))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn requests_run_directly_without_host_origin_or_token() {
        let dir = temp_dir("zalo-embedded-req");
        let core = start(&dir);

        // No Host header, no token: the network checks are skipped.
        let res = call(&core, "GET", "/api/status", None, vec![]).await;
        assert_eq!(res.status, 200);
        assert!(res.content_type.starts_with("application/json"));
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v["state"], "logged_out");

        // Handlers' own errors are unchanged.
        let res = call(&core, "GET", "/api/nope", None, vec![]).await;
        assert_eq!(res.status, 404);
        let res = call(
            &core,
            "POST",
            "/api/threads/user/1/messages",
            Some("application/json"),
            b"{".to_vec(),
        )
        .await;
        assert_eq!(res.status, 400);

        // Body-size limits stay: 64 KiB for JSON, uploads allow more.
        let big = vec![b' '; 70 * 1024];
        let res = call(
            &core,
            "POST",
            "/api/threads/user/1/messages",
            Some("application/json"),
            big.clone(),
        )
        .await;
        assert_eq!(res.status, 413);
        let res = call(
            &core,
            "POST",
            "/api/threads/user/1/attachments?name=a.bin",
            Some("application/octet-stream"),
            big,
        )
        .await;
        assert_ne!(res.status, 413);

        // A file body reads through the same dispatcher.
        let file = dir.join("upload.bin");
        fs::write(&file, vec![7u8; 200 * 1024]).unwrap();
        let res = core
            .request_file(
                "POST",
                "/api/threads/user/1/attachments?name=a.bin",
                Some("application/octet-stream"),
                &file,
            )
            .await
            .unwrap();
        assert_ne!(res.status, 413);
        assert_ne!(res.status, 400);
        assert!(core
            .request_file("POST", "/api/status", None, &dir.join("missing"))
            .await
            .is_err());

        // Requests that cannot be built are errors, not panics.
        assert!(core
            .request("BAD METHOD", "/api/status", None, Body::new())
            .await
            .is_err());
        assert!(core
            .request("GET", "not a path", None, Body::new())
            .await
            .is_err());

        core.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn embedded_dispatch_parity_across_key_routes() {
        let dir = temp_dir("zalo-embedded-parity");
        let core = start(&dir);

        // /api/status
        let res = call(&core, "GET", "/api/status", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v["state"], "logged_out");

        // /api/reactions
        let res = call(&core, "GET", "/api/reactions", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 15);

        // /api/threads
        let res = call(&core, "GET", "/api/threads", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert!(v.is_array());

        // /api/search (missing q -> 400; valid q -> 200)
        let res = call(&core, "GET", "/api/search", None, vec![]).await;
        assert_eq!(res.status, 400);
        let res = call(&core, "GET", "/api/search?q=hello&limit=5", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert!(v["results"].is_array());

        // /api/calls/current -> 404 JSON error
        let res = call(&core, "GET", "/api/calls/current", None, vec![]).await;
        assert_eq!(res.status, 404);
        assert!(res.content_type.starts_with("application/json"));
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v["error"], "No active call");

        // /api/history-sync/status -> 200 with log array
        let res = call(&core, "GET", "/api/history-sync/status", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v["state"], "idle");
        assert!(v["log"].is_array());

        // Debug logs (plain text GET + 204 DELETE)
        for path in [
            "/api/debug/call-log",
            "/api/debug/socket-log",
            "/api/debug/replay-log",
        ] {
            let res = call(&core, "GET", path, None, vec![]).await;
            assert_eq!(res.status, 200, "GET {path}");
            assert!(
                res.content_type.starts_with("text/plain"),
                "GET {path} content-type={}",
                res.content_type
            );
            let del = call(&core, "DELETE", path, None, vec![]).await;
            assert_eq!(del.status, 204, "DELETE {path}");
        }

        // /api/friends/{id}/alias (validation 400 when missing alias; 503 when logged out)
        let res = call(
            &core,
            "PUT",
            "/api/friends/42/alias",
            Some("application/json"),
            b"{}".to_vec(),
        )
        .await;
        assert_eq!(res.status, 400);
        let res = call(
            &core,
            "PUT",
            "/api/friends/42/alias",
            Some("application/json"),
            br#"{"alias":"Buddy"}"#.to_vec(),
        )
        .await;
        assert_eq!(res.status, 409);
        let res = call(&core, "DELETE", "/api/friends/42/alias", None, vec![]).await;
        assert_eq!(res.status, 409);

        // /api/groups/{id}/link (GET / POST / DELETE -> 409 when logged out)
        for method in ["GET", "POST", "DELETE"] {
            let res = call(&core, method, "/api/groups/g1/link", None, vec![]).await;
            assert_eq!(res.status, 409, "{method} /api/groups/g1/link");
        }

        // Unknown route -> 404, wrong method -> 405
        let res = call(&core, "GET", "/api/does-not-exist", None, vec![]).await;
        assert_eq!(res.status, 404);
        let res = call(&core, "DELETE", "/api/status", None, vec![]).await;
        assert_eq!(res.status, 405);

        core.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    /// The cold-start case: the saved session is still logging in (`resuming`), but the chats
    /// are on the phone and the reads that show them must not wait for the login.
    #[tokio::test]
    async fn chats_are_served_from_the_store_while_the_saved_session_is_still_resuming() {
        use crate::testing::{FakeLogin, LoginStep};
        use crate::types::{StoredMessage, ThreadInfo, ThreadKind};

        let dir = temp_dir("zalo-embedded-resuming");
        fs::write(
            dir.join("credentials.json"),
            r#"{"imei":"i","cookie":[],"userAgent":"ua"}"#,
        )
        .unwrap();
        let store = ChatStore::in_memory();
        store.upsert_thread(ThreadInfo {
            id: "100".into(),
            kind: ThreadKind::User,
            name: "Alice".into(),
            avatar: String::new(),
        });
        let message: StoredMessage = serde_json::from_value(serde_json::json!({
            "id": "1",
            "threadId": "100",
            "threadKind": "user",
            "fromId": "100",
            "ts": 1_700_000_000_000i64,
            "text": "saved earlier",
        }))
        .unwrap();
        assert!(store.add_message(message));

        // The login never finishes: Zalo is still being reached.
        let login = FakeLogin::new(vec![LoginStep::cookie(|_| Box::pin(std::future::pending()))]);
        let session = Arc::new(ZaloSession::with_login(
            store.clone(),
            dir.clone(),
            true,
            login,
        ));
        let state = AppState {
            config: Arc::new(Config::from_lookup(|_| None).unwrap()),
            session: Arc::clone(&session),
            store: store.clone(),
        };
        let calls = crate::routes::calls::start_manager(&state);
        let history_sync = crate::routes::history_sync::start_manager(state.clone());
        session.start();
        let (shutdown_tx, _) = watch::channel(false);
        let core = EmbeddedCore {
            state,
            calls,
            history_sync,
            session: Arc::clone(&session),
            store,
            shutdown_tx,
        };

        let res = call(&core, "GET", "/api/status", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v["state"], "logging_in");
        assert_eq!(v["resuming"], true);

        let res = call(&core, "GET", "/api/threads", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["name"], "Alice");
        assert_eq!(v[0]["lastMessage"]["text"], "saved earlier");

        let res = call(&core, "GET", "/api/threads/user/100/messages", None, vec![]).await;
        assert_eq!(res.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["text"], "saved earlier");

        // Reading a chat marks it read locally; the receipt to Zalo is skipped, not an error.
        let res = call(&core, "POST", "/api/threads/user/100/read", None, vec![]).await;
        assert_eq!(res.status, 204);

        // Sending needs the login: a plain 409 the app can queue and retry.
        let res = call(
            &core,
            "POST",
            "/api/threads/user/100/messages",
            Some("application/json"),
            br#"{"text":"hi"}"#.to_vec(),
        )
        .await;
        assert_eq!(res.status, 409);

        core.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn events_start_with_status_then_stream_broadcasts_and_close_on_shutdown() {
        let dir = temp_dir("zalo-embedded-events");
        let core = start(&dir);
        let events = core.events();
        let wait = Duration::from_millis(300);

        match events.next(wait).await {
            EventPoll::Event(text) => {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(v["type"], "status");
                assert_eq!(v["status"]["state"], "logged_out");
            }
            other => panic!("expected the status event, got {other:?}"),
        }
        assert_eq!(
            events.next(Duration::from_millis(20)).await,
            EventPoll::Timeout
        );

        core.session
            .broadcast_event(serde_json::json!({ "type": "threads" }));
        assert_eq!(
            events.next(wait).await,
            EventPoll::Event(r#"{"type":"threads"}"#.to_string())
        );

        core.shutdown();
        assert_eq!(events.next(wait).await, EventPoll::Closed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn closing_an_events_handle_stops_a_waiting_reader() {
        let dir = temp_dir("zalo-embedded-close");
        let core = start(&dir);
        let events = Arc::new(core.events());
        let _ = events.next(Duration::from_millis(50)).await; // the status event
        let waiter = {
            let events = Arc::clone(&events);
            tokio::spawn(async move { events.next(Duration::from_secs(30)).await })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        events.close();
        assert_eq!(waiter.await.unwrap(), EventPoll::Closed);
        core.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn audio_without_a_call_is_closed_1008() {
        let dir = temp_dir("zalo-embedded-audio");
        let core = start(&dir);
        match core.open_audio() {
            Err((code, reason)) => assert_eq!((code, reason.as_str()), (1008, "No active call")),
            Ok(_) => panic!("no call is active"),
        }
        core.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_core_starts_again_after_shutdown() {
        let dir = temp_dir("zalo-embedded-restart");
        for _ in 0..3 {
            let core = start(&dir);
            let res = call(&core, "GET", "/api/status", None, vec![]).await;
            assert_eq!(res.status, 200);
            core.shutdown();
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
