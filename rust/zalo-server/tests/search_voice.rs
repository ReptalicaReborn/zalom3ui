//! Ports of `test/search-voice.test.ts`, one Rust test per TS test, in the same order: message
//! search in the store and over HTTP, and `ZaloSession::send_voice` (with its HTTP upload routes)
//! against the fake Zalo client.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use zalo_server::config::Config;
use zalo_server::http::build_router;
use zalo_server::session::ZaloSession;
use zalo_server::store::{normalize_message, ChatStore};
use zalo_server::testing::{FakeLogin, FakeZaloApi};
use zalo_server::types::{StoredMessage, ThreadInfo, ThreadKind};

/// A stored message in thread `100` (user) or `g1` (group), like the TS `makeMsg`.
fn make_msg(id: &str, ts: i64, overrides: Value, group: bool) -> StoredMessage {
    let mut data = json!({
        "msgId": id,
        "cliMsgId": format!("c{id}"),
        "msgType": "webchat",
        "uidFrom": "100",
        "idTo": "me",
        "dName": "Alice",
        "ts": ts.to_string(),
        "content": "hello",
    });
    for (k, v) in overrides.as_object().unwrap() {
        data[k] = v.clone();
    }
    let (kind, thread) = if group {
        (ThreadKind::Group, "g1")
    } else {
        (ThreadKind::User, "100")
    };
    normalize_message(&data, kind, thread, false, Some("me"))
}

fn thread(id: &str, kind: ThreadKind, name: &str, avatar: &str) -> ThreadInfo {
    ThreadInfo {
        id: id.into(),
        kind,
        name: name.into(),
        avatar: avatar.into(),
    }
}

fn ids(hits: &[zalo_server::types::MessageSearchHit]) -> Vec<&str> {
    hits.iter().map(|h| h.message.id.as_str()).collect()
}

/// The full HTTP app over a session wired to a recording fake Zalo client.
struct TestApp {
    app: Router,
    session: Arc<ZaloSession>,
    store: ChatStore,
    api: Arc<FakeZaloApi>,
    dir: std::path::PathBuf,
}

impl TestApp {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("zalo-search-voice-{:016x}", rand::random::<u64>()));
        let store = ChatStore::in_memory();
        store.set_own_id(Some("me".into()));
        let session = Arc::new(ZaloSession::with_login(
            store.clone(),
            dir.clone(),
            true,
            FakeLogin::new(vec![]),
        ));
        let api = FakeZaloApi::new("me");
        session.testing_attach(api.clone());
        let config = Config {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: dir.clone(),
            access_token: None,
            read_receipts: true,
            log_file: None,
            public_dir: None,
        };
        let app = build_router(Arc::new(config), Arc::clone(&session));
        Self {
            app,
            session,
            store,
            api,
            dir,
        }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> (StatusCode, String) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1");
        if let Some(ct) = content_type {
            req = req.header("content-type", ct);
        }
        let resp = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn get(&self, path: &str) -> (StatusCode, String) {
        self.request("GET", path, None, Vec::new()).await
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn chat_store_search_messages_matches_diacritics_dj_case_attachments_skips_recalled_sorts_newest_first(
) {
    let store = ChatStore::in_memory();
    store.set_own_id(Some("me".into()));
    store.upsert_thread(thread("100", ThreadKind::User, "Minh", "https://a/100.jpg"));
    store.upsert_thread(thread(
        "g1",
        ThreadKind::Group,
        "Team Dev",
        "https://a/g1.jpg",
    ));

    store.add_message(make_msg(
        "1",
        1000,
        json!({ "content": "Đi uống Cà Phê Sữa Đá nhé!" }),
        false,
    ));
    store.add_message(make_msg(
        "2",
        2000,
        json!({ "content": "ĐẶT lịch họp tiếng Việt" }),
        true,
    ));
    store.add_message(make_msg(
        "3",
        3000,
        json!({ "content": "Cà phê sáng nay bị thu hồi" }),
        false,
    ));
    store.apply_undo("3");
    store.add_message(make_msg(
        "4",
        4000,
        json!({
            "msgType": "chat.recommended",
            "content": {
                "action": "recommened.link",
                "title": "Hướng dẫn Café Hà Nội",
                "description": "Quán đường Điện Biên Phủ rất ngon",
                "href": "https://example.com/cafe",
                "params": "{}",
            },
        }),
        true,
    ));
    store.add_message(make_msg(
        "5",
        5000,
        json!({
            "msgType": "share.file",
            "content": { "title": "Báo_cáo_Quý_4.pdf", "href": "https://example.com/q4.pdf", "params": "{}" },
        }),
        false,
    ));

    // Blank query returns empty
    assert!(store.search_messages("   ", None).is_empty());

    // Diacritic-insensitive + đ/Đ folding + skips recalled message #3
    let cafe = store.search_messages("ca phe sua da", None);
    assert_eq!(cafe.len(), 1);
    assert_eq!(cafe[0].message.id, "1");
    assert_eq!(cafe[0].thread_id, "100");
    assert_eq!(cafe[0].thread_kind, ThreadKind::User);
    assert_eq!(cafe[0].thread_name, "Minh");
    assert_eq!(cafe[0].thread_avatar, "https://a/100.jpg");

    // Recalled message is never matched (neither original text nor "Message recalled")
    assert!(store.search_messages("thu hoi", None).is_empty());
    assert!(store.search_messages("recalled", None).is_empty());

    // Uppercase Đ folded to d, case-insensitive, matches group message
    let viet = store.search_messages("dat lich hop TIENG VIET", None);
    assert_eq!(viet.len(), 1);
    assert_eq!(viet[0].message.id, "2");
    assert_eq!(viet[0].thread_kind, ThreadKind::Group);
    assert_eq!(viet[0].thread_name, "Team Dev");

    // Attachment title (Latin diacritics) & description (Vietnamese đ/Đ)
    assert_eq!(ids(&store.search_messages("cafe ha noi", None)), vec!["4"]);
    assert_eq!(
        ids(&store.search_messages("dien bien phu", None)),
        vec!["4"]
    );
    assert_eq!(
        ids(&store.search_messages("bao_cao_quy_4", None)),
        vec!["5"]
    );

    // Broad match across threads is sorted newest-first
    store.add_message(make_msg(
        "6",
        1500,
        json!({ "content": "duong pho" }),
        false,
    ));
    store.add_message(make_msg("7", 6000, json!({ "content": "Đường lớn" }), true));
    assert_eq!(
        ids(&store.search_messages("duong", None)),
        vec!["7", "4", "6"]
    );
}

#[test]
fn chat_store_search_messages_respects_limit_and_clamps_to_1_100() {
    let store = ChatStore::in_memory();
    for i in 1..=120i64 {
        store.add_message(make_msg(
            &i.to_string(),
            i * 1000,
            json!({ "content": format!("item {i}") }),
            false,
        ));
    }
    assert_eq!(store.search_messages("item", None).len(), 50);
    assert_eq!(store.search_messages("item", Some(5)).len(), 5);
    assert_eq!(
        ids(&store.search_messages("item", Some(3))),
        vec!["120", "119", "118"]
    );
    assert_eq!(store.search_messages("item", Some(0)).len(), 1);
    assert_eq!(store.search_messages("item", Some(500)).len(), 100);
    // The TS test also passes -10; a negative limit cannot be expressed as a usize here, and the
    // HTTP layer rejects it (see the /api/search test).
}

#[tokio::test]
async fn get_api_search_validates_q_and_limit_and_returns_search_results() {
    let app = TestApp::new();
    app.store
        .upsert_thread(thread("100", ThreadKind::User, "Lan", "https://a/lan.jpg"));
    app.store.add_message(make_msg(
        "1",
        1000,
        json!({ "content": "Chào buổi sáng" }),
        false,
    ));
    app.store.add_message(make_msg(
        "2",
        2000,
        json!({ "content": "Chào buổi tối" }),
        false,
    ));

    assert_eq!(app.get("/api/search").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        app.get("/api/search?q=%20%20").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get("/api/search?q=chao&limit=0").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get("/api/search?q=chao&limit=abc").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get("/api/search?q=chao&limit=-3").await.0,
        StatusCode::BAD_REQUEST
    );

    let (status, body) = app.get("/api/search?q=chao%20buoi&limit=1").await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_str(&body).unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["threadId"], "100");
    assert_eq!(results[0]["threadName"], "Lan");
    assert_eq!(results[0]["message"]["id"], "2");
}

fn uploaded_voice() -> Value {
    json!([{
        "fileType": "others",
        "fileUrl": "https://cdn.zaloapp.com/voice/abc.m4a",
        "fileId": "f1",
        "fileName": "abc.m4a",
        "totalSize": 6,
        "checksum": "",
        "clientFileId": 1,
    }])
}

#[tokio::test]
async fn zalo_session_send_voice_uploads_audio_normalizes_extension_validates_file_url_and_calls_send_voice(
) {
    let app = TestApp::new();
    app.api.respond("upload_attachment", uploaded_voice());
    app.api.respond("send_voice", json!({ "msgId": "900" }));
    let audio = vec![0x00u8, 0x01, 0x02, 0x0d, 0x0a, 0xff];
    let bytes = json!([0, 1, 2, 13, 10, 255]);
    let voice_url = "https://cdn.zaloapp.com/voice/abc.m4a";

    // Filename without audio extension gets .m4a appended
    app.session
        .send_voice(ThreadKind::User, "100", audio.clone(), "recording")
        .await
        .unwrap();
    let calls = app.api.calls();
    assert_eq!(calls[0].method, "upload_attachment");
    assert_eq!(
        calls[0].args,
        vec![
            json!([{ "filename": "recording.m4a", "data": bytes, "width": null, "height": null }]),
            json!("100"),
            json!("User"),
        ]
    );
    assert_eq!(calls[1].method, "send_voice");
    assert_eq!(
        calls[1].args,
        vec![json!(voice_url), json!(null), json!("100"), json!("User")]
    );

    // Existing audio extension is preserved for group threads
    app.session
        .send_voice(ThreadKind::Group, "g1", audio.clone(), "clip.aac")
        .await
        .unwrap();
    let calls = app.api.calls();
    assert_eq!(
        calls[2].args,
        vec![
            json!([{ "filename": "clip.aac", "data": bytes, "width": null, "height": null }]),
            json!("g1"),
            json!("Group"),
        ]
    );
    assert_eq!(
        calls[3].args,
        vec![json!(voice_url), json!(null), json!("g1"), json!("Group")]
    );

    // Empty buffer rejected before calling API
    let before = app.api.call_count();
    let err = app
        .session
        .send_voice(ThreadKind::User, "100", Vec::new(), "v.m4a")
        .await
        .unwrap_err();
    assert!(err.message.contains("empty"), "{}", err.message);
    assert_eq!(app.api.call_count(), before);

    // HTTP: missing body -> 400
    let (status, _) = app
        .request("POST", "/api/threads/user/100/voice", None, Vec::new())
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // HTTP: raw binary body upload -> 201
    let (status, _) = app
        .request(
            "POST",
            "/api/threads/user/100/voice?name=memo.m4a",
            Some("audio/mp4"),
            audio.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let uploads = app.api.calls_to("upload_attachment");
    assert_eq!(uploads.len(), 3);
    assert_eq!(uploads[2][0][0]["filename"], "memo.m4a");
    assert_eq!(uploads[2][0][0]["data"], bytes);

    // HTTP: multipart/form-data upload with field "voice" -> 201 with exact binary bytes
    let boundary = "----ZaloVoiceBoundary1234";
    let mut multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"voice\"; filename=\"note.m4a\"\r\nContent-Type: audio/mp4\r\n\r\n"
    )
    .into_bytes();
    multipart.extend_from_slice(&audio);
    multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let (status, _) = app
        .request(
            "POST",
            "/api/threads/group/g1/voice",
            Some(&format!("multipart/form-data; boundary={boundary}")),
            multipart,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let uploads = app.api.calls_to("upload_attachment");
    assert_eq!(uploads.len(), 4);
    assert_eq!(uploads[3][0][0]["filename"], "note.m4a");
    assert_eq!(uploads[3][0][0]["data"], bytes);
    assert_eq!(uploads[3][2], "Group");

    // HTTP: multipart/form-data missing "voice" field -> 400
    let wrong_field = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"other\"; filename=\"a.m4a\"\r\n\r\nabc\r\n--{boundary}--\r\n"
    );
    let (status, _) = app
        .request(
            "POST",
            "/api/threads/user/100/voice",
            Some(&format!("multipart/form-data; boundary={boundary}")),
            wrong_field.into_bytes(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn zalo_session_send_voice_fails_with_502_if_upload_attachment_returns_no_file_url() {
    let app = TestApp::new();
    // Zalo "uploaded" the file but handed back no URL.
    app.api.respond(
        "upload_attachment",
        json!([{
            "fileType": "others", "fileUrl": "", "fileId": "", "fileName": "v.m4a",
            "totalSize": 3, "checksum": "", "clientFileId": 1,
        }]),
    );
    let err = app
        .session
        .send_voice(ThreadKind::User, "100", vec![1, 2, 3], "v.m4a")
        .await
        .unwrap_err();
    assert_eq!(err.status, 502);
    assert!(
        err.message.contains("Failed to upload voice recording"),
        "{}",
        err.message
    );
    assert_eq!(app.api.methods(), vec!["upload_attachment"]);
}
