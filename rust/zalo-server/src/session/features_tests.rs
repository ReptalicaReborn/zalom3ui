//! Ports of `test/features.test.ts`, one Rust test per TS test, in the same order.

use serde_json::json;

use super::test_support::{incoming, of_type, rig, Rig};
use super::*;

/// The TS features setup: a signed-in session whose `sendMessage` answers with one attachment id.
fn features_rig() -> Rig {
    let r = rig();
    r.api.respond(
        "send_message",
        json!({ "message": null, "attachment": [{ "msgId": 77 }] }),
    );
    r
}

const STICKER_URL: &str = "https://api.zaloapp.com/api/emoticon/sticker/webpc?size=130&eid=";

#[tokio::test]
async fn a_photo_is_sent_with_its_pixel_size_a_file_with_only_its_size() {
    let r = features_rig();
    let data = vec![1u8, 2, 3, 4];
    let ids = r
        .session
        .send_attachment(
            ThreadKind::Group,
            "g1",
            UploadFile {
                data: data.clone(),
                filename: "a.JPG".into(),
                width: Some(640),
                height: Some(480),
            },
        )
        .await
        .unwrap();
    assert_eq!(ids, vec!["77".to_string()]);
    let calls = r.api.calls();
    let [content, thread_id, thread_type] = &calls[0].args[..] else {
        panic!("unexpected args {:?}", calls[0].args)
    };
    assert_eq!(content["msg"], "");
    assert_eq!(
        content["attachments"],
        json!([{ "filename": "a.JPG", "data": [1, 2, 3, 4], "width": 640, "height": 480 }])
    );
    assert_eq!(thread_id, "g1");
    assert_eq!(thread_type, "Group");

    r.session
        .send_attachment(
            ThreadKind::User,
            "100",
            UploadFile {
                data,
                filename: "notes.pdf".into(),
                width: None,
                height: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        r.api.calls()[1].args[0]["attachments"],
        json!([{ "filename": "notes.pdf", "data": [1, 2, 3, 4], "width": null, "height": null }])
    );
}

#[tokio::test]
async fn attachments_zalo_cant_take_here_are_refused_before_anything_is_sent() {
    let r = features_rig();
    let file = |filename: &str, data: Vec<u8>| UploadFile {
        data,
        filename: filename.into(),
        width: None,
        height: None,
    };
    let refused = |name: &'static str, data: Vec<u8>, expected: &'static str| {
        let session = Arc::clone(&r.session);
        async move {
            let err = session
                .send_attachment(ThreadKind::User, "100", file(name, data))
                .await
                .unwrap_err();
            assert!(err.message.contains(expected), "{name}: {}", err.message);
        }
    };
    refused("noext", vec![1], "extension").await;
    refused("a.gif", vec![1], "GIF").await;
    refused("a.png", vec![1], "width and height").await;
    refused("a.txt", vec![], "empty").await;
    assert_eq!(r.api.call_count(), 0);
}

#[tokio::test]
async fn sticker_search_returns_images_and_a_found_sticker_can_be_sent() {
    let r = features_rig();
    r.api.respond("get_stickers", json!([5, 6]));
    r.api.respond(
        "get_stickers_detail",
        json!([
            { "id": 5, "cateId": 50, "type": 7, "stickerWebpUrl": "https://s/5.webp", "stickerUrl": "https://s/5.png" },
            { "id": 6, "cateId": 60, "type": 7, "stickerWebpUrl": null, "stickerUrl": "https://s/6.png" },
        ]),
    );
    assert_eq!(
        r.session.search_stickers("hi").await.unwrap(),
        vec![
            StickerInfo {
                id: 5,
                cat_id: 50,
                sticker_type: 7,
                url: format!("{STICKER_URL}5"),
            },
            StickerInfo {
                id: 6,
                cat_id: 60,
                sticker_type: 7,
                url: format!("{STICKER_URL}6"),
            },
        ]
    );
    r.session
        .send_sticker(ThreadKind::User, "100", 6)
        .await
        .unwrap();
    assert_eq!(
        r.api.calls_to("send_sticker"),
        vec![vec![
            json!({ "id": 6, "cateId": 60, "type": 7 }),
            json!("100"),
            json!("User")
        ]]
    );
}

#[tokio::test]
async fn a_received_sticker_shows_its_picture_right_away_with_no_lookup() {
    let r = features_rig();
    r.ingest(incoming(
        "1",
        now_ms(),
        json!({ "msgType": "chat.sticker", "content": { "id": 9, "catId": 1, "type": 7 } }),
        false,
    ))
    .await;
    assert_eq!(
        r.store
            .get_message("1")
            .unwrap()
            .attachment
            .unwrap()
            .thumb
            .as_deref(),
        Some(format!("{STICKER_URL}9").as_str())
    );
    assert_eq!(r.api.call_count(), 0);
}

#[tokio::test]
async fn notifications_not_mine_not_muted_not_the_open_chat_not_old_history() {
    let r = features_rig();
    let now = now_ms();
    r.ingest(incoming("1", now, json!({}), false)).await;
    let msg = r.store.to_public(&r.store.get_message("1").unwrap());
    let session = &r.session;
    assert!(session.should_notify_at(&msg, false, now));

    session.set_presence_at(Some(ThreadKind::User), Some("100"), now);
    assert!(!session.should_notify_at(&msg, false, now));
    // A client that stopped saying so no longer counts as looking.
    assert!(session.should_notify_at(&msg, false, now + 91_000));
    session.set_presence_at(None, None, now);

    r.store.set_muted(ThreadKind::User, "100", Some(-1));
    assert!(!session.should_notify_at(&msg, false, now));
    r.store.set_muted(ThreadKind::User, "100", None);

    let mine = ChatMessage {
        is_self: true,
        ..msg.clone()
    };
    assert!(!session.should_notify_at(&mine, false, now));
    let stale = ChatMessage {
        ts: now - 11 * 60_000,
        ..msg.clone()
    };
    assert!(!session.should_notify_at(&stale, false, now));
    assert!(
        !session.should_notify_at(&msg, true, now),
        "old history never notifies"
    );
}

#[tokio::test]
async fn reading_a_chat_tells_clients_so_its_notification_goes_away() {
    let mut r = features_rig();
    r.drain_events();
    r.session.mark_read(ThreadKind::Group, "g1").await.unwrap();
    let events = r.drain_events();
    assert_eq!(
        of_type(&events, "read")[0],
        &json!({ "type": "read", "threadKind": "group", "threadId": "g1" })
    );
}

#[tokio::test]
async fn friend_requests_only_received_ones_are_listed_accepting_reloads_friends() {
    let r = features_rig();
    r.api.respond(
        "get_friend_recommendations",
        json!({
            "recommItems": [
                { "dataInfo": { "userId": "7", "displayName": "Bảo", "avatar": "a7", "recommType": 2, "recommTime": 5, "recommInfo": { "message": "Chào" } } },
                { "dataInfo": { "userId": "8", "displayName": "Gợi ý", "avatar": "a8", "recommType": 1, "recommTime": 6, "recommInfo": { "message": "" } } },
            ]
        }),
    );
    r.api.respond(
        "get_all_friends",
        json!([{ "userId": "7", "displayName": "Bảo", "zaloName": "Bao", "avatar": "a7" }]),
    );
    assert_eq!(
        r.session.friend_requests().await.unwrap(),
        vec![FriendRequest {
            id: "7".into(),
            name: "Bảo".into(),
            avatar: "a7".into(),
            message: "Chào".into(),
            time: 5
        }]
    );
    r.session.accept_friend_request("7").await.unwrap();
    assert_eq!(
        r.api.calls_to("accept_friend_request"),
        vec![vec![json!("7")]]
    );
    assert_eq!(
        r.session.friends(),
        vec![Person {
            id: "7".into(),
            name: "Bảo".into(),
            avatar: "a7".into()
        }]
    );
}

#[tokio::test]
async fn sent_friend_requests_are_listed_newest_first_and_none_is_not_an_error() {
    let r = features_rig();
    r.api.respond(
        "get_sent_friend_request",
        json!({
            "5": { "userId": "5", "displayName": "Lan", "zaloName": "lan", "avatar": "a5", "fReqInfo": { "message": "Hi", "time": 10 } },
            "6": { "userId": "6", "displayName": "", "zaloName": "minh", "avatar": "", "fReqInfo": { "message": "", "time": 20 } },
        }),
    );
    assert_eq!(
        r.session.sent_friend_requests().await.unwrap(),
        vec![
            FriendRequest {
                id: "6".into(),
                name: "minh".into(),
                avatar: String::new(),
                message: String::new(),
                time: 20
            },
            FriendRequest {
                id: "5".into(),
                name: "Lan".into(),
                avatar: "a5".into(),
                message: "Hi".into(),
                time: 10
            },
        ]
    );
    let none = features_rig();
    none.api.fail("get_sent_friend_request", || {
        ProtoError::InvalidArg("none".into())
    });
    assert_eq!(none.session.sent_friend_requests().await.unwrap(), vec![]);
}

#[tokio::test]
async fn finding_someone_by_phone_says_whether_they_are_a_friend_or_already_asked() {
    let r = features_rig();
    r.api.respond(
        "find_user",
        json!({ "uid": "42", "display_name": "Chi", "zalo_name": "chi", "avatar": "a42" }),
    );
    r.api.respond(
        "get_sent_friend_request",
        json!({ "42": { "userId": "42" } }),
    );
    assert_eq!(
        r.session.find_user("0901234567").await.unwrap(),
        FoundUser {
            id: "42".into(),
            name: "Chi".into(),
            avatar: "a42".into(),
            is_friend: false,
            requested: true
        }
    );

    let missing = features_rig();
    missing
        .api
        .fail("find_user", || ProtoError::InvalidArg("not found".into()));
    let err = missing.session.find_user("0900000000").await.unwrap_err();
    assert!(err.message.contains("No Zalo account"), "{}", err.message);
}

#[tokio::test]
async fn groups_create_with_2_plus_people_add_members_rename_leave_removes_the_chat() {
    let r = features_rig();
    r.api.respond(
        "create_group",
        json!({ "groupId": "555", "sucessMembers": ["2", "3"], "errorMembers": ["4"], "groupType": 0, "error_data": {} }),
    );
    r.api.respond(
        "add_user_to_group",
        json!({ "errorMembers": [], "error_data": {} }),
    );
    r.api
        .respond("get_group_info", json!({ "gridInfoMap": {} }));
    let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    let err = r
        .session
        .create_group("x", &ids(&["2", "me"]))
        .await
        .unwrap_err();
    assert!(err.message.contains("at least 2"), "{}", err.message);
    assert_eq!(
        r.session
            .create_group("  Nhóm bạn  ", &ids(&["2", "3", "4", "2"]))
            .await
            .unwrap(),
        ("555".to_string(), ids(&["4"]))
    );
    assert_eq!(
        r.api.calls_to("create_group"),
        vec![vec![
            json!({ "name": "Nhóm bạn", "members": ["2", "3", "4"] })
        ]]
    );
    assert_eq!(
        r.store.get_thread(ThreadKind::Group, "555").unwrap().name,
        "Nhóm bạn"
    );

    assert_eq!(
        r.session
            .add_group_members("555", &ids(&["5", "me"]))
            .await
            .unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        r.api.calls_to("add_user_to_group"),
        vec![vec![json!(["5"]), json!("555")]]
    );

    r.session.rename_group("555", "Tên mới").await.unwrap();
    assert_eq!(
        r.api.calls_to("change_group_name"),
        vec![vec![json!("Tên mới"), json!("555")]]
    );
    assert_eq!(
        r.store.get_thread(ThreadKind::Group, "555").unwrap().name,
        "Tên mới"
    );

    r.session.leave_group("555", true).await.unwrap();
    assert_eq!(
        r.api.calls_to("leave_group"),
        vec![vec![json!("555"), json!(true)]]
    );
    assert!(r.store.get_thread(ThreadKind::Group, "555").is_none());
}

#[tokio::test]
async fn removing_a_member_zalo_refuses_says_why() {
    let r = features_rig();
    r.api
        .respond("remove_user_from_group", json!({ "errorMembers": ["5"] }));
    let err = r.session.remove_group_member("555", "5").await.unwrap_err();
    assert_eq!(err.status, 403);
    assert!(
        err.message.contains("owner and deputies"),
        "{}",
        err.message
    );
}
