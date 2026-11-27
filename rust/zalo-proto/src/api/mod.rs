pub mod account;
pub mod conversations;
pub mod ctrlpoll;
pub mod friends;
pub mod groups;
pub mod messaging;
pub mod polls;
pub mod voicecall;

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::Method;
use serde_json::{json, Value};

use crate::crypto::{decode_aes, encode_aes, parse_json_keeping_bigints};
use crate::error::{ProtoError, Result};
use crate::http::ZaloHttp;
use crate::listener::ZaloListener;
use crate::types::ZaloContext;

pub const FILE_SERVICE_FALLBACK: &str = "https://tt-files-wpa.chat.zalo.me";
pub const PROFILE_SERVICE_FALLBACK: &str = "https://tt-profile-wpa.chat.zalo.me";
pub const MAX_BACKUP_BYTES: usize = 256 * 1024 * 1024;

/// Reply from `/api/znoise` mapping plain mobile user/group IDs to Zalo Web noised IDs.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NoisedIdsReply {
    pub users: HashMap<String, String>,
    pub groups: HashMap<String, String>,
    pub reply_shape: String,
}

/// Logged-in Zalo Web protocol client exposing all HTTP APIs and WebSocket listener creation.
#[derive(Clone, Debug)]
pub struct ZaloClient {
    pub(crate) ctx: Arc<ZaloContext>,
    pub(crate) http: ZaloHttp,
}

impl ZaloClient {
    pub fn new(ctx: Arc<ZaloContext>, http: ZaloHttp) -> Self {
        Self { ctx, http }
    }

    pub fn ctx(&self) -> &Arc<ZaloContext> {
        &self.ctx
    }

    pub fn http(&self) -> &ZaloHttp {
        &self.http
    }

    pub fn own_id(&self) -> &str {
        &self.ctx.uid
    }

    /// Creates a [`ZaloListener`] bound to this session's context.
    pub fn listener(&self, self_listen: bool) -> ZaloListener {
        ZaloListener::new(Arc::clone(&self.ctx), self_listen)
    }

    /// Creates a [`voicecall::VoiceCallApi`] bound to this client.
    pub fn voice_call(&self) -> voicecall::VoiceCallApi {
        voicecall::VoiceCallApi::new(self.clone())
    }

    /// Resolves a service base URL from `zpw_service_map_v3`, returning an error if missing.
    pub(crate) fn require_service(&self, name: &str) -> Result<String> {
        self.ctx
            .service_url(name)
            .ok_or_else(|| ProtoError::InvalidArg(format!("service '{name}' missing from zpw_service_map_v3")))
    }

    /// Generic encrypted GET to `/api/message/<endpoint>` on the file service (used by history sync).
    pub async fn get_file_service(&self, endpoint: &str, params: Value) -> Result<Value> {
        let base = self
            .ctx
            .service_url("file")
            .filter(|s| s.starts_with("https://"))
            .unwrap_or_else(|| FILE_SERVICE_FALLBACK.to_string());
        let url = format!("{base}/api/message/{}", endpoint.trim_start_matches('/'));
        self.http.get_encrypted(&self.ctx, &url, &params, &[]).await
    }

    /// Calls `/api/znoise` on the profile service to translate plain mobile user/group IDs to
    /// Zalo Web noised IDs (preserving 19-digit BigInt IDs as strings).
    pub async fn get_znoise(
        &self,
        user_ids: &[String],
        group_ids: &[String],
    ) -> Result<NoisedIdsReply> {
        let base = self
            .ctx
            .service_url("profile")
            .filter(|s| s.starts_with("https://"))
            .unwrap_or_else(|| PROFILE_SERVICE_FALLBACK.to_string());
        let payload = json!({
            "fids": user_ids,
            "gids": group_ids,
        });
        let encrypted = encode_aes(&self.ctx.secret_key, &payload.to_string())?;
        let url = ZaloHttp::make_ctx_url(
            &self.ctx,
            &format!("{base}/api/znoise"),
            &[("params", &encrypted)],
            true,
        )?;
        let resp = self
            .http
            .request_with_cookies(
                &self.ctx.cookies,
                &self.ctx.user_agent,
                Method::GET,
                &url,
                None,
                None,
            )
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(ProtoError::HttpStatus {
                status: status.as_u16(),
                message: format!("znoise HTTP {}", status.as_u16()),
            });
        }

        let outer = parse_json_keeping_bigints(&text)?;
        let outer_code = outer
            .get("error_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if outer_code != 0 {
            let msg = outer
                .get("error_message")
                .and_then(|v| v.as_str())
                .unwrap_or("znoise error")
                .to_string();
            return Err(ProtoError::Api {
                code: outer_code,
                message: msg,
            });
        }

        let inner_data = match outer.get("data") {
            Some(Value::String(enc)) if !self.ctx.secret_key.is_empty() => {
                let plain = decode_aes(&self.ctx.secret_key, enc)?;
                let inner = parse_json_keeping_bigints(&plain)?;
                let inner_code = inner
                    .get("error_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1);
                if inner_code != 0 {
                    let msg = inner
                        .get("error_message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("znoise decrypted error")
                        .to_string();
                    return Err(ProtoError::Api {
                        code: inner_code,
                        message: msg,
                    });
                }
                inner.get("data").cloned().unwrap_or(Value::Null)
            }
            Some(other) => other.clone(),
            None => Value::Null,
        };

        Ok(parse_noised_ids_reply(user_ids, group_ids, &inner_data))
    }

    /// Downloads a history-sync backup blob (enforcing `MAX_BACKUP_BYTES` and Zalo-host-only cookies).
    pub async fn download_backup(&self, url: &str) -> Result<Vec<u8>> {
        self.http
            .download_backup_bytes(&self.ctx, url, MAX_BACKUP_BYTES)
            .await
    }
}

/// Parses `/api/znoise`'s `fids` and `gids` fields, supporting both parallel arrays and `{ plain: noised }` maps.
pub fn parse_noised_ids_reply(
    asked_users: &[String],
    asked_groups: &[String],
    data: &Value,
) -> NoisedIdsReply {
    fn maybe_parse(v: &Value) -> Value {
        if let Some(s) = v.as_str() {
            let trimmed = s.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(p) = parse_json_keeping_bigints(trimmed) {
                    return p;
                }
            }
        }
        v.clone()
    }

    fn val_str(v: &Value) -> String {
        match v {
            Value::String(s) => s.trim().to_string(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        }
    }

    fn apply(asked: &[String], raw: Option<&Value>, out: &mut HashMap<String, String>) {
        let Some(raw_v) = raw else { return };
        let parsed = maybe_parse(raw_v);
        match parsed {
            Value::Array(arr) => {
                if arr.len() != asked.len() {
                    return;
                }
                for (plain, item) in asked.iter().zip(arr.iter()) {
                    let noised = val_str(item);
                    if !noised.is_empty() && noised != "0" {
                        out.insert(plain.clone(), noised);
                    }
                }
            }
            Value::Object(map) => {
                for plain in asked {
                    if let Some(v) = map.get(plain) {
                        let noised = val_str(v);
                        if !noised.is_empty() && noised != "0" {
                            out.insert(plain.clone(), noised);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let unpacked = maybe_parse(data);
    let mut users = HashMap::new();
    let mut groups = HashMap::new();
    let mut reply_shape = "{}".to_string();

    if let Some(obj) = unpacked.as_object() {
        apply(asked_users, obj.get("fids"), &mut users);
        apply(asked_groups, obj.get("gids"), &mut groups);
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        let parts: Vec<String> = keys
            .into_iter()
            .map(|k| {
                let kind = match &obj[k] {
                    Value::Array(a) => format!("arr({})", a.len()),
                    Value::Object(o) => format!("obj({})", o.len()),
                    Value::String(_) => "string".to_string(),
                    Value::Number(_) => "number".to_string(),
                    Value::Bool(_) => "boolean".to_string(),
                    Value::Null => "null".to_string(),
                };
                format!("{k}:{kind}")
            })
            .collect();
        reply_shape = format!("{{{}}}", parts.join(","));
    }

    NoisedIdsReply {
        users,
        groups,
        reply_shape,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_noised_ids_supports_arrays_and_maps() {
        let u = vec!["111".to_string(), "222".to_string()];
        let g = vec!["999".to_string()];

        let arr_data = json!({
            "fids": ["7000000000000000111", "0"],
            "gids": ["8000000000000000999"]
        });
        let r1 = parse_noised_ids_reply(&u, &g, &arr_data);
        assert_eq!(r1.users.get("111").map(|s| s.as_str()), Some("7000000000000000111"));
        assert!(!r1.users.contains_key("222"));
        assert_eq!(r1.groups.get("999").map(|s| s.as_str()), Some("8000000000000000999"));

        let map_data = json!({
            "fids": "{\"111\":7000000000000000111,\"222\":7000000000000000222}",
            "gids": { "999": "8000000000000000999" }
        });
        let r2 = parse_noised_ids_reply(&u, &g, &map_data);
        assert_eq!(r2.users.len(), 2);
        assert_eq!(r2.users.get("222").map(|s| s.as_str()), Some("7000000000000000222"));
        assert_eq!(r2.groups.get("999").map(|s| s.as_str()), Some("8000000000000000999"));
    }

    mod swarm2_proto_tests {
        use super::*;
        use std::sync::{Mutex, RwLock};
        use base64::Engine;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use crate::cookies::ZaloCookieJar;
        use crate::types::{SendMessageOptions, ThreadType, DEFAULT_API_TYPE, DEFAULT_API_VERSION};

        #[derive(Debug, Clone)]
        struct CapturedReq {
            method: String,
            path: String,
            query: HashMap<String, String>,
            params: Value,
        }

        fn test_secret_key() -> String {
            // 32-byte AES-256 key encoded as base64.
            base64::engine::general_purpose::STANDARD.encode([0x42u8; 32])
        }

        async fn spawn_mock_zalo(
            secret_key: String,
            responder: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
        ) -> (String, Arc<Mutex<Vec<CapturedReq>>>) {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let captured: Arc<Mutex<Vec<CapturedReq>>> = Arc::new(Mutex::new(Vec::new()));
            let captured_bg = Arc::clone(&captured);
            let responder = Arc::new(responder);

            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let captured_bg = Arc::clone(&captured_bg);
                    let secret_key = secret_key.clone();
                    let responder = Arc::clone(&responder);
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        let mut tmp = [0u8; 4096];
                        let header_end;
                        loop {
                            let n = stream.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                return;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                header_end = pos + 4;
                                break;
                            }
                        }
                        let headers_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
                        let mut lines = headers_str.lines();
                        let req_line = lines.next().unwrap_or("");
                        let mut parts = req_line.split_whitespace();
                        let method = parts.next().unwrap_or("").to_string();
                        let target = parts.next().unwrap_or("/");
                        let content_len: usize = lines
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                if k.eq_ignore_ascii_case("content-length") {
                                    v.trim().parse().ok()
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(0);
                        while buf.len() < header_end + content_len {
                            let n = stream.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        let body = &buf[header_end..buf.len().min(header_end + content_len)];

                        let full_url = url::Url::parse(&format!("http://127.0.0.1{target}")).unwrap();
                        let path = full_url.path().to_string();
                        let query: HashMap<String, String> = full_url
                            .query_pairs()
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect();

                        let enc_params = if method == "POST" {
                            url::form_urlencoded::parse(body)
                                .find(|(k, _)| k == "params")
                                .map(|(_, v)| v.into_owned())
                                .or_else(|| query.get("params").cloned())
                        } else {
                            query.get("params").cloned()
                        };
                        let params = enc_params
                            .as_deref()
                            .map(|enc| {
                                let plain = decode_aes(&secret_key, enc).unwrap();
                                serde_json::from_str::<Value>(&plain).unwrap()
                            })
                            .unwrap_or(Value::Null);

                        let reply_data = responder(&path, &params);
                        captured_bg.lock().unwrap().push(CapturedReq {
                            method,
                            path,
                            query,
                            params,
                        });

                        let inner = json!({ "error_code": 0, "data": reply_data }).to_string();
                        let enc_reply = encode_aes(&secret_key, &inner).unwrap();
                        let outer = json!({ "error_code": 0, "data": enc_reply }).to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            outer.len(),
                            outer
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                    });
                }
            });

            (format!("http://{addr}"), captured)
        }

        fn make_client(base_url: &str, secret_key: String) -> ZaloClient {
            let mut service_map = HashMap::new();
            for key in ["alias", "friend", "group", "group_board", "file", "chat", "profile"] {
                service_map.insert(key.to_string(), vec![format!("{base_url}/{key}")]);
            }
            let ctx = Arc::new(ZaloContext {
                uid: "own-1".into(),
                imei: "test-imei-123".into(),
                user_agent: "TestUA".into(),
                language: "vi".into(),
                api_type: DEFAULT_API_TYPE,
                api_version: DEFAULT_API_VERSION,
                secret_key,
                service_map,
                ws_urls: vec![],
                ctrs_urls: vec![],
                settings: json!({}),
                extra_ver: json!({}),
                login_info: json!({}),
                cookies: Arc::new(RwLock::new(ZaloCookieJar::default())),
                upload_callbacks: Arc::new(Mutex::new(HashMap::new())),
            });
            ZaloClient::new(ctx, ZaloHttp::new().unwrap())
        }

        #[tokio::test]
        async fn friend_alias_remove_and_block_payloads_match_zca_js() {
            let key = test_secret_key();
            let (base, captured) = spawn_mock_zalo(key.clone(), |_, _| json!({})).await;
            let client = make_client(&base, key);

            client.change_friend_alias("Bestie", "u42").await.unwrap();
            client.remove_friend_alias("u42").await.unwrap();
            client.remove_friend("u42").await.unwrap();
            client.block_user("u99").await.unwrap();
            client.unblock_user("u99").await.unwrap();

            let reqs = captured.lock().unwrap().clone();
            assert_eq!(reqs.len(), 5);

            assert_eq!(reqs[0].method, "GET");
            assert_eq!(reqs[0].path, "/alias/api/alias/update");
            assert_eq!(
                reqs[0].params,
                json!({ "friendId": "u42", "alias": "Bestie", "imei": "test-imei-123" })
            );

            assert_eq!(reqs[1].method, "GET");
            assert_eq!(reqs[1].path, "/alias/api/alias/remove");
            assert_eq!(reqs[1].params, json!({ "friendId": "u42" }));

            assert_eq!(reqs[2].method, "POST");
            assert_eq!(reqs[2].path, "/friend/api/friend/remove");
            assert_eq!(reqs[2].params, json!({ "fid": "u42", "imei": "test-imei-123" }));

            assert_eq!(reqs[3].method, "POST");
            assert_eq!(reqs[3].path, "/friend/api/friend/block");
            assert_eq!(reqs[3].params, json!({ "fid": "u99", "imei": "test-imei-123" }));

            assert_eq!(reqs[4].method, "POST");
            assert_eq!(reqs[4].path, "/friend/api/friend/unblock");
            assert_eq!(reqs[4].params, json!({ "fid": "u99", "imei": "test-imei-123" }));
        }

        #[tokio::test]
        async fn group_links_deputies_board_and_notes_match_zca_js() {
            let key = test_secret_key();
            let (base, captured) = spawn_mock_zalo(key.clone(), |path, _| match path {
                "/group_board/api/board/list" => json!({
                    "count": 2,
                    "items": [
                        {
                            "boardType": 1,
                            "data": {
                                "id": "n1",
                                "params": "{\"title\":\"Standup\"}",
                                "creatorId": "u1",
                                "createTime": 1700000000
                            }
                        },
                        {
                            "boardType": 3,
                            "data": {
                                "poll_id": 77,
                                "question": "Lunch?"
                            }
                        }
                    ]
                }),
                "/group_board/api/board/topic/createv2" => json!({
                    "id": "n2",
                    "params": "{\"title\":\"Pinned note\"}",
                    "creatorId": "own-1",
                    "createTime": 1700000010
                }),
                _ => json!({ "link": "https://zalo.me/g/abc", "enabled": 1, "expiration_date": 1800000000 }),
            })
            .await;
            let client = make_client(&base, key);

            client.get_group_link_detail("g1").await.unwrap();
            client.enable_group_link("g1").await.unwrap();
            client.disable_group_link("g1").await.unwrap();
            client.join_group_link("https://zalo.me/g/abc").await.unwrap();
            client.add_group_deputy("u5", "g1").await.unwrap();
            client.remove_group_deputy("u5", "g1").await.unwrap();
            let board = client.get_list_board("g1", Some(2), Some(10)).await.unwrap();
            let note = client.create_note("g1", "Pinned note", true).await.unwrap();

            assert_eq!(board["items"][0]["data"]["params"]["title"], "Standup");
            assert_eq!(board["items"][1]["data"]["question"], "Lunch?");
            assert_eq!(note["params"]["title"], "Pinned note");

            let reqs = captured.lock().unwrap().clone();
            assert_eq!(reqs.len(), 8);

            assert_eq!((reqs[0].method.as_str(), reqs[0].path.as_str()), ("GET", "/group/api/group/link/detail"));
            assert_eq!(reqs[0].params, json!({ "grid": "g1", "imei": "test-imei-123" }));

            assert_eq!((reqs[1].method.as_str(), reqs[1].path.as_str()), ("GET", "/group/api/group/link/new"));
            assert_eq!(reqs[1].params, json!({ "grid": "g1", "imei": "test-imei-123" }));

            assert_eq!((reqs[2].method.as_str(), reqs[2].path.as_str()), ("GET", "/group/api/group/link/disable"));
            assert_eq!(reqs[2].params, json!({ "grid": "g1" }));

            assert_eq!((reqs[3].method.as_str(), reqs[3].path.as_str()), ("GET", "/group/api/group/link/join"));
            assert_eq!(reqs[3].params, json!({ "link": "https://zalo.me/g/abc", "clientLang": "vi" }));

            assert_eq!((reqs[4].method.as_str(), reqs[4].path.as_str()), ("GET", "/group/api/group/admins/add"));
            assert_eq!(reqs[4].params, json!({ "grid": "g1", "members": ["u5"], "imei": "test-imei-123" }));

            assert_eq!((reqs[5].method.as_str(), reqs[5].path.as_str()), ("GET", "/group/api/group/admins/remove"));
            assert_eq!(reqs[5].params, json!({ "grid": "g1", "members": ["u5"], "imei": "test-imei-123" }));

            assert_eq!((reqs[6].method.as_str(), reqs[6].path.as_str()), ("GET", "/group_board/api/board/list"));
            assert_eq!(
                reqs[6].params,
                json!({
                    "group_id": "g1",
                    "board_type": 0,
                    "page": 2,
                    "count": 10,
                    "last_id": 0,
                    "last_type": 0,
                    "imei": "test-imei-123"
                })
            );

            assert_eq!((reqs[7].method.as_str(), reqs[7].path.as_str()), ("POST", "/group_board/api/board/topic/createv2"));
            assert_eq!(reqs[7].params["grid"], "g1");
            assert_eq!(reqs[7].params["pinAct"], 1);
            assert_eq!(reqs[7].params["color"], -16777216);
            assert_eq!(reqs[7].params["params"], "{\"title\":\"Pinned note\"}");
        }

        #[tokio::test]
        async fn send_message_ttl_send_card_and_send_link_match_zca_js() {
            let key = test_secret_key();
            let (base, captured) = spawn_mock_zalo(key.clone(), |path, _| match path {
                "/friend/api/friend/mget-qr" => json!({ "u7": "https://qr.zalo.me/u7" }),
                "/file/api/message/parselink" => json!({
                    "data": {
                        "href": "https://example.com/post",
                        "src": "example.com",
                        "title": "Example Post",
                        "desc": "Post summary",
                        "thumb": "https://example.com/thumb.png",
                        "media": { "type": 0, "count": 0 }
                    },
                    "error_maps": {}
                }),
                _ => json!({ "msgId": 555 }),
            })
            .await;
            let client = make_client(&base, key);

            // 1. send_message with disappearing TTL
            client
                .send_message(
                    SendMessageOptions {
                        msg: "ephemeral".into(),
                        ttl: Some(60_000),
                        ..Default::default()
                    },
                    "u1",
                    ThreadType::User,
                )
                .await
                .unwrap();

            // 2. send_card to user (with phone) and group
            client
                .send_card("u7", Some("0901234567"), "u1", ThreadType::User, Some(30_000))
                .await
                .unwrap();
            client
                .send_card("u7", None, "g1", ThreadType::Group, None)
                .await
                .unwrap();

            // 3. send_link to user (with custom msg) and group
            client
                .send_link(
                    "https://example.com/post",
                    Some("Check this out"),
                    "u1",
                    ThreadType::User,
                    Some(120_000),
                )
                .await
                .unwrap();
            client
                .send_link("https://example.com/post", None, "g1", ThreadType::Group, None)
                .await
                .unwrap();

            let reqs = captured.lock().unwrap().clone();
            // send_message (1) + send_card user (get_qr + forward = 2) + send_card group (2) + send_link user (parse + link = 2) + send_link group (2) = 9
            assert_eq!(reqs.len(), 9);

            // send_message ttl
            assert_eq!(reqs[0].path, "/chat/api/message/sms");
            assert_eq!(reqs[0].params["ttl"], 60_000);
            assert_eq!(reqs[0].params["message"], "ephemeral");

            // send_card user
            assert_eq!((reqs[1].method.as_str(), reqs[1].path.as_str()), ("POST", "/friend/api/friend/mget-qr"));
            assert_eq!(reqs[1].params, json!({ "fids": ["u7"] }));
            assert_eq!((reqs[2].method.as_str(), reqs[2].path.as_str()), ("POST", "/file/api/message/forward"));
            assert_eq!(reqs[2].params["msgType"], 6);
            assert_eq!(reqs[2].params["toId"], "u1");
            assert_eq!(reqs[2].params["ttl"], 30_000);
            assert_eq!(reqs[2].params["imei"], "test-imei-123");
            let card_info: Value = serde_json::from_str(reqs[2].params["msgInfo"].as_str().unwrap()).unwrap();
            assert_eq!(
                card_info,
                json!({
                    "contactUid": "u7",
                    "qrCodeUrl": "https://qr.zalo.me/u7",
                    "phone": "0901234567"
                })
            );

            // send_card group
            assert_eq!((reqs[4].method.as_str(), reqs[4].path.as_str()), ("POST", "/file/api/group/forward"));
            assert_eq!(reqs[4].params["grid"], "g1");
            assert_eq!(reqs[4].params["visibility"], 0);
            assert_eq!(reqs[4].params["msgType"], 6);
            assert!(reqs[4].params.get("imei").is_none());

            // send_link user
            assert_eq!((reqs[5].method.as_str(), reqs[5].path.as_str()), ("GET", "/file/api/message/parselink"));
            assert_eq!(
                reqs[5].params,
                json!({ "link": "https://example.com/post", "version": 1, "imei": "test-imei-123" })
            );
            assert_eq!((reqs[6].method.as_str(), reqs[6].path.as_str()), ("POST", "/chat/api/message/link"));
            assert_eq!(reqs[6].query.get("nretry").map(String::as_str), Some("0"));
            assert_eq!(reqs[6].params["toId"], "u1");
            assert_eq!(reqs[6].params["mentionInfo"], "");
            assert_eq!(reqs[6].params["msg"], "Check this out https://example.com/post");
            assert_eq!(reqs[6].params["title"], "Example Post");
            assert_eq!(reqs[6].params["type"], 2);
            assert_eq!(reqs[6].params["ttl"], 120_000);

            // send_link group
            assert_eq!((reqs[8].method.as_str(), reqs[8].path.as_str()), ("POST", "/group/api/group/sendlink"));
            assert_eq!(reqs[8].query.get("nretry").map(String::as_str), Some("0"));
            assert_eq!(reqs[8].params["grid"], "g1");
            assert_eq!(reqs[8].params["imei"], "test-imei-123");
            assert_eq!(reqs[8].params["msg"], "https://example.com/post");
            assert_eq!(reqs[8].params["ttl"], 0);
        }
    }
}

