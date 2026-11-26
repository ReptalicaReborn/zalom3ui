//! Test doubles for the Zalo protocol client (enabled by the `testing` cargo feature, and always
//! in this crate's own unit tests; never part of a production build).
//!
//! * [`FakeZaloApi`] implements [`ZaloApi`]: it records every call (method name and arguments as
//!   JSON), answers from scripted responses, and lets a test push [`WsEvent`]s into the session's
//!   listener loop with [`FakeZaloApi::emit`].
//! * [`FakeLogin`] implements [`ZaloLogin`] with scripted login outcomes (QR generated, scanned,
//!   declined, success with credentials, transient network error versus rejected session).
//!
//! Wire a session up with [`ZaloSession::with_login`] and, to skip logging in, attach a fake API
//! with [`ZaloSession::testing_attach`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use zalo_proto::{
    AddPollOptionsInput, AvatarSize, CreateGroupOptions, CreatePollInput, Credentials,
    DeleteMessageTarget, DestType, ForwardMessagePayload, LastMessageInfo, LoginQrEvent,
    MuteAction, MuteDuration, NoisedIdsReply, ProtoError, ReactionTarget, Result as ProtoResult,
    SeenMessageInput, SendMessageOptions, SendMessageResponse, StickerParam, ThreadType, UndoTarget,
    UploadResult, UploadSource, WsEvent, ZaloClient,
};

use crate::api::{BoxFuture, ListenerControl, ListenerStart, ZaloApi, ZaloLogin};
use crate::with_api_methods;

/// One recorded call to the fake client: the snake_case method name and its arguments as JSON
/// (serialized with serde, in declaration order).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: &'static str,
    pub args: Vec<Value>,
}

type Handler = Arc<dyn Fn(&[Value]) -> BoxFuture<'static, ProtoResult<Value>> + Send + Sync>;

struct ListenerSlot {
    tx: mpsc::UnboundedSender<WsEvent>,
}

#[derive(Default)]
struct FakeState {
    calls: Vec<Call>,
    handlers: HashMap<&'static str, Handler>,
    listeners: Vec<ListenerSlot>,
    listener_log: Vec<String>,
    listener_start_fails: bool,
}

/// A scripted, recording [`ZaloApi`]. Unscripted methods answer `null`, which deserializes into
/// an empty list/object for methods returning one (and fails loudly otherwise).
pub struct FakeZaloApi {
    own_id: String,
    state: Arc<Mutex<FakeState>>,
}

impl FakeZaloApi {
    pub fn new(own_id: &str) -> Arc<Self> {
        Arc::new(Self {
            own_id: own_id.to_string(),
            state: Arc::new(Mutex::new(FakeState::default())),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- scripting ----

    /// Always answers `method` with `value`.
    pub fn respond(&self, method: &'static str, value: Value) {
        self.on(method, move |_| Ok(value.clone()));
    }

    /// Always fails `method` with the error `make` builds.
    pub fn fail(&self, method: &'static str, make: impl Fn() -> ProtoError + Send + Sync + 'static) {
        self.on(method, move |_| Err(make()));
    }

    /// Answers `method` by calling `f` with the recorded-form arguments.
    pub fn on(
        &self,
        method: &'static str,
        f: impl Fn(&[Value]) -> ProtoResult<Value> + Send + Sync + 'static,
    ) {
        self.on_async(method, move |args| {
            let out = f(args);
            Box::pin(async move { out })
        });
    }

    /// Like [`Self::on`], with an answer that may take time (e.g. wait for a oneshot).
    pub fn on_async(
        &self,
        method: &'static str,
        f: impl Fn(&[Value]) -> BoxFuture<'static, ProtoResult<Value>> + Send + Sync + 'static,
    ) {
        self.lock().handlers.insert(method, Arc::new(f));
    }

    /// Makes the next (and every) `start_listener` fail like a missing WebSocket URL would.
    pub fn fail_listener_start(&self, fails: bool) {
        self.lock().listener_start_fails = fails;
    }

    // ---- inspection ----

    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    pub fn call_count(&self) -> usize {
        self.lock().calls.len()
    }

    /// Method names in call order.
    pub fn methods(&self) -> Vec<&'static str> {
        self.lock().calls.iter().map(|c| c.method).collect()
    }

    /// Arguments of every call to `method`, in call order.
    pub fn calls_to(&self, method: &str) -> Vec<Vec<Value>> {
        self.lock()
            .calls
            .iter()
            .filter(|c| c.method == method)
            .map(|c| c.args.clone())
            .collect()
    }

    pub fn clear_calls(&self) {
        self.lock().calls.clear();
    }

    /// Listener control calls (`start`, `stop`, `request_old_messages:<kind>`,
    /// `request_old_reactions:<kind>`), in order. Kept apart from [`Self::calls`].
    pub fn listener_log(&self) -> Vec<String> {
        self.lock().listener_log.clone()
    }

    /// How many times the session started a listener on this client.
    pub fn listener_starts(&self) -> usize {
        self.lock()
            .listener_log
            .iter()
            .filter(|l| l.as_str() == "start")
            .count()
    }

    /// Pushes an event into the most recently started listener's stream (as the WebSocket would).
    /// Returns `false` if no listener is running.
    pub fn emit(&self, event: WsEvent) -> bool {
        self.lock()
            .listeners
            .last()
            .is_some_and(|l| l.tx.send(event).is_ok())
    }

    fn dispatch(&self, method: &'static str, args: Vec<Value>) -> BoxFuture<'static, ProtoResult<Value>> {
        let handler = {
            let mut st = self.lock();
            st.calls.push(Call {
                method,
                args: args.clone(),
            });
            st.handlers.get(method).cloned()
        };
        match handler {
            Some(h) => h(&args),
            None => Box::pin(async { Ok(Value::Null) }),
        }
    }
}

/// Turns a scripted JSON answer into the method's return type; `null` becomes `[]` or `{}` where
/// the type takes one.
fn from_script<T: DeserializeOwned>(method: &str, value: Value) -> ProtoResult<T> {
    let candidates = if value.is_null() {
        vec![Value::Null, json!([]), json!({})]
    } else {
        vec![value]
    };
    let mut last_err = None;
    for v in candidates {
        match serde_json::from_value::<T>(v) {
            Ok(t) => return Ok(t),
            Err(e) => last_err = Some(e),
        }
    }
    Err(ProtoError::InvalidArg(format!(
        "fake {method}: scripted answer has the wrong shape ({})",
        last_err.map(|e| e.to_string()).unwrap_or_default()
    )))
}

struct FakeListenerControl {
    state: Arc<Mutex<FakeState>>,
}

impl FakeListenerControl {
    fn log(&self, line: String) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .listener_log
            .push(line);
    }
}

fn kind_of(thread_type: ThreadType) -> &'static str {
    match thread_type {
        ThreadType::User => "user",
        ThreadType::Group => "group",
    }
}

impl ListenerControl for FakeListenerControl {
    fn request_old_messages(&self, thread_type: ThreadType) {
        self.log(format!("request_old_messages:{}", kind_of(thread_type)));
    }

    fn request_old_reactions(&self, thread_type: ThreadType) {
        self.log(format!("request_old_reactions:{}", kind_of(thread_type)));
    }

    fn stop(&self) {
        self.log("stop".to_string());
    }
}

macro_rules! define_fake_methods {
    ($( $name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty ; )*) => {
        impl ZaloApi for FakeZaloApi {
            fn own_id(&self) -> String {
                self.own_id.clone()
            }

            fn concrete(&self) -> Option<Arc<ZaloClient>> {
                None
            }

            fn start_listener(&self) -> ProtoResult<ListenerStart> {
                let mut st = self.lock();
                if st.listener_start_fails {
                    return Err(ProtoError::WebSocket("fake: listener start failed".into()));
                }
                let (tx, rx) = mpsc::unbounded_channel();
                st.listeners.push(ListenerSlot { tx });
                st.listener_log.push("start".to_string());
                Ok(ListenerStart {
                    events: rx,
                    control: Arc::new(FakeListenerControl {
                        state: Arc::clone(&self.state),
                    }),
                })
            }

            $( fn $name<'a>(&'a self $(, $arg: $ty)*) -> BoxFuture<'a, ProtoResult<$ret>> {
                let args = vec![$( serde_json::to_value(&$arg).unwrap_or(Value::Null) ),*];
                let answer = self.dispatch(stringify!($name), args);
                Box::pin(async move { from_script::<$ret>(stringify!($name), answer.await?) })
            } )*
        }
    };
}

with_api_methods!(define_fake_methods);

// ---- login ----

/// Scripted `login_cookie` behavior.
pub type CookieLoginFn =
    Arc<dyn Fn(Credentials) -> BoxFuture<'static, ProtoResult<Arc<dyn ZaloApi>>> + Send + Sync>;

/// Scripted `login_qr` behavior: gets the event sender (to report QR progress) and the abort
/// receiver (fires on logout or a replacing login).
pub type QrLoginFn = Arc<
    dyn Fn(
            mpsc::UnboundedSender<LoginQrEvent>,
            oneshot::Receiver<()>,
        ) -> BoxFuture<'static, ProtoResult<(Credentials, Arc<dyn ZaloApi>)>>
        + Send
        + Sync,
>;

/// What one login attempt does. Like the TS tests' fake `Zalo` instances, each attempt (cookie or
/// QR) takes the next step; the last step repeats.
#[derive(Clone, Default)]
pub struct LoginStep {
    cookie: Option<CookieLoginFn>,
    qr: Option<QrLoginFn>,
}

impl LoginStep {
    pub fn cookie<F>(f: F) -> Self
    where
        F: Fn(Credentials) -> BoxFuture<'static, ProtoResult<Arc<dyn ZaloApi>>> + Send + Sync + 'static,
    {
        Self {
            cookie: Some(Arc::new(f)),
            qr: None,
        }
    }

    pub fn qr<F>(f: F) -> Self
    where
        F: Fn(
                mpsc::UnboundedSender<LoginQrEvent>,
                oneshot::Receiver<()>,
            ) -> BoxFuture<'static, ProtoResult<(Credentials, Arc<dyn ZaloApi>)>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            cookie: None,
            qr: Some(Arc::new(f)),
        }
    }

    /// A cookie login that always fails with the error `make` builds.
    pub fn cookie_error(make: impl Fn() -> ProtoError + Send + Sync + 'static) -> Self {
        Self::cookie(move |_| {
            let err = make();
            Box::pin(async move { Err(err) })
        })
    }

    /// A cookie login that always succeeds with `api`.
    pub fn cookie_ok(api: Arc<dyn ZaloApi>) -> Self {
        Self::cookie(move |_| {
            let api = Arc::clone(&api);
            Box::pin(async move { Ok(api) })
        })
    }
}

/// A [`ZaloLogin`] that follows a script of [`LoginStep`]s and counts attempts.
pub struct FakeLogin {
    steps: Vec<LoginStep>,
    next: AtomicUsize,
}

impl FakeLogin {
    pub fn new(steps: Vec<LoginStep>) -> Arc<Self> {
        Arc::new(Self {
            steps,
            next: AtomicUsize::new(0),
        })
    }

    /// How many login attempts (cookie or QR) were made.
    pub fn attempts(&self) -> usize {
        self.next.load(Ordering::SeqCst)
    }

    fn step(&self) -> Option<LoginStep> {
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        self.steps.get(n.min(self.steps.len().saturating_sub(1))).cloned()
    }
}

impl ZaloLogin for FakeLogin {
    fn login_cookie(&self, credentials: Credentials) -> BoxFuture<'_, ProtoResult<Arc<dyn ZaloApi>>> {
        let f = self.step().and_then(|s| s.cookie);
        Box::pin(async move {
            match f {
                Some(f) => f(credentials).await,
                None => Err(ProtoError::InvalidArg(
                    "fake login: no cookie login scripted for this attempt".into(),
                )),
            }
        })
    }

    fn login_qr(
        &self,
        event_tx: mpsc::UnboundedSender<LoginQrEvent>,
        abort_rx: oneshot::Receiver<()>,
    ) -> BoxFuture<'_, ProtoResult<(Credentials, Arc<dyn ZaloApi>)>> {
        let f = self.step().and_then(|s| s.qr);
        Box::pin(async move {
            match f {
                Some(f) => f(event_tx, abort_rx).await,
                None => Err(ProtoError::InvalidArg(
                    "fake login: no QR login scripted for this attempt".into(),
                )),
            }
        })
    }
}

/// Credentials as the TS login tests write them (`{ imei, cookie: [], userAgent }`).
pub fn test_credentials() -> Credentials {
    serde_json::from_value(json!({ "imei": "i", "cookie": [], "userAgent": "ua" }))
        .expect("test credentials")
}
