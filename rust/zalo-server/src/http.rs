//! Axum HTTP/WebSocket router and security middleware matching `src/server.ts`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
#[cfg(feature = "tcp")]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
#[cfg(feature = "tcp")]
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
#[cfg(feature = "tcp")]
use axum::extract::Request;
use axum::extract::{DefaultBodyLimit, Path as AxumPath, Query, State};
#[cfg(feature = "tcp")]
use axum::http::header::{HeaderName, HeaderValue};
#[cfg(feature = "tcp")]
use axum::http::Method;
use axum::http::{header, HeaderMap, StatusCode};
#[cfg(feature = "tcp")]
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
#[cfg(feature = "tcp")]
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};
#[cfg(feature = "tcp")]
use subtle::ConstantTimeEq;
#[cfg(feature = "tcp")]
use tower_http::services::ServeDir;
#[cfg(feature = "tcp")]
use url::Url;
use zalo_proto::CreatePollInput;

#[cfg(feature = "tcp")]
use crate::config::is_loopback_host;
use crate::config::Config;
use crate::errors::{ApiResult, HttpError};
use crate::routes::{self, AppState};
use crate::session::{MuteOption, UploadFile, ZaloSession};
use crate::types::{MentionInfo, ReactionChoice, ThreadKind, REACTIONS};

const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
const MAX_TEXT_LENGTH: usize = 4000;
const MAX_TTL_MS: u64 = 2_592_000_000;
const MAX_NOTE_TITLE: usize = 1000;
const MAX_FRIEND_MESSAGE: usize = 150;
const MAX_FORWARD_TARGETS: usize = 20;
const MAX_POLL_TEXT: usize = 200;
const MAX_POLL_OPTIONS: usize = 30;
#[cfg(feature = "tcp")]
const WS_MAX_PAYLOAD: usize = 64 * 1024;
#[cfg(feature = "tcp")]
const WS_HEARTBEAT_SECS: u64 = 30;

#[cfg(feature = "tcp")]
const STATIC_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https: http:; connect-src 'self' ws: wss:; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// Builds the complete Axum [`Router`] with all REST routes, `/ws`, Track 1/2 sub-routers,
/// optional static file serving (`config.public_dir`), and security middleware.
pub fn build_router(config: Arc<Config>, session: Arc<ZaloSession>) -> Router {
    build_app(config, session, true).router
}

/// The built application: the [`Router`] plus the call manager behind `/api/calls/*` and
/// `/ws/call-audio`, which an in-process embedder needs to open an audio link without a socket.
pub struct App {
    pub router: Router,
    pub calls: Arc<routes::calls::CallManager>,
}

/// Builds the application. With `network_facing` (and the `tcp` feature enabled), the
/// security middleware (Host, Origin and bearer-token checks, security headers) wraps every route,
/// as for the TCP server. Without it (the in-process embedded core, reachable only by the app
/// itself) those checks are skipped; body-size limits stay either way.
pub fn build_app(config: Arc<Config>, session: Arc<ZaloSession>, network_facing: bool) -> App {
    #[cfg(not(feature = "tcp"))]
    let _ = network_facing;

    let store = session.store().clone();
    let state = AppState {
        config: Arc::clone(&config),
        session,
        store,
    };

    let upload_routes = Router::new()
        .route(
            "/api/threads/{kind}/{id}/attachments",
            post(upload_attachment),
        )
        .route("/api/threads/{kind}/{id}/voice", post(upload_voice))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES));

    let (call_routes, calls) = routes::calls::router(state.clone());

    let json_routes = Router::new()
        // Session & auth
        .route("/api/status", get(get_status))
        .route("/api/login", post(post_login))
        .route("/api/retry", post(post_retry))
        .route("/api/logout", post(post_logout))
        .route("/api/reconnect", post(post_reconnect))
        .route("/api/listener/pause", post(post_listener_pause))
        .route("/api/listener/resume", post(post_listener_resume))
        .route(
            "/api/notify-window",
            put(put_notify_window).delete(delete_notify_window),
        )
        .route(
            "/api/debug/replay-log",
            get(get_replay_log).delete(clear_replay_log),
        )
        .route(
            "/api/debug/socket-log",
            get(get_socket_log).delete(clear_socket_log),
        )
        .route(
            "/api/debug/replay-mode",
            get(get_replay_mode).put(put_replay_mode),
        )
        .route("/api/reactions", get(get_reactions))
        // Conversations
        .route("/api/threads", get(list_threads))
        .route("/api/threads/{kind}/{id}/info", get(get_thread_info))
        .route("/api/threads/{kind}/{id}/read", post(post_mark_read))
        .route("/api/threads/{kind}/{id}/unread", post(post_mark_unread))
        .route("/api/threads/{kind}/{id}/pin", put(put_pin))
        .route("/api/threads/{kind}/{id}/mute", put(put_mute))
        .route("/api/threads/{kind}/{id}/typing", post(post_typing))
        .route("/api/threads/{kind}/{id}", delete(delete_thread))
        .route("/api/presence", put(put_presence))
        // Messages, cards, links & stickers
        .route(
            "/api/threads/{kind}/{id}/messages",
            get(get_messages).post(post_message),
        )
        .route("/api/threads/{kind}/{id}/card", post(post_card))
        .route("/api/threads/{kind}/{id}/link", post(post_link))
        .route("/api/search", get(get_search))
        .route("/api/stickers", get(get_stickers))
        .route("/api/threads/{kind}/{id}/stickers", post(post_sticker))
        .route(
            "/api/threads/{kind}/{id}/messages/{msgId}/reaction",
            put(put_reaction),
        )
        .route(
            "/api/threads/{kind}/{id}/messages/{msgId}/recall",
            post(post_recall),
        )
        .route(
            "/api/threads/{kind}/{id}/messages/{msgId}",
            delete(delete_message),
        )
        .route(
            "/api/threads/{kind}/{id}/messages/{msgId}/forward",
            post(post_forward),
        )
        // Friends & users
        .route("/api/friends", get(get_friends))
        .route(
            "/api/friends/{userId}/alias",
            put(put_friend_alias).delete(delete_friend_alias),
        )
        .route("/api/friends/{userId}", delete(delete_friend))
        .route(
            "/api/users/{userId}/block",
            post(post_block_user).delete(delete_unblock_user),
        )
        .route(
            "/api/friend-requests",
            get(get_friend_requests).post(post_friend_request),
        )
        .route("/api/friend-requests/sent", get(get_sent_friend_requests))
        .route(
            "/api/friend-requests/{userId}/accept",
            post(post_accept_friend),
        )
        .route(
            "/api/friend-requests/{userId}/reject",
            post(post_reject_friend),
        )
        .route(
            "/api/friend-requests/{userId}",
            delete(delete_friend_request),
        )
        .route("/api/users/find", get(get_find_user))
        // Groups, links, deputies, board & notes
        .route("/api/groups", post(post_create_group))
        .route("/api/groups/join-link", post(post_join_group_link))
        .route(
            "/api/groups/{id}/link",
            get(get_group_link)
                .post(post_enable_group_link)
                .delete(delete_disable_group_link),
        )
        .route("/api/groups/{id}/members", post(post_add_group_members))
        .route(
            "/api/groups/{id}/members/{userId}",
            delete(delete_group_member),
        )
        .route(
            "/api/groups/{id}/deputies/{userId}",
            post(post_add_group_deputy).delete(delete_remove_group_deputy),
        )
        .route("/api/groups/{id}/board", get(get_group_board))
        .route("/api/groups/{id}/notes", post(post_create_group_note))
        .route("/api/groups/{id}/name", put(put_group_name))
        .route("/api/groups/{id}/leave", post(post_leave_group))
        // Polls
        .route("/api/groups/{id}/polls", post(post_create_poll))
        .route("/api/polls/{pollId}", get(get_poll))
        .route("/api/polls/{pollId}/vote", post(post_vote_poll))
        .route("/api/polls/{pollId}/options", post(post_add_poll_options))
        .route("/api/polls/{pollId}/lock", post(post_lock_poll))
        // Modular Track 1 & Track 2 sub-routers
        .merge(call_routes)
        .merge(routes::history_sync::router(state.clone()))
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES));

    #[allow(unused_mut)]
    let mut router = Router::new()
        .merge(upload_routes)
        .merge(json_routes);

    #[cfg(feature = "tcp")]
    {
        router = router.route("/ws", get(ws_events));

        if let Some(ref public_dir) = config.public_dir {
            if public_dir.exists() {
                router = router.fallback_service(ServeDir::new(public_dir));
            }
        }

        if network_facing {
            router = router.layer(middleware::from_fn_with_state(
                state.clone(),
                security_middleware,
            ));
        }
    }

    App {
        router: router.with_state(state),
        calls,
    }
}

// ---- Security Middleware ----

#[cfg(feature = "tcp")]
async fn security_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let is_api = path == "/api" || path.starts_with("/api/");
    let is_ws = path == "/ws" || path.starts_with("/ws/");

    let host_header = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().authority().map(|a| a.as_str().to_string()));

    let mut response = if !is_valid_host(&state.config, host_header.as_deref()) {
        HttpError::forbidden("Forbidden").into_response()
    } else if is_api || is_ws {
        let is_mutation_or_ws =
            is_ws || !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
        if !is_allowed_origin(req.headers(), host_header.as_deref(), is_mutation_or_ws) {
            HttpError::forbidden("Forbidden").into_response()
        } else if !is_authorized(
            &state.config,
            req.headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok()),
            req.uri().query(),
        ) {
            HttpError::unauthorized("Unauthorized").into_response()
        } else {
            next.run(req).await
        }
    } else {
        next.run(req).await
    };

    apply_security_headers(response.headers_mut(), !is_api);
    response
}

#[cfg(feature = "tcp")]
fn apply_security_headers(headers: &mut HeaderMap, include_csp: bool) {
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if include_csp {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(STATIC_CSP),
        );
    }
}

#[cfg(feature = "tcp")]
fn has_bad_host_chars(host: &str) -> bool {
    host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '\\'))
}

#[cfg(feature = "tcp")]
fn is_valid_host(config: &Config, host_header: Option<&str>) -> bool {
    if !is_loopback_host(&config.host) {
        return true;
    }
    let Some(host) = host_header else {
        return false;
    };
    if has_bad_host_chars(host) {
        return false;
    }
    let Ok(url) = Url::parse(&format!("http://{host}")) else {
        return false;
    };
    url.host_str().is_some_and(is_loopback_host)
}

#[cfg(feature = "tcp")]
fn origin_matches_host(origin: &str, host_header: Option<&str>) -> bool {
    let Some(host) = host_header else {
        return false;
    };
    if has_bad_host_chars(host) {
        return false;
    }
    let Ok(origin_url) = Url::parse(origin) else {
        return false;
    };
    if origin_url.scheme() != "http" && origin_url.scheme() != "https" {
        return false;
    }
    let Ok(host_url) = Url::parse(&format!("{}://{host}", origin_url.scheme())) else {
        return false;
    };
    let origin_host = origin_url.host_str().map(|s| s.to_ascii_lowercase());
    let target_host = host_url.host_str().map(|s| s.to_ascii_lowercase());
    origin_host.is_some()
        && origin_host == target_host
        && origin_url.port_or_known_default() == host_url.port_or_known_default()
}

#[cfg(feature = "tcp")]
fn is_allowed_origin(
    headers: &HeaderMap,
    host_header: Option<&str>,
    is_mutation_or_ws: bool,
) -> bool {
    let sec_fetch_site = headers
        .get(HeaderName::from_static("sec-fetch-site"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();

    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::trim);

    if sec_fetch_site == "cross-site" {
        return false;
    }
    if let Some(orig) = origin {
        if !origin_matches_host(orig, host_header) {
            return false;
        }
    }
    if is_mutation_or_ws && sec_fetch_site == "same-site" {
        let Some(orig) = origin else {
            return false;
        };
        if !origin_matches_host(orig, host_header) {
            return false;
        }
    }
    true
}

#[cfg(feature = "tcp")]
fn is_authorized(
    config: &Config,
    auth_header: Option<&str>,
    query: Option<&str>,
) -> bool {
    let Some(expected) = config.access_token.as_deref() else {
        return true;
    };
    let presented: String = if let Some(h) = auth_header.and_then(|s| s.strip_prefix("Bearer ")) {
        h.to_string()
    } else if let Some(q) = query {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == "token")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let a = presented.as_bytes();
    let b = expected.as_bytes();
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

// ---- WebSocket `/ws` ----

#[cfg(feature = "tcp")]
async fn ws_events(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.max_message_size(WS_MAX_PAYLOAD)
        .on_upgrade(move |socket| handle_ws_events(socket, state))
}

pub use crate::embedded::EventStream;

#[cfg(feature = "tcp")]
async fn handle_ws_events(mut socket: WebSocket, state: AppState) {
    let mut events = EventStream::new(&state.session);

    if let Some(initial) = events.next_shared().await {
        if socket
            .send(Message::Text(initial.as_ref().into()))
            .await
            .is_err()
        {
            return;
        }
    }

    // Ping/pong keep-alive is a WebSocket-only concern.
    let mut heartbeat = tokio::time::interval(Duration::from_secs(WS_HEARTBEAT_SECS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await; // consume immediate tick
    let mut alive = true;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if !alive {
                    break;
                }
                alive = false;
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Pong(_))) => {
                        alive = true;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        alive = true;
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        break;
                    }
                    Some(Ok(_)) => {
                        alive = true;
                    }
                }
            }
            ev = events.next_shared() => {
                match ev {
                    Some(text) => {
                        if socket.send(Message::Text(text.as_ref().into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

// ---- Session & Reactions handlers ----

async fn get_status(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.session.get_status())
}

async fn post_login(State(state): State<AppState>) -> impl IntoResponse {
    state.session.start_qr_login();
    (StatusCode::ACCEPTED, Json(state.session.get_status()))
}

async fn post_retry(State(state): State<AppState>) -> impl IntoResponse {
    state.session.retry_now();
    (StatusCode::ACCEPTED, Json(state.session.get_status()))
}

async fn post_logout(State(state): State<AppState>) -> impl IntoResponse {
    state.session.logout().await;
    Json(state.session.get_status())
}

async fn post_reconnect(State(state): State<AppState>) -> ApiResult<impl IntoResponse> {
    state.session.reconnect()?;
    Ok((StatusCode::ACCEPTED, Json(state.session.get_status())))
}

/// Pauses the chat listener (no auto-reconnect while paused); login and credentials stay.
/// Does nothing when not logged in.
async fn post_listener_pause(State(state): State<AppState>) -> impl IntoResponse {
    state.session.pause_async().await;
    Json(state.session.get_status())
}

/// Starts the chat listener again after a pause; the replay and its report follow `Connected`.
async fn post_listener_resume(State(state): State<AppState>) -> impl IntoResponse {
    state.session.resume();
    (StatusCode::ACCEPTED, Json(state.session.get_status()))
}

/// Opens or extends the notify window: `{ "triggerTs": <epoch ms, default now>, "ttlMs": <default
/// 60000, max 600000> }`. While open, replayed messages newer than the trigger minus 2 minutes
/// can notify.
async fn put_notify_window(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let trigger = match val.get("triggerTs").filter(|v| !v.is_null()) {
        None => now,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| HttpError::bad_request("triggerTs must be epoch milliseconds"))?,
    };
    let ttl = match val.get("ttlMs").filter(|v| !v.is_null()) {
        None => 60_000,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| HttpError::bad_request("ttlMs must be a number of milliseconds"))?,
    };
    let w = state.session.set_notify_window(trigger, ttl);
    Ok(Json(json!({ "triggerTs": w.trigger_ts, "expiresAt": w.expires_at })))
}

async fn delete_notify_window(State(state): State<AppState>) -> StatusCode {
    state.session.clear_notify_window();
    StatusCode::NO_CONTENT
}

/// The replay reports as plain text, newest first (ids, counts and timestamps only).
async fn get_replay_log(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        state.session.replay().render_text(),
    )
}

async fn clear_replay_log(State(state): State<AppState>) -> StatusCode {
    state.session.replay().clear();
    StatusCode::NO_CONTENT
}

/// The socket log as plain text, newest first (frame structure only, never message text).
async fn get_socket_log(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        state.session.socket_log().render_text(),
    )
}

async fn clear_socket_log(State(state): State<AppState>) -> StatusCode {
    state.session.socket_log().clear();
    StatusCode::NO_CONTENT
}

async fn get_replay_mode(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "mode": state.session.replay().mode().as_str() }))
}

/// `{ "mode": "null" | "newest" | "page" }` (or a JSON `null` for the default).
async fn put_replay_mode(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let mode = val
        .get("mode")
        .and_then(crate::replay::ReplayMode::parse)
        .ok_or_else(|| HttpError::bad_request("mode must be \"null\", \"newest\" or \"page\""))?;
    state.session.replay().set_mode(mode);
    state.session.replay().flush().await;
    Ok(Json(json!({ "mode": mode.as_str() })))
}

async fn get_reactions() -> impl IntoResponse {
    let list: Vec<ReactionChoice> = REACTIONS
        .iter()
        .map(|r| ReactionChoice {
            key: r.key,
            emoji: r.emoji,
        })
        .collect();
    Json(list)
}

// ---- Conversations handlers ----

async fn list_threads(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.session.list_threads())
}

async fn get_thread_info(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let kind = parse_kind(&kind)?;
    let info = state.session.thread_info(kind, &id).await?;
    Ok(Json(info))
}

async fn post_mark_read(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    state.session.mark_read(kind, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_mark_unread(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    state.session.mark_unread(kind, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_pin(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let pinned = val
        .get("pinned")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| HttpError::bad_request("pinned must be a boolean"))?;
    state.session.set_pinned(kind, &id, pinned).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_mute(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let option = match val.get("duration") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(MuteOption::parse(s).ok_or_else(|| {
            HttpError::bad_request("duration must be one of 1h, 4h, 8am, forever or null")
        })?),
        _ => {
            return Err(HttpError::bad_request(
                "duration must be one of 1h, 4h, 8am, forever or null",
            ))
        }
    };
    state.session.set_muted(kind, &id, option).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_typing(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    state.session.send_typing(kind, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_thread(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    state.session.delete_thread(kind, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_presence(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let val = parse_json_body(&body)?;
    let raw_kind = val.get("kind").filter(|v| !v.is_null());
    let raw_id = val.get("id").filter(|v| !v.is_null());
    if raw_kind.is_none() && raw_id.is_none() {
        state.session.set_presence(None, None);
    } else {
        let id = raw_id
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HttpError::bad_request("id is required"))?;
        let kind_str = raw_kind.and_then(|v| v.as_str()).unwrap_or("");
        let kind = parse_kind(kind_str)?;
        state.session.set_presence(Some(kind), Some(id));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- Messages, uploads, search & stickers handlers ----

async fn upload_attachment(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let kind = parse_kind(&kind)?;
    let raw_name = query.get("name").map(|s| s.trim()).unwrap_or("");
    let name = Path::new(raw_name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() || !name.contains('.') {
        return Err(HttpError::bad_request(
            "name must be a file name with an extension",
        ));
    }
    if body.is_empty() {
        return Err(HttpError::bad_request("The request body is the file"));
    }
    let width = query
        .get("width")
        .and_then(|s| positive_usize(s))
        .and_then(|n| u32::try_from(n).ok());
    let height = query
        .get("height")
        .and_then(|s| positive_usize(s))
        .and_then(|n| u32::try_from(n).ok());
    let file = UploadFile {
        data: Vec::from(body),
        filename: name,
        width,
        height,
    };
    let msg_ids = state.session.send_attachment(kind, &id, file).await?;
    Ok((StatusCode::CREATED, Json(json!({ "msgIds": msg_ids }))))
}

async fn upload_voice(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    if body.is_empty() {
        return Err(HttpError::bad_request("Voice file is required"));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let (data, filename) = if content_type
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        let boundary = extract_multipart_boundary(content_type)
            .ok_or_else(|| HttpError::bad_request("Invalid multipart boundary"))?;
        let part = find_multipart_file(&body, &boundary, "voice")
            .ok_or_else(|| HttpError::bad_request("Voice file is required"))?;
        if part.0.is_empty() {
            return Err(HttpError::bad_request("Voice file is required"));
        }
        part
    } else {
        let name = query
            .get("name")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .and_then(|s| Path::new(s).file_name().and_then(|f| f.to_str()))
            .unwrap_or("voice.m4a")
            .to_string();
        (Vec::from(body), name)
    };
    state
        .session
        .send_voice(kind, &id, data, &filename)
        .await?;
    Ok(StatusCode::CREATED)
}

async fn get_messages(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let kind = parse_kind(&kind)?;
    let before = match query.get("before") {
        None => None,
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(HttpError::bad_request("before must be a finite number"));
            }
            let n: f64 = trimmed
                .parse()
                .map_err(|_| HttpError::bad_request("before must be a finite number"))?;
            if !n.is_finite() {
                return Err(HttpError::bad_request("before must be a finite number"));
            }
            Some(n as i64)
        }
    };
    let limit = match query.get("limit") {
        None => None,
        Some(raw) => Some(
            positive_usize(raw)
                .ok_or_else(|| HttpError::bad_request("limit must be a positive integer"))?,
        ),
    };
    let (thread, messages) = state
        .session
        .get_messages(kind, &id, before, limit)
        .await?;
    Ok(Json(json!({
        "thread": thread,
        "messages": messages,
    })))
}

async fn get_search(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let q = query.get("q").map(|s| s.trim()).unwrap_or("");
    if q.is_empty() {
        return Err(HttpError::bad_request("q is required"));
    }
    let limit = match query.get("limit") {
        None => 50,
        Some(raw) => positive_usize(raw)
            .ok_or_else(|| HttpError::bad_request("limit must be a positive integer"))?,
    };
    let results = state.session.search_messages(q, Some(limit));
    Ok(Json(json!({ "results": results })))
}

async fn get_stickers(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let q = query.get("q").map(|s| s.trim()).unwrap_or("");
    if q.is_empty() {
        return Err(HttpError::bad_request("q is required"));
    }
    let list = state.session.search_stickers(q).await?;
    Ok(Json(list))
}

async fn post_sticker(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let sticker_id = val
        .get("id")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .filter(|&n| n > 0)
        .ok_or_else(|| HttpError::bad_request("id must be a sticker id"))?;
    state.session.send_sticker(kind, &id, sticker_id).await?;
    Ok(StatusCode::CREATED)
}

async fn post_message(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let text = val
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if text.is_empty() {
        return Err(HttpError::bad_request("text is required"));
    }
    if text.chars().count() > MAX_TEXT_LENGTH {
        return Err(HttpError::bad_request(format!(
            "text is longer than {MAX_TEXT_LENGTH} characters"
        )));
    }
    let quote_id = match val.get("quoteId") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(other) => Some(other.to_string()),
    };
    let mentions = parse_mentions(val.get("mentions"), text)?;
    let ttl = parse_optional_ttl(val.get("ttl"))?;
    let msg_id = state
        .session
        .send_text_with_ttl(kind, &id, text, quote_id.as_deref(), mentions, ttl)
        .await?;
    Ok((StatusCode::CREATED, Json(json!({ "msgId": msg_id }))))
}

async fn post_card(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let user_id = val
        .get("userId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("userId is required"))?;
    let phone = match val.get("phone") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        _ => return Err(HttpError::bad_request("phone must be a string")),
    };
    let ttl = parse_optional_ttl(val.get("ttl"))?;
    state
        .session
        .send_card(kind, &id, user_id, phone, ttl)
        .await?;
    Ok(StatusCode::CREATED)
}

async fn post_link(
    State(state): State<AppState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let link = val
        .get("link")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("link is required"))?;
    let message = match val.get("message") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        _ => return Err(HttpError::bad_request("message must be a string")),
    };
    let ttl = parse_optional_ttl(val.get("ttl"))?;
    state
        .session
        .send_link(kind, &id, link, message, ttl)
        .await?;
    Ok(StatusCode::CREATED)
}

async fn put_reaction(
    State(state): State<AppState>,
    AxumPath((kind, id, msg_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let reaction = match val.get("reaction") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        _ => return Err(HttpError::bad_request("reaction must be a string or null")),
    };
    state
        .session
        .set_reaction(kind, &id, &msg_id, reaction)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_recall(
    State(state): State<AppState>,
    AxumPath((kind, id, msg_id)): AxumPath<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    state.session.recall_message(kind, &id, &msg_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_message(
    State(state): State<AppState>,
    AxumPath((kind, id, msg_id)): AxumPath<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    state.session.delete_message(kind, &id, &msg_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_forward(
    State(state): State<AppState>,
    AxumPath((kind, id, msg_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let kind = parse_kind(&kind)?;
    let val = parse_json_body(&body)?;
    let arr = val
        .get("to")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| HttpError::bad_request("to must be a non-empty array"))?;

    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for item in arr {
        let tid = item
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HttpError::bad_request("each target needs an id"))?;
        let tkind_str = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let tkind = parse_kind(tkind_str)?;
        let key = format!("{}:{tid}", tkind.as_str());
        if seen.insert(key) {
            targets.push((tkind, tid.to_string()));
            if targets.len() > MAX_FORWARD_TARGETS {
                return Err(HttpError::bad_request(format!(
                    "Cannot forward to more than {MAX_FORWARD_TARGETS} chats at once"
                )));
            }
        }
    }
    state
        .session
        .forward_message(kind, &id, &msg_id, &targets)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Friends handlers ----

async fn get_friends(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.session.friends())
}

async fn get_friend_requests(State(state): State<AppState>) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.session.friend_requests().await?))
}

async fn get_sent_friend_requests(State(state): State<AppState>) -> ApiResult<impl IntoResponse> {
    Ok(Json(state.session.sent_friend_requests().await?))
}

async fn post_accept_friend(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.accept_friend_request(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_reject_friend(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.reject_friend_request(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_friend_request(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let val = parse_json_body(&body)?;
    let user_id = val
        .get("userId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("userId is required"))?;
    let message: String = val
        .get("message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .chars()
        .take(MAX_FRIEND_MESSAGE)
        .collect();
    state
        .session
        .send_friend_request(user_id, &message)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_friend_request(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.undo_friend_request(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_find_user(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let raw = query.get("phone").map(String::as_str).unwrap_or("");
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '.' | '(' | ')' | '-'))
        .collect();
    let digits = cleaned.strip_prefix('+').unwrap_or(&cleaned);
    if !(8..=15).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(HttpError::bad_request("phone must be a phone number"));
    }
    let user = state.session.find_user(digits).await?;
    Ok(Json(user))
}

async fn put_friend_alias(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let val = parse_json_body(&body)?;
    let alias = val
        .get("alias")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("alias is required"))?;
    state.session.set_friend_alias(&user_id, alias).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_friend_alias(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.remove_friend_alias(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_friend(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.remove_friend(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_block_user(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.block_user(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_unblock_user(
    State(state): State<AppState>,
    AxumPath(user_id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.unblock_user(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Groups handlers ----

async fn post_create_group(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let members = parse_id_list(val.get("members"), "members")?;
    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let (group_id, failed) = state.session.create_group(name, &members).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "groupId": group_id, "failed": failed })),
    ))
}

async fn post_add_group_members(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let members = parse_id_list(val.get("members"), "members")?;
    let failed = state.session.add_group_members(&id, &members).await?;
    Ok(Json(json!({ "failed": failed })))
}

async fn delete_group_member(
    State(state): State<AppState>,
    AxumPath((id, user_id)): AxumPath<(String, String)>,
) -> ApiResult<StatusCode> {
    state.session.remove_group_member(&id, &user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_group_name(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let val = parse_json_body(&body)?;
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HttpError::bad_request("name must be a string"))?;
    state.session.rename_group(&id, name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_leave_group(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let val = parse_json_body(&body)?;
    let silent = val.get("silent").and_then(|v| v.as_bool()) == Some(true);
    state.session.leave_group(&id, silent).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_group_link(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    let info = state.session.get_group_link(&id).await?;
    Ok(Json(info))
}

async fn post_enable_group_link(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    let info = state.session.enable_group_link(&id).await?;
    Ok(Json(info))
}

async fn delete_disable_group_link(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    state.session.disable_group_link(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_join_group_link(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let link = val
        .get("link")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("link is required"))?;
    let group_id = state.session.join_group_link(link).await?;
    Ok(Json(json!({ "groupId": group_id })))
}

async fn post_add_group_deputy(
    State(state): State<AppState>,
    AxumPath((id, user_id)): AxumPath<(String, String)>,
) -> ApiResult<StatusCode> {
    state.session.add_group_deputy(&id, &user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_remove_group_deputy(
    State(state): State<AppState>,
    AxumPath((id, user_id)): AxumPath<(String, String)>,
) -> ApiResult<StatusCode> {
    state.session.remove_group_deputy(&id, &user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_group_board(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let page = match query.get("page") {
        None => None,
        Some(raw) => Some(
            positive_usize(raw)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| HttpError::bad_request("page must be a positive integer"))?,
        ),
    };
    let count = match query.get("count") {
        None => None,
        Some(raw) => Some(
            positive_usize(raw)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| HttpError::bad_request("count must be a positive integer"))?,
        ),
    };
    let items = state.session.get_group_board(&id, page, count).await?;
    Ok(Json(json!({ "items": items })))
}

async fn post_create_group_note(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let title = val
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("title is required"))?;
    if title.chars().count() > MAX_NOTE_TITLE {
        return Err(HttpError::bad_request(format!(
            "title is longer than {MAX_NOTE_TITLE} characters"
        )));
    }
    let pin = val.get("pin").and_then(|v| v.as_bool()).unwrap_or(false);
    let item = state.session.create_group_note(&id, title, pin).await?;
    Ok((StatusCode::CREATED, Json(item)))
}

// ---- Polls handlers ----

async fn post_create_poll(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let val = parse_json_body(&body)?;
    let question = val
        .get("question")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("question is required"))?;
    if question.chars().count() > MAX_POLL_TEXT {
        return Err(HttpError::bad_request(format!(
            "question is limited to {MAX_POLL_TEXT} characters"
        )));
    }
    let cleaned = clean_poll_options(val.get("options"));
    if cleaned.len() < 2 {
        return Err(HttpError::bad_request(
            "A poll needs at least 2 different options",
        ));
    }
    if cleaned.len() > MAX_POLL_OPTIONS {
        return Err(HttpError::bad_request(format!(
            "A poll takes at most {MAX_POLL_OPTIONS} options"
        )));
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let expires_at = match val.get("expiresAt") {
        None | Some(Value::Null) => 0i64,
        Some(Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            if !f.is_finite() {
                return Err(HttpError::bad_request(
                    "expiresAt must be a time in the future (epoch ms)",
                ));
            }
            if f == 0.0 {
                0
            } else if f <= now_ms as f64 {
                return Err(HttpError::bad_request(
                    "expiresAt must be a time in the future (epoch ms)",
                ));
            } else {
                f.floor() as i64
            }
        }
        _ => {
            return Err(HttpError::bad_request(
                "expiresAt must be a time in the future (epoch ms)",
            ))
        }
    };
    let flag = |k: &str| val.get(k).and_then(|v| v.as_bool()) == Some(true);
    let input = CreatePollInput {
        question: question.to_string(),
        options: cleaned,
        allow_multiple: flag("allowMultiple"),
        allow_add_options: flag("allowAddOptions"),
        anonymous: flag("anonymous"),
        hide_vote_preview: flag("hideVotePreview"),
        expires_at,
    };
    let poll = state.session.create_poll(&group_id, input).await?;
    Ok((StatusCode::CREATED, Json(poll)))
}

async fn get_poll(
    State(state): State<AppState>,
    AxumPath(poll_id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    let poll_id = parse_poll_id(&poll_id)?;
    Ok(Json(state.session.get_poll(poll_id).await?))
}

async fn post_vote_poll(
    State(state): State<AppState>,
    AxumPath(poll_id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let poll_id = parse_poll_id(&poll_id)?;
    let val = parse_json_body(&body)?;
    let arr = val
        .get("optionIds")
        .and_then(|v| v.as_array())
        .ok_or_else(|| HttpError::bad_request("optionIds must be an array of option ids"))?;
    if arr.len() > MAX_POLL_OPTIONS {
        return Err(HttpError::bad_request(
            "optionIds must be an array of option ids",
        ));
    }
    let mut seen = HashSet::new();
    let mut ids = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(id) = item.as_i64().filter(|&n| n >= 0) else {
            return Err(HttpError::bad_request(
                "optionIds must be an array of option ids",
            ));
        };
        if seen.insert(id) {
            ids.push(id);
        }
    }
    Ok(Json(state.session.vote_poll(poll_id, &ids).await?))
}

async fn post_add_poll_options(
    State(state): State<AppState>,
    AxumPath(poll_id): AxumPath<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let poll_id = parse_poll_id(&poll_id)?;
    let val = parse_json_body(&body)?;
    let cleaned = clean_poll_options(val.get("options"));
    if cleaned.is_empty() {
        return Err(HttpError::bad_request(
            "options must hold at least one option",
        ));
    }
    if cleaned.len() > MAX_POLL_OPTIONS {
        return Err(HttpError::bad_request(format!(
            "At most {MAX_POLL_OPTIONS} options at a time"
        )));
    }
    Ok(Json(
        state.session.add_poll_options(poll_id, &cleaned).await?,
    ))
}

async fn post_lock_poll(
    State(state): State<AppState>,
    AxumPath(poll_id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    let poll_id = parse_poll_id(&poll_id)?;
    Ok(Json(state.session.lock_poll(poll_id).await?))
}

// ---- Parsing helpers ----

fn parse_json_body(body: &Bytes) -> ApiResult<Value> {
    if body.is_empty() || body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(json!({}));
    }
    serde_json::from_slice::<Value>(body)
        .map_err(|e| HttpError::bad_request(format!("Invalid JSON body: {e}")))
}

fn parse_kind(kind: &str) -> ApiResult<ThreadKind> {
    kind.parse::<ThreadKind>()
        .map_err(|_| HttpError::bad_request(format!("Unknown thread kind: {kind}")))
}

fn parse_poll_id(value: &str) -> ApiResult<i64> {
    value
        .parse::<i64>()
        .ok()
        .filter(|&id| id > 0)
        .ok_or_else(|| HttpError::bad_request("Invalid poll id"))
}

fn parse_optional_ttl(val: Option<&Value>) -> ApiResult<Option<u64>> {
    match val {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            let ms = n
                .as_u64()
                .ok_or_else(|| HttpError::bad_request("ttl must be a non-negative integer"))?;
            if ms > MAX_TTL_MS {
                return Err(HttpError::bad_request(format!(
                    "ttl must be at most {MAX_TTL_MS} milliseconds"
                )));
            }
            Ok(Some(ms))
        }
        _ => Err(HttpError::bad_request("ttl must be a non-negative integer")),
    }
}

fn positive_usize(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<usize>().ok().filter(|&n| n > 0)
}

fn parse_id_list(val: Option<&Value>, field: &str) -> ApiResult<Vec<String>> {
    let arr = val
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| HttpError::bad_request(format!("{field} must be a non-empty array of user ids")))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(s) = item.as_str().filter(|s| !s.is_empty()) else {
            return Err(HttpError::bad_request(format!(
                "{field} must be a non-empty array of user ids"
            )));
        };
        out.push(s.to_string());
    }
    Ok(out)
}

fn parse_mentions(input: Option<&Value>, text: &str) -> ApiResult<Option<Vec<MentionInfo>>> {
    let Some(val) = input else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let arr = val
        .as_array()
        .ok_or_else(|| HttpError::bad_request("mentions must be an array"))?;
    let text_utf16_len = text.encode_utf16().count() as i64;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item
            .as_object()
            .ok_or_else(|| HttpError::bad_request("each mention needs uid, pos and len"))?;
        let uid = obj
            .get("uid")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HttpError::bad_request("each mention needs uid, pos and len"))?;
        let pos = obj
            .get("pos")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| HttpError::bad_request("each mention needs uid, pos and len"))?;
        let len = obj
            .get("len")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| HttpError::bad_request("each mention needs uid, pos and len"))?;
        if pos < 0 || len <= 0 || pos + len > text_utf16_len {
            return Err(HttpError::bad_request("mention is outside the text"));
        }
        out.push(MentionInfo {
            uid: uid.to_string(),
            pos: pos as i32,
            len: len as i32,
        });
    }
    Ok(Some(out))
}

fn clean_poll_options(val: Option<&Value>) -> Vec<String> {
    let Some(arr) = val.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in arr {
        if let Some(s) = item.as_str() {
            let trimmed: String = s.trim().chars().take(MAX_POLL_TEXT).collect();
            if !trimmed.is_empty() && seen.insert(trimmed.clone()) {
                out.push(trimmed);
            }
        }
    }
    out
}

// ---- Multipart voice parser (matching src/server.ts) ----

fn extract_multipart_boundary(content_type: &str) -> Option<String> {
    for part in content_type.split(';').skip(1) {
        let trimmed = part.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("boundary=") {
            let val = trimmed["boundary=".len()..].trim();
            let unquoted = val
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(val);
            if !unquoted.is_empty() {
                return Some(unquoted.to_string());
            }
        }
    }
    None
}

fn find_bytes(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || start + needle.len() > haystack.len() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| start + p)
}

fn extract_disposition_param(line: &str, param: &str) -> Option<String> {
    let target = format!("{param}=");
    for part in line.split(';').skip(1) {
        let trimmed = part.trim();
        if trimmed.len() >= target.len()
            && trimmed[..target.len()].eq_ignore_ascii_case(&target)
        {
            let val = trimmed[target.len()..].trim();
            let unquoted = val
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(val);
            return Some(unquoted.to_string());
        }
    }
    None
}

fn find_multipart_file(
    body: &[u8],
    boundary: &str,
    field_name: &str,
) -> Option<(Vec<u8>, String)> {
    let delim = format!("--{boundary}");
    let delim_bytes = delim.as_bytes();
    let mut pos = find_bytes(body, delim_bytes, 0)?;

    loop {
        pos += delim_bytes.len();
        if pos + 1 < body.len() && body[pos] == b'-' && body[pos + 1] == b'-' {
            break;
        }
        if pos + 1 < body.len() && body[pos] == b'\r' && body[pos + 1] == b'\n' {
            pos += 2;
        } else if pos < body.len() && body[pos] == b'\n' {
            pos += 1;
        } else {
            break;
        }

        let crlf_idx = find_bytes(body, b"\r\n\r\n", pos);
        let lf_idx = find_bytes(body, b"\n\n", pos);
        let (header_end, sep_len) = match (crlf_idx, lf_idx) {
            (Some(c), Some(l)) if c < l => (c, 4),
            (Some(c), None) => (c, 4),
            (_, Some(l)) => (l, 2),
            (None, None) => break,
        };

        let headers_text = String::from_utf8_lossy(&body[pos..header_end]);
        let data_start = header_end + sep_len;
        let next_boundary = find_bytes(body, delim_bytes, data_start)?;

        let mut data_end = next_boundary;
        if data_end >= data_start + 2
            && body[data_end - 2] == b'\r'
            && body[data_end - 1] == b'\n'
        {
            data_end -= 2;
        } else if data_end >= data_start + 1 && body[data_end - 1] == b'\n' {
            data_end -= 1;
        }

        if let Some(disp_line) = headers_text
            .lines()
            .find(|line| line.trim_start().to_ascii_lowercase().starts_with("content-disposition:"))
        {
            if let Some(name) = extract_disposition_param(disp_line, "name") {
                if name == field_name {
                    let filename = extract_disposition_param(disp_line, "filename")
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .and_then(|s| {
                            Path::new(&s)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "voice.m4a".to_string());
                    return Some((body[data_start..data_end].to_vec(), filename));
                }
            }
        }
        pos = next_boundary;
    }
    None
}
