//! Ports of `test/session.test.ts`, one Rust test per TS test, in the same order. The TS tests
//! call session internals (`ingest`, `onSeen`, ...); here the same events go through the
//! listener's event handler (`testing_dispatch`).

use serde_json::json;

use zalo_proto::{RawWsMessage, ThreadType, WsEvent};

use super::test_support::{incoming, of_type, rig, rig_with};
use super::*;
use crate::testing::{settle, FakeLogin, FakeZaloApi};
use crate::types::{MessageStatus, ReactionInfo};

#[tokio::test]
async fn replying_with_a_quote_sends_the_quoted_messages_raw_fields() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;

    let msg_id = r
        .session
        .send_text(ThreadKind::User, "100", "sure", Some("1"), None)
        .await
        .unwrap();
    assert_eq!(msg_id.as_deref(), Some("999"));
    let calls = r.api.calls();
    assert_eq!(calls[0].method, "send_message");
    let [content, thread_id, thread_type] = &calls[0].args[..] else {
        panic!("unexpected args {:?}", calls[0].args)
    };
    assert_eq!(thread_id, "100");
    assert_eq!(thread_type, "User");
    assert_eq!(content["msg"], "sure");
    assert_eq!(
        content["quote"],
        json!({
            "content": "text 1",
            "msg_type": "webchat",
            "property_ext": null,
            "uid_from": "100",
            "msg_id": "1",
            "cli_msg_id": "c1",
            "ts": "1000",
            "ttl": 0,
        })
    );
    assert_eq!(
        content["mentions"],
        json!([]),
        "mentions are only sent to groups"
    );
}

#[tokio::test]
async fn quoting_a_message_from_another_thread_is_rejected() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    let err = r
        .session
        .send_text(ThreadKind::User, "other", "x", Some("1"), None)
        .await
        .unwrap_err();
    assert!(err.message.contains("not found"), "{}", err.message);
}

#[tokio::test]
async fn reacting_calls_add_reaction_and_updates_the_message_immediately() {
    let mut r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.drain_events();

    r.session
        .set_reaction(ThreadKind::User, "100", "1", Some("heart"))
        .await
        .unwrap();
    let calls = r.api.calls();
    assert_eq!(calls[0].method, "add_reaction");
    assert_eq!(calls[0].args[0], "/-heart");
    assert_eq!(
        calls[0].args[1],
        json!({ "thread_id": "100", "thread_type": "User", "msg_id": "1", "cli_msg_id": "c1" })
    );
    let events = r.drain_events();
    let updated = of_type(&events, "message_updated");
    assert_eq!(
        updated.last().unwrap()["message"]["reactions"],
        json!([{ "key": "heart", "emoji": "❤️", "count": 1, "mine": true }])
    );

    r.session
        .set_reaction(ThreadKind::User, "100", "1", None)
        .await
        .unwrap();
    assert_eq!(r.api.calls()[1].args[0], "", "null removes the reaction");
    let err = r
        .session
        .set_reaction(ThreadKind::User, "100", "1", Some("dance"))
        .await
        .unwrap_err();
    assert!(err.message.contains("Unknown reaction"), "{}", err.message);
}

#[tokio::test]
async fn recall_is_limited_to_own_messages() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.ingest(incoming("2", 2000, json!({ "uidFrom": "me" }), false))
        .await;

    let err = r
        .session
        .recall_message(ThreadKind::User, "100", "1")
        .await
        .unwrap_err();
    assert!(err.message.contains("own messages"), "{}", err.message);
    r.session
        .recall_message(ThreadKind::User, "100", "2")
        .await
        .unwrap();
    let last = r.api.calls().pop().unwrap();
    assert_eq!(last.method, "undo");
    assert_eq!(
        last.args,
        vec![
            json!({ "msg_id": "2", "cli_msg_id": "c2" }),
            json!("100"),
            json!("User")
        ]
    );
    assert!(r.store.get_message("2").unwrap().recalled);
}

#[tokio::test]
async fn incoming_recall_records_and_undo_events_mark_messages_recalled() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.ingest(incoming("2", 2000, json!({}), false)).await;
    r.ingest(incoming(
        "u1",
        3000,
        json!({ "content": { "deleteMsg": 1, "globalMsgId": 1, "cliMsgId": 1 } }),
        false,
    ))
    .await;
    assert!(r.store.get_message("1").unwrap().recalled);
    assert!(
        r.store.get_message("u1").is_none(),
        "the recall record itself is not shown"
    );

    r.session
        .testing_dispatch(WsEvent::Undo(RawWsMessage {
            thread_type: ThreadType::User,
            thread_id: "100".into(),
            is_self: false,
            data: json!({ "content": { "globalMsgId": 2 } }),
        }))
        .await;
    assert!(r.store.get_message("2").unwrap().recalled);
}

#[tokio::test]
async fn delete_for_me_calls_delete_message_with_only_me_and_drops_the_message() {
    let mut r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.drain_events();
    r.session
        .delete_message(ThreadKind::User, "100", "1")
        .await
        .unwrap();
    assert_eq!(
        r.api.calls()[0].args,
        vec![
            json!({ "thread_id": "100", "thread_type": "User", "msg_id": "1", "cli_msg_id": "c1", "uid_from": "100" }),
            json!(true),
        ]
    );
    assert_eq!(r.api.calls()[0].method, "delete_message");
    assert!(r.store.get_message("1").is_none());
    let events = r.drain_events();
    assert_eq!(
        events.last().unwrap(),
        &json!({ "type": "message_deleted", "threadKind": "user", "threadId": "100", "id": "1" })
    );
}

#[tokio::test]
async fn delete_for_me_is_tombstoned_and_replay_never_resurrects_or_recalls_it() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.session
        .delete_message(ThreadKind::User, "100", "1")
        .await
        .unwrap();
    assert!(r.store.get_message("1").is_none());
    assert!(r.store.is_deleted_for_me(ThreadKind::User, "100", "1"));
    // history replay of the original message
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    assert!(r.store.get_message("1").is_none(), "no resurrection");
    // a late echo naming the message, and one naming nothing: neither makes a placeholder
    r.ingest(incoming(
        "u1",
        3000,
        json!({ "content": { "deleteMsg": 1, "globalMsgId": 1 } }),
        false,
    ))
    .await;
    r.ingest(incoming("u2", 3100, json!({ "content": { "deleteMsg": 1 } }), false))
        .await;
    assert!(r.store.get_message("1").is_none());
    assert!(r.store.get_message("u1").is_none());
    assert!(r.store.get_message("u2").is_none(), "no stray recalled bubble");
    assert!(r.store.get_messages(ThreadKind::User, "100", None, None).is_empty());
}

#[tokio::test]
async fn recalled_message_can_be_deleted_even_if_zalo_rejects_it() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.ingest(incoming(
        "u1",
        2000,
        json!({ "content": { "deleteMsg": 1, "globalMsgId": 1 } }),
        false,
    ))
    .await;
    assert!(r.store.get_message("1").unwrap().recalled);
    r.api
        .fail("delete_message", || zalo_proto::ProtoError::InvalidArg("rejected".into()));
    r.session
        .delete_message(ThreadKind::User, "100", "1")
        .await
        .unwrap();
    assert!(r.store.get_message("1").is_none());
    assert!(r.store.is_deleted_for_me(ThreadKind::User, "100", "1"));
}

#[tokio::test]
async fn forward_splits_targets_by_thread_type_and_forwards_media() {
    let r = rig();
    r.api
        .respond("forward_message", json!({ "success": [], "fail": [] }));
    r.api.respond("send_link", json!({ "msgId": 88 }));
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.ingest(incoming(
        "2",
        2000,
        json!({ "msgType": "chat.photo", "content": { "href": "https://x.example/p.jpg" } }),
        false,
    ))
    .await;
    r.ingest(incoming("3", 3000, json!({}), false)).await;
    r.store.apply_undo("3");

    r.session
        .forward_message(
            ThreadKind::User,
            "100",
            "1",
            &[
                (ThreadKind::User, "a".to_string()),
                (ThreadKind::Group, "g".to_string()),
                (ThreadKind::User, "b".to_string()),
            ],
        )
        .await
        .unwrap();
    let summary: Vec<_> = r
        .api
        .calls()
        .iter()
        .map(|c| (c.method, c.args[1].clone(), c.args[2].clone()))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("forward_message", json!(["a", "b"]), json!("User")),
            ("forward_message", json!(["g"]), json!("Group")),
        ]
    );

    // Photo message now forwards cleanly via send_link (instead of rejecting with 400).
    r.api.clear_calls();
    r.session
        .forward_message(
            ThreadKind::User,
            "100",
            "2",
            &[(ThreadKind::User, "a".to_string())],
        )
        .await
        .unwrap();
    let media_calls = r.api.calls();
    assert_eq!(media_calls.len(), 1);
    assert_eq!(media_calls[0].method, "send_link");
    assert_eq!(media_calls[0].args[0], "https://x.example/p.jpg");
    assert_eq!(media_calls[0].args[2], "a");

    // Recalled messages are still rejected with 400.
    let err = r
        .session
        .forward_message(
            ThreadKind::User,
            "100",
            "3",
            &[(ThreadKind::User, "a".to_string())],
        )
        .await
        .unwrap_err();
    assert_eq!(err.status, 400);
    assert!(err.message.contains("recalled"), "{}", err.message);
}

#[tokio::test]
async fn marking_read_sends_seen_receipts_for_unread_incoming_messages() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.ingest(incoming("2", 2000, json!({}), false)).await;
    r.session.mark_read(ThreadKind::User, "100").await.unwrap();

    let calls = r.api.calls();
    assert_eq!(calls[0].method, "send_seen_event");
    let params = calls[0].args[0].as_array().unwrap();
    assert_eq!(calls[0].args[1], "User");
    let ids: Vec<_> = params.iter().map(|p| p["msg_id"].clone()).collect();
    assert_eq!(ids, vec![json!("1"), json!("2")]);
    // `ts` is a number here (the TS client takes the string); the rest is identical.
    assert_eq!(
        params[0],
        json!({
            "msg_id": "1",
            "cli_msg_id": "c1",
            "uid_from": "100",
            "id_to": "me",
            "msg_type": "webchat",
            "st": 3,
            "at": 5,
            "cmd": 501,
            "ts": 1000,
        })
    );
    assert_eq!(r.store.list_threads()[0].unread, 0);

    r.session.mark_read(ThreadKind::User, "100").await.unwrap();
    assert_eq!(r.api.call_count(), 1, "nothing new to report");
}

#[tokio::test]
async fn read_receipts_can_be_turned_off() {
    let r = rig_with(false);
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.session.mark_read(ThreadKind::User, "100").await.unwrap();
    assert_eq!(r.api.call_count(), 0);
}

#[tokio::test]
async fn group_mentions_are_passed_through_reactions_and_receipts_from_the_listener_apply() {
    let r = rig();
    r.ingest(incoming("m1", 1000, json!({ "uidFrom": "me" }), true))
        .await;

    r.session
        .send_text(
            ThreadKind::Group,
            "g1",
            "@Bob hi",
            None,
            Some(vec![MentionInfo {
                uid: "200".into(),
                pos: 0,
                len: 4,
            }]),
        )
        .await
        .unwrap();
    assert_eq!(
        r.api.calls()[0].args[0]["mentions"],
        json!([{ "pos": 0, "uid": "200", "len": 4 }])
    );

    r.session
        .testing_dispatch(WsEvent::Reactions {
            thread_type: ThreadType::Group,
            reactions: vec![json!({
                "data": { "uidFrom": "200", "content": { "rMsg": [{ "gMsgID": "m1" }], "rIcon": ":>" } }
            })],
            is_old: false,
        })
        .await;
    r.session
        .testing_dispatch(WsEvent::Seen {
            thread_type: ThreadType::Group,
            data: json!([{ "msgId": "m1", "groupId": "g1", "isSelf": false, "seenUids": ["200", "300"] }]),
        })
        .await;

    let m = r
        .store
        .get_messages(ThreadKind::Group, "g1", None, None)
        .remove(0);
    assert_eq!(
        m.reactions,
        vec![ReactionInfo {
            key: "haha".into(),
            emoji: "😆".into(),
            count: 1,
            mine: false
        }]
    );
    assert_eq!(m.status, Some(MessageStatus::Seen));
    assert_eq!(m.seen_count, Some(2));
}

#[tokio::test]
async fn seen_on_another_device_clears_the_groups_unread_count() {
    let r = rig();
    r.ingest(incoming("m1", 1000, json!({}), true)).await;
    r.ingest(incoming("m2", 2000, json!({}), true)).await;
    assert_eq!(r.store.list_threads()[0].unread, 2);

    r.session
        .testing_dispatch(WsEvent::Seen {
            thread_type: ThreadType::Group,
            data: json!([{ "msgId": "m2", "groupId": "g1", "isSelf": true, "seenUids": ["me"] }]),
        })
        .await;
    assert_eq!(r.store.list_threads()[0].unread, 0);
}

#[tokio::test]
async fn pin_mute_and_mark_unread_update_zalo_and_the_local_summary() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), false)).await;
    r.session.mark_read(ThreadKind::User, "100").await.unwrap();
    r.api.clear_calls();

    r.session
        .set_pinned(ThreadKind::User, "100", true)
        .await
        .unwrap();
    r.session
        .set_muted(ThreadKind::User, "100", Some(MuteOption::Forever))
        .await
        .unwrap();
    r.session
        .mark_unread(ThreadKind::User, "100")
        .await
        .unwrap();
    assert_eq!(
        r.api.methods(),
        vec!["set_pinned_conversations", "set_mute", "add_unread_mark"]
    );
    let t = r.store.list_threads().remove(0);
    assert!(t.pinned);
    assert_eq!(t.muted_until, Some(-1));
    assert_eq!(t.unread, 1);

    r.session
        .set_muted(ThreadKind::User, "100", None)
        .await
        .unwrap();
    assert_eq!(r.store.list_threads()[0].muted_until, None);
}

#[tokio::test]
async fn typing_events_resolve_the_typists_name() {
    let mut r = rig();
    r.store.upsert_thread(ThreadInfo {
        id: "100".into(),
        kind: ThreadKind::User,
        name: "Alice Nguyen".into(),
        avatar: String::new(),
    });
    r.drain_events();
    r.session
        .testing_dispatch(WsEvent::Typing {
            thread_type: ThreadType::User,
            thread_id: "100".into(),
            uid: "100".into(),
            is_pc: false,
        })
        .await;
    let events = r.drain_events();
    assert_eq!(
        events.last().unwrap(),
        &json!({
            "type": "typing",
            "threadKind": "user",
            "threadId": "100",
            "uid": "100",
            "name": "Alice Nguyen",
        })
    );
}

#[test]
fn normalize_message_output_for_a_self_message_from_another_device() {
    let raw = incoming("1", 1000, json!({ "uidFrom": "me" }), false);
    let m = normalize_message(&raw.data, ThreadKind::User, "100", raw.is_self, Some("me"));
    assert!(m.is_self);
}

#[tokio::test]
async fn on_seen_guards_missing_seen_uids_in_groups_and_handles_is_self_in_1to1_chats() {
    let mut r = rig();
    r.ingest(incoming("g1m", 1000, json!({ "uidFrom": "me" }), true))
        .await;
    // No seenUids: must not panic or change anything.
    r.session
        .testing_dispatch(WsEvent::Seen {
            thread_type: ThreadType::Group,
            data: json!([{ "msgId": "g1m", "groupId": "g1", "isSelf": false }]),
        })
        .await;

    // 1:1 chat: self seen from another device clears unread instead of marking our own msg as seen by peer.
    r.ingest(incoming("u1", 1000, json!({}), false)).await;
    r.ingest(incoming("u2", 2000, json!({ "uidFrom": "me" }), false))
        .await;
    r.ingest(incoming("u3", 3000, json!({}), false)).await;
    let unread = |r: &super::test_support::Rig| {
        r.store
            .list_threads()
            .into_iter()
            .find(|t| t.id == "100")
            .map(|t| t.unread)
    };
    assert_eq!(unread(&r), Some(1));
    r.drain_events();
    r.session
        .testing_dispatch(WsEvent::Seen {
            thread_type: ThreadType::User,
            data: json!([{ "msgId": "u3", "idTo": "100", "isSelf": true }]),
        })
        .await;
    assert_eq!(unread(&r), Some(0));
    let u2 = r
        .store
        .get_messages(ThreadKind::User, "100", None, None)
        .into_iter()
        .find(|m| m.id == "u2")
        .unwrap();
    assert_eq!(u2.status, Some(MessageStatus::Sent));
    assert!(!of_type(&r.drain_events(), "threads").is_empty());
}

#[tokio::test]
async fn logout_clears_all_in_memory_caches_call_trace_own_id_and_flushes_store() {
    let dir = std::env::temp_dir().join(format!("zalo-logout-test-{:016x}", rand::random::<u64>()));
    let store = ChatStore::open(&dir);
    let session = Arc::new(ZaloSession::with_login(
        store.clone(),
        dir.clone(),
        true,
        FakeLogin::new(vec![]),
    ));
    let api = FakeZaloApi::new("me");
    api.respond(
        "get_all_friends",
        json!([{ "userId": "100", "displayName": "Alice", "zaloName": "Alice", "avatar": "" }]),
    );
    api.respond("get_stickers", json!([1]));
    api.respond(
        "get_stickers_detail",
        json!([{ "id": 1, "cateId": 1, "type": 1 }]),
    );
    session.testing_attach(api.clone());

    // Populate every cache: friends, stickers, presence, call trace, messages.
    session.accept_friend_request("100").await.unwrap();
    session.search_stickers("x").await.unwrap();
    session.set_presence(Some(ThreadKind::User), Some("100"));
    session
        .call_trace()
        .record(&json!({ "kind": "control", "source": "poll", "act": "cancel" }));
    let raw = incoming("1", now_ms(), json!({}), false);
    let msg = normalize_message(&raw.data, ThreadKind::User, "100", false, Some("me"));
    store.add_message(msg.clone());
    store.flush();
    assert!(!session.call_trace().read().is_empty());
    assert_eq!(session.friends().len(), 1);
    assert!(
        !session.should_notify(&store.to_public(&msg), false),
        "viewing this chat"
    );

    session.logout().await;
    assert_eq!(session.friends().len(), 0);
    assert_eq!(session.call_trace().read(), "");
    assert_eq!(store.own_id(), None);
    assert!(store.list_threads().is_empty());
    assert!(
        session.should_notify(&store.to_public(&msg), false),
        "presence was cleared"
    );
    // The sticker cache is gone too: signing in again has to look the sticker up.
    let again = FakeZaloApi::new("me");
    again.respond(
        "get_stickers_detail",
        json!([{ "id": 1, "cateId": 1, "type": 1 }]),
    );
    session.testing_attach(again.clone());
    session
        .send_sticker(ThreadKind::User, "100", 1)
        .await
        .unwrap();
    assert_eq!(again.calls_to("get_stickers_detail").len(), 1);
    // Flushed synchronously to disk as empty.
    assert!(ChatStore::open(&dir).list_threads().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn sticker_cache_is_capped_at_500_entries_and_evicts_oldest() {
    let r = rig();
    // Keyword "kN" finds stickers 30(N-1)+1 ..= 30N; details answer for whatever is asked.
    r.api.on("get_stickers", |args| {
        let n: i64 = args[0].as_str().unwrap()[1..].parse().unwrap();
        Ok(json!(((n - 1) * 30 + 1..=n * 30).collect::<Vec<i64>>()))
    });
    r.api.on("get_stickers_detail", |args| {
        let ids = args[0].as_array().unwrap();
        Ok(json!(ids
            .iter()
            .map(|id| json!({ "id": id, "cateId": 1, "type": 1 }))
            .collect::<Vec<_>>()))
    });
    for n in 1..=17 {
        r.session.search_stickers(&format!("k{n}")).await.unwrap(); // 510 stickers
    }
    r.api.clear_calls();

    // 11 and 510 are still cached (no lookup); 1 and 10 were evicted (looked up again).
    for id in [510, 11] {
        r.session
            .send_sticker(ThreadKind::User, "100", id)
            .await
            .unwrap();
    }
    assert!(r.api.calls_to("get_stickers_detail").is_empty());
    for id in [1, 10] {
        r.session
            .send_sticker(ThreadKind::User, "100", id)
            .await
            .unwrap();
    }
    assert_eq!(
        r.api.calls_to("get_stickers_detail"),
        vec![vec![json!([1])], vec![json!([10])]]
    );
}

#[tokio::test]
async fn stale_on_logged_in_and_load_friends_drop_results_after_logout() {
    let dir = std::env::temp_dir().join(format!("zalo-stale-test-{:016x}", rand::random::<u64>()));
    let store = ChatStore::in_memory();
    let session = Arc::new(ZaloSession::with_login(
        store.clone(),
        dir.clone(),
        true,
        FakeLogin::new(vec![]),
    ));
    let api = FakeZaloApi::new("me");
    let (resolve_account, account) = tokio::sync::oneshot::channel::<Value>();
    let account = std::sync::Mutex::new(Some(account));
    api.on_async("fetch_account_info", move |_| {
        let rx = account.lock().unwrap().take();
        Box::pin(async move {
            match rx {
                Some(rx) => Ok(rx.await.unwrap_or(Value::Null)),
                None => Ok(Value::Null),
            }
        })
    });
    api.respond(
        "get_all_friends",
        json!([{ "userId": "100", "displayName": "Alice", "zaloName": "Alice", "avatar": "" }]),
    );
    api.respond("get_all_groups", json!({ "gridVerMap": {} }));
    api.respond("get_pin_conversations", json!({ "conversations": [] }));
    api.respond(
        "get_mute",
        json!({ "chatEntries": [], "groupChatEntries": [] }),
    );

    let generation = session.inner.read().unwrap().generation;
    let login = {
        let session = Arc::clone(&session);
        let api: Arc<dyn ZaloApi> = api.clone();
        tokio::spawn(async move { session.on_logged_in(api, generation).await })
    };
    settle().await; // on_logged_in is now waiting for the account info
    session.logout().await;
    let _ = resolve_account.send(json!({ "profile": { "displayName": "Me" } }));
    login.await.unwrap();
    settle().await;

    assert_eq!(api.listener_starts(), 0);
    assert_eq!(session.get_status().state, SessionState::LoggedOut);
    assert!(store.list_threads().is_empty());

    // A friends load that started before the logout drops its result as well.
    let stale: Arc<dyn ZaloApi> = api.clone();
    session.load_friends(&stale, generation).await.unwrap();
    assert!(store.list_threads().is_empty());
    assert!(session.friends().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn frame_events_feed_the_socket_log_and_history_page_msgs_are_stored_silently() {
    let mut r = rig();
    let mut report = zalo_proto::framelog::FrameReport::new(1, 501, 3);
    report.payload_keys = vec!["data".into()];
    r.session.testing_dispatch(WsEvent::Frame(report)).await;
    let text = r.session.socket_log().render_text();
    assert!(text.contains("cmd=501 sub=3"), "{text}");
    assert!(text.contains("handled"), "{text}");

    r.drain_events();
    r.session
        .testing_dispatch(WsEvent::OldPageMessages(vec![incoming("77", 1000, json!({}), false)]))
        .await;
    assert!(r.store.get_message("77").is_some());
    assert!(
        r.drain_events().iter().all(|e| e["notify"] != json!(true)),
        "history never notifies"
    );
}

// ---- delete-for-me sync records (self-sent echo of "Delete for me"; shape inferred) ----

fn sync_shapes() -> Vec<(&'static str, Value)> {
    vec![
        ("chat.delete", json!({ "globalMsgId": 1 })),
        ("chat.undo", json!({ "globalMsgId": "1", "cliMsgId": "c1" })),
        ("chat.delete", json!({ "msgs": [{ "globalMsgId": 1, "cliMsgId": "c1" }], "onlyMe": 1 })),
        // by client id only
        ("chat.deleted", json!({ "cliMsgId": "c1" })),
        // untyped (webchat) reference-only content
        ("webchat", json!({ "msgs": [{ "globalMsgId": 1, "cliMsgId": "c1" }] })),
    ]
}

#[tokio::test]
async fn delete_sync_record_removes_target_in_user_and_group_threads_without_a_bubble() {
    for group in [false, true] {
        for (i, (ty, content)) in sync_shapes().into_iter().enumerate() {
            let mut r = rig();
            let kind = if group { ThreadKind::Group } else { ThreadKind::User };
            let tid = if group { "g1" } else { "100" };
            r.ingest(incoming("1", 1000, json!({}), group)).await;
            r.ingest(incoming("2", 1100, json!({}), group)).await;
            r.drain_events();
            let rec_id = format!("sync{i}");
            r.ingest(incoming(
                &rec_id,
                2000,
                json!({ "uidFrom": "me", "msgType": ty, "content": content }),
                group,
            ))
            .await;
            assert!(r.store.get_message("1").is_none(), "{group} {ty} {content}");
            assert!(r.store.is_deleted_for_me(kind, tid, "1"));
            assert!(r.store.get_message("2").is_some(), "other message kept");
            assert!(r.store.get_message(&rec_id).is_none(), "no bubble");
            assert!(r.store.is_deleted_for_me(kind, tid, &rec_id));
            let events = r.drain_events();
            assert!(of_type(&events, "message").is_empty());
            assert_eq!(
                of_type(&events, "message_deleted")[0],
                &json!({ "type": "message_deleted", "threadKind": kind, "threadId": tid, "id": "1" })
            );
        }
    }
}

#[tokio::test]
async fn delete_sync_record_without_target_is_dropped() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), true)).await;
    r.ingest(incoming(
        "s1",
        2000,
        json!({ "uidFrom": "me", "msgType": "chat.delete", "content": {} }),
        true,
    ))
    .await;
    r.ingest(incoming(
        "s2",
        2100,
        json!({ "uidFrom": "me", "msgType": "chat.undo", "content": { "globalMsgId": 999 } }),
        true,
    ))
    .await;
    assert!(r.store.get_message("s1").is_none());
    assert!(r.store.get_message("s2").is_none());
    assert!(r.store.get_message("1").is_some());
}

#[tokio::test]
async fn someone_elses_delete_shaped_message_never_deletes_anything() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({}), true)).await;
    r.ingest(incoming(
        "x1",
        2000,
        json!({ "msgType": "chat.delete", "content": { "globalMsgId": 1 } }),
        true,
    ))
    .await;
    r.ingest(incoming("x2", 2100, json!({ "content": "chat.delete 1" }), true))
        .await;
    assert!(r.store.get_message("1").is_some());
    assert!(r.store.get_message("x2").is_some());
    // our own ordinary text is untouched too
    r.ingest(incoming("m1", 2200, json!({ "uidFrom": "me", "content": "delete 1" }), true))
        .await;
    assert!(r.store.get_message("m1").is_some());
    assert!(r.store.get_message("1").is_some());
}

#[tokio::test]
async fn real_recall_still_shows_message_recalled() {
    let r = rig();
    r.ingest(incoming("1", 1000, json!({ "uidFrom": "me" }), true)).await;
    r.ingest(incoming(
        "u1",
        2000,
        json!({ "uidFrom": "me", "msgType": "chat.undo", "content": { "deleteMsg": 1, "globalMsgId": 1 } }),
        true,
    ))
    .await;
    let m = r.store.get_message("1").unwrap();
    assert!(m.recalled);
    assert_eq!(m.text, "Message recalled");
    assert!(!r.store.is_deleted_for_me(ThreadKind::Group, "g1", "1"));
}
