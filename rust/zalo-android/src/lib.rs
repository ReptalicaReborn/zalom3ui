//! Android JNI bridge (`libzalocore.so`): the Zalo core running in-process, called directly by the app.
//!
//! No socket, port or token. Kotlin (`com.zalom3ui.app.server.ZaloCore`) calls blocking native
//! functions from its own background threads (a pull model): Rust never calls into the JVM from a
//! Rust-owned thread, so there is no `AttachCurrentThread` and no global ref to a Kotlin callback.
//!
//! The functions run the app's axum router in-process ([`zalo_server::embedded`]):
//! - `start(dataDir, logFile): Int` (idempotent) and `stop()`;
//! - `request` / `requestFile` return one encoded response, see [`encode_response`];
//! - `eventsOpen` / `eventsNext` / `eventsClose`: the `/ws` event stream;
//! - `audioOpen` / `audioSend` / `audioNext` / `audioCloseReason` / `audioClose`: `/ws/call-audio`.
//!
//! Nothing here panics across the FFI boundary: every entry point catches panics and turns errors
//! into return values. (Release builds use `panic = "unwind"`, so panics caught at the JNI
//! boundary degrade cleanly into error values instead of aborting the Android process.)

use std::collections::HashMap;
use std::fmt::{self, Write as FmtWrite};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, Once, OnceLock};
use std::time::Duration;

use jni::objects::{JByteArray, JObject, JString};
use jni::sys::{jboolean, jbyte, jbyteArray, jint, jlong, jsize, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use tokio::runtime::Runtime;
use tracing::field::{Field, Visit};
use tracing::{Event, Id, Level, Metadata, Subscriber};
use zalo_server::embedded::{
    AudioPoll, EmbeddedAudio, EmbeddedConfig, EmbeddedCore, EmbeddedEvents, EmbeddedResponse,
    EventPoll,
};
use zalo_server::log::RotatingFileLogger;

static LOGGER_INIT: Once = Once::new();
static ACTIVE_LOG_FILE: Mutex<Option<Arc<RotatingFileLogger>>> = Mutex::new(None);
static FILE_LOG_TX: OnceLock<SyncSender<(Arc<RotatingFileLogger>, &'static str, String)>> =
    OnceLock::new();

fn file_log_tx() -> &'static SyncSender<(Arc<RotatingFileLogger>, &'static str, String)> {
    FILE_LOG_TX.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<(Arc<RotatingFileLogger>, &'static str, String)>(256);
        let _ = std::thread::Builder::new()
            .name("zalocore-log".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                while let Ok((logger, level, msg)) = rx.recv() {
                    logger.log(level, &msg);
                }
            });
        tx
    })
}

fn init_logging() {
    LOGGER_INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag("ZalooCore")
                .with_max_level(log::LevelFilter::Info),
        );
        let _ = tracing::subscriber::set_global_default(AndroidTracingSubscriber);
    });
}

struct FieldVisitor {
    message: String,
    fields: String,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={}", field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}

struct AndroidTracingSubscriber;

impl Subscriber for AndroidTracingSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::INFO
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let meta = event.metadata();
        let (log_level, level_str) = match *meta.level() {
            Level::ERROR => (log::Level::Error, "error"),
            Level::WARN => (log::Level::Warn, "warn"),
            Level::INFO => (log::Level::Info, "info"),
            Level::DEBUG => (log::Level::Debug, "debug"),
            Level::TRACE => (log::Level::Trace, "trace"),
        };
        let mut visitor = FieldVisitor {
            message: String::new(),
            fields: String::new(),
        };
        event.record(&mut visitor);
        let mut text = visitor.message;
        if !visitor.fields.is_empty() {
            text.push_str(&visitor.fields);
        }
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        log::log!(target: meta.target(), log_level, "{trimmed}");
        if let Ok(guard) = ACTIVE_LOG_FILE.lock() {
            if let Some(ref logger) = *guard {
                let _ = file_log_tx().try_send((Arc::clone(logger), level_str, trimmed.to_string()));
            }
        }
    }
}

// ---- Process-wide state ----

/// The running core and the runtime that drives it. A new runtime per start, so a restart in the
/// same process begins with no leftover tasks.
#[derive(Clone)]
struct Running {
    rt: Arc<Runtime>,
    core: Arc<EmbeddedCore>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

/// A subscription together with the runtime it waits on (which it keeps alive).
struct Sub<T> {
    rt: Arc<Runtime>,
    inner: T,
}

type SubMap<T> = Mutex<Option<HashMap<i64, Arc<Sub<T>>>>>;
static EVENTS: SubMap<EmbeddedEvents> = Mutex::new(None);
static AUDIO: SubMap<EmbeddedAudio> = Mutex::new(None);

/// Result codes of `audioOpen` below zero.
const AUDIO_NO_CALL: jlong = -1008;
const AUDIO_UNAVAILABLE: jlong = -1;
/// Longest wait one `eventsNext` / `audioNext` call may make.
const MAX_WAIT_MS: i64 = 60_000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn running() -> Option<Running> {
    lock(&RUNNING).clone()
}

fn insert_sub<T>(map: &SubMap<T>, sub: Sub<T>) -> i64 {
    let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    lock(map)
        .get_or_insert_with(HashMap::new)
        .insert(id, Arc::new(sub));
    id
}

fn get_sub<T>(map: &SubMap<T>, id: i64) -> Option<Arc<Sub<T>>> {
    lock(map).as_ref().and_then(|m| m.get(&id).cloned())
}

fn remove_sub<T>(map: &SubMap<T>, id: i64) -> Option<Arc<Sub<T>>> {
    lock(map).as_mut().and_then(|m| m.remove(&id))
}

fn timeout_of(ms: jlong) -> Duration {
    Duration::from_millis(ms.clamp(0, MAX_WAIT_MS) as u64)
}

/// Runs `f`, returning `default` if it panics.
fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(_) => {
            log::error!("Panic caught at the JNI boundary");
            default
        }
    }
}

// ---- Wire encoding ----

/// Response of `request` / `requestFile`: `u16 BE status | u16 BE contentTypeLength | contentType
/// (UTF-8) | body`. Status `0` means the call itself failed (core not running, bad arguments,
/// unreadable file): the content type is `text/plain` and the body is the message.
pub fn encode_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let ct = &content_type.as_bytes()[..content_type.len().min(u16::MAX as usize)];
    let mut out = Vec::with_capacity(4 + ct.len() + body.len());
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&(ct.len() as u16).to_be_bytes());
    out.extend_from_slice(ct);
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
fn encode_failure(message: &str) -> Vec<u8> {
    encode_response(0, "text/plain", message.as_bytes())
}

// ---- JNI helpers ----

fn as_jbytes(bytes: &[u8]) -> &[jbyte] {
    // SAFETY: `u8` and `jbyte` (`i8`) have identical size (1) and alignment (1), and all bit
    // patterns are valid for both types.
    unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<jbyte>(), bytes.len()) }
}

/// Single-copy JNI encoding: allocates the Java `byte[]` once and copies the 4-byte header,
/// content type, and body directly into it without an intermediate `Vec<u8>`.
fn encode_response_to_jbytes(
    env: &mut JNIEnv,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> jbyteArray {
    let ct = &content_type.as_bytes()[..content_type.len().min(u16::MAX as usize)];
    let Ok(total) = jsize::try_from(4 + ct.len() + body.len()) else {
        return ptr::null_mut();
    };
    let Ok(arr) = env.new_byte_array(total) else {
        return ptr::null_mut();
    };
    let status_be = status.to_be_bytes();
    let ct_len_be = (ct.len() as u16).to_be_bytes();
    let header = [status_be[0], status_be[1], ct_len_be[0], ct_len_be[1]];
    if env
        .set_byte_array_region(&arr, 0, as_jbytes(&header))
        .is_err()
    {
        return ptr::null_mut();
    }
    if !ct.is_empty()
        && env
            .set_byte_array_region(&arr, 4, as_jbytes(ct))
            .is_err()
    {
        return ptr::null_mut();
    }
    if !body.is_empty()
        && env
            .set_byte_array_region(&arr, (4 + ct.len()) as jsize, as_jbytes(body))
            .is_err()
    {
        return ptr::null_mut();
    }
    arr.into_raw()
}

fn encode_result_to_jbytes(env: &mut JNIEnv, res: Result<EmbeddedResponse, String>) -> jbyteArray {
    match res {
        Ok(r) => encode_response_to_jbytes(env, r.status, &r.content_type, &r.body),
        Err(msg) => encode_response_to_jbytes(env, 0, "text/plain", msg.as_bytes()),
    }
}

fn read_jstring(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    if s.is_null() {
        return Ok(String::new());
    }
    env.get_string(s)
        .map(|js| js.into())
        .map_err(|e| format!("Failed to read JNI string: {e}"))
}

fn read_bytes(env: &mut JNIEnv, b: &JByteArray) -> Result<Vec<u8>, String> {
    if b.is_null() {
        return Ok(Vec::new());
    }
    env.convert_byte_array(b)
        .map_err(|e| format!("Failed to read JNI byte array: {e}"))
}

fn to_jbytes(env: &mut JNIEnv, data: &[u8]) -> jbyteArray {
    env.byte_array_from_slice(data)
        .map(|a| a.into_raw())
        .unwrap_or(ptr::null_mut())
}

fn to_jstring(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|s| s.into_raw())
        .unwrap_or(ptr::null_mut())
}

fn record_error(file_logger: &Option<Arc<RotatingFileLogger>>, msg: &str) {
    log::error!("{msg}");
    if let Some(ref logger) = file_logger {
        logger.log("error", msg);
    }
}

// ---- start / stop ----

/// Starts the core in this process. Idempotent: a second call while it runs returns 0 at once.
/// Opens the store and starts the session before returning (a moment of disk work), then the core
/// runs on its own threads. Returns 0 on success, 1 on failure (logged).
fn start_core(data_dir: &str, log_file: &str) -> jint {
    init_logging();

    let mut running = lock(&RUNNING);
    if running.is_some() {
        return 0;
    }

    let log_file_path = {
        let trimmed = log_file.trim();
        (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
    };
    let file_logger = log_file_path
        .as_ref()
        .map(|p| Arc::new(RotatingFileLogger::new(p)));
    *lock(&ACTIVE_LOG_FILE) = file_logger.clone();

    let data_dir = data_dir.trim();
    if data_dir.is_empty() {
        record_error(&file_logger, "dataDir must not be empty");
        return 1;
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .thread_stack_size(512 * 1024)
        .enable_all()
        .thread_name("zalocore")
        .build()
    {
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            record_error(&file_logger, &format!("Failed to build Tokio runtime: {e}"));
            return 1;
        }
    };
    let config = EmbeddedConfig {
        data_dir: PathBuf::from(data_dir),
        read_receipts: true,
    };
    let core = Arc::new(rt.block_on(async move { EmbeddedCore::start(config) }));
    *running = Some(Running { rt, core });
    log::info!("Zalỏ core started");
    if let Some(ref logger) = file_logger {
        logger.log("log", "Zalỏ core started");
    }
    0
}

/// Stops the core: closes every subscription, stops the session, flushes the store, and winds the
/// runtime down. Safe to call when not running; `start` works again afterwards.
fn stop_core() {
    let Some(r) = lock(&RUNNING).take() else {
        return;
    };
    for map in [&EVENTS as &dyn CloseAll, &AUDIO as &dyn CloseAll] {
        map.close_all();
    }
    {
        let _enter = r.rt.enter();
        r.core.shutdown();
    }
    drop(r.core);
    // A call still in flight keeps the runtime until it returns; it then drops it.
    if let Ok(rt) = Arc::try_unwrap(r.rt) {
        rt.shutdown_timeout(Duration::from_secs(2));
    }
    log::info!("Zalỏ core stopped");
    if let Some(logger) = lock(&ACTIVE_LOG_FILE).as_ref() {
        logger.log("log", "Zalỏ core stopped");
    }
}

trait CloseAll {
    fn close_all(&self);
}

impl CloseAll for SubMap<EmbeddedEvents> {
    fn close_all(&self) {
        if let Some(m) = lock(self).take() {
            for sub in m.values() {
                sub.inner.close();
            }
        }
    }
}

impl CloseAll for SubMap<EmbeddedAudio> {
    fn close_all(&self) {
        if let Some(m) = lock(self).take() {
            for sub in m.values() {
                sub.inner.close();
            }
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_start(
    mut env: JNIEnv,
    _this: JObject,
    data_dir: JString,
    log_file: JString,
) -> jint {
    guard(1, || {
        let data_dir = read_jstring(&mut env, &data_dir);
        let log_file = read_jstring(&mut env, &log_file).unwrap_or_default();
        match data_dir {
            Ok(dir) => start_core(&dir, &log_file),
            Err(e) => {
                log::error!("{e}");
                1
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_stop(
    _env: JNIEnv,
    _this: JObject,
) {
    guard((), stop_core)
}

// ---- request / requestFile ----

fn run_request(
    method: &str,
    path: &str,
    content_type: &str,
    body: Result<Vec<u8>, String>,
    file: Option<&Path>,
) -> Result<EmbeddedResponse, String> {
    let Some(r) = running() else {
        return Err("The Zalo core is not running".to_string());
    };
    let ct = (!content_type.is_empty()).then_some(content_type);
    r.rt.block_on(async {
        match file {
            Some(file) => r.core.request_file(method, path, ct, file).await,
            None => {
                let body = body?;
                r.core
                    .request(method, path, ct, axum_body(body))
                    .await
            }
        }
    })
}

#[cfg(test)]
fn do_request(
    method: &str,
    path: &str,
    content_type: &str,
    body: Result<Vec<u8>, String>,
    file: Option<&Path>,
) -> Vec<u8> {
    match run_request(method, path, content_type, body, file) {
        Ok(res) => encode_response(res.status, &res.content_type, &res.body),
        Err(e) => encode_failure(&e),
    }
}

fn axum_body(bytes: Vec<u8>) -> zalo_server::embedded::Body {
    zalo_server::embedded::Body::from(bytes)
}

/// Runs one request through the in-process router. See [`encode_response`] for the result.
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_request(
    mut env: JNIEnv,
    _this: JObject,
    method: JString,
    path_and_query: JString,
    content_type: JString,
    body: JByteArray,
) -> jbyteArray {
    let res = guard(Err("Native error".to_string()), || {
        let method = read_jstring(&mut env, &method);
        let path = read_jstring(&mut env, &path_and_query);
        let ct = read_jstring(&mut env, &content_type);
        let body = read_bytes(&mut env, &body);
        match (method, path, ct) {
            (Ok(m), Ok(p), Ok(c)) => run_request(&m, &p, &c, body, None),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        }
    });
    encode_result_to_jbytes(&mut env, res)
}

/// Like `request`, with the body read from `file_path` in chunks (never held whole in memory).
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_requestFile(
    mut env: JNIEnv,
    _this: JObject,
    method: JString,
    path_and_query: JString,
    content_type: JString,
    file_path: JString,
) -> jbyteArray {
    let res = guard(Err("Native error".to_string()), || {
        let method = read_jstring(&mut env, &method);
        let path = read_jstring(&mut env, &path_and_query);
        let ct = read_jstring(&mut env, &content_type);
        let file = read_jstring(&mut env, &file_path);
        match (method, path, ct, file) {
            (Ok(m), Ok(p), Ok(c), Ok(f)) => {
                run_request(&m, &p, &c, Ok(Vec::new()), Some(Path::new(&f)))
            }
            (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (_, _, _, Err(e)) => Err(e),
        }
    });
    encode_result_to_jbytes(&mut env, res)
}

// ---- Events ----

/// Opens an event subscription. Returns a handle above zero, or 0 when the core is not running.
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_eventsOpen(
    _env: JNIEnv,
    _this: JObject,
) -> jlong {
    guard(0, || match running() {
        Some(r) => {
            let inner = r.core.events();
            insert_sub(&EVENTS, Sub { rt: r.rt, inner })
        }
        None => 0,
    })
}

/// Waits up to `timeout_ms` for the next event: its JSON text; `null` on timeout; the empty
/// string once the subscription is closed (closed handle, unknown handle, or core stopped).
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_eventsNext(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    timeout_ms: jlong,
) -> jstring {
    let poll = guard(EventPoll::Closed, || match get_sub(&EVENTS, handle) {
        Some(sub) => sub.rt.block_on(sub.inner.next(timeout_of(timeout_ms))),
        None => EventPoll::Closed,
    });
    match poll {
        EventPoll::Event(text) => to_jstring(&mut env, &text),
        EventPoll::Timeout => ptr::null_mut(),
        EventPoll::Closed => to_jstring(&mut env, ""),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_eventsClose(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    guard((), || {
        if let Some(sub) = remove_sub(&EVENTS, handle) {
            sub.inner.close();
        }
    })
}

// ---- Call audio ----

/// Opens the call-audio link. Returns a handle above zero; `-1008` when there is no active call
/// (what the WebSocket path closes with); `-1` when the core is not running.
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_audioOpen(
    _env: JNIEnv,
    _this: JObject,
) -> jlong {
    guard(AUDIO_UNAVAILABLE, || match running() {
        Some(r) => {
            let _enter = r.rt.enter();
            match r.core.open_audio() {
                Ok(inner) => insert_sub(&AUDIO, Sub { rt: r.rt.clone(), inner }),
                Err(_) => AUDIO_NO_CALL,
            }
        }
        None => AUDIO_UNAVAILABLE,
    })
}

/// Sends one app-to-server frame (`0x02` audio, `0x04` video, `0x06` key-frame request). False
/// when the handle is unknown or the link is closed.
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_audioSend(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    data: JByteArray,
) -> jboolean {
    let ok = guard(false, || {
        let Some(sub) = get_sub(&AUDIO, handle) else {
            return false;
        };
        if sub.inner.is_closed() {
            return false;
        }
        match read_bytes(&mut env, &data) {
            Ok(bytes) => {
                let _enter = sub.rt.enter();
                sub.inner.send_owned(bytes);
                true
            }
            Err(_) => false,
        }
    });
    if ok {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// Waits up to `timeout_ms` for the next server-to-app frame (`0x01` audio, `0x03` video AU,
/// `0x05` key-frame request): the frame; `null` on timeout; an empty array once the link is
/// closed (then read [`audioCloseReason`]). Real frames are never empty.
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_audioNext(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    timeout_ms: jlong,
) -> jbyteArray {
    let poll = guard(AudioPoll::Closed, || match get_sub(&AUDIO, handle) {
        Some(sub) => sub.rt.block_on(sub.inner.next(timeout_of(timeout_ms))),
        None => AudioPoll::Closed,
    });
    match poll {
        AudioPoll::Frame(data) => to_jbytes(&mut env, &data),
        AudioPoll::Timeout => ptr::null_mut(),
        AudioPoll::Closed => to_jbytes(&mut env, &[]),
    }
}

/// Why the link closed, as `"<code> <reason>"` (for example `"1000 call ended"`); `null` while it
/// is open or the handle is unknown.
#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_audioCloseReason(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jstring {
    let info = guard(None, || {
        get_sub(&AUDIO, handle).and_then(|sub| sub.inner.close_info())
    });
    match info {
        Some((code, reason)) => to_jstring(&mut env, &format!("{code} {reason}")),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_ZaloCore_audioClose(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    guard((), || {
        if let Some(sub) = remove_sub(&AUDIO, handle) {
            sub.inner.close();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_encoding_is_status_content_type_then_body() {
        assert_eq!(
            encode_response(201, "application/json", b"{}"),
            [&[0u8, 201, 0, 16][..], b"application/json", b"{}"].concat()
        );
        // Status above 255 uses both bytes; an empty content type and body are fine.
        assert_eq!(encode_response(404, "", b""), vec![1, 0x94, 0, 0]);
        let failure = encode_failure("boom");
        assert_eq!(&failure[..2], &[0, 0]);
        assert_eq!(&failure[4 + "text/plain".len()..], b"boom");
    }

    #[test]
    fn the_core_requests_streams_and_restarts_through_the_native_layer() {
        let dir = std::env::temp_dir().join(format!("zalo-android-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.to_str().unwrap();

        // Before start: a failure value, never a panic.
        let res = do_request("GET", "/api/status", "", Ok(vec![]), None);
        assert_eq!(&res[..2], &[0, 0]);

        for _ in 0..2 {
            assert_eq!(start_core(data, ""), 0);
            assert_eq!(start_core(data, ""), 0, "start is idempotent");

            let res = do_request("GET", "/api/status", "", Ok(vec![]), None);
            assert_eq!(u16::from_be_bytes([res[0], res[1]]), 200);
            let ct_len = u16::from_be_bytes([res[2], res[3]]) as usize;
            assert!(std::str::from_utf8(&res[4..4 + ct_len])
                .unwrap()
                .starts_with("application/json"));
            assert!(std::str::from_utf8(&res[4 + ct_len..])
                .unwrap()
                .contains("logged_out"));

            let r = running().unwrap();
            let handle = insert_sub(&EVENTS, Sub { rt: r.rt.clone(), inner: r.core.events() });
            let sub = get_sub(&EVENTS, handle).unwrap();
            match sub.rt.block_on(sub.inner.next(Duration::from_secs(1))) {
                EventPoll::Event(text) => assert!(text.contains("\"status\"")),
                other => panic!("{other:?}"),
            }
            drop((sub, r));

            stop_core();
            // After stop the handle reports closed.
            assert!(get_sub(&EVENTS, handle).is_none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
