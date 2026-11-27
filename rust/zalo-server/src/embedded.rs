//! In-process core for embedders such as the Android app: the same axum [`Router`] the TCP server
//! uses, called directly (no socket, port or token), plus the event stream and the call-audio
//! link as plain handles. Every handler, validation and error mapping is the one the HTTP server
//! runs; only the transport differs.
//!
//! [`EmbeddedCore::start`] spawns tasks, so it must run inside a Tokio runtime. A core can be shut
//! down and a new one started in the same process.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use axum::body::Body;
use axum::body::Bytes;
use axum::http::{header, Method, Request, Uri};
use axum::Router;
use tokio::sync::watch;
use tower::ServiceExt;

use crate::config::Config;
use crate::http::{self, EventStream};
use crate::routes::calls::{AudioLink, AudioWsOut, CallManager, NO_ACTIVE_CALL_CLOSE};
use crate::session::ZaloSession;
use crate::store::ChatStore;

/// How the embedded core is set up.
#[derive(Debug, Clone)]
pub struct EmbeddedConfig {
    /// Where `credentials.json`, the message store and media live.
    pub data_dir: PathBuf,
    /// Send "seen" receipts to Zalo when a chat is read.
    pub read_receipts: bool,
}

/// What a request to the in-process router answered.
#[derive(Debug, Clone)]
pub struct EmbeddedResponse {
    pub status: u16,
    /// The `Content-Type` header, empty when there is none.
    pub content_type: String,
    pub body: Bytes,
}

/// The running core: session, store, and the app router.
pub struct EmbeddedCore {
    router: Router,
    calls: Arc<CallManager>,
    session: Arc<ZaloSession>,
    store: ChatStore,
    shutdown_tx: watch::Sender<bool>,
}

impl EmbeddedCore {
    /// Opens the store, creates and starts the session (resuming saved credentials), and builds
    /// the router. Must be called inside a Tokio runtime.
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
        // No network-facing checks (Host, Origin, token): only the app itself can call in here.
        let app = http::build_app(config, Arc::clone(&session), false);
        session.start();
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            router: app.router,
            calls: app.calls,
            session,
            store,
            shutdown_tx,
        }
    }

    /// Runs one request through the router. `path_and_query` is already percent-encoded, e.g.
    /// `/api/threads?limit=5`. `Err` only for a request that could not be built (bad method or
    /// URI); HTTP errors come back as ordinary responses.
    pub async fn request(
        &self,
        method: &str,
        path_and_query: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> Result<EmbeddedResponse, String> {
        let method = Method::from_bytes(method.trim().to_ascii_uppercase().as_bytes())
            .map_err(|_| format!("Invalid method: {method}"))?;
        let uri: Uri = path_and_query
            .parse()
            .map_err(|e| format!("Invalid path {path_and_query:?}: {e}"))?;
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(ct) = content_type.filter(|c| !c.is_empty()) {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }
        let request = builder
            .body(body)
            .map_err(|e| format!("Invalid request: {e}"))?;

        let response = match self.router.clone().oneshot(request).await {
            Ok(r) => r,
            Err(e) => match e {},
        };
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|e| format!("Could not read the response: {e}"))?;
        Ok(EmbeddedResponse {
            status,
            content_type,
            body,
        })
    }

    /// Like [`request`](Self::request), with the body streamed from a file in 64 KiB chunks, so a
    /// 100 MB video never has to sit in the caller's memory.
    pub async fn request_file(
        &self,
        method: &str,
        path_and_query: &str,
        content_type: Option<&str>,
        file_path: &std::path::Path,
    ) -> Result<EmbeddedResponse, String> {
        use tokio::io::AsyncReadExt;
        let file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| format!("Could not open {}: {e}", file_path.display()))?;
        let stream = futures_util::stream::unfold(file, |mut file| async move {
            let mut buf = vec![0u8; 64 * 1024];
            match file.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    Some((Ok::<Bytes, std::io::Error>(Bytes::from(buf)), file))
                }
                Err(e) => Some((Err(e), file)),
            }
        });
        self.request(method, path_and_query, content_type, Body::from_stream(stream))
            .await
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
        core.request(method, path, ct, Body::from(body)).await.unwrap()
    }

    #[tokio::test]
    async fn requests_run_the_real_router_without_host_origin_or_token() {
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

        // A file body streams through the same router.
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
            .request("BAD METHOD", "/api/status", None, Body::empty())
            .await
            .is_err());
        assert!(core
            .request("GET", "not a path", None, Body::empty())
            .await
            .is_err());

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
