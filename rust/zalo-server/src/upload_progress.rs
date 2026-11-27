//! Turns the protocol client's per-file [`UploadProgress`] reports into throttled
//! `upload_progress` broadcast events:
//!
//! ```json
//! {"type":"upload_progress","threadKind":"user","threadId":"1","uploadId":"..","sent":0,"total":9,"done":false}
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use zalo_proto::{UploadProgress, UploadProgressSink};

use crate::types::ThreadKind;

/// At most one in-flight progress event per upload per this interval.
pub const MIN_INTERVAL: Duration = Duration::from_millis(100);

struct Slot {
    last_emit: Instant,
    last_sent: u64,
}

/// Decides which reports become events. The first report of an upload (its start) and the final
/// `done` one always pass; in between, a report passes if [`MIN_INTERVAL`] has elapsed since the
/// last emitted one and `sent` has advanced. Time is passed in so tests control it.
#[derive(Default)]
pub struct Throttle {
    slots: HashMap<usize, Slot>,
}

impl Throttle {
    pub fn admit(&mut self, index: usize, sent: u64, done: bool, now: Instant) -> bool {
        let pass = match self.slots.get(&index) {
            None => true,
            Some(_) if done => true,
            Some(slot) => {
                sent > slot.last_sent && now.duration_since(slot.last_emit) >= MIN_INTERVAL
            }
        };
        if pass {
            self.slots.insert(
                index,
                Slot {
                    last_emit: now,
                    last_sent: sent,
                },
            );
        }
        pass
    }
}

/// The wire event for one report.
pub fn progress_event(
    kind: ThreadKind,
    thread_id: &str,
    upload_id: &str,
    p: &UploadProgress,
) -> Value {
    json!({
        "type": "upload_progress",
        "threadKind": kind,
        "threadId": thread_id,
        "uploadId": upload_id,
        "sent": p.sent,
        "total": p.total,
        "done": p.done,
    })
}

static BATCH: AtomicU64 = AtomicU64::new(0);

/// A sink for one upload batch (one `upload_attachment` call: one `uploadId` per file) that
/// broadcasts throttled `upload_progress` events through `emit`.
pub fn progress_sink(
    kind: ThreadKind,
    thread_id: &str,
    emit: Arc<dyn Fn(Value) + Send + Sync>,
) -> UploadProgressSink {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let batch = format!("up-{ms}-{}", BATCH.fetch_add(1, Ordering::Relaxed));
    let thread_id = thread_id.to_string();
    let throttle = Mutex::new(Throttle::default());
    UploadProgressSink::new(move |p| {
        let pass = throttle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admit(p.index, p.sent, p.done, Instant::now());
        if pass {
            let id = format!("{batch}-{}", p.index);
            emit(progress_event(kind, &thread_id, &id, &p));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(index: usize, sent: u64, done: bool) -> UploadProgress {
        UploadProgress {
            index,
            filename: "a.png".into(),
            sent,
            total: 100,
            done,
        }
    }

    #[test]
    fn throttle_passes_start_and_done_and_spaces_progress() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut t = Throttle::default();
        assert!(t.admit(0, 0, false, ms(0)), "start always passes");
        assert!(!t.admit(0, 10, false, ms(50)), "too soon");
        assert!(t.admit(0, 20, false, ms(100)), "interval elapsed");
        assert!(!t.admit(0, 20, false, ms(250)), "no advance");
        assert!(!t.admit(0, 30, false, ms(150)), "still within 100ms of 100");
        assert!(t.admit(0, 30, false, ms(200)));
        assert!(t.admit(0, 31, true, ms(201)), "done always passes");
    }

    #[test]
    fn throttle_tracks_each_upload_separately() {
        let t0 = Instant::now();
        let mut t = Throttle::default();
        assert!(t.admit(0, 0, false, t0));
        assert!(t.admit(1, 0, false, t0), "second file's start is not throttled");
        assert!(!t.admit(1, 5, false, t0 + Duration::from_millis(10)));
    }

    #[test]
    fn sink_emits_contract_events_with_stable_unique_ids() {
        let out: Arc<Mutex<Vec<Value>>> = Default::default();
        let o = out.clone();
        let emit: Arc<dyn Fn(Value) + Send + Sync> = Arc::new(move |v| o.lock().unwrap().push(v));
        let sink = progress_sink(ThreadKind::Group, "g1", emit.clone());
        sink.report(prog(0, 0, false));
        sink.report(prog(1, 0, false));
        sink.report(prog(0, 50, false)); // throttled (same instant)
        sink.report(prog(0, 100, true));
        let sink2 = progress_sink(ThreadKind::User, "u", emit);
        sink2.report(prog(0, 0, false));
        let ev = out.lock().unwrap().clone();
        assert_eq!(ev.len(), 4);
        assert_eq!(ev[0]["type"], "upload_progress");
        assert_eq!(ev[0]["threadKind"], "group");
        assert_eq!(ev[0]["threadId"], "g1");
        assert_eq!(ev[0]["sent"], 0);
        assert_eq!(ev[0]["total"], 100);
        assert_eq!(ev[0]["done"], false);
        assert_eq!(ev[0]["uploadId"], ev[2]["uploadId"]);
        assert_ne!(ev[0]["uploadId"], ev[1]["uploadId"]);
        assert_eq!(ev[2]["done"], true);
        assert_eq!(ev[2]["sent"], 100);
        assert_ne!(ev[0]["uploadId"], ev[3]["uploadId"]);
    }
}
