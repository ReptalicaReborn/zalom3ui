//! Replay report, replay modes, listener pause/resume and the notify window, driven through the
//! fake Zalo client: tests emit `Connected` and `OldMessages` events and assert the requests the
//! listener received and the report that was written.

use std::time::Duration;

use serde_json::json;

use zalo_proto::{RawWsMessage, ThreadType, WsEvent};

use super::test_support::{incoming, of_type, rig};
use super::*;
use crate::replay::{OfflineSource, ReplayMode, Verdict, MAX_PAGES};
use crate::testing::{settle, test_credentials, FakeLogin, FakeZaloApi, LoginStep};

/// A rig with the listener started on the fake client.
fn listening() -> super::test_support::Rig {
    let r = rig();
    r.session.testing_start_listener().unwrap();
    r
}

fn old_batch(thread_type: ThreadType, raws: Vec<RawWsMessage>) -> WsEvent {
    WsEvent::OldMessages {
        thread_type,
        messages: raws,
    }
}

/// `n` incoming 1:1 messages with ids and timestamps `from..from+n`.
fn user_msgs(from: i64, n: i64) -> Vec<RawWsMessage> {
    (from..from + n)
        .map(|i| incoming(&i.to_string(), i, json!({}), false))
        .collect()
}

async fn emit(r: &super::test_support::Rig, ev: WsEvent) {
    assert!(r.api.emit(ev));
    settle().await;
}

fn requests(r: &super::test_support::Rig) -> Vec<String> {
    r.api
        .listener_log()
        .into_iter()
        .filter(|l| l.starts_with("request_old_messages"))
        .collect()
}

fn last_report(r: &super::test_support::Rig) -> crate::replay::ReplayReport {
    r.session.replay().reports().pop().expect("a report")
}

// ---- report ----

#[tokio::test(start_paused = true)]
async fn connect_writes_a_report_with_what_we_had_and_what_came_back() {
    let r = listening();
    r.ingest(incoming("10", 1_000, json!({}), false)).await;
    r.ingest(incoming("g5", 5_000, json!({}), true)).await;

    emit(&r, WsEvent::Connected).await;
    assert_eq!(
        requests(&r),
        vec![
            "request_old_messages:user:null",
            "request_old_messages:group:null"
        ],
        "null mode is today's behavior"
    );
    let rep = last_report(&r);
    assert_eq!(rep.mode, ReplayMode::Null);
    assert_eq!(rep.user.newest_stored_ts, Some(1_000));
    assert_eq!(rep.user.newest_stored_id.as_deref(), Some("10"));
    assert_eq!(rep.group.newest_stored_id.as_deref(), Some("g5"));
    assert_eq!(rep.user.verdict, Verdict::Unknown, "no reply yet");

    // 1:1 batch overlapping what we have: 1 already stored, 2 new.
    let mut batch = user_msgs(900, 1);
    batch.push(incoming("10", 1_000, json!({}), false));
    batch.push(incoming("11", 1_500, json!({}), false));
    emit(&r, old_batch(ThreadType::User, batch)).await;
    // Empty group batch.
    emit(&r, old_batch(ThreadType::Group, vec![])).await;

    let rep = last_report(&r);
    let page = &rep.user.pages[0];
    assert_eq!((page.count, page.already_stored, page.new), (3, 1, 2));
    assert_eq!((page.oldest_ts, page.newest_ts), (Some(900), Some(1_500)));
    assert_eq!(page.oldest_id.as_deref(), Some("900"));
    assert_eq!(page.newest_id.as_deref(), Some("11"));
    assert_eq!(rep.user.verdict, Verdict::Covered);
    assert_eq!(rep.group.verdict, Verdict::NothingMissed);
    assert_eq!(rep.group.pages[0].count, 0);
}

#[tokio::test(start_paused = true)]
async fn a_batch_newer_than_everything_we_had_is_a_possible_gap_when_it_looks_full() {
    let r = listening();
    r.ingest(incoming("10", 1_000, json!({}), false)).await;
    emit(&r, WsEvent::Connected).await;
    emit(&r, old_batch(ThreadType::User, user_msgs(5_000, 30))).await;
    let rep = last_report(&r);
    assert_eq!(rep.user.verdict, Verdict::PossibleGap);
    assert_eq!(rep.user.pages[0].new, 30);
    assert_eq!(
        rep.max_batch_seen, 30,
        "the page size is learned, not assumed"
    );

    // A small batch newer than everything is unknown, not a gap.
    emit(&r, WsEvent::Connected).await;
    emit(&r, old_batch(ThreadType::User, user_msgs(9_000, 3))).await;
    assert_eq!(last_report(&r).user.verdict, Verdict::Unknown);
}

#[tokio::test(start_paused = true)]
async fn the_report_holds_no_message_content() {
    let r = listening();
    r.ingest(incoming(
        "10",
        1_000,
        json!({ "content": "SECRET-TEXT" }),
        false,
    ))
    .await;
    emit(&r, WsEvent::Connected).await;
    emit(
        &r,
        old_batch(
            ThreadType::User,
            vec![incoming(
                "11",
                2_000,
                json!({ "content": "SECRET-TEXT-2" }),
                false,
            )],
        ),
    )
    .await;
    r.session.replay().flush().await;
    let json_text = serde_json::to_string(&r.session.replay().reports()).unwrap();
    let on_disk = std::fs::read_to_string(r.dir.join("replay-log.json")).unwrap();
    let text = r.session.replay().render_text();
    for s in [&json_text, &on_disk, &text] {
        assert!(!s.contains("SECRET"), "{s}");
        assert!(!s.contains("Alice"), "no sender names either");
    }
    assert!(on_disk.contains("\"oldestId\":\"11\""));
}

#[tokio::test(start_paused = true)]
async fn each_connect_gets_its_own_report_and_offline_time_survives_process_death() {
    let dir = std::env::temp_dir().join(format!("zalo-replay-sess-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    // The previous process left a disconnect time one hour ago.
    let an_hour_ago = now_ms() - 3_600_000;
    std::fs::write(
        dir.join("listener-state.json"),
        json!({ "disconnectedMs": an_hour_ago }).to_string(),
    )
    .unwrap();
    let session = Arc::new(ZaloSession::with_login(
        ChatStore::in_memory(),
        dir.clone(),
        true,
        FakeLogin::new(vec![]),
    ));
    let api = FakeZaloApi::new("me");
    session.testing_attach(api.clone());
    session.testing_start_listener().unwrap();

    assert!(api.emit(WsEvent::Connected));
    settle().await;
    let rep = session.replay().reports().pop().unwrap();
    assert_eq!(rep.offline_source, OfflineSource::Close);
    let ms = rep.offline_ms.unwrap();
    assert!((3_599_000..=3_700_000).contains(&ms), "{ms}");

    // An unexpected close records the disconnect time on disk.
    assert!(api.emit(WsEvent::Closed {
        code: 1006,
        reason: String::new()
    }));
    settle().await;
    session.replay().flush().await;
    let file = std::fs::read_to_string(dir.join("listener-state.json")).unwrap();
    assert!(file.contains("disconnectedMs\":1"), "{file}");

    // The reconnect makes a second report.
    tokio::time::advance(Duration::from_secs(6)).await;
    settle().await;
    assert!(api.emit(WsEvent::Connected));
    settle().await;
    let reports = session.replay().reports();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[1].offline_source, OfflineSource::Close);
    assert!(reports[1].offline_ms.unwrap() < 60_000);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- modes ----

#[tokio::test(start_paused = true)]
async fn newest_mode_sends_the_newest_stored_id_of_each_kind() {
    let r = listening();
    r.session.replay().set_mode(ReplayMode::Newest);
    r.ingest(incoming("10", 1_000, json!({}), false)).await;
    r.ingest(incoming("12", 3_000, json!({}), false)).await;
    r.ingest(incoming("11", 2_000, json!({}), false)).await;
    // No group message stored: that kind still asks with null.
    emit(&r, WsEvent::Connected).await;
    assert_eq!(
        requests(&r),
        vec![
            "request_old_messages:user:12",
            "request_old_messages:group:null"
        ]
    );
    let rep = last_report(&r);
    assert_eq!(rep.mode, ReplayMode::Newest);
    assert_eq!(rep.user.pages[0].last_id.as_deref(), Some("12"));
    assert_eq!(rep.group.pages[0].last_id, None);
}

#[tokio::test(start_paused = true)]
async fn page_mode_pages_back_with_the_oldest_id_until_it_overlaps() {
    let r = listening();
    r.session.replay().set_mode(ReplayMode::Page);
    r.ingest(incoming("1", 1_000, json!({}), false)).await;
    emit(&r, WsEvent::Connected).await;
    assert_eq!(requests(&r).len(), 2, "user and group, both null first");

    // Full, newer than everything: ask again from the oldest id (5000).
    emit(&r, old_batch(ThreadType::User, user_msgs(5_000, 20))).await;
    assert_eq!(
        requests(&r).last().unwrap(),
        "request_old_messages:user:5000"
    );
    // Still full and newer: page again from 3000.
    emit(&r, old_batch(ThreadType::User, user_msgs(3_000, 20))).await;
    assert_eq!(
        requests(&r).last().unwrap(),
        "request_old_messages:user:3000"
    );
    // This one reaches back to what we had: stop.
    emit(&r, old_batch(ThreadType::User, user_msgs(950, 20))).await;
    assert_eq!(requests(&r).len(), 4);

    let rep = last_report(&r);
    assert_eq!(rep.mode, ReplayMode::Page);
    let ids: Vec<_> = rep.user.pages.iter().map(|p| p.last_id.clone()).collect();
    assert_eq!(
        ids,
        vec![None, Some("5000".to_string()), Some("3000".to_string())]
    );
    assert!(rep.user.pages.iter().all(|p| p.received && p.count == 20));
    assert_eq!(rep.user.verdict, Verdict::Covered);
}

#[tokio::test(start_paused = true)]
async fn page_mode_gives_up_after_five_requests() {
    let r = listening();
    r.session.replay().set_mode(ReplayMode::Page);
    r.ingest(incoming("1", 1, json!({}), false)).await;
    emit(&r, WsEvent::Connected).await;
    for page in 0..10 {
        emit(
            &r,
            old_batch(ThreadType::User, user_msgs(1_000_000 - page * 1000, 20)),
        )
        .await;
    }
    let user_requests = requests(&r).iter().filter(|l| l.contains(":user:")).count();
    assert_eq!(user_requests, MAX_PAGES);
    assert_eq!(last_report(&r).user.verdict, Verdict::PossibleGap);
}

#[tokio::test(start_paused = true)]
async fn null_mode_never_pages() {
    let r = listening();
    r.ingest(incoming("1", 1_000, json!({}), false)).await;
    emit(&r, WsEvent::Connected).await;
    emit(&r, old_batch(ThreadType::User, user_msgs(5_000, 20))).await;
    assert_eq!(requests(&r).len(), 2);
}

// ---- pause / resume ----

#[tokio::test(start_paused = true)]
async fn pause_stops_the_listener_and_nothing_reconnects_until_resume() {
    let mut r = listening();
    emit(&r, WsEvent::Connected).await;
    r.drain_events();

    r.session.pause();
    assert_eq!(r.session.get_status().state, SessionState::Paused);
    assert!(r.api.listener_log().contains(&"stop".to_string()));
    let statuses = r.drain_events();
    assert!(of_type(&statuses, "status")
        .iter()
        .any(|e| e["status"]["state"] == "paused"));

    // No reconnect, however long we wait.
    tokio::time::advance(Duration::from_secs(3_600)).await;
    settle().await;
    r.session.retry_now();
    settle().await;
    assert_eq!(r.api.listener_starts(), 1);
    assert_eq!(r.session.get_status().state, SessionState::Paused);

    // Pausing again is harmless, and REST calls keep working while paused.
    r.session.pause();
    assert!(r.session.api().is_ok());

    r.session.resume();
    settle().await;
    assert_eq!(r.api.listener_starts(), 2);
    assert_eq!(r.session.get_status().state, SessionState::Ready);
    // Connected then replays and writes a report.
    let before = r.session.replay().reports().len();
    emit(&r, WsEvent::Connected).await;
    assert_eq!(r.session.replay().reports().len(), before + 1);
    assert_eq!(requests(&r).len(), 4);

    // Resuming a running listener changes nothing.
    r.session.resume();
    settle().await;
    assert_eq!(r.api.listener_starts(), 2);
}

#[tokio::test(start_paused = true)]
async fn pause_cancels_a_pending_reconnect() {
    let r = listening();
    emit(
        &r,
        WsEvent::Closed {
            code: CloseReason::AbnormalClosure as u16,
            reason: String::new(),
        },
    )
    .await;
    assert_eq!(r.session.get_status().state, SessionState::Disconnected);

    r.session.pause();
    assert_eq!(r.session.get_status().state, SessionState::Paused);
    tokio::time::advance(Duration::from_secs(600)).await;
    settle().await;
    assert_eq!(r.api.listener_starts(), 1, "the reconnect was cancelled");

    // A manual reconnect while paused resumes.
    r.session.reconnect().unwrap();
    settle().await;
    assert_eq!(r.api.listener_starts(), 2);
    assert_ne!(r.session.get_status().state, SessionState::Paused);
}

#[tokio::test(start_paused = true)]
async fn pause_and_resume_do_nothing_when_logged_out() {
    let dir = std::env::temp_dir().join(format!("zalo-pause-{:016x}", rand::random::<u64>()));
    let session = Arc::new(ZaloSession::with_login(
        ChatStore::in_memory(),
        dir.clone(),
        true,
        FakeLogin::new(vec![]),
    ));
    session.pause();
    assert_eq!(session.get_status().state, SessionState::LoggedOut);
    session.resume();
    assert_eq!(session.get_status().state, SessionState::LoggedOut);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn logout_works_from_paused_and_login_is_not_replaced_while_paused() {
    let r = listening();
    r.session.pause();
    // Already logged in: a new QR login does not replace the paused session.
    r.session.start_qr_login();
    settle().await;
    assert_eq!(r.session.get_status().state, SessionState::Paused);

    r.session.logout().await;
    assert_eq!(r.session.get_status().state, SessionState::LoggedOut);
    // The pause does not outlive the login.
    r.session.resume();
    assert_eq!(r.session.get_status().state, SessionState::LoggedOut);
}

#[tokio::test(start_paused = true)]
async fn pause_flushes_the_store() {
    let dir = std::env::temp_dir().join(format!("zalo-pause-flush-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = ChatStore::open(&dir);
    let session = Arc::new(ZaloSession::with_login(
        store.clone(),
        dir.clone(),
        true,
        FakeLogin::new(vec![]),
    ));
    session.testing_attach(FakeZaloApi::new("me"));
    session.testing_start_listener().unwrap();
    session
        .testing_dispatch(WsEvent::Messages(vec![incoming(
            "1",
            1_000,
            json!({}),
            false,
        )]))
        .await;
    session.pause_async().await;
    let reopened = ChatStore::open(&dir);
    assert!(
        reopened.get_message("1").is_some(),
        "flushed before pausing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn a_pause_during_the_login_in_flight_applies_when_it_completes() {
    let dir = std::env::temp_dir().join(format!("zalo-pause-login-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("credentials.json"),
        serde_json::to_string(&test_credentials()).unwrap(),
    )
    .unwrap();
    let api = FakeZaloApi::new("me");
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let release_rx = Arc::new(std::sync::Mutex::new(Some(release_rx)));
    let api_for_login = api.clone();
    let login = FakeLogin::new(vec![LoginStep::cookie(move |_| {
        let rx = release_rx.lock().unwrap().take();
        let api = api_for_login.clone();
        Box::pin(async move {
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            Ok(api as Arc<dyn ZaloApi>)
        })
    })]);
    let session = Arc::new(ZaloSession::with_login(
        ChatStore::in_memory(),
        dir.clone(),
        true,
        login,
    ));
    session.start();
    settle().await;
    assert_eq!(session.get_status().state, SessionState::LoggingIn);

    session.pause();
    release_tx.send(()).unwrap();
    settle().await;
    assert_eq!(session.get_status().state, SessionState::Paused);
    assert_eq!(api.listener_starts(), 0, "no listener while paused");

    session.resume();
    settle().await;
    assert_eq!(api.listener_starts(), 1);
    assert_eq!(session.get_status().state, SessionState::Ready);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- notify window ----

fn public_msg(r: &super::test_support::Rig, id: &str) -> ChatMessage {
    r.store
        .to_public(&r.store.get_message(id).unwrap_or_else(|| {
            panic!("message {id} is not stored");
        }))
}

#[tokio::test]
async fn replayed_messages_notify_only_inside_an_open_notify_window() {
    let r = rig();
    let now = 10_000_000i64;
    let trigger = now;
    r.ingest(incoming("fresh", now - 60_000, json!({}), false))
        .await;
    r.ingest(incoming("stale", now - 3 * 60_000, json!({}), false))
        .await;
    let fresh = public_msg(&r, "fresh");
    let stale = public_msg(&r, "stale");
    let s = &r.session;

    // No window: replay never notifies (today's behavior).
    assert!(!s.should_notify_at(&fresh, true, now));

    s.set_notify_window_at(trigger, 60_000, now);
    assert!(
        s.should_notify_at(&fresh, true, now),
        "newer than trigger minus 2 minutes"
    );
    assert!(
        !s.should_notify_at(&stale, true, now),
        "older than trigger minus 2 minutes"
    );
    // Live messages follow the usual rules (the 10-minute age limit is checked against `now`).
    assert!(s.should_notify_at(&fresh, false, now));

    // Muted chat.
    r.store.set_muted(ThreadKind::User, "100", Some(-1));
    assert!(!s.should_notify_at(&fresh, true, now));
    r.store.set_muted(ThreadKind::User, "100", None);
    // Sent by me.
    let mine = ChatMessage {
        is_self: true,
        ..fresh.clone()
    };
    assert!(!s.should_notify_at(&mine, true, now));
    // The chat on screen.
    s.set_presence_at(Some(ThreadKind::User), Some("100"), now);
    assert!(!s.should_notify_at(&fresh, true, now));
    s.set_presence_at(None, None, now);
    assert!(s.should_notify_at(&fresh, true, now));

    // Already read: the chat's read marker covers it; a newer message does not count as read.
    r.store
        .mark_read(ThreadKind::User, "100", Some(now - 30_000));
    assert!(!s.should_notify_at(&fresh, true, now));
    let unread = ChatMessage {
        ts: now - 10_000,
        ..fresh.clone()
    };
    assert!(
        s.should_notify_at(&unread, true, now),
        "after the read marker"
    );

    // The window closes by itself, and on demand.
    assert!(!s.should_notify_at(&unread, true, now + 60_001));
    s.clear_notify_window();
    assert!(!s.should_notify_at(&unread, true, now));
}

#[tokio::test]
async fn bursts_coalesce_into_one_window_with_the_earliest_trigger() {
    let r = rig();
    let s = &r.session;
    let now = 1_000_000i64;
    let w = s.set_notify_window_at(now, 60_000, now);
    assert_eq!((w.trigger_ts, w.expires_at), (now, now + 60_000));
    // 30 s later another Zalo notification: the window extends, the trigger stays.
    let w = s.set_notify_window_at(now + 30_000, 60_000, now + 30_000);
    assert_eq!((w.trigger_ts, w.expires_at), (now, now + 90_000));
    // An older trigger pulls it earlier.
    let w = s.set_notify_window_at(now - 5_000, 10_000, now + 40_000);
    assert_eq!((w.trigger_ts, w.expires_at), (now - 5_000, now + 90_000));
    // After it expired, a new one starts fresh.
    let w = s.set_notify_window_at(now + 500_000, 60_000, now + 500_000);
    assert_eq!(w.trigger_ts, now + 500_000);
    // ttl is clamped to 10 minutes.
    let w = s.set_notify_window_at(now + 500_000, 99_999_999, now + 500_000);
    assert_eq!(w.expires_at, now + 500_000 + NOTIFY_WINDOW_MAX_TTL_MS);
    assert_eq!(s.notify_window_at(now + 500_000), Some(w));
}

#[tokio::test(start_paused = true)]
async fn the_replay_notifies_through_the_event_stream_while_the_window_is_open() {
    let mut r = listening();
    let now = now_ms();
    r.session.set_notify_window(now, 60_000);
    r.ingest(incoming("1", 1, json!({}), false)).await; // stored long ago
    r.drain_events();
    emit(&r, WsEvent::Connected).await;
    // One replayed message from a minute ago, one from an hour ago.
    emit(
        &r,
        old_batch(
            ThreadType::User,
            vec![
                incoming("recent", now - 60_000, json!({}), false),
                incoming("ancient", now - 3_600_000, json!({}), false),
            ],
        ),
    )
    .await;
    let events = r.drain_events();
    let messages = of_type(&events, "message");
    assert_eq!(messages.len(), 2);
    let recent = messages
        .iter()
        .find(|e| e["message"]["id"] == "recent")
        .unwrap();
    assert_eq!(recent["notify"], true);
    assert_eq!(recent["threadName"], "Alice");
    let ancient = messages
        .iter()
        .find(|e| e["message"]["id"] == "ancient")
        .unwrap();
    assert_eq!(ancient["notify"], false);

    // Without a window, the same replay stays silent.
    r.session.clear_notify_window();
    emit(
        &r,
        old_batch(
            ThreadType::User,
            vec![incoming("recent2", now - 60_000, json!({}), false)],
        ),
    )
    .await;
    let events = r.drain_events();
    assert_eq!(of_type(&events, "message")[0]["notify"], false);
}

// ---- asking for missed messages after an empty 501 ----

fn frame_501(lens: &[(&str, usize)]) -> WsEvent {
    let mut r = zalo_proto::framelog::FrameReport::new(1, 501, 0);
    r.arrays = lens
        .iter()
        .map(|(name, len)| zalo_proto::framelog::ArrayReport {
            name: (*name).to_string(),
            len: *len,
            read: true,
            items: Vec::new(),
            items_truncated: false,
        })
        .collect();
    WsEvent::Frame(r)
}

/// What arrived on a web session when an Official Account OTP reached the phone.
fn empty_501() -> WsEvent {
    frame_501(&[("msgs", 0), ("pageMsgs", 0), ("seens", 0), ("groupMsgs", 0)])
}

#[tokio::test(start_paused = true)]
async fn an_empty_501_asks_for_missed_messages_and_a_recent_one_in_the_answer_notifies() {
    let mut r = listening();
    emit(&r, empty_501()).await;
    assert_eq!(requests(&r), vec!["request_old_messages:user:null"]);

    // The answer: a recent message (the OTP), delivered as 1:1 history.
    let now = now_ms();
    emit(
        &r,
        old_batch(ThreadType::User, vec![incoming("otp1", now, json!({}), false)]),
    )
    .await;
    let events = r.drain_events();
    let msgs = of_type(&events, "message");
    assert_eq!(msgs.len(), 1, "{events:?}");
    assert_eq!(msgs[0]["notify"], true, "history from the answer notifies");

    // Kept out of the replay report (no connect happened, so there is none at all).
    assert!(r.session.replay().reports().is_empty());
    // The socket log says why Zalỏ asked.
    assert!(r.session.socket_log().render_text().contains(POKE_PULL_NOTE));
}

#[tokio::test(start_paused = true)]
async fn empty_501s_ask_at_most_once_per_interval() {
    let r = listening();
    emit(&r, empty_501()).await;
    emit(&r, empty_501()).await;
    assert_eq!(requests(&r).len(), 1);

    // Once the interval has passed, the next empty 501 asks again.
    if let Ok(mut s) = r.session.inner.write() {
        s.poke_pull_at -= POKE_PULL_MIN_INTERVAL_MS;
    }
    emit(&r, empty_501()).await;
    assert_eq!(requests(&r).len(), 2);
}

#[tokio::test(start_paused = true)]
async fn an_empty_501_right_after_connecting_does_not_ask() {
    let r = listening();
    emit(&r, WsEvent::Connected).await;
    let after_connect = requests(&r).len();
    emit(&r, empty_501()).await;
    assert_eq!(requests(&r).len(), after_connect, "the connect's own replay is still arriving");
}

#[tokio::test(start_paused = true)]
async fn a_501_with_messages_does_not_ask() {
    let r = listening();
    emit(&r, frame_501(&[("msgs", 1), ("pageMsgs", 0)])).await;
    assert!(requests(&r).is_empty());
}
