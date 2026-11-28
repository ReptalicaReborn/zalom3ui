//! Listener reconnect after an unexpected close: backoff schedule, single tracked task, and the
//! cancel / wake paths. All on tokio's paused clock.

use std::time::Duration;

use super::*;
use crate::testing::{settle, FakeLogin, FakeZaloApi};

fn session_with_listener() -> (Arc<ZaloSession>, Arc<FakeZaloApi>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("zalo-reconnect-{:016x}", rand::random::<u64>()));
    let session = Arc::new(ZaloSession::with_login(
        ChatStore::in_memory(),
        dir.clone(),
        true,
        FakeLogin::new(vec![]),
    ));
    let api = FakeZaloApi::new("me");
    session.testing_attach(api.clone());
    session.testing_start_listener().unwrap();
    (session, api, dir)
}

fn closed(code: u16) -> WsEvent {
    WsEvent::Closed {
        code,
        reason: String::new(),
    }
}

/// Emits an abnormal close and lets the session process it.
async fn drop_connection(api: &FakeZaloApi) {
    assert!(api.emit(closed(CloseReason::AbnormalClosure as u16)));
    settle().await;
}

async fn advance(d: Duration) {
    tokio::time::advance(d).await;
    settle().await;
}

#[test]
fn backoff_schedule_doubles_from_5s_to_a_5_minute_cap() {
    let secs: Vec<u64> = (0..10).map(|n| reconnect_base_delay(n).as_secs()).collect();
    assert_eq!(secs, vec![5, 10, 20, 40, 80, 160, 300, 300, 300, 300]);
    assert_eq!(
        reconnect_base_delay(u32::MAX).as_secs(),
        300,
        "no overflow far out"
    );
    for attempt in 0..10 {
        let base = reconnect_base_delay(attempt);
        for _ in 0..200 {
            let d = reconnect_delay(attempt);
            assert!(
                d <= base,
                "jitter never lengthens the delay past the schedule"
            );
            assert!(
                d >= base.mul_f64(0.9),
                "jitter is at most 10%: {d:?} vs {base:?}"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn an_unexpected_close_reconnects_with_growing_delays_until_connected() {
    let (session, api, _dir) = session_with_listener();
    assert_eq!(api.listener_starts(), 1);

    // 1st close: reconnect after ~5 s (4.5..=5 s with jitter).
    drop_connection(&api).await;
    assert_eq!(session.get_status().state, SessionState::Disconnected);
    advance(Duration::from_millis(4_400)).await;
    assert_eq!(
        api.listener_starts(),
        1,
        "not before the shortest jittered delay"
    );
    advance(Duration::from_millis(700)).await;
    assert_eq!(api.listener_starts(), 2);

    // The new listener is closed right away too: the next wait is ~10 s, not 5 s again.
    drop_connection(&api).await;
    advance(Duration::from_millis(8_900)).await;
    assert_eq!(api.listener_starts(), 2);
    advance(Duration::from_millis(1_200)).await;
    assert_eq!(api.listener_starts(), 3);

    // ...then ~20 s.
    drop_connection(&api).await;
    advance(Duration::from_millis(17_900)).await;
    assert_eq!(api.listener_starts(), 3);
    advance(Duration::from_millis(2_200)).await;
    assert_eq!(api.listener_starts(), 4);

    // A successful connection resets the schedule to 5 s.
    assert!(api.emit(WsEvent::Connected));
    settle().await;
    assert_eq!(session.get_status().state, SessionState::Ready);
    drop_connection(&api).await;
    advance(Duration::from_millis(5_100)).await;
    assert_eq!(api.listener_starts(), 5);
}

#[tokio::test(start_paused = true)]
async fn the_delay_stops_growing_at_five_minutes() {
    let (_session, api, _dir) = session_with_listener();
    // Delays 5, 10, 20, 40, 80, 160, then capped at 300 s.
    for n in 0..9u32 {
        drop_connection(&api).await;
        let upper = reconnect_base_delay(n);
        // It has not fired just before the shortest jittered delay...
        advance(upper.mul_f64(0.9) - Duration::from_millis(100)).await;
        assert_eq!(
            api.listener_starts(),
            1 + n as usize,
            "attempt {n} fired early"
        );
        // ...and has by the schedule's value.
        advance(upper.mul_f64(0.1) + Duration::from_millis(200)).await;
        assert_eq!(
            api.listener_starts(),
            2 + n as usize,
            "attempt {n} did not fire"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn closes_never_pile_up_overlapping_reconnects() {
    let (_session, api, _dir) = session_with_listener();
    // Three closes reported on the same listener before the first delay is over.
    drop_connection(&api).await;
    drop_connection(&api).await;
    drop_connection(&api).await;
    advance(Duration::from_secs(60)).await;
    assert_eq!(
        api.listener_starts(),
        2,
        "one tracked reconnect, one restart"
    );
}

#[tokio::test(start_paused = true)]
async fn a_kicked_close_is_not_reconnected() {
    let (session, api, _dir) = session_with_listener();
    for code in [
        CloseReason::DuplicateConnection,
        CloseReason::KickConnection,
    ] {
        assert!(api.emit(closed(code as u16)));
        settle().await;
        assert_eq!(session.get_status().state, SessionState::Disconnected);
        advance(Duration::from_secs(600)).await;
        assert_eq!(api.listener_starts(), 1, "{code:?} must not reconnect");
    }
}

#[tokio::test(start_paused = true)]
async fn logout_cancels_a_pending_reconnect() {
    let (session, api, dir) = session_with_listener();
    drop_connection(&api).await;
    session.logout().await;
    advance(Duration::from_secs(600)).await;
    assert_eq!(api.listener_starts(), 1);
    assert_eq!(session.get_status().state, SessionState::LoggedOut);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn shutdown_cancels_a_pending_reconnect() {
    let (session, api, _dir) = session_with_listener();
    drop_connection(&api).await;
    session.shutdown();
    advance(Duration::from_secs(600)).await;
    assert_eq!(api.listener_starts(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_reconnect_for_a_replaced_login_does_nothing() {
    let (session, api, _dir) = session_with_listener();
    drop_connection(&api).await;
    // The session moves to another login generation (and a different client) meanwhile.
    session.logout().await;
    let other = FakeZaloApi::new("me");
    session.testing_attach(other.clone());
    session.patch_status(|st| st.state = SessionState::Disconnected);
    advance(Duration::from_secs(600)).await;
    assert_eq!(api.listener_starts(), 1);
    assert_eq!(other.listener_starts(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_manual_reconnect_replaces_the_pending_one_and_resets_the_backoff() {
    let (session, api, _dir) = session_with_listener();
    drop_connection(&api).await;
    session.reconnect().unwrap();
    assert_eq!(
        api.listener_starts(),
        2,
        "manual reconnect starts the listener right away"
    );
    advance(Duration::from_secs(600)).await;
    assert_eq!(
        api.listener_starts(),
        2,
        "the pending automatic reconnect was cancelled"
    );

    // Build the backoff up to ~10 s, then reset it with a manual reconnect.
    let (session2, api2, _dir2) = session_with_listener();
    drop_connection(&api2).await;
    advance(Duration::from_millis(5_100)).await;
    drop_connection(&api2).await; // would now wait ~10 s
    session2.reconnect().unwrap();
    drop_connection(&api2).await;
    advance(Duration::from_millis(5_100)).await;
    assert_eq!(api2.listener_starts(), 4, "back to a ~5 s wait");
}

#[tokio::test(start_paused = true)]
async fn retry_now_wakes_a_pending_reconnect_immediately() {
    let (session, api, _dir) = session_with_listener();
    drop_connection(&api).await;
    assert_eq!(api.listener_starts(), 1);
    session.retry_now();
    settle().await;
    assert_eq!(api.listener_starts(), 2, "no waiting for the backoff");

    // Waking does not escalate the backoff: the next close still waits ~5 s.
    drop_connection(&api).await;
    advance(Duration::from_millis(5_100)).await;
    assert_eq!(api.listener_starts(), 3);
}

#[tokio::test(start_paused = true)]
async fn an_early_retry_now_is_not_banked_for_a_later_close() {
    let (session, api, _dir) = session_with_listener();
    // The app calls /api/retry whenever the network is back, usually with nothing waiting.
    session.retry_now();
    settle().await;
    drop_connection(&api).await;
    assert_eq!(
        api.listener_starts(),
        1,
        "the earlier retry must not skip this backoff"
    );
    advance(Duration::from_millis(5_100)).await;
    assert_eq!(api.listener_starts(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_listener_that_cannot_start_is_retried_with_backoff() {
    let (session, api, _dir) = session_with_listener();
    drop_connection(&api).await;
    api.fail_listener_start(true);
    advance(Duration::from_millis(5_100)).await; // fails; waits ~10 s now
    assert_eq!(api.listener_starts(), 1);
    api.fail_listener_start(false);
    advance(Duration::from_millis(5_000)).await;
    assert_eq!(
        api.listener_starts(),
        1,
        "second try waits longer than the first"
    );
    advance(Duration::from_millis(5_100)).await;
    assert_eq!(api.listener_starts(), 2);
    assert_eq!(session.get_status().state, SessionState::Disconnected);
}

// ---- a dead session is told apart from a dropped network ----

/// Like [`session_with_listener`], but with saved credentials and scripted re-logins (what the
/// reconnect uses to check that Zalo still honours the session).
fn session_with_logins(
    steps: Vec<crate::testing::LoginStep>,
) -> (Arc<ZaloSession>, Arc<FakeZaloApi>, Arc<FakeLogin>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("zalo-reconnect-{:016x}", rand::random::<u64>()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("credentials.json"),
        json!({ "imei": "i", "cookie": [], "userAgent": "ua" }).to_string(),
    )
    .unwrap();
    let login = FakeLogin::new(steps);
    let session = Arc::new(ZaloSession::with_login(
        ChatStore::in_memory(),
        dir.clone(),
        true,
        login.clone(),
    ));
    let api = FakeZaloApi::new("me");
    session.testing_attach(api.clone());
    session.testing_start_listener().unwrap();
    (session, api, login, dir)
}

fn rejected() -> ProtoError {
    ProtoError::Login(-1, "Đăng nhập thất bại".into())
}

#[tokio::test(start_paused = true)]
async fn a_close_with_a_rejected_session_goes_logged_out_and_stops_reconnecting() {
    use crate::testing::LoginStep;
    let (session, api, login, dir) = session_with_logins(vec![LoginStep::cookie_error(rejected)]);
    drop_connection(&api).await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggedOut);
    assert_eq!(s.logged_out_reason.as_deref(), Some(LOGGED_OUT_ELSEWHERE));
    assert_eq!(s.error.as_deref(), Some(LOGGED_OUT_ELSEWHERE_MSG));
    assert!(!s.resuming && !s.retrying);
    assert_eq!(login.attempts(), 1);

    advance(Duration::from_secs(600)).await;
    assert_eq!(api.listener_starts(), 1, "no reconnect with a dead session");
    assert_eq!(login.attempts(), 1, "and no more login attempts");
    assert_eq!(session.get_status().state, SessionState::LoggedOut);
    assert!(dir.join("credentials.json").exists(), "credentials stay on disk");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn a_kick_close_with_a_rejected_session_goes_logged_out() {
    use crate::testing::LoginStep;
    let (session, api, _login, dir) = session_with_logins(vec![LoginStep::cookie_error(rejected)]);
    assert!(api.emit(closed(CloseReason::KickConnection as u16)));
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggedOut);
    assert_eq!(s.logged_out_reason.as_deref(), Some(LOGGED_OUT_ELSEWHERE));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn a_duplicate_connection_close_stays_disconnected_without_a_login_check() {
    use crate::testing::LoginStep;
    let (session, api, login, dir) = session_with_logins(vec![LoginStep::cookie_error(rejected)]);
    assert!(api.emit(closed(CloseReason::DuplicateConnection as u16)));
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::Disconnected);
    assert_eq!(s.logged_out_reason, None);
    assert_eq!(login.attempts(), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn a_close_while_offline_keeps_reconnecting_and_is_not_a_logout() {
    use crate::testing::LoginStep;
    let (session, api, login, dir) = session_with_logins(vec![LoginStep::cookie_error(|| {
        ProtoError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "peer closed connection without sending TLS close_notify",
        ))
    })]);
    drop_connection(&api).await;
    assert_eq!(session.get_status().state, SessionState::Disconnected);
    advance(Duration::from_millis(5_100)).await;
    assert_eq!(api.listener_starts(), 2, "reconnected after the backoff");
    assert!(login.attempts() >= 1, "the session was checked");
    let s = session.get_status();
    assert_eq!(s.state, SessionState::Disconnected);
    assert_eq!(s.logged_out_reason, None);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn a_close_with_a_still_valid_session_keeps_reconnecting() {
    use crate::testing::LoginStep;
    let (session, api, _login, dir) =
        session_with_logins(vec![LoginStep::cookie_ok(FakeZaloApi::new("me"))]);
    drop_connection(&api).await;
    advance(Duration::from_millis(5_100)).await;
    assert_eq!(api.listener_starts(), 2);
    assert_ne!(session.get_status().state, SessionState::LoggedOut);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn only_a_zalo_refusal_counts_as_a_rejected_session() {
    assert!(is_session_rejected(&ProtoError::Login(-1, "x".into())));
    assert!(is_session_rejected(&ProtoError::Login(114, "x".into())));
    assert!(is_session_rejected(&ProtoError::Login(401, "x".into())));
    assert!(is_session_rejected(&ProtoError::Api { code: 3000, message: "x".into() }));
    assert!(is_session_rejected(&ProtoError::HttpStatus { status: 403, message: "x".into() }));
    assert!(!is_session_rejected(&ProtoError::Login(503, "HTTP 503".into())));
    assert!(!is_session_rejected(&ProtoError::Login(429, "HTTP 429".into())));
    assert!(!is_session_rejected(&ProtoError::HttpStatus { status: 502, message: "x".into() }));
    assert!(!is_session_rejected(&ProtoError::WebSocket("x".into())));
    assert!(!is_session_rejected(&ProtoError::Io(std::io::Error::other("x"))));
    assert!(!is_session_rejected(&ProtoError::Crypto("x".into())));
    // And a 5xx at sign-in is retried, not treated as an expired session.
    assert!(is_transient_proto_error(&ProtoError::Login(503, "HTTP 503".into())));
    assert!(!is_transient_proto_error(&ProtoError::Login(-1, "x".into())));
}
