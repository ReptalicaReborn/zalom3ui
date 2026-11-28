//! Ports of `test/login.test.ts`, one Rust test per TS test, in the same order. The TS tests fake
//! `newZalo()` (login / loginQR); here `FakeLogin` plays that part.

use std::io;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::oneshot;

use super::*;
use crate::testing::{settle, test_credentials, FakeLogin, FakeZaloApi, LoginStep};

/// A real "connection refused" error from the HTTP stack (nothing listens on port 1).
async fn refused_http_error() -> ProtoError {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    ProtoError::Http(client.get("http://127.0.0.1:1/").send().await.unwrap_err())
}

/// Stands in for the TS `fetchFailed("ENOTFOUND")`: a host that cannot be resolved.
fn dns_failure() -> ProtoError {
    ProtoError::Io(io::Error::other(
        "dns error: failed to lookup address information: Name or service not known",
    ))
}

fn rejected_session() -> ProtoError {
    ProtoError::Login(-1, "Đăng nhập thất bại".into())
}

/// A session whose logins follow `steps`; the data dir holds saved credentials if asked.
fn session_with(
    steps: Vec<LoginStep>,
    with_credentials: bool,
) -> (Arc<ZaloSession>, Arc<FakeLogin>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("zalo-login-test-{:016x}", rand::random::<u64>()));
    fs::create_dir_all(&dir).unwrap();
    if with_credentials {
        fs::write(
            dir.join("credentials.json"),
            json!({ "imei": "i", "cookie": [], "userAgent": "ua" }).to_string(),
        )
        .unwrap();
    }
    let login = FakeLogin::new(steps);
    let session = Arc::new(ZaloSession::with_login(
        ChatStore::in_memory(),
        dir.clone(),
        true,
        login.clone(),
    ));
    (session, login, dir)
}

fn qr_generated(image: &str) -> LoginQrEvent {
    LoginQrEvent::QrCodeGenerated {
        image_base64: image.to_string(),
        code: "code".to_string(),
    }
}

/// A QR login that reports `image` and then waits to be aborted.
fn qr_showing_until_aborted(image: &'static str) -> LoginStep {
    LoginStep::qr(move |events, abort_rx| {
        Box::pin(async move {
            let _ = events.send(qr_generated(image));
            let _ = abort_rx.await;
            Err(ProtoError::QrAborted)
        })
    })
}

#[tokio::test]
async fn is_transient_error_tells_network_failures_from_rejected_sessions() {
    assert!(is_transient_proto_error(&dns_failure()));
    assert!(is_transient_proto_error(&refused_http_error().await));
    // "Failed to fetch login info: Bad Gateway"
    assert!(is_transient_proto_error(&ProtoError::HttpStatus {
        status: 502,
        message: "Bad Gateway".into()
    }));
    assert!(is_transient_proto_error(&ProtoError::HttpStatus {
        status: 429,
        message: String::new()
    }));
    // "Đăng nhập thất bại": Zalo rejected the session.
    assert!(!is_transient_proto_error(&rejected_session()));
    assert!(!is_transient_proto_error(&ProtoError::HttpStatus {
        status: 403,
        message: String::new()
    }));
    // "Missing required params"
    assert!(!is_transient_proto_error(&ProtoError::InvalidArg(
        "Missing required params".into()
    )));
}

#[tokio::test]
async fn describe_error_explains_fetch_failures() {
    assert_eq!(
        describe_proto_error(&dns_failure()),
        "no internet connection"
    );
    assert_eq!(
        describe_proto_error(&refused_http_error().await),
        "connection refused"
    );
    assert_eq!(
        describe_proto_error(&ProtoError::Io(io::Error::from(io::ErrorKind::TimedOut))),
        "connection timed out"
    );
    // Other errors keep their own text (the Rust error type adds a category prefix).
    assert!(describe_proto_error(&ProtoError::InvalidArg("boom".into())).contains("boom"));
}

#[tokio::test]
async fn a_saved_session_that_cant_reach_zalo_is_retried_not_logged_out() {
    let (session, login, dir) = session_with(
        vec![
            LoginStep::cookie_error(dns_failure),
            LoginStep::cookie_error(rejected_session),
        ],
        true,
    );
    session.start();
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggingIn);
    assert!(s.retrying);
    assert!(s.resuming, "still resuming while it waits to retry");
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("no internet connection"),
        "{:?}",
        s.error
    );
    assert!(
        dir.join("credentials.json").exists(),
        "credentials are kept"
    );

    session.retry_now();
    settle().await;
    let s = session.get_status();
    assert_eq!(login.attempts(), 2);
    assert_eq!(s.state, SessionState::LoggedOut);
    assert!(!s.retrying);
    assert!(!s.resuming, "a rejected saved session is not resuming");
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("session has ended"),
        "{:?}",
        s.error
    );
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_qr_login_replaces_a_resume_that_is_waiting_to_retry() {
    let (session, _login, dir) = session_with(
        vec![
            LoginStep::cookie_error(dns_failure),
            // Shows a QR code and never finishes.
            LoginStep::qr(|events, _abort| {
                Box::pin(async move {
                    let _ = events.send(qr_generated("AAAA"));
                    std::future::pending().await
                })
            }),
        ],
        true,
    );
    session.start();
    settle().await;
    assert!(session.get_status().retrying);
    assert!(session.get_status().resuming);

    session.start_qr_login();
    // The replacement is visible at once, before the QR code arrives.
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggingIn);
    assert!(!s.resuming, "a QR login is not a resume");
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::AwaitingQrScan);
    assert!(!s.retrying);
    assert!(!s.resuming);
    assert_eq!(s.qr_image.as_deref(), Some("data:image/png;base64,AAAA"));
    let _ = fs::remove_dir_all(&dir);
}

// ---- `resuming`: a saved session reconnecting in the background ----

/// A cookie login that waits for the returned sender, then signs in as `api`.
fn cookie_login_held(api: Arc<FakeZaloApi>) -> (LoginStep, oneshot::Sender<()>) {
    let (release_tx, release_rx) = oneshot::channel::<()>();
    let release_rx = Mutex::new(Some(release_rx));
    let step = LoginStep::cookie(move |_| {
        let release = release_rx.lock().unwrap().take();
        let api = Arc::clone(&api);
        Box::pin(async move {
            if let Some(release) = release {
                let _ = release.await;
            }
            Ok(api as Arc<dyn ZaloApi>)
        })
    });
    (step, release_tx)
}

/// The `status` events broadcast so far, in order.
fn statuses(events: &mut broadcast::Receiver<ServerBroadcastEvent>) -> Vec<SessionStatus> {
    let mut out = Vec::new();
    while let Ok(ev) = events.try_recv() {
        let v: serde_json::Value = serde_json::from_str(&ev).unwrap();
        if v["type"] == "status" {
            out.push(serde_json::from_value(v["status"].clone()).unwrap());
        }
    }
    out
}

#[tokio::test]
async fn a_saved_session_resumes_in_the_background_and_says_so_until_it_is_logged_in() {
    let api = FakeZaloApi::new("me");
    let (step, release) = cookie_login_held(api.clone());
    let (session, _login, dir) = session_with(vec![step], true);
    let mut events = session.subscribe_events();
    assert!(!session.get_status().resuming, "nothing resumes before start");

    session.start();
    // No settle: a client asking right after `start` already sees the resume, never `logged_out`.
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggingIn);
    assert!(s.resuming);
    assert!(!s.retrying);
    assert!(s.qr_image.is_none());

    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggingIn);
    assert!(s.resuming, "still resuming while the login is in flight");

    release.send(()).unwrap();
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::Ready);
    assert!(!s.resuming);
    assert!(!s.retrying);
    assert_eq!(s.own_id.as_deref(), Some("me"));

    // Every status a client saw agrees: resuming exactly while `logging_in`, and the last one is
    // the settled one.
    let seen = statuses(&mut events);
    assert!(seen.len() >= 2, "{seen:?}");
    for st in &seen {
        assert_eq!(
            st.resuming,
            st.state == SessionState::LoggingIn,
            "inconsistent status {st:?}"
        );
    }
    assert!(seen.iter().any(|st| st.resuming));
    let last = seen.last().unwrap();
    assert_eq!(last.state, SessionState::Ready);
    assert!(!last.resuming);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn resuming_lasts_through_offline_retries_and_ends_when_the_session_signs_in() {
    let api = FakeZaloApi::new("me");
    let (session, login, dir) = session_with(
        vec![
            LoginStep::cookie_error(dns_failure),
            LoginStep::cookie_error(dns_failure),
            LoginStep::cookie_ok(api.clone()),
        ],
        true,
    );
    let mut events = session.subscribe_events();
    session.start();
    settle().await;
    let s = session.get_status();
    assert!(s.retrying && s.resuming, "{s:?}");

    session.retry_now();
    settle().await;
    let s = session.get_status();
    assert_eq!(login.attempts(), 2);
    assert!(s.retrying && s.resuming, "still offline: {s:?}");
    assert_eq!(s.state, SessionState::LoggingIn);

    session.retry_now();
    settle().await;
    let s = session.get_status();
    assert_eq!(login.attempts(), 3);
    assert_eq!(s.state, SessionState::Ready);
    assert!(!s.resuming);
    assert!(!s.retrying);

    // `resuming` held in every status between the first and the sign-in.
    let seen = statuses(&mut events);
    let ready_at = seen
        .iter()
        .position(|st| st.state == SessionState::Ready)
        .expect("a ready status");
    assert!(seen[..ready_at].iter().all(|st| st.resuming), "{seen:?}");
    assert!(seen[ready_at..].iter().all(|st| !st.resuming), "{seen:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_rejected_saved_session_stops_resuming_and_keeps_the_expiry_error() {
    let (session, _login, dir) = session_with(vec![LoginStep::cookie_error(rejected_session)], true);
    let mut events = session.subscribe_events();
    session.start();
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggedOut);
    assert!(!s.resuming);
    assert!(!s.retrying);
    assert_eq!(s.error.as_deref(), Some(SESSION_EXPIRED));
    assert_eq!(s.logged_out_reason.as_deref(), Some(LOGGED_OUT_EXPIRED));

    let seen = statuses(&mut events);
    let last = seen.last().expect("status events");
    assert_eq!(last.state, SessionState::LoggedOut);
    assert!(!last.resuming);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn logging_out_while_a_saved_session_resumes_stops_resuming() {
    let api = FakeZaloApi::new("me");
    let (step, _release) = cookie_login_held(api);
    let (session, _login, dir) = session_with(vec![step], true);
    let mut events = session.subscribe_events();
    session.start();
    settle().await;
    assert!(session.get_status().resuming);

    session.logout().await;
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggedOut);
    assert!(!s.resuming);
    assert_eq!(s.error, None);
    let seen = statuses(&mut events);
    let last = seen.last().expect("status events");
    assert_eq!(last.state, SessionState::LoggedOut);
    assert!(!last.resuming);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_qr_login_is_never_reported_as_resuming() {
    let (session, _login, dir) = session_with(vec![qr_showing_until_aborted("AAAA")], false);
    let mut events = session.subscribe_events();
    // No saved credentials: `start` has nothing to resume.
    session.start();
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggedOut);
    assert!(!s.resuming);

    session.start_qr_login();
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggingIn);
    assert!(!s.resuming, "the QR login's own logging_in is not a resume");
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::AwaitingQrScan);
    assert!(!s.resuming);

    let seen = statuses(&mut events);
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|st| !st.resuming), "{seen:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn declining_on_the_phone_says_so_instead_of_cancelled() {
    let (session, _login, dir) = session_with(
        vec![LoginStep::qr(|events, _abort| {
            Box::pin(async move {
                let _ = events.send(LoginQrEvent::QrCodeDeclined);
                Err(ProtoError::QrDeclined)
            })
        })],
        false,
    );
    session.start_qr_login();
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggedOut);
    assert!(
        s.error.as_deref().unwrap_or("").contains("declined"),
        "{:?}",
        s.error
    );
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn logging_out_during_a_qr_login_leaves_no_error_behind() {
    let (session, _login, dir) = session_with(vec![qr_showing_until_aborted("AAAA")], false);
    session.start_qr_login();
    settle().await;
    assert_eq!(session.get_status().state, SessionState::AwaitingQrScan);
    session.logout().await;
    settle().await;
    let s = session.get_status();
    assert_eq!(s.state, SessionState::LoggedOut);
    assert_eq!(s.error, None);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn logout_followed_immediately_by_start_qr_login_starts_a_fresh_login_and_is_not_clobbered_by_the_old_one(
) {
    let (reject_first, first_rejected) = oneshot::channel::<()>();
    let first_rejected = Mutex::new(Some(first_rejected));
    let second_abort_called = Arc::new(Mutex::new(false));
    let second_flag = Arc::clone(&second_abort_called);
    let (session, _login, dir) = session_with(
        vec![
            LoginStep::qr(move |events, _abort| {
                let rejected = first_rejected.lock().unwrap().take();
                Box::pin(async move {
                    let _ = events.send(qr_generated("FIRST"));
                    if let Some(rejected) = rejected {
                        let _ = rejected.await;
                    }
                    Err(ProtoError::QrAborted)
                })
            }),
            LoginStep::qr(move |events, abort_rx| {
                let flag = Arc::clone(&second_flag);
                Box::pin(async move {
                    let _ = events.send(qr_generated("SECOND"));
                    // Only an abort ends this login. The session also drops the login task right
                    // away, so notice the abort signal when this future is dropped.
                    struct NoticeAbort(oneshot::Receiver<()>, Arc<Mutex<bool>>);
                    impl Drop for NoticeAbort {
                        fn drop(&mut self) {
                            if self.0.try_recv().is_ok() {
                                *self.1.lock().unwrap() = true;
                            }
                        }
                    }
                    let mut guard = NoticeAbort(abort_rx, flag);
                    if (&mut guard.0).await.is_ok() {
                        *guard.1.lock().unwrap() = true;
                    }
                    Err(ProtoError::QrAborted)
                })
            }),
        ],
        false,
    );

    session.start_qr_login();
    settle().await;
    assert_eq!(
        session.get_status().qr_image.as_deref(),
        Some("data:image/png;base64,FIRST")
    );

    session.logout().await;
    // Immediately start a second QR login while the first is still unsettled.
    session.start_qr_login();
    settle().await;
    assert_eq!(
        session.get_status().qr_image.as_deref(),
        Some("data:image/png;base64,SECOND")
    );

    // Now the first login finishes rejecting; it must not clear the second login's abort handle or state.
    let _ = reject_first.send(());
    settle().await;
    assert_eq!(session.get_status().state, SessionState::AwaitingQrScan);
    assert_eq!(
        session.get_status().qr_image.as_deref(),
        Some("data:image/png;base64,SECOND")
    );

    session.logout().await;
    settle().await;
    assert!(*second_abort_called.lock().unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn got_login_info_writes_credentials_atomically_with_mode_0600() {
    let (session, _login, dir) = session_with(
        vec![LoginStep::qr(|events, _abort| {
            Box::pin(async move {
                let _ = events.send(LoginQrEvent::GotLoginInfo(test_credentials()));
                Err(ProtoError::QrAborted)
            })
        })],
        false,
    );
    session.start_qr_login();
    let cred_path = dir.join("credentials.json");
    for _ in 0..100 {
        if cred_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(cred_path.exists());
    assert!(!dir.join("credentials.json.tmp").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&cred_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_signed_in_profiles_name_and_picture_are_part_of_the_status() {
    let (session, _login, dir) = session_with(vec![], false);
    let api = FakeZaloApi::new("me");
    api.respond(
        "fetch_account_info",
        json!({ "profile": { "displayName": "Chi", "avatar": "https://a/me.jpg" } }),
    );
    let generation = session.inner.read().unwrap().generation;
    session.on_logged_in(api, generation).await;
    let s = session.get_status();
    assert_eq!(s.own_name.as_deref(), Some("Chi"));
    assert_eq!(s.own_avatar.as_deref(), Some("https://a/me.jpg"));
    assert_eq!(s.state, SessionState::Ready);
    let _ = fs::remove_dir_all(&dir);
}
