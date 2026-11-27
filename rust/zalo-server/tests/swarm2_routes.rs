//! Integration tests for Daily-Driver Swarm 2 HTTP routes and session logic.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;
use zalo_proto::ProtoError;

use zalo_server::config::Config;
use zalo_server::http::build_router;
use zalo_server::session::ZaloSession;
use zalo_server::store::{normalize_message, ChatStore};
use zalo_server::testing::{FakeLogin, FakeZaloApi};
use zalo_server::types::{ThreadInfo, ThreadKind};

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
            std::env::temp_dir().join(format!("zalo-swarm2-{:016x}", rand::random::<u64>()));
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

    async fn req(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1");
        let req_body = match body {
            Some(v) => {
                builder = builder.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = self
            .app
            .clone()
            .oneshot(builder.body(req_body).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    fn ingest(&self, kind: ThreadKind, thread_id: &str, data: Value) {
        let msg = normalize_message(&data, kind, thread_id, false, Some("me"));
        self.store.add_message(msg);
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn friend_alias_remove_friend_and_block_unblock_routes() {
    let t = TestApp::new();
    t.store.upsert_thread(ThreadInfo {
        id: "u42".into(),
        kind: ThreadKind::User,
        name: "Original Name".into(),
        avatar: "https://a/u42.jpg".into(),
    });
    let mut events = t.session.subscribe_events();

    // Validation: empty alias -> 400
    let (status, _) = t
        .req(Method::PUT, "/api/friends/u42/alias", Some(json!({ "alias": "   " })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // PUT /api/friends/{userId}/alias -> 204 + updates thread name + broadcasts threads
    let (status, _) = t
        .req(Method::PUT, "/api/friends/u42/alias", Some(json!({ "alias": "Bestie" })))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        t.store.get_thread(ThreadKind::User, "u42").unwrap().name,
        "Bestie"
    );
    let ev: Value = serde_json::from_str(&events.try_recv().unwrap()).unwrap();
    assert_eq!(ev["type"], "threads");

    // DELETE /api/friends/{userId}/alias -> 204 + refreshes profile name + broadcasts threads
    t.api.respond(
        "get_user_info",
        json!({
            "changed_profiles": {
                "u42_0": { "displayName": "Real Name", "avatar": "https://a/u42_new.jpg" }
            }
        }),
    );
    let (status, _) = t
        .req(Method::DELETE, "/api/friends/u42/alias", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        t.store.get_thread(ThreadKind::User, "u42").unwrap().name,
        "Real Name"
    );
    let ev: Value = serde_json::from_str(&events.try_recv().unwrap()).unwrap();
    assert_eq!(ev["type"], "threads");

    // DELETE /api/friends/{userId} -> 204 + broadcasts threads
    let (status, _) = t.req(Method::DELETE, "/api/friends/u42", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let ev: Value = serde_json::from_str(&events.try_recv().unwrap()).unwrap();
    assert_eq!(ev["type"], "threads");

    // POST /api/users/{userId}/block and DELETE /api/users/{userId}/block -> 204
    let (status, _) = t.req(Method::POST, "/api/users/u99/block", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = t.req(Method::DELETE, "/api/users/u99/block", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(
        t.api.methods(),
        vec![
            "change_friend_alias",
            "remove_friend_alias",
            "get_user_info",
            "remove_friend",
            "block_user",
            "unblock_user",
        ]
    );
    assert_eq!(t.api.calls_to("change_friend_alias")[0], vec![json!("Bestie"), json!("u42")]);
    assert_eq!(t.api.calls_to("remove_friend_alias")[0], vec![json!("u42")]);
    assert_eq!(t.api.calls_to("remove_friend")[0], vec![json!("u42")]);
    assert_eq!(t.api.calls_to("block_user")[0], vec![json!("u99")]);
    assert_eq!(t.api.calls_to("unblock_user")[0], vec![json!("u99")]);
}

#[tokio::test]
async fn group_links_join_deputies_board_and_notes_routes() {
    let t = TestApp::new();
    t.api.respond(
        "get_group_link_detail",
        json!({ "link": "https://zalo.me/g/abc", "enabled": 1, "expiration_date": 1800000000 }),
    );
    t.api.respond(
        "enable_group_link",
        json!({ "link": "https://zalo.me/g/new1", "enabled": 1, "expiration_date": 1900000000 }),
    );
    t.api.respond("join_group_link", json!(""));
    t.api.respond("get_all_groups", json!({ "gridVerMap": { "g_new": "1" } }));
    t.api.respond(
        "get_group_info",
        json!({
            "gridInfoMap": {
                "g_new": {
                    "name": "Joined Squad",
                    "avt": "https://a/g.jpg",
                    "memberIds": ["u1", "me"]
                }
            }
        }),
    );
    t.api.respond(
        "get_list_board",
        json!({
            "count": 3,
            "items": [
                {
                    "boardType": 1,
                    "data": {
                        "id": "note1",
                        "params": { "title": "Sprint goals" },
                        "creatorId": "u1",
                        "createTime": 1700000001,
                        "pinAct": 1
                    }
                },
                {
                    "boardType": 2,
                    "data": {
                        "id": "pin1",
                        "params": { "title": "Pinned announcement" },
                        "creatorId": "u2",
                        "createTime": 1700000002
                    }
                },
                {
                    "boardType": 3,
                    "data": {
                        "poll_id": 42,
                        "question": "Friday dinner?",
                        "creator": "u3",
                        "created_time": 1700000003
                    }
                }
            ]
        }),
    );
    t.api.respond(
        "create_note",
        json!({
            "id": "note2",
            "params": { "title": "Release checklist" },
            "creatorId": "me",
            "createTime": 1700000099
        }),
    );

    // GET /api/groups/{id}/link
    let (status, body) = t.req(Method::GET, "/api/groups/g1/link", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "link": "https://zalo.me/g/abc",
            "enabled": true,
            "expirationDate": 1800000000
        })
    );

    // POST /api/groups/{id}/link
    let (status, body) = t.req(Method::POST, "/api/groups/g1/link", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "link": "https://zalo.me/g/new1",
            "enabled": true,
            "expirationDate": 1900000000
        })
    );

    // DELETE /api/groups/{id}/link
    let (status, _) = t.req(Method::DELETE, "/api/groups/g1/link", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // POST /api/groups/join-link
    let mut events = t.session.subscribe_events();
    let (status, body) = t
        .req(
            Method::POST,
            "/api/groups/join-link",
            Some(json!({ "link": "https://zalo.me/g/joinme" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "groupId": "g_new" }));
    assert_eq!(
        t.store.get_thread(ThreadKind::Group, "g_new").unwrap().name,
        "Joined Squad"
    );
    let ev: Value = serde_json::from_str(&events.try_recv().unwrap()).unwrap();
    assert_eq!(ev["type"], "threads");

    // POST & DELETE /api/groups/{id}/deputies/{userId}
    let (status, _) = t
        .req(Method::POST, "/api/groups/g1/deputies/u5", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = t
        .req(Method::DELETE, "/api/groups/g1/deputies/u5", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(t.api.calls_to("add_group_deputy")[0], vec![json!("u5"), json!("g1")]);
    assert_eq!(t.api.calls_to("remove_group_deputy")[0], vec![json!("u5"), json!("g1")]);

    // GET /api/groups/{id}/board
    let (status, body) = t
        .req(Method::GET, "/api/groups/g1/board?page=1&count=20", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "items": [
                {
                    "id": "note1",
                    "kind": "note",
                    "title": "Sprint goals",
                    "creatorId": "u1",
                    "createTime": 1700000001,
                    "pinned": true
                },
                {
                    "id": "pin1",
                    "kind": "pinned_message",
                    "title": "Pinned announcement",
                    "creatorId": "u2",
                    "createTime": 1700000002,
                    "pinned": true
                },
                {
                    "id": "42",
                    "kind": "poll",
                    "title": "Friday dinner?",
                    "creatorId": "u3",
                    "createTime": 1700000003,
                    "pinned": false
                }
            ]
        })
    );

    // POST /api/groups/{id}/notes (validation + 201)
    let (status, _) = t
        .req(Method::POST, "/api/groups/g1/notes", Some(json!({ "title": "" })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = t
        .req(
            Method::POST,
            "/api/groups/g1/notes",
            Some(json!({ "title": "x".repeat(1001) })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body) = t
        .req(
            Method::POST,
            "/api/groups/g1/notes",
            Some(json!({ "title": "Release checklist", "pin": true })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        body,
        json!({
            "id": "note2",
            "kind": "note",
            "title": "Release checklist",
            "creatorId": "me",
            "createTime": 1700000099,
            "pinned": true
        })
    );
    assert_eq!(
        t.api.calls_to("create_note")[0],
        vec![json!("g1"), json!("Release checklist"), json!(true)]
    );
}

#[tokio::test]
async fn messaging_ttl_card_link_and_media_forward_routes() {
    let t = TestApp::new();
    t.api.respond(
        "send_message",
        json!({ "message": { "msgId": "501" }, "attachment": [] }),
    );
    t.api.respond("forward_message", json!({ "success": [], "fail": [] }));

    // 1. Disappearing message TTL
    let (status, body) = t
        .req(
            Method::POST,
            "/api/threads/user/u1/messages",
            Some(json!({ "text": "Self destruct", "ttl": 60_000 })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body, json!({ "msgId": "501" }));
    assert_eq!(t.api.calls_to("send_message")[0][0]["ttl"], json!(60_000));

    // Invalid TTL rejected with 400
    let (status, _) = t
        .req(
            Method::POST,
            "/api/threads/user/u1/messages",
            Some(json!({ "text": "Bad ttl", "ttl": -5 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 2. Send card
    let (status, _) = t
        .req(
            Method::POST,
            "/api/threads/group/g1/card",
            Some(json!({ "userId": "u7", "phone": "0901234567" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        t.api.calls_to("send_card")[0],
        vec![json!("u7"), json!("0901234567"), json!("g1"), json!("Group"), Value::Null]
    );

    // 3. Send link
    let (status, _) = t
        .req(
            Method::POST,
            "/api/threads/user/u1/link",
            Some(json!({ "link": "https://example.com", "message": "Read", "ttl": 300_000 })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        t.api.calls_to("send_link")[0],
        vec![
            json!("https://example.com"),
            json!("Read"),
            json!("u1"),
            json!("User"),
            json!(300_000)
        ]
    );

    // Invalid link rejected with 400
    let (status, _) = t
        .req(
            Method::POST,
            "/api/threads/user/u1/link",
            Some(json!({ "link": "file:///etc/passwd" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 4. Forward voice -> send_voice
    t.ingest(
        ThreadKind::User,
        "u1",
        json!({
            "msgId": "m_voice",
            "cliMsgId": "c_voice",
            "msgType": "chat.voice",
            "uidFrom": "u1",
            "idTo": "me",
            "ts": "1000",
            "content": { "href": "https://cdn.zalo.me/v.m4a" }
        }),
    );
    let (status, _) = t
        .req(
            Method::POST,
            "/api/threads/user/u1/messages/m_voice/forward",
            Some(json!({ "to": [{ "kind": "group", "id": "g2" }] })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        t.api.calls_to("send_voice")[0],
        vec![json!("https://cdn.zalo.me/v.m4a"), Value::Null, json!("g2"), json!("Group")]
    );

    // 5. Forward photo where send_link fails -> cleanly falls back to forward_message without 400
    t.api.fail("send_link", || ProtoError::Api {
        code: 500,
        message: "parselink unavailable".into(),
    });
    t.ingest(
        ThreadKind::User,
        "u1",
        json!({
            "msgId": "m_photo",
            "cliMsgId": "c_photo",
            "msgType": "chat.photo",
            "uidFrom": "u1",
            "idTo": "me",
            "ts": "2000",
            "content": { "href": "https://cdn.zalo.me/pic.jpg", "title": "Sunset" }
        }),
    );
    let (status, _) = t
        .req(
            Method::POST,
            "/api/threads/user/u1/messages/m_photo/forward",
            Some(json!({ "to": [{ "kind": "user", "id": "u2" }] })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let fw_calls = t.api.calls_to("forward_message");
    assert_eq!(fw_calls.len(), 1);
    assert_eq!(fw_calls[0][0]["message"], json!("Sunset\nhttps://cdn.zalo.me/pic.jpg"));
    assert_eq!(fw_calls[0][1], json!(["u2"]));
}

#[tokio::test]
async fn user_thread_info_includes_common_groups_from_cached_groups() {
    let t = TestApp::new();
    t.store.upsert_thread(ThreadInfo {
        id: "g_alpha".into(),
        kind: ThreadKind::Group,
        name: "Alpha Team".into(),
        avatar: "https://a/alpha.jpg".into(),
    });
    t.store.upsert_thread(ThreadInfo {
        id: "g_beta".into(),
        kind: ThreadKind::Group,
        name: "Beta Team".into(),
        avatar: "https://a/beta.jpg".into(),
    });
    t.store.upsert_thread(ThreadInfo {
        id: "g_other".into(),
        kind: ThreadKind::Group,
        name: "Other Team".into(),
        avatar: "".into(),
    });

    t.api.respond(
        "get_group_info",
        json!({
            "gridInfoMap": {
                "g_alpha": {
                    "currentMems": [
                        { "id": "u10", "dName": "Bob" },
                        { "id": "me", "dName": "Me" }
                    ]
                },
                "g_beta": {
                    "currentMems": [
                        { "id": "u10", "dName": "Bob" }
                    ]
                },
                "g_other": {
                    "currentMems": [
                        { "id": "u99", "dName": "Someone Else" }
                    ]
                }
            }
        }),
    );
    // Warm member caches via group_members (same as opening group info or load_groups)
    t.session.group_members("g_alpha").await.unwrap();
    t.session.group_members("g_beta").await.unwrap();
    t.session.group_members("g_other").await.unwrap();

    t.api.respond(
        "last_online",
        json!({
            "settings": { "show_online_status": 1 },
            "lastOnline": 1700001234
        }),
    );

    let (status, body) = t
        .req(Method::GET, "/api/threads/user/u10/info", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "lastOnline": 1700001234,
            "commonGroups": [
                {
                    "id": "g_alpha",
                    "kind": "group",
                    "name": "Alpha Team",
                    "avatar": "https://a/alpha.jpg"
                },
                {
                    "id": "g_beta",
                    "kind": "group",
                    "name": "Beta Team",
                    "avatar": "https://a/beta.jpg"
                }
            ]
        })
    );
}
