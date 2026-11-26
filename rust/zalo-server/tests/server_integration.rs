use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, HOST, ORIGIN};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;

use zalo_server::config::Config;

struct TestServer {
    addr: SocketAddr,
    token: String,
    data_dir: PathBuf,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl TestServer {
    async fn start() -> Self {
        let token = "test-secret-token-12345".to_string();
        let data_dir = std::env::temp_dir().join(format!(
            "zalo-server-it-{:016x}",
            rand::random::<u64>()
        ));
        fs::create_dir_all(&data_dir).unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let config = Config {
            host: "127.0.0.1".to_string(),
            port: 0,
            data_dir: data_dir.clone(),
            access_token: Some(token.clone()),
            read_receipts: true,
            log_file: None,
            public_dir: None,
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = zalo_server::serve_on_listener(config, listener, async move {
                let _ = shutdown_rx.await;
            })
            .await;
        });

        Self {
            addr,
            token,
            data_dir,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{path}", self.addr)
    }

    async fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            let _ = h.await;
        }
        let _ = fs::remove_dir_all(&self.data_dir);
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = fs::remove_dir_all(&self.data_dir);
    }
}

#[tokio::test]
async fn auth_security_headers_and_csrf_checks() {
    let srv = TestServer::start().await;
    let client = reqwest::Client::new();

    // 1. Missing token -> 401
    let resp = client
        .get(format!("{}/api/status", srv.base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers()["x-content-type-options"], "nosniff");
    assert_eq!(resp.headers()["x-frame-options"], "DENY");
    assert_eq!(resp.headers()["referrer-policy"], "no-referrer");

    // 2. Wrong token -> 401
    let resp = client
        .get(format!("{}/api/status", srv.base_url()))
        .header(AUTHORIZATION, "Bearer wrong-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 3. Authorization: Bearer -> 200
    let resp = client
        .get(format!("{}/api/status", srv.base_url()))
        .header(AUTHORIZATION, format!("Bearer {}", srv.token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["state"], "logged_out");
    assert_eq!(body["retrying"], false);

    // 4. Query ?token= -> 200
    let resp = client
        .get(format!("{}/api/status?token={}", srv.base_url(), srv.token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["state"], "logged_out");

    // 5. DNS-rebinding Host header rejection -> 403
    let resp = client
        .get(format!("{}/api/status", srv.base_url()))
        .header(HOST, "evil.example.com")
        .header(AUTHORIZATION, format!("Bearer {}", srv.token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 6. Cross-site Sec-Fetch-Site rejection -> 403
    let resp = client
        .get(format!("{}/api/status", srv.base_url()))
        .header("Sec-Fetch-Site", "cross-site")
        .header(AUTHORIZATION, format!("Bearer {}", srv.token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 7. Mismatched Origin rejection -> 403
    let resp = client
        .put(format!("{}/api/presence", srv.base_url()))
        .header(ORIGIN, "https://attacker.example")
        .header(AUTHORIZATION, format!("Bearer {}", srv.token))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    srv.stop().await;
}

#[tokio::test]
async fn rest_endpoints_reactions_threads_search_presence_calls_and_history_sync() {
    let srv = TestServer::start().await;
    let client = reqwest::Client::new();
    let bearer = format!("Bearer {}", srv.token);

    // GET /api/reactions -> all 15 reactions
    let resp = client
        .get(format!("{}/api/reactions", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let reactions: Vec<Value> = resp.json().await.unwrap();
    assert_eq!(reactions.len(), 15);
    assert_eq!(reactions[0]["key"], "heart");
    assert_eq!(reactions[0]["emoji"], "❤️");

    // GET /api/threads -> 200 empty array
    let resp = client
        .get(format!("{}/api/threads", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let threads: Vec<Value> = resp.json().await.unwrap();
    assert!(threads.is_empty());

    // GET /api/search -> 400 when missing q, 200 with q
    let resp = client
        .get(format!("{}/api/search", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let resp = client
        .get(format!("{}/api/search?q=xin+chao&limit=10", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let search_body: Value = resp.json().await.unwrap();
    assert_eq!(search_body, json!({ "results": [] }));

    // PUT /api/presence -> 204 for empty object and for active thread
    let resp = client
        .put(format!("{}/api/presence", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = client
        .put(format!("{}/api/presence", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .json(&json!({ "kind": "user", "id": "12345" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET /api/calls/current -> 404
    let resp = client
        .get(format!("{}/api/calls/current", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let call_err: Value = resp.json().await.unwrap();
    assert_eq!(call_err["error"], "No active call");

    // POST /api/calls/start -> 409 when logged out (Calls implemented)
    let resp = client
        .post(format!("{}/api/calls/start", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .json(&json!({ "userId": "123", "video": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // GET /api/history-sync/status -> 200 idle + log: []
    let resp = client
        .get(format!("{}/api/history-sync/status", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let sync_status: Value = resp.json().await.unwrap();
    assert_eq!(sync_status["state"], "idle");
    assert_eq!(sync_status["progress"], 0);
    assert_eq!(sync_status["log"], json!([]));

    // POST /api/history-sync/start -> 409 when logged out (History Sync implemented)
    let resp = client
        .post(format!("{}/api/history-sync/start", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // GET & DELETE /api/debug/call-log -> 200 text/plain & 204
    let resp = client
        .get(format!("{}/api/debug/call-log", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/plain"));

    let resp = client
        .delete(format!("{}/api/debug/call-log", srv.base_url()))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    srv.stop().await;
}

#[tokio::test]
async fn websocket_events_and_call_audio_close_code() {
    let srv = TestServer::start().await;

    // 1. WS /ws without token fails handshake with 401
    let unauth_req = srv.ws_url("/ws").into_client_request().unwrap();
    let unauth_err = tokio_tungstenite::connect_async(unauth_req)
        .await
        .unwrap_err();
    match unauth_err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => {
            assert_eq!(resp.status().as_u16(), 401);
        }
        other => panic!("expected HTTP 401 error, got: {other:?}"),
    }

    // 2. WS /ws with Bearer token connects and immediately sends status logged_out
    let mut req = srv.ws_url("/ws").into_client_request().unwrap();
    req.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", srv.token)).unwrap(),
    );
    let (mut ws_stream, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    let first_msg = ws_stream.next().await.unwrap().unwrap();
    let text = match first_msg {
        Message::Text(t) => t.to_string(),
        other => panic!("expected text status message, got {other:?}"),
    };
    let ev: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ev["type"], "status");
    assert_eq!(ev["status"]["state"], "logged_out");
    assert_eq!(ev["status"]["retrying"], false);
    let _ = ws_stream.close(None).await;

    // 3. WS /ws/call-audio closes cleanly with code 1008 when no call is active
    let mut audio_req = srv.ws_url("/ws/call-audio").into_client_request().unwrap();
    audio_req.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", srv.token)).unwrap(),
    );
    let (mut audio_ws, _resp) = tokio_tungstenite::connect_async(audio_req).await.unwrap();
    let msg = audio_ws.next().await.unwrap().unwrap();
    match msg {
        Message::Close(Some(frame)) => {
            assert_eq!(frame.code, CloseCode::Policy);
            assert_eq!(u16::from(frame.code), 1008);
        }
        other => panic!("expected Close(1008) frame, got {other:?}"),
    }

    srv.stop().await;
}
