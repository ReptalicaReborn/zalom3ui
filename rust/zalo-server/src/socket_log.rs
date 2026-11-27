//! Socket diagnostic log: one structure-only entry per decoded WebSocket frame.
//!
//! The listener describes every frame it decodes as a [`FrameReport`] (header, whether a handler
//! arm consumed it, key names, array lengths and, per message item, msgType / content shape /
//! sender class / kept-or-dropped with the reason). The last [`MAX_ENTRIES`] are kept in memory and
//! appended to `<data_dir>/socket-log.jsonl` so they survive a restart. `GET /api/debug/socket-log`
//! renders them as plain text, newest first; `DELETE` clears them.
//!
//! **No message text, content values, names or phone numbers are ever recorded** (see
//! `zalo_proto::framelog`); ids, cmd numbers, key names, counts and drop reasons are.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zalo_proto::framelog::FrameReport;
use zalo_proto::time_util::format_rfc3339_millis;

/// Entries kept in memory and (after compaction) on disk.
pub const MAX_ENTRIES: usize = 500;
/// The file is rewritten down to [`MAX_ENTRIES`] lines once it reaches this many.
const COMPACT_AT: usize = MAX_ENTRIES * 2;
const FILE_NAME: &str = "socket-log.jsonl";
const FLUSH_DEBOUNCE: Duration = Duration::from_millis(250);

struct Inner {
    entries: VecDeque<FrameReport>,
    file_lines: usize,
    pending_lines: Vec<String>,
    pending_clear: bool,
    flush_scheduled: bool,
}

struct SharedLog {
    file: PathBuf,
    inner: Mutex<Inner>,
    io_lock: Mutex<()>,
}

fn is_keepalive_ping_reply(report: &FrameReport) -> bool {
    report.version == 1
        && report.cmd == 2
        && report.error.is_none()
        && report.arrays.is_empty()
}

impl SharedLog {
    fn flush_disk(&self) {
        let Ok(_io) = self.io_lock.lock() else {
            return;
        };

        enum Action {
            None,
            Clear,
            Append(Vec<String>),
            Compact(String),
        }

        let (do_clear, action) = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            inner.flush_scheduled = false;
            let do_clear = inner.pending_clear;
            inner.pending_clear = false;
            let lines = std::mem::take(&mut inner.pending_lines);

            if lines.is_empty() {
                (do_clear, if do_clear { Action::Clear } else { Action::None })
            } else if inner.file_lines + lines.len() >= COMPACT_AT {
                let mut text = String::new();
                for r in &inner.entries {
                    if !is_keepalive_ping_reply(r) {
                        if let Ok(line) = serde_json::to_string(r) {
                            text.push_str(&line);
                            text.push('\n');
                        }
                    }
                }
                inner.file_lines = inner
                    .entries
                    .iter()
                    .filter(|r| !is_keepalive_ping_reply(r))
                    .count();
                (false, Action::Compact(text))
            } else {
                inner.file_lines += lines.len();
                (do_clear, Action::Append(lines))
            }
        };

        if do_clear {
            let _ = fs::remove_file(&self.file);
        }
        match action {
            Action::None | Action::Clear => {}
            Action::Append(lines) => {
                if let Some(parent) = self.file.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.file) {
                    for line in &lines {
                        if writeln!(f, "{line}").is_err() {
                            break;
                        }
                    }
                }
            }
            Action::Compact(text) => {
                if let Some(parent) = self.file.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let tmp = self.file.with_extension("jsonl.tmp");
                if fs::write(&tmp, text).is_ok() {
                    let _ = fs::rename(&tmp, &self.file);
                }
            }
        }
    }
}

/// Bounded in-memory log with a JSONL file behind it.
pub struct SocketLog {
    shared: Arc<SharedLog>,
}

impl SocketLog {
    /// Opens the log, loading the last [`MAX_ENTRIES`] entries from `<data_dir>/socket-log.jsonl`.
    pub fn open(data_dir: impl AsRef<Path>) -> Self {
        let file = data_dir.as_ref().join(FILE_NAME);
        let mut entries = VecDeque::new();
        let mut file_lines = 0;
        if let Ok(text) = fs::read_to_string(&file) {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                file_lines += 1;
                if let Ok(report) = serde_json::from_str::<ReportOwned>(line) {
                    if entries.len() == MAX_ENTRIES {
                        entries.pop_front();
                    }
                    entries.push_back(report.into_report());
                }
            }
        }
        Self {
            shared: Arc::new(SharedLog {
                file,
                inner: Mutex::new(Inner {
                    entries,
                    file_lines,
                    pending_lines: Vec::new(),
                    pending_clear: false,
                    flush_scheduled: false,
                }),
                io_lock: Mutex::new(()),
            }),
        }
    }

    /// Records one frame (and persists non-keepalive frames to disk).
    /// When running inside Tokio, JSONL append and compaction are batched on `spawn_blocking`
    /// so WS frame dispatch never blocks on filesystem I/O.
    pub fn record(&self, report: FrameReport) {
        let persist_line = if is_keepalive_ping_reply(&report) {
            None
        } else {
            serde_json::to_string(&report).ok()
        };

        let tokio_handle = tokio::runtime::Handle::try_current().ok();
        let mut schedule_async = false;
        let mut flush_sync_now = false;

        {
            let Ok(mut inner) = self.shared.inner.lock() else {
                return;
            };
            if inner.entries.len() == MAX_ENTRIES {
                inner.entries.pop_front();
            }
            inner.entries.push_back(report);
            if let Some(line) = persist_line {
                inner.pending_lines.push(line);
                if tokio_handle.is_some() {
                    if !inner.flush_scheduled {
                        inner.flush_scheduled = true;
                        schedule_async = true;
                    }
                } else {
                    flush_sync_now = true;
                }
            }
        }

        if schedule_async {
            if let Some(handle) = tokio_handle {
                let shared = Arc::clone(&self.shared);
                handle.spawn(async move {
                    tokio::time::sleep(FLUSH_DEBOUNCE).await;
                    let _ = tokio::task::spawn_blocking(move || shared.flush_disk()).await;
                });
            }
        } else if flush_sync_now {
            self.shared.flush_disk();
        }
    }

    pub fn len(&self) -> usize {
        self.shared.inner.lock().map(|i| i.entries.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drops all entries and removes the backing file.
    pub fn clear(&self) {
        let tokio_handle = tokio::runtime::Handle::try_current().ok();
        {
            let Ok(mut inner) = self.shared.inner.lock() else {
                return;
            };
            inner.entries.clear();
            inner.file_lines = 0;
            inner.pending_lines.clear();
            inner.pending_clear = true;
        }
        if let Some(handle) = tokio_handle {
            let shared = Arc::clone(&self.shared);
            handle.spawn_blocking(move || shared.flush_disk());
        } else {
            self.shared.flush_disk();
        }
    }

    /// Plain-text rendering, newest first.
    pub fn render_text(&self) -> String {
        let Ok(inner) = self.shared.inner.lock() else {
            return String::new();
        };
        let mut out = String::new();
        out.push_str(&format!(
            "Zalo socket log: {} of the last {} frames, newest first. Structure only: no message text, names or numbers.\n\n",
            inner.entries.len(),
            MAX_ENTRIES
        ));
        if inner.entries.is_empty() {
            out.push_str("(empty: no frame has been decoded since the log was last cleared)\n");
        }
        for r in inner.entries.iter().rev() {
            render_entry(&mut out, r);
        }
        out
    }
}

fn render_entry(out: &mut String, r: &FrameReport) {
    let when = format_rfc3339_millis(r.ts);
    out.push_str(&format!(
        "{when}  v{} cmd={} sub={}  {}\n",
        r.version,
        r.cmd,
        r.sub_cmd,
        if r.consumed { "handled" } else { "RAW (no handler)" }
    ));
    if let Some(e) = r.error {
        out.push_str(&format!("  error: {e}\n"));
    }
    if let Some(d) = &r.error_detail {
        out.push_str(&format!("  why: {d}\n"));
    }
    if !r.envelope.is_empty() {
        out.push_str(&format!("  envelope: {}\n", r.envelope.join(", ")));
    }
    if !r.payload_keys.is_empty() {
        out.push_str(&format!("  keys: {}\n", r.payload_keys.join(",")));
    }
    if !r.data_keys.is_empty() {
        out.push_str(&format!("  data keys: {}\n", r.data_keys.join(",")));
    }
    if !r.scalars.is_empty() {
        out.push_str(&format!("  values: {}\n", r.scalars.join(", ")));
    }
    if !r.acts.is_empty() {
        out.push_str(&format!("  act: {}\n", r.acts.join(", ")));
    }
    if let Some(n) = r.note {
        out.push_str(&format!("  zalỏ: {n}\n"));
    }
    for a in &r.arrays {
        out.push_str(&format!(
            "  {}[{}] {}\n",
            a.name,
            a.len,
            if a.read { "read" } else { "NOT READ by any handler" }
        ));
        for it in &a.items {
            let mut line = format!(
                "    - msgType={} content={} uidFrom={} -> {}",
                it.msg_type.as_deref().unwrap_or("-"),
                it.content,
                it.uid_from,
                it.outcome
            );
            if let Some(reason) = it.reason {
                line.push_str(&format!(" ({reason})"));
            }
            if let Some(note) = it.note {
                line.push_str(&format!(" [{note}]"));
            }
            if let Some(id) = &it.msg_id {
                line.push_str(&format!(" msgId={id}"));
            }
            line.push('\n');
            out.push_str(&line);
        }
        if a.items_truncated {
            out.push_str("    - ... (more items not listed)\n");
        }
    }
    out.push('\n');
}

// `FrameReport` holds `&'static str` fields, so reading it back from disk goes through an owned
// mirror whose strings are mapped onto the known static values (anything else is `?`).
/// Keeps the characters a `key=value` description uses (`[A-Za-z0-9._:-=(){}[],]`), caps it, and
/// turns anything else into `?`.
fn safe_entry(s: &str) -> String {
    s.chars()
        .take(200)
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._:-=(){}[],".contains(c) {
                c
            } else {
                '?'
            }
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct ReportOwned {
    ts: i64,
    version: u8,
    cmd: u16,
    #[serde(rename = "subCmd")]
    sub_cmd: u8,
    consumed: bool,
    #[serde(rename = "payloadKeys", default)]
    payload_keys: Vec<String>,
    #[serde(rename = "dataKeys", default)]
    data_keys: Vec<String>,
    #[serde(default)]
    arrays: Vec<ArrayOwned>,
    #[serde(default)]
    acts: Vec<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    scalars: Vec<String>,
    #[serde(default)]
    envelope: Vec<String>,
    #[serde(rename = "errorDetail", default)]
    error_detail: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(serde::Deserialize)]
struct ArrayOwned {
    name: String,
    len: usize,
    read: bool,
    #[serde(default)]
    items: Vec<ItemOwned>,
    #[serde(default)]
    items_truncated: bool,
}

#[derive(serde::Deserialize)]
struct ItemOwned {
    #[serde(rename = "msgType")]
    msg_type: Option<String>,
    content: String,
    #[serde(rename = "uidFrom")]
    uid_from: String,
    outcome: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(rename = "msgId", default)]
    msg_id: Option<String>,
}

impl ReportOwned {
    fn into_report(self) -> FrameReport {
        use zalo_proto::framelog::{ArrayReport, ItemReport};
        // Persisted text is mapped back onto static strings; leaking is bounded by the small
        // fixed vocabulary, and unknown values collapse to one constant.
        fn pick(s: &str, known: &[&'static str]) -> &'static str {
            known.iter().copied().find(|k| *k == s).unwrap_or("?")
        }
        let reasons = [
            "not processed",
            "not an object",
            "no msgId",
            "undo content",
            "thread id empty/0",
            "own undo (selfListen off)",
            "own message (selfListen off)",
        ];
        let notes = ["e2ee-masked", "thread id empty/0"];
        let errors = [
            "empty body",
            "body not utf-8",
            "body not json",
            "payload decode failed",
        ];
        FrameReport {
            ts: self.ts,
            version: self.version,
            cmd: self.cmd,
            sub_cmd: self.sub_cmd,
            consumed: self.consumed,
            payload_keys: self.payload_keys,
            data_keys: self.data_keys,
            arrays: self
                .arrays
                .into_iter()
                .map(|a| ArrayReport {
                    name: a.name,
                    len: a.len,
                    read: a.read,
                    items_truncated: a.items_truncated,
                    items: a
                        .items
                        .into_iter()
                        .map(|i| ItemReport {
                            msg_type: i.msg_type,
                            content: i.content,
                            uid_from: pick(&i.uid_from, &["zero", "own", "other", "missing"]),
                            outcome: pick(&i.outcome, &["message", "undo", "dropped"]),
                            reason: i.reason.as_deref().map(|r| pick(r, &reasons)),
                            note: i.note.as_deref().map(|n| pick(n, &notes)),
                            msg_id: i.msg_id,
                        })
                        .collect(),
                })
                .collect(),
            acts: self.acts,
            error: self.error.as_deref().map(|e| pick(e, &errors)),
            // Written by us already reduced to safe tokens; re-reduce in case the file was edited.
            scalars: self.scalars.iter().map(|s| safe_entry(s)).collect(),
            envelope: self.envelope.iter().map(|s| safe_entry(s)).collect(),
            error_detail: self.error_detail.as_deref().map(|d| zalo_proto::framelog::safe_token(d, 80)),
            note: self.note.as_deref().map(|n| pick(n, &[crate::session::POKE_PULL_NOTE])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zalo_proto::framelog::{ArrayReport, ItemReport};

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("zalo-socklog-{:016x}", rand::random::<u64>()))
    }

    fn report(cmd: u16) -> FrameReport {
        let mut r = FrameReport::new(1, cmd, 0);
        r.ts = 1_700_000_000_000;
        r.payload_keys = vec!["data".into()];
        r.arrays.push(ArrayReport {
            name: "pageMsgs".into(),
            len: 1,
            read: true,
            items: vec![{
                let mut i = ItemReport {
                    msg_type: Some("chat.webcontent".into()),
                    content: "{href,title}".into(),
                    uid_from: "other",
                    outcome: "dropped",
                    reason: Some("no msgId"),
                    note: None,
                    msg_id: None,
                };
                i.dropped("no msgId");
                i
            }],
            items_truncated: false,
        });
        r
    }

    #[test]
    fn bounded_persisted_and_reloaded() {
        let dir = temp_dir();
        let log = SocketLog::open(&dir);
        for i in 0..(MAX_ENTRIES as u16 + 30) {
            log.record(report(i));
        }
        assert_eq!(log.len(), MAX_ENTRIES);
        let text = log.render_text();
        // Newest first.
        let newest = text.find(&format!("cmd={} ", MAX_ENTRIES + 29)).unwrap();
        let older = text.find("cmd=100 ").unwrap();
        assert!(newest < older);
        assert!(!text.contains("cmd=5 "));
        drop(log);
        let reopened = SocketLog::open(&dir);
        assert_eq!(reopened.len(), MAX_ENTRIES);
        assert!(reopened.render_text().contains("dropped (no msgId)"));
    }

    #[test]
    fn file_is_compacted_and_clear_removes_it() {
        let dir = temp_dir();
        let log = SocketLog::open(&dir);
        for i in 0..(COMPACT_AT as u16 + 5) {
            log.record(report(i));
        }
        let lines = fs::read_to_string(dir.join(FILE_NAME)).unwrap().lines().count();
        assert!(lines < COMPACT_AT, "{lines}");
        log.clear();
        assert!(log.is_empty());
        assert!(!&dir.join(FILE_NAME).exists());
        assert!(log.render_text().contains("empty"));
    }
}
