//! Replay instrumentation: what Zalo replays after time offline, and how much of it we already had.
//!
//! On every `Connected`, the session asks Zalo to replay what was missed
//! (`request_old_messages`). Nobody knows how much Zalo replays, nor what `lastId` does, so this
//! module records, per connect, a *replay report*:
//!
//! * how long we were offline (see [`ListenerClock`]),
//! * per kind (1:1 / group): the newest stored message before the replay,
//! * every request sent and every batch that came back (count, oldest/newest timestamp and id,
//!   how many were already stored versus new),
//! * a [`Verdict`] per kind.
//!
//! The last [`MAX_REPORTS`] reports are kept in `<data_dir>/replay-log.json` (written atomically)
//! and rendered as plain text by `GET /api/debug/replay-log`. **No message content is recorded:**
//! ids, counts, timestamps and kinds only.
//!
//! [`ReplayMode`] (`PUT /api/debug/replay-mode`) selects the experiment: `null` (today's request
//! with `lastId: null`), `newest` (`lastId` = newest stored id of that kind) or `page` (`null`
//! first, then page back with the oldest id of each full, non-overlapping batch, up to
//! [`MAX_PAGES`] requests per kind).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::log::create_dir_private;
use crate::types::ThreadKind;

/// Reports kept on disk (the newest ones).
pub const MAX_REPORTS: usize = 50;
/// Most replay requests per kind in one report (`page` mode stops here).
pub const MAX_PAGES: usize = 5;
/// A batch smaller than this never "looks full". Zalo's real page size is unknown (the point of
/// the experiment); this is only the floor under the observed maximum, see [`looks_full`].
pub const FULL_BATCH_FLOOR: usize = 10;
/// How often the "last seen alive" time is rewritten while connected.
const LAST_SEEN_WRITE_EVERY_MS: i64 = 30_000;

const REPORTS_FILE: &str = "replay-log.json";
const MODE_FILE: &str = "replay-mode.json";
const CLOCK_FILE: &str = "listener-state.json";

/// Which `lastId` the replay requests carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMode {
    /// `lastId: null`, once (today's behavior).
    #[default]
    Null,
    /// `lastId` = the newest stored message id of that kind.
    Newest,
    /// `null` first, then keep paging back while a batch looks full and does not overlap.
    Page,
}

impl ReplayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ReplayMode::Null => "null",
            ReplayMode::Newest => "newest",
            ReplayMode::Page => "page",
        }
    }

    /// Parses the `mode` of `PUT /api/debug/replay-mode`: `"null"`, `"newest"`, `"page"`, or a
    /// JSON `null`.
    pub fn parse(v: &Value) -> Option<Self> {
        match v {
            Value::Null => Some(ReplayMode::Null),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "null" => Some(ReplayMode::Null),
                "newest" => Some(ReplayMode::Newest),
                "page" => Some(ReplayMode::Page),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Whether a replay left a gap in our history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The oldest replayed message is at or before our newest stored one: overlap, no gap.
    Covered,
    /// Everything replayed is newer than what we had and a batch looks full: there may be more.
    PossibleGap,
    /// Zalo replayed nothing.
    NothingMissed,
    /// Anything else (no reply yet, nothing stored to compare with, small batch without overlap).
    Unknown,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Covered => "covered",
            Verdict::PossibleGap => "possible_gap",
            Verdict::NothingMissed => "nothing_missed",
            Verdict::Unknown => "unknown",
        }
    }
}

/// Where the offline duration of a report starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineSource {
    /// The previous listener close (or pause).
    Close,
    /// The process died while connected, so the last time we heard from Zalo stands in.
    LastSeen,
    /// No earlier connection is known; counted from the start of this process.
    AppStart,
}

impl OfflineSource {
    fn describe(self) -> &'static str {
        match self {
            OfflineSource::Close => "since the listener closed",
            OfflineSource::LastSeen => "since last heard from Zalo (process died while connected)",
            OfflineSource::AppStart => "since the app started (no earlier connection known)",
        }
    }
}

/// One replay request and the batch that answered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRecord {
    /// We sent a request for this page (`false`: a batch arrived that we did not ask for).
    pub requested: bool,
    /// The `lastId` the request carried (`null` for the first request).
    pub last_id: Option<String>,
    pub sent_at: i64,
    pub received: bool,
    pub received_at: Option<i64>,
    pub count: usize,
    pub oldest_ts: Option<i64>,
    pub newest_ts: Option<i64>,
    pub oldest_id: Option<String>,
    pub newest_id: Option<String>,
    /// Of `count`: messages that were already in the store.
    pub already_stored: usize,
    /// Of `count`: messages that were new to the store.
    pub new: usize,
    pub looks_full: bool,
}

impl PageRecord {
    fn requested(last_id: Option<String>, now: i64) -> Self {
        Self {
            requested: true,
            last_id,
            sent_at: now,
            received: false,
            received_at: None,
            count: 0,
            oldest_ts: None,
            newest_ts: None,
            oldest_id: None,
            newest_id: None,
            already_stored: 0,
            new: 0,
            looks_full: false,
        }
    }
}

/// What arrived in one batch (computed by the session while ingesting it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchStats {
    pub count: usize,
    pub oldest: Option<(i64, String)>,
    pub newest: Option<(i64, String)>,
    pub already_stored: usize,
    pub new: usize,
}

impl BatchStats {
    /// Counts a message without a usable timestamp (a recall): it takes part in `count` and the
    /// stored/new split, not in oldest/newest.
    pub fn add_untimed(&mut self, already_stored: bool) {
        self.count += 1;
        if already_stored {
            self.already_stored += 1;
        } else {
            self.new += 1;
        }
    }

    /// Adds one message to the stats.
    pub fn add(&mut self, ts: i64, id: &str, already_stored: bool) {
        self.count += 1;
        if already_stored {
            self.already_stored += 1;
        } else {
            self.new += 1;
        }
        if self.oldest.as_ref().is_none_or(|(t, _)| ts < *t) {
            self.oldest = Some((ts, id.to_string()));
        }
        if self.newest.as_ref().is_none_or(|(t, _)| ts >= *t) {
            self.newest = Some((ts, id.to_string()));
        }
    }
}

/// The replay of one kind within a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindReport {
    /// Newest stored message of this kind before the replay.
    pub newest_stored_ts: Option<i64>,
    pub newest_stored_id: Option<String>,
    pub pages: Vec<PageRecord>,
    pub verdict: Verdict,
    pub verdict_reason: String,
}

impl KindReport {
    fn new(before: Option<(i64, String)>) -> Self {
        Self {
            newest_stored_ts: before.as_ref().map(|(t, _)| *t),
            newest_stored_id: before.map(|(_, id)| id),
            pages: Vec::new(),
            verdict: Verdict::Unknown,
            verdict_reason: "no reply yet".to_string(),
        }
    }
}

/// Everything recorded for one `Connected`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayReport {
    pub connected_at: i64,
    pub mode: ReplayMode,
    pub offline_ms: Option<i64>,
    pub offline_source: OfflineSource,
    /// The largest batch seen in any report so far, this one included (the page size, if any).
    pub max_batch_seen: usize,
    pub user: KindReport,
    pub group: KindReport,
}

impl ReplayReport {
    fn kind(&self, kind: ThreadKind) -> &KindReport {
        match kind {
            ThreadKind::User => &self.user,
            ThreadKind::Group => &self.group,
        }
    }

    fn kind_mut(&mut self, kind: ThreadKind) -> &mut KindReport {
        match kind {
            ThreadKind::User => &mut self.user,
            ThreadKind::Group => &mut self.group,
        }
    }
}

/// A batch looks full when it is at least [`FULL_BATCH_FLOOR`] messages and at least as big as the
/// biggest batch seen before (`observed_max`): Zalo probably cut it off at its page size.
pub fn looks_full(count: usize, observed_max: usize) -> bool {
    count >= FULL_BATCH_FLOOR && count >= observed_max
}

/// The verdict for one kind, from the newest stored timestamp before the replay and the pages
/// received so far. Returns the verdict and a short reason.
pub fn verdict_for(before_newest_ts: Option<i64>, pages: &[PageRecord]) -> (Verdict, &'static str) {
    let received: Vec<&PageRecord> = pages.iter().filter(|p| p.received).collect();
    if received.is_empty() {
        return (Verdict::Unknown, "no reply yet");
    }
    let oldest = received.iter().filter_map(|p| p.oldest_ts).min();
    let Some(oldest) = oldest else {
        return (Verdict::NothingMissed, "Zalo replayed nothing");
    };
    let Some(before) = before_newest_ts else {
        return (
            Verdict::Unknown,
            "nothing stored before the replay to compare with",
        );
    };
    if oldest <= before {
        return (
            Verdict::Covered,
            "oldest replayed message is at or before our newest stored one",
        );
    }
    if received.iter().any(|p| p.looks_full) {
        (
            Verdict::PossibleGap,
            "everything replayed is newer than what we had and a batch looks full",
        )
    } else {
        (
            Verdict::Unknown,
            "everything replayed is newer than what we had, but no batch looks full",
        )
    }
}

// ---- Listener clock: how long were we offline ----

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClockFile {
    /// When the listener last closed or was paused (cleared on the next connect).
    #[serde(default)]
    disconnected_ms: Option<i64>,
    /// The last time we heard from Zalo while connected (survives a process death).
    #[serde(default)]
    last_seen_ms: Option<i64>,
}

struct DiskWriter {
    pending: Mutex<std::collections::HashMap<PathBuf, Vec<u8>>>,
    flush_scheduled: std::sync::atomic::AtomicBool,
    io_lock: Mutex<()>,
}

impl DiskWriter {
    fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            pending: Mutex::new(std::collections::HashMap::new()),
            flush_scheduled: std::sync::atomic::AtomicBool::new(false),
            io_lock: Mutex::new(()),
        })
    }

    fn write_value(self: &std::sync::Arc<Self>, path: PathBuf, value: &impl Serialize) {
        use std::sync::atomic::Ordering;
        let Ok(payload) = serde_json::to_vec(value) else {
            return;
        };
        if let Ok(mut map) = self.pending.lock() {
            map.insert(path, payload);
        } else {
            return;
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if self
                .flush_scheduled
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let writer = std::sync::Arc::clone(self);
                handle.spawn_blocking(move || writer.flush_sync());
            }
        } else {
            self.flush_sync();
        }
    }

    fn flush_sync(&self) {
        use std::sync::atomic::Ordering;
        let Ok(_io) = self.io_lock.lock() else {
            return;
        };
        loop {
            let batch = {
                let Ok(mut map) = self.pending.lock() else {
                    self.flush_scheduled.store(false, Ordering::Release);
                    return;
                };
                self.flush_scheduled.store(false, Ordering::Release);
                if map.is_empty() {
                    return;
                }
                std::mem::take(&mut *map)
            };
            for (path, payload) in batch {
                write_atomic_bytes(&path, &payload);
            }
        }
    }
}

/// Tracks the last listener disconnect (persisted, so it survives process death) to tell how long
/// we were offline when the next `Connected` arrives.
struct ListenerClock {
    path: PathBuf,
    writer: std::sync::Arc<DiskWriter>,
    process_start_ms: i64,
    state: ClockFile,
    last_seen_written_ms: i64,
}

impl ListenerClock {
    fn open(dir: &Path, now: i64, writer: std::sync::Arc<DiskWriter>) -> Self {
        let path = dir.join(CLOCK_FILE);
        let state = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            path,
            writer,
            process_start_ms: now,
            state,
            last_seen_written_ms: 0,
        }
    }

    fn save(&self) {
        self.writer.write_value(self.path.clone(), &self.state);
    }

    /// Offline duration at `now` and where it started; then marks us connected.
    fn on_connected(&mut self, now: i64) -> (Option<i64>, OfflineSource) {
        let (since, source) = if let Some(t) = self.state.disconnected_ms {
            (t, OfflineSource::Close)
        } else if let Some(t) = self.state.last_seen_ms {
            (t, OfflineSource::LastSeen)
        } else {
            (self.process_start_ms, OfflineSource::AppStart)
        };
        self.state.disconnected_ms = None;
        self.state.last_seen_ms = Some(now);
        self.last_seen_written_ms = now;
        self.save();
        ((now >= since).then_some(now - since), source)
    }

    fn on_disconnected(&mut self, now: i64) {
        self.state.disconnected_ms = Some(now);
        self.save();
    }

    /// Notes we just heard from Zalo; rewrites the file at most every 30 s.
    fn touch(&mut self, now: i64) {
        self.state.last_seen_ms = Some(now);
        if now - self.last_seen_written_ms >= LAST_SEEN_WRITE_EVERY_MS {
            self.last_seen_written_ms = now;
            self.save();
        }
    }
}

/// Writes `payload` to `path` atomically (temp file in the same directory, then rename).
/// Diagnostic files skip `fsync` to avoid flash stalls on mobile devices.
fn write_atomic_bytes(path: &Path, payload: &[u8]) {
    if let Some(parent) = path.parent() {
        if create_dir_private(parent).is_err() {
            return;
        }
    }
    let tmp = path.with_extension("json.tmp");
    let result = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(payload)?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(err) = result {
        tracing::warn!("Replay log: {err}");
    }
}

// ---- The hub the session talks to ----

/// What `begin` tells the session to request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayPlan {
    pub user_last_id: Option<String>,
    pub group_last_id: Option<String>,
}

struct HubState {
    reports: Vec<ReplayReport>,
    clock: ListenerClock,
}

/// Replay reports, mode and listener clock for one session (all persisted in the data dir).
pub struct ReplayHub {
    dir: PathBuf,
    writer: std::sync::Arc<DiskWriter>,
    mode: Mutex<ReplayMode>,
    state: Mutex<HubState>,
}

impl ReplayHub {
    pub fn open(dir: &Path, now: i64) -> Self {
        let reports: Vec<ReplayReport> = fs::read_to_string(dir.join(REPORTS_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mode = fs::read_to_string(dir.join(MODE_FILE))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("mode").and_then(ReplayMode::parse))
            .unwrap_or_default();
        let writer = DiskWriter::new();
        Self {
            dir: dir.to_path_buf(),
            writer: std::sync::Arc::clone(&writer),
            mode: Mutex::new(mode),
            state: Mutex::new(HubState {
                reports,
                clock: ListenerClock::open(dir, now, writer),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn mode(&self) -> ReplayMode {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets and persists the replay-request mode.
    pub fn set_mode(&self, mode: ReplayMode) {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = mode;
        self.writer.write_value(
            self.dir.join(MODE_FILE),
            &serde_json::json!({ "mode": mode.as_str() }),
        );
    }

    /// Waits for any queued replay/clock/mode writes to reach disk (on a blocking thread).
    pub async fn flush(&self) {
        let writer = std::sync::Arc::clone(&self.writer);
        let _ = tokio::task::spawn_blocking(move || writer.flush_sync()).await;
    }

    /// Starts the report for a `Connected` and says which `lastId` to request for each kind.
    pub fn begin(
        &self,
        now: i64,
        before_user: Option<(i64, String)>,
        before_group: Option<(i64, String)>,
    ) -> ReplayPlan {
        let mode = self.mode();
        let mut st = self.lock();
        let (offline_ms, offline_source) = st.clock.on_connected(now);
        let pick = |before: &Option<(i64, String)>| match mode {
            ReplayMode::Newest => before.as_ref().map(|(_, id)| id.clone()),
            ReplayMode::Null | ReplayMode::Page => None,
        };
        let plan = ReplayPlan {
            user_last_id: pick(&before_user),
            group_last_id: pick(&before_group),
        };
        let mut user = KindReport::new(before_user);
        user.pages
            .push(PageRecord::requested(plan.user_last_id.clone(), now));
        let mut group = KindReport::new(before_group);
        group
            .pages
            .push(PageRecord::requested(plan.group_last_id.clone(), now));
        let max_batch_seen = max_batch(&st.reports);
        st.reports.push(ReplayReport {
            connected_at: now,
            mode,
            offline_ms,
            offline_source,
            max_batch_seen,
            user,
            group,
        });
        trim(&mut st.reports);
        self.save(&st);
        plan
    }

    /// Records one received batch for `kind`. In `page` mode, returns the `lastId` of the next
    /// request the session should send (and records that request).
    pub fn record_batch(&self, now: i64, kind: ThreadKind, stats: BatchStats) -> Option<String> {
        let mode = self.mode();
        let mut st = self.lock();
        let prior_max = max_batch(&st.reports[..st.reports.len().saturating_sub(1)]);
        let report = st.reports.last_mut()?;
        let kr = report.kind_mut(kind);
        let full = looks_full(stats.count, prior_max);
        let slot = kr.pages.iter().position(|p| p.requested && !p.received);
        let idx = match slot {
            Some(i) => i,
            None => {
                let mut page = PageRecord::requested(None, now);
                page.requested = false;
                page.sent_at = 0;
                kr.pages.push(page);
                kr.pages.len() - 1
            }
        };
        let is_last = idx + 1 == kr.pages.len();
        let page = &mut kr.pages[idx];
        page.received = true;
        page.received_at = Some(now);
        page.count = stats.count;
        page.oldest_ts = stats.oldest.as_ref().map(|(t, _)| *t);
        page.oldest_id = stats.oldest.as_ref().map(|(_, id)| id.clone());
        page.newest_ts = stats.newest.as_ref().map(|(t, _)| *t);
        page.newest_id = stats.newest.as_ref().map(|(_, id)| id.clone());
        page.already_stored = stats.already_stored;
        page.new = stats.new;
        page.looks_full = full;
        let page_oldest_ts = page.oldest_ts;
        let page_oldest_id = page.oldest_id.clone();
        let page_last_id = page.last_id.clone();
        let (verdict, reason) = verdict_for(kr.newest_stored_ts, &kr.pages);
        kr.verdict = verdict;
        kr.verdict_reason = reason.to_string();

        let mut next = None;
        if mode == ReplayMode::Page && is_last && full {
            let requested = kr.pages.iter().filter(|p| p.requested).count();
            let overlaps = match (kr.newest_stored_ts, page_oldest_ts) {
                (Some(before), Some(oldest)) => oldest <= before,
                // Nothing stored: no baseline to page back to.
                (None, _) => true,
                (_, None) => true,
            };
            if requested < MAX_PAGES && !overlaps && page_oldest_id != page_last_id {
                if let Some(id) = page_oldest_id {
                    kr.pages.push(PageRecord::requested(Some(id.clone()), now));
                    next = Some(id);
                }
            }
        }
        report.max_batch_seen = report.max_batch_seen.max(stats.count);
        self.save(&st);
        next
    }

    /// The listener closed or was paused.
    pub fn note_disconnected(&self, now: i64) {
        self.lock().clock.on_disconnected(now);
    }

    /// We just heard from Zalo.
    pub fn touch(&self, now: i64) {
        self.lock().clock.touch(now);
    }

    /// The reports, oldest first.
    pub fn reports(&self) -> Vec<ReplayReport> {
        self.lock().reports.clone()
    }

    /// Deletes all reports (the listener clock and mode stay).
    pub fn clear(&self) {
        let mut st = self.lock();
        st.reports.clear();
        self.save(&st);
    }

    /// The reports as plain text, newest first.
    pub fn render_text(&self) -> String {
        render_reports(&self.lock().reports)
    }

    fn save(&self, st: &HubState) {
        self.writer
            .write_value(self.dir.join(REPORTS_FILE), &st.reports);
    }
}

fn trim(reports: &mut Vec<ReplayReport>) {
    if reports.len() > MAX_REPORTS {
        let excess = reports.len() - MAX_REPORTS;
        reports.drain(0..excess);
    }
}

fn max_batch(reports: &[ReplayReport]) -> usize {
    reports
        .iter()
        .flat_map(|r| r.user.pages.iter().chain(r.group.pages.iter()))
        .map(|p| p.count)
        .max()
        .unwrap_or(0)
}

// ---- Text rendering ----

fn fmt_ts(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| ms.to_string())
}

fn fmt_opt_ts(ms: Option<i64>) -> String {
    ms.map(fmt_ts).unwrap_or_else(|| "-".to_string())
}

fn fmt_duration(ms: i64) -> String {
    let total = ms / 1000;
    let (d, h, m, s) = (
        total / 86_400,
        total % 86_400 / 3600,
        total % 3600 / 60,
        total % 60,
    );
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

fn render_kind(out: &mut String, label: &str, kr: &KindReport) {
    out.push_str(&format!(
        "{label}: verdict={} ({})\n",
        kr.verdict.as_str(),
        kr.verdict_reason
    ));
    out.push_str(&format!(
        "  newest stored before replay: ts={} id={}\n",
        fmt_opt_ts(kr.newest_stored_ts),
        kr.newest_stored_id.as_deref().unwrap_or("-")
    ));
    for (i, p) in kr.pages.iter().enumerate() {
        let n = i + 1;
        let req = if p.requested {
            format!("lastId={}", p.last_id.as_deref().unwrap_or("null"))
        } else {
            "unrequested".to_string()
        };
        if !p.received {
            out.push_str(&format!("  page {n} [{req}]: no reply\n"));
        } else if p.count == 0 {
            out.push_str(&format!("  page {n} [{req}]: 0 messages\n"));
        } else {
            out.push_str(&format!(
                "  page {n} [{req}]: {} messages, oldest {} (id {}), newest {} (id {}), \
                 already stored {}, new {}{}\n",
                p.count,
                fmt_opt_ts(p.oldest_ts),
                p.oldest_id.as_deref().unwrap_or("-"),
                fmt_opt_ts(p.newest_ts),
                p.newest_id.as_deref().unwrap_or("-"),
                p.already_stored,
                p.new,
                if p.looks_full { ", looks full" } else { "" },
            ));
        }
    }
}

/// Renders `reports` (oldest first) as plain text, newest first.
pub fn render_reports(reports: &[ReplayReport]) -> String {
    if reports.is_empty() {
        return "No replay reports yet. One is written each time the chat listener connects.\n"
            .to_string();
    }
    let mut out = String::new();
    for r in reports.iter().rev() {
        out.push_str(&format!(
            "=== connected {}  mode={}\n",
            fmt_ts(r.connected_at),
            r.mode.as_str()
        ));
        match r.offline_ms {
            Some(ms) => out.push_str(&format!(
                "offline: {} ({})\n",
                fmt_duration(ms),
                r.offline_source.describe()
            )),
            None => out.push_str("offline: unknown (clock went backwards)\n"),
        }
        out.push_str(&format!(
            "largest batch seen so far: {}\n",
            r.max_batch_seen
        ));
        render_kind(&mut out, "user", r.kind(ThreadKind::User));
        render_kind(&mut out, "group", r.kind(ThreadKind::Group));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("zalo-replay-{:016x}", rand::random::<u64>()))
    }

    fn page(count: usize, oldest: Option<i64>, full: bool) -> PageRecord {
        PageRecord {
            requested: true,
            last_id: None,
            sent_at: 0,
            received: true,
            received_at: Some(0),
            count,
            oldest_ts: oldest,
            newest_ts: oldest,
            oldest_id: oldest.map(|t| t.to_string()),
            newest_id: oldest.map(|t| t.to_string()),
            already_stored: 0,
            new: count,
            looks_full: full,
        }
    }

    fn stats(msgs: &[(i64, &str, bool)]) -> BatchStats {
        let mut s = BatchStats::default();
        for (ts, id, stored) in msgs {
            s.add(*ts, id, *stored);
        }
        s
    }

    #[test]
    fn mode_parses_strings_and_json_null() {
        use serde_json::json;
        assert_eq!(ReplayMode::parse(&json!("null")), Some(ReplayMode::Null));
        assert_eq!(ReplayMode::parse(&Value::Null), Some(ReplayMode::Null));
        assert_eq!(
            ReplayMode::parse(&json!("Newest")),
            Some(ReplayMode::Newest)
        );
        assert_eq!(ReplayMode::parse(&json!("page")), Some(ReplayMode::Page));
        assert_eq!(ReplayMode::parse(&json!("all")), None);
        assert_eq!(ReplayMode::parse(&json!(3)), None);
        assert_eq!(ReplayMode::default(), ReplayMode::Null);
    }

    #[test]
    fn verdict_rules() {
        // No reply.
        assert_eq!(verdict_for(Some(100), &[]).0, Verdict::Unknown);
        let mut unanswered = page(0, None, false);
        unanswered.received = false;
        assert_eq!(verdict_for(Some(100), &[unanswered]).0, Verdict::Unknown);
        // Empty.
        assert_eq!(
            verdict_for(Some(100), &[page(0, None, false)]).0,
            Verdict::NothingMissed
        );
        assert_eq!(
            verdict_for(None, &[page(0, None, false)]).0,
            Verdict::NothingMissed
        );
        // Overlap, including exactly equal.
        assert_eq!(
            verdict_for(Some(100), &[page(5, Some(90), false)]).0,
            Verdict::Covered
        );
        assert_eq!(
            verdict_for(Some(100), &[page(5, Some(100), true)]).0,
            Verdict::Covered
        );
        // Newer than everything we had: full means a possible gap, otherwise unknown.
        assert_eq!(
            verdict_for(Some(100), &[page(50, Some(150), true)]).0,
            Verdict::PossibleGap
        );
        assert_eq!(
            verdict_for(Some(100), &[page(3, Some(150), false)]).0,
            Verdict::Unknown
        );
        // Nothing stored to compare with.
        assert_eq!(
            verdict_for(None, &[page(50, Some(150), true)]).0,
            Verdict::Unknown
        );
        // Several pages: the oldest one decides.
        assert_eq!(
            verdict_for(
                Some(100),
                &[
                    page(50, Some(300), true),
                    page(50, Some(120), true),
                    page(20, Some(80), false)
                ]
            )
            .0,
            Verdict::Covered
        );
    }

    #[test]
    fn full_means_at_least_the_floor_and_the_biggest_seen() {
        assert!(!looks_full(9, 0));
        assert!(looks_full(10, 0));
        assert!(!looks_full(30, 50));
        assert!(looks_full(50, 50));
        assert!(looks_full(60, 50));
    }

    #[test]
    fn batch_stats_track_oldest_newest_and_stored_split() {
        let s = stats(&[(30, "c", false), (10, "a", true), (20, "b", false)]);
        assert_eq!(s.count, 3);
        assert_eq!(s.oldest, Some((10, "a".to_string())));
        assert_eq!(s.newest, Some((30, "c".to_string())));
        assert_eq!((s.already_stored, s.new), (1, 2));
    }

    #[test]
    fn null_mode_plans_null_and_records_the_batch() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 1_000);
        let plan = hub.begin(5_000, Some((100, "u9".into())), None);
        assert_eq!(
            plan,
            ReplayPlan {
                user_last_id: None,
                group_last_id: None
            }
        );
        let next = hub.record_batch(
            5_100,
            ThreadKind::User,
            stats(&[(90, "u1", true), (120, "u10", false)]),
        );
        assert_eq!(next, None);
        let r = hub.reports().pop().unwrap();
        assert_eq!(r.mode, ReplayMode::Null);
        assert_eq!(r.user.verdict, Verdict::Covered);
        assert_eq!(r.user.pages.len(), 1);
        assert_eq!(r.user.pages[0].count, 2);
        assert_eq!(r.user.pages[0].already_stored, 1);
        assert_eq!(r.user.pages[0].new, 1);
        assert_eq!(r.group.verdict, Verdict::Unknown, "no group reply yet");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_mode_plans_the_newest_stored_id_per_kind() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 0);
        hub.set_mode(ReplayMode::Newest);
        let plan = hub.begin(10, Some((100, "u9".into())), Some((200, "g4".into())));
        assert_eq!(plan.user_last_id.as_deref(), Some("u9"));
        assert_eq!(plan.group_last_id.as_deref(), Some("g4"));
        let r = hub.reports().pop().unwrap();
        assert_eq!(r.user.pages[0].last_id.as_deref(), Some("u9"));
        // Nothing stored: falls back to null.
        let plan = hub.begin(20, None, None);
        assert_eq!(
            plan,
            ReplayPlan {
                user_last_id: None,
                group_last_id: None
            }
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn full_batch(from_ts: i64, n: usize) -> BatchStats {
        let mut s = BatchStats::default();
        for i in 0..n {
            let ts = from_ts + i as i64;
            s.add(ts, &format!("m{ts}"), false);
        }
        s
    }

    #[test]
    fn page_mode_pages_back_until_overlap() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 0);
        hub.set_mode(ReplayMode::Page);
        hub.begin(10, Some((100, "u0".into())), Some((100, "g0".into())));
        // First page: 20 messages, all newer than what we had: full, no overlap -> page back.
        let next = hub.record_batch(11, ThreadKind::User, full_batch(1000, 20));
        assert_eq!(next.as_deref(), Some("m1000"));
        // Second page: still newer, full -> page again from its oldest.
        let next = hub.record_batch(12, ThreadKind::User, full_batch(500, 20));
        assert_eq!(next.as_deref(), Some("m500"));
        // Third page overlaps what we have -> stop.
        let next = hub.record_batch(13, ThreadKind::User, full_batch(90, 20));
        assert_eq!(next, None);
        let r = hub.reports().pop().unwrap();
        assert_eq!(r.user.pages.len(), 3);
        assert_eq!(r.user.pages[1].last_id.as_deref(), Some("m1000"));
        assert_eq!(r.user.verdict, Verdict::Covered);
        // The group kind is independent: one small page, newer but not full -> stop.
        let next = hub.record_batch(14, ThreadKind::Group, full_batch(1000, 3));
        assert_eq!(next, None);
        assert_eq!(hub.reports().pop().unwrap().group.pages.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_mode_stops_after_five_requests() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 0);
        hub.set_mode(ReplayMode::Page);
        hub.begin(10, Some((1, "u0".into())), None);
        let mut requests = 1;
        let mut ts = 100_000;
        loop {
            let next = hub.record_batch(11, ThreadKind::User, full_batch(ts, 20));
            if next.is_none() {
                break;
            }
            requests += 1;
            ts -= 1000;
            assert!(requests <= 10, "must stop");
        }
        assert_eq!(requests, MAX_PAGES);
        assert_eq!(
            hub.reports().pop().unwrap().user.verdict,
            Verdict::PossibleGap
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_mode_does_not_loop_on_the_same_page() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 0);
        hub.set_mode(ReplayMode::Page);
        hub.begin(10, Some((1, "u0".into())), None);
        let next = hub.record_batch(11, ThreadKind::User, full_batch(1000, 20));
        assert_eq!(next.as_deref(), Some("m1000"));
        // Zalo answers the follow-up with the very same page.
        let next = hub.record_batch(12, ThreadKind::User, full_batch(1000, 20));
        assert_eq!(next, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unrequested_batch_is_recorded_as_such() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 0);
        hub.begin(10, None, None);
        hub.record_batch(11, ThreadKind::User, full_batch(1, 2));
        hub.record_batch(12, ThreadKind::User, full_batch(5, 2));
        let r = hub.reports().pop().unwrap();
        assert_eq!(r.user.pages.len(), 2);
        assert!(r.user.pages[0].requested);
        assert!(!r.user.pages[1].requested);
        assert!(render_reports(&[r]).contains("unrequested"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_size_is_learned_from_earlier_reports() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 0);
        hub.begin(10, Some((1, "u0".into())), None);
        hub.record_batch(11, ThreadKind::User, full_batch(1000, 40));
        // Next connect: a 25-message batch is smaller than the 40 we have seen, so it is not "full".
        hub.begin(20, Some((1, "u0".into())), None);
        hub.record_batch(21, ThreadKind::User, full_batch(1000, 25));
        let r = hub.reports().pop().unwrap();
        assert_eq!(r.max_batch_seen, 40);
        assert!(!r.user.pages[0].looks_full);
        assert_eq!(r.user.verdict, Verdict::Unknown);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_persist_are_capped_and_clear() {
        let dir = temp_dir();
        {
            let hub = ReplayHub::open(&dir, 0);
            hub.set_mode(ReplayMode::Newest);
            for i in 0..(MAX_REPORTS as i64 + 7) {
                hub.begin(1000 + i, None, None);
            }
            assert_eq!(hub.reports().len(), MAX_REPORTS);
        }
        let hub = ReplayHub::open(&dir, 0);
        assert_eq!(hub.mode(), ReplayMode::Newest, "mode persists");
        let reports = hub.reports();
        assert_eq!(reports.len(), MAX_REPORTS);
        assert_eq!(
            reports.last().unwrap().connected_at,
            1000 + MAX_REPORTS as i64 + 6
        );
        assert_eq!(
            reports.first().unwrap().connected_at,
            1007,
            "the oldest were dropped"
        );
        assert!(
            !dir.join("replay-log.json.tmp").exists(),
            "atomic write leaves no temp file"
        );
        hub.clear();
        assert!(hub.reports().is_empty());
        assert!(ReplayHub::open(&dir, 0).reports().is_empty());
        assert!(hub.render_text().contains("No replay reports"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_time_comes_from_the_close_then_last_seen_then_app_start() {
        let dir = temp_dir();
        // First ever connect: counted from process start.
        let hub = ReplayHub::open(&dir, 1_000);
        hub.begin(61_000, None, None);
        let r = hub.reports().pop().unwrap();
        assert_eq!(
            (r.offline_ms, r.offline_source),
            (Some(60_000), OfflineSource::AppStart)
        );

        // A close is persisted; a new process counts from it.
        hub.note_disconnected(100_000);
        drop(hub);
        let hub = ReplayHub::open(&dir, 9_000_000);
        hub.begin(100_000 + 3_600_000, None, None);
        let r = hub.reports().pop().unwrap();
        assert_eq!(
            (r.offline_ms, r.offline_source),
            (Some(3_600_000), OfflineSource::Close)
        );

        // Process died while connected: last seen stands in. (Written at most every 30 s.)
        let connected = 100_000 + 3_600_000;
        hub.touch(connected + 10_000); // too soon after the connect: kept in memory only
        hub.touch(connected + 40_000); // written
        drop(hub);
        let hub = ReplayHub::open(&dir, 9_000_000);
        hub.begin(connected + 40_000 + 8 * 3_600_000, None, None);
        let r = hub.reports().pop().unwrap();
        assert_eq!(r.offline_source, OfflineSource::LastSeen);
        assert_eq!(r.offline_ms, Some(8 * 3_600_000));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_has_no_message_content_and_lists_newest_first() {
        let dir = temp_dir();
        let hub = ReplayHub::open(&dir, 0);
        hub.begin(
            1_700_000_000_000,
            Some((1_699_999_000_000, "u9".into())),
            None,
        );
        hub.record_batch(
            1_700_000_001_000,
            ThreadKind::User,
            stats(&[
                (1_699_998_000_000, "u1", true),
                (1_700_000_000_500, "u10", false),
            ]),
        );
        hub.begin(1_700_000_100_000, None, None);
        let text = hub.render_text();
        let first = text
            .find("2023-11-14T22:15:00Z")
            .expect("newest report first");
        let second = text.find("2023-11-14T22:13:20Z").expect("older report");
        assert!(first < second);
        assert!(text.contains("verdict=covered"));
        assert!(text.contains("2 messages"));
        assert!(text.contains("already stored 1, new 1"));
        assert!(text.contains("lastId=null"));
        let _ = fs::remove_dir_all(&dir);
    }
}
