//! Rotating file logger (matching `src/index.ts`) and redacted JSONL `CallTrace`
//! (matching `src/calls.ts`).

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value};

/// Startup rotation threshold for the server log file (256 KiB).
pub const LOG_STARTUP_ROTATE_BYTES: u64 = 256 * 1024;
/// Runtime rotation threshold for the server log file (2 MiB).
pub const LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Rotation threshold for `call-trace.jsonl` (2 MiB).
pub const TRACE_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Maximum recursion depth when redacting nested JSON structures in call traces.
const REDACT_MAX_DEPTH: usize = 16;

/// Sensitive field keys stripped from `call-trace.jsonl` (matched case-insensitively).
const TRACE_SENSITIVE_KEYS: &[&str] = &[
    "session",
    "sessid",
    "session_id",
    "srtpkey",
    "srtp_key",
    "token",
    "p2p",
    "nataddress",
    "address",
    "displayname",
    "callername",
    "dname",
    "avatar",
    "calleravatar",
];

fn backup_path(path: &Path) -> PathBuf {
    let mut os = OsString::from(path.as_os_str());
    os.push(".1");
    PathBuf::from(os)
}

pub(crate) fn create_dir_private(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn open_append_private(path: &Path) -> io::Result<fs::File> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

struct LoggerInner {
    bytes: u64,
}

/// Thread-safe rotating file logger matching `src/index.ts`:
/// - On startup: if existing file exceeds 256 KiB, rotates to `<file>.1`.
/// - At runtime: if writing a line would exceed 2 MiB, rotates to `<file>.1`.
/// - Uses mode `0o600` for files and `0o700` for created parent directories on Unix.
pub struct RotatingFileLogger {
    file: PathBuf,
    inner: Mutex<LoggerInner>,
}

impl RotatingFileLogger {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        let file = file.into();
        let mut bytes = 0u64;
        if let Some(parent) = file.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = create_dir_private(parent);
            }
        }
        if let Ok(meta) = fs::metadata(&file) {
            let size = meta.len();
            if size > LOG_STARTUP_ROTATE_BYTES {
                let _ = fs::rename(&file, backup_path(&file));
                bytes = 0;
            } else {
                bytes = size;
            }
        }
        Self {
            file,
            inner: Mutex::new(LoggerInner { bytes }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    /// Writes a timestamped log line (`<iso> <level> <message>\n`). Never panics on I/O errors.
    pub fn log(&self, level: &str, message: &str) {
        let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let line = format!("{ts} {level} {message}\n");
        self.write_raw(line.as_bytes());
    }

    /// Writes raw bytes to the rotating file (rotates at `LOG_MAX_BYTES`).
    pub fn write_raw(&self, data: &[u8]) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        let len = data.len() as u64;
        if state.bytes + len > LOG_MAX_BYTES && self.file.exists() {
            let _ = fs::rename(&self.file, backup_path(&self.file));
            state.bytes = 0;
        }
        if let Ok(mut f) = open_append_private(&self.file) {
            if f.write_all(data).is_ok() {
                state.bytes += len;
            }
        }
    }

    /// Reads the rotated backup (if present) followed by the current log file.
    pub fn read(&self) -> String {
        let mut out = String::new();
        for p in [backup_path(&self.file), self.file.clone()] {
            if let Ok(s) = fs::read_to_string(&p) {
                out.push_str(&s);
            }
        }
        out
    }
}

impl Write for &RotatingFileLogger {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_raw(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    TRACE_SENSITIVE_KEYS.contains(&lower.as_str())
}

/// Parses JSON while converting bare 16+ digit integers to strings (matching `parseJsonKeepingBigInts`
/// in `src/calls.ts`) so long Zalo IDs inside nested JSON strings never lose precision.
pub fn parse_json_keeping_bigints(text: &str) -> Result<Value, serde_json::Error> {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\\' && i + 1 < bytes.len() {
                // Copy escape sequence verbatim (may be multibyte UTF-8 after backslash, handle char safely).
                out.push('\\');
                i += 1;
                if let Some(ch) = text[i..].chars().next() {
                    out.push(ch);
                    i += ch.len_utf8();
                }
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        if b == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }

        if b == b'-' || b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit()
                    || matches!(bytes[i], b'.' | b'e' | b'E' | b'+' | b'-'))
            {
                i += 1;
            }
            let num = &text[start..i];
            let digits = num.strip_prefix('-').unwrap_or(num);
            if digits.len() >= 16 && digits.bytes().all(|c| c.is_ascii_digit()) {
                out.push('"');
                out.push_str(num);
                out.push('"');
            } else {
                out.push_str(num);
            }
            continue;
        }

        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    serde_json::from_str(&out)
}

fn redact_walk(value: &Value, depth: usize) -> (Value, bool) {
    if depth > REDACT_MAX_DEPTH {
        return (value.clone(), false);
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => (value.clone(), false),
        Value::String(s) => {
            let trimmed = s.trim();
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                if let Ok(parsed) = parse_json_keeping_bigints(s) {
                    if parsed.is_object() || parsed.is_array() {
                        let (inner_val, changed) = redact_walk(&parsed, depth + 1);
                        if changed {
                            if let Ok(serialized) = serde_json::to_string(&inner_val) {
                                return (Value::String(serialized), true);
                            }
                        }
                    }
                }
            }
            (value.clone(), false)
        }
        Value::Array(arr) => {
            let mut changed = false;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let (v, c) = redact_walk(item, depth + 1);
                if c {
                    changed = true;
                }
                out.push(v);
            }
            (Value::Array(out), changed)
        }
        Value::Object(map) => {
            let mut changed = false;
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_key(k) {
                    if v.is_null() {
                        out.insert(k.clone(), Value::Null);
                    } else {
                        out.insert(k.clone(), Value::String("[redacted]".to_string()));
                        changed = true;
                    }
                } else {
                    let (rv, c) = redact_walk(v, depth + 1);
                    if c {
                        changed = true;
                    }
                    out.insert(k.clone(), rv);
                }
            }
            (Value::Object(out), changed)
        }
    }
}

/// Recursively redacts sensitive secrets and PII (session/SRTP keys, media tokens, P2P/NAT
/// addresses, caller names and avatars) from a call-trace entry while keeping call IDs, action
/// names, user IDs, server host/ports and stats. Also parses and re-serializes nested JSON strings.
pub fn redact_for_trace(value: &Value) -> Value {
    redact_walk(value, 0).0
}

/// Rotating JSONL call trace (`<data_dir>/call-trace.jsonl`, rotates at 2 MiB to `.1`).
pub struct CallTrace {
    file: PathBuf,
    lock: Mutex<()>,
}

impl CallTrace {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            file: data_dir.as_ref().join("call-trace.jsonl"),
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    /// Redacts `entry`, adds an ISO-8601 `"at"` timestamp, and appends a JSON line.
    pub fn record(&self, entry: &Value) {
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        if let Err(err) = self.record_inner(entry) {
            tracing::warn!("Call log: {err}");
        }
    }

    fn record_inner(&self, entry: &Value) -> io::Result<()> {
        if let Some(parent) = self.file.parent() {
            create_dir_private(parent)?;
        }
        if let Ok(meta) = fs::metadata(&self.file) {
            if meta.len() > TRACE_MAX_BYTES {
                let _ = fs::rename(&self.file, backup_path(&self.file));
            }
        }
        let safe = redact_for_trace(entry);
        let at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let mut obj = Map::new();
        obj.insert("at".to_string(), Value::String(at));
        match safe {
            Value::Object(m) => {
                for (k, v) in m {
                    obj.insert(k, v);
                }
            }
            other => {
                obj.insert("value".to_string(), other);
            }
        }
        let mut line = serde_json::to_string(&Value::Object(obj))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        let mut f = open_append_private(&self.file)?;
        f.write_all(line.as_bytes())
    }

    /// Reads the rotated backup (if present) followed by the current trace file.
    pub fn read(&self) -> String {
        let mut out = String::new();
        for p in [backup_path(&self.file), self.file.clone()] {
            if let Ok(s) = fs::read_to_string(&p) {
                out.push_str(&s);
            }
        }
        out
    }

    /// Removes both the active trace file and its `.1` backup.
    pub fn clear(&self) {
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        for p in [self.file.clone(), backup_path(&self.file)] {
            let _ = fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(prefix: &str) -> PathBuf {
        let n: u64 = rand::random();
        let dir = std::env::temp_dir().join(format!("{prefix}-{n:016x}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn redact_for_trace_strips_secrets_and_nested_json_strings() {
        let nested_extend = json!({ "p2p": [{ "ip": "10.0.0.1", "port": 5000 }], "callType": 0 });
        let nested_params = json!({
            "sessId": "NESTED-SESS-ID",
            "Dname": "Alice",
            "avatar": "https://cdn.example/nested.jpg",
            "extendData": nested_extend.to_string(),
        });
        let nested_data = json!({
            "callId": "77",
            "session": "NESTED-SECRET-SESSION",
            "params": nested_params.to_string(),
        });
        let sample = json!({
            "kind": "test",
            "callId": "77",
            "callerId": "12345",
            "server": "1.2.3.4:4200",
            "rtpAddress": "1.2.3.4:4200",
            "token": "998877",
            "address": "198.51.100.2:55555",
            "natAddress": "198.51.100.2:55555",
            "session": "SECRET-SESSION-KEY-0123456789abcdef",
            "sessId": "SECRET-SESS-ID",
            "session_id": "SECRET-SESSION-ID",
            "srtpKey": "SECRET-SRTP-KEY",
            "srtp_key": "SECRET-SRTP-KEY-2",
            "p2p": [{ "ip": "198.51.100.7", "port": 1234 }],
            "displayName": "Alice",
            "callerName": "Alice",
            "avatar": "https://cdn.example/avatar.jpg",
            "callerAvatar": "https://cdn.example/caller.jpg",
            "stats": { "sent": { "audio": 10 }, "received": { "audio": 12 } },
            "control": {
                "content": {
                    "act_type": "voip",
                    "act": "request",
                    "data": nested_data.to_string(),
                }
            }
        });

        let redacted = redact_for_trace(&sample);
        assert_eq!(redacted["callId"], "77");
        assert_eq!(redacted["callerId"], "12345");
        assert_eq!(redacted["server"], "1.2.3.4:4200");
        assert_eq!(redacted["rtpAddress"], "1.2.3.4:4200");
        assert_eq!(
            redacted["stats"],
            json!({ "sent": { "audio": 10 }, "received": { "audio": 12 } })
        );
        for k in [
            "token",
            "address",
            "natAddress",
            "session",
            "sessId",
            "session_id",
            "srtpKey",
            "srtp_key",
            "p2p",
            "displayName",
            "callerName",
            "avatar",
            "callerAvatar",
        ] {
            assert_eq!(redacted[k], "[redacted]", "{k} should be redacted");
        }

        let dir = temp_dir("zalo-trace-test").join("nested");
        let trace = CallTrace::new(&dir);
        trace.record(&sample);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }

        let raw = trace.read();
        for secret in [
            "SECRET-SESSION",
            "SECRET-SESS",
            "SECRET-SRTP",
            "998877",
            "198.51.100",
            "10.0.0.1",
            "Alice",
            "cdn.example",
            "NESTED-SECRET",
            "NESTED-SESS",
        ] {
            assert!(!raw.contains(secret), "trace must not contain {secret}");
        }
        assert!(raw.includes_str("1.2.3.4:4200"));
        assert!(raw.includes_str("\"callId\":\"77\""));

        trace.clear();
        assert!(trace.read().is_empty());
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    trait StrExt {
        fn includes_str(&self, needle: &str) -> bool;
    }
    impl StrExt for str {
        fn includes_str(&self, needle: &str) -> bool {
            self.contains(needle)
        }
    }

    #[test]
    fn rotating_logger_rotates_on_startup_over_256kib() {
        let dir = temp_dir("zalo-log-test");
        let log_path = dir.join("server.log");
        fs::write(&log_path, vec![b'a'; (LOG_STARTUP_ROTATE_BYTES as usize) + 10]).unwrap();

        let logger = RotatingFileLogger::new(&log_path);
        logger.log("log", "fresh start");

        assert!(backup_path(&log_path).exists());
        let current = fs::read_to_string(&log_path).unwrap();
        assert!(current.contains("fresh start"));
        assert!(current.len() < 200);

        let _ = fs::remove_dir_all(&dir);
    }
}
