//! Shared setup for the session-level tests ported from `test/session.test.ts`,
//! `test/login.test.ts` and `test/features.test.ts`.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::broadcast;

use zalo_proto::{RawWsMessage, ThreadType, WsEvent};

use crate::session::ZaloSession;
use crate::store::ChatStore;
use crate::testing::{FakeLogin, FakeZaloApi};

/// A signed-in session (own id `me`) wired to a recording fake client; the Rust counterpart of
/// the TS tests' `setup()`.
pub struct Rig {
    pub session: Arc<ZaloSession>,
    pub store: ChatStore,
    pub api: Arc<FakeZaloApi>,
    pub events: broadcast::Receiver<Value>,
    pub dir: PathBuf,
}

pub fn rig() -> Rig {
    rig_with(true)
}

pub fn rig_with(read_receipts: bool) -> Rig {
    let dir = std::env::temp_dir().join(format!("zalo-session-test-{:016x}", rand::random::<u64>()));
    let store = ChatStore::in_memory();
    let session = Arc::new(ZaloSession::with_login(
        store.clone(),
        dir.clone(),
        read_receipts,
        FakeLogin::new(vec![]),
    ));
    let api = FakeZaloApi::new("me");
    // The TS setup's defaults.
    api.respond("send_message", json!({ "message": { "msgId": 999 }, "attachment": [] }));
    session.testing_attach(api.clone());
    let events = session.subscribe_events();
    Rig {
        session,
        store,
        api,
        events,
        dir,
    }
}

impl Rig {
    /// Feeds one chat message through the listener's message handler.
    pub async fn ingest(&self, raw: RawWsMessage) {
        self.session.testing_dispatch(WsEvent::Messages(vec![raw])).await;
    }

    /// Everything broadcast since the last call (status, message, threads... events).
    pub fn drain_events(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An incoming webchat message, like the TS tests' `incoming()`: thread `100` (1:1) or `g1`
/// (group), from `100` unless `extra` says otherwise; `extra` keys override the defaults.
pub fn incoming(id: &str, ts: i64, extra: Value, group: bool) -> RawWsMessage {
    let mut data = json!({
        "msgId": id,
        "cliMsgId": format!("c{id}"),
        "msgType": "webchat",
        "uidFrom": "100",
        "idTo": if group { "g1" } else { "me" },
        "dName": "Alice",
        "ts": ts.to_string(),
        "content": format!("text {id}"),
        "ttl": 0,
        "st": 3,
        "at": 5,
        "cmd": 501,
    });
    if let (Some(base), Some(over)) = (data.as_object_mut(), extra.as_object()) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
    RawWsMessage {
        thread_type: if group { ThreadType::Group } else { ThreadType::User },
        thread_id: if group { "g1" } else { "100" }.to_string(),
        is_self: data["uidFrom"] == "me",
        data,
    }
}

/// The events with `"type": kind`.
pub fn of_type<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["type"] == kind).collect()
}
