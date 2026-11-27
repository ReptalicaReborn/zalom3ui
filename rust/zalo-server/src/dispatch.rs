//! Direct in-process REST API dispatcher used by [`crate::embedded::EmbeddedCore`].
//!
//! Runs every `/api/*` route (core routes, Track 1 calls, Track 2 history sync, and debug logs)
//! directly against `ZaloSession`, `CallManager`, and `HistorySyncManager` with zero `axum`,
//! `tower`, or `http` dependencies.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use serde::Serialize;
use serde_json::{json, Map, Value};
use zalo_proto::time_util::now_ms;
use zalo_proto::CreatePollInput;

use crate::embedded::EmbeddedResponse;
use crate::errors::{ApiResult, HttpError};
use crate::routes::calls::{CallManager, ANSWER_TEST_SECONDS};
use crate::routes::history_sync::HistorySyncManager;
use crate::routes::AppState;
use crate::session::{MuteOption, UploadFile};
use crate::types::{MentionInfo, ReactionChoice, ThreadKind, REACTIONS};

pub(crate) const MAX_JSON_BYTES: usize = 64 * 1024;
pub(crate) const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
const MAX_TEXT_LENGTH: usize = 4000;
const MAX_TTL_MS: u64 = 2_592_000_000;
const MAX_NOTE_TITLE: usize = 1000;
const MAX_FRIEND_MESSAGE: usize = 150;
const MAX_FORWARD_TARGETS: usize = 20;
const MAX_POLL_TEXT: usize = 200;
const MAX_POLL_OPTIONS: usize = 30;

const CT_JSON: &str = "application/json";
const CT_TEXT: &str = "text/plain; charset=utf-8";

/// Parsed method, percent-decoded path segments, and query parameters for one embedded request.
pub(crate) struct ParsedRequest {
    pub method: String,
    pub segments: Vec<String>,
    pub query: HashMap<String, String>,
}

fn is_valid_method_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Validates and parses an embedded request's method and `path_and_query`.
///
/// Returns `Err` only for malformed methods or paths (matching `http::Method` / `http::Uri`
/// parsing errors).
pub(crate) fn parse_request(method: &str, path_and_query: &str) -> Result<ParsedRequest, String> {
    let trimmed_method = method.trim().to_ascii_uppercase();
    if trimmed_method.is_empty() || !trimmed_method.bytes().all(is_valid_method_byte) {
        return Err(format!("Invalid method: {method}"));
    }

    if !path_and_query.starts_with('/')
        || path_and_query.bytes().any(|b| b <= 0x20 || b == 0x7f)
    {
        return Err(format!("Invalid path {path_and_query:?}"));
    }

    let (raw_path, raw_query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };

    let segments = if raw_path == "/" {
        Vec::new()
    } else {
        let mut out = Vec::new();
        for seg in raw_path[1..].split('/') {
            let decoded = percent_encoding::percent_decode_str(seg)
                .decode_utf8()
                .map_err(|e| format!("Invalid path encoding in {path_and_query:?}: {e}"))?;
            out.push(decoded.into_owned());
        }
        out
    };

    let mut query = HashMap::new();
    if let Some(q) = raw_query {
        for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
            query.insert(k.into_owned(), v.into_owned());
        }
    }

    Ok(ParsedRequest {
        method: trimmed_method,
        segments,
        query,
    })
}

/// Body byte limit for the given parsed request (100 MiB for attachment/voice uploads, 64 KiB
/// for everything else).
pub(crate) fn body_limit_for(req: &ParsedRequest) -> usize {
    let segs: Vec<&str> = req.segments.iter().map(String::as_str).collect();
    if req.method == "POST"
        && matches!(
            segs.as_slice(),
            ["api", "threads", _, _, "attachments" | "voice"]
        )
    {
        MAX_UPLOAD_BYTES
    } else {
        MAX_JSON_BYTES
    }
}

pub(crate) fn payload_too_large() -> EmbeddedResponse {
    EmbeddedResponse {
        status: 413,
        content_type: CT_TEXT.to_string(),
        body: Bytes::from_static(b"length limit exceeded"),
    }
}

fn json_response(status: u16, value: &impl Serialize) -> EmbeddedResponse {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    EmbeddedResponse {
        status,
        content_type: CT_JSON.to_string(),
        body: Bytes::from(body),
    }
}

fn text_response(status: u16, text: String) -> EmbeddedResponse {
    EmbeddedResponse {
        status,
        content_type: CT_TEXT.to_string(),
        body: Bytes::from(text),
    }
}

fn empty_response(status: u16) -> EmbeddedResponse {
    EmbeddedResponse {
        status,
        content_type: String::new(),
        body: Bytes::new(),
    }
}

fn error_response(err: HttpError) -> EmbeddedResponse {
    let status = if (100..=599).contains(&err.status) {
        err.status
    } else {
        500
    };
    json_response(status, &json!({ "error": err.message }))
}

fn from_api_result(res: ApiResult<EmbeddedResponse>) -> EmbeddedResponse {
    match res {
        Ok(resp) => resp,
        Err(err) => error_response(err),
    }
}

/// Dispatches one parsed request directly to the matching handler.
pub(crate) async fn dispatch_parsed(
    state: &AppState,
    calls: &Arc<CallManager>,
    history_sync: &Arc<HistorySyncManager>,
    req: ParsedRequest,
    content_type: Option<&str>,
    body: Bytes,
) -> EmbeddedResponse {
    if body.len() > body_limit_for(&req) {
        return payload_too_large();
    }
    from_api_result(dispatch_parsed_inner(state, calls, history_sync, req, content_type, body).await)
}

async fn dispatch_parsed_inner(
    state: &AppState,
    calls: &Arc<CallManager>,
    history_sync: &Arc<HistorySyncManager>,
    req: ParsedRequest,
    content_type: Option<&str>,
    body: Bytes,
) -> ApiResult<EmbeddedResponse> {
    let method = req.method.as_str();
    let segs: Vec<&str> = req.segments.iter().map(String::as_str).collect();
    let query = &req.query;

    match segs.as_slice() {
        // ---- Session & auth ----
        ["api", "status"] => match method {
            "GET" => Ok(json_response(200, &state.session.get_status())),
            _ => Ok(empty_response(405)),
        },
        ["api", "login"] => match method {
            "POST" => {
                state.session.start_qr_login();
                Ok(json_response(202, &state.session.get_status()))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "retry"] => match method {
            "POST" => {
                state.session.retry_now();
                Ok(json_response(202, &state.session.get_status()))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "logout"] => match method {
            "POST" => {
                state.session.logout().await;
                Ok(json_response(200, &state.session.get_status()))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "reconnect"] => match method {
            "POST" => {
                state.session.reconnect()?;
                Ok(json_response(202, &state.session.get_status()))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "listener", "pause"] => match method {
            "POST" => {
                state.session.pause_async().await;
                Ok(json_response(200, &state.session.get_status()))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "listener", "resume"] => match method {
            "POST" => {
                state.session.resume();
                Ok(json_response(202, &state.session.get_status()))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "notify-window"] => match method {
            "PUT" => handle_put_notify_window(state, &body),
            "DELETE" => {
                state.session.clear_notify_window();
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "debug", "replay-log"] => match method {
            "GET" => Ok(text_response(200, state.session.replay().render_text())),
            "DELETE" => {
                state.session.replay().clear();
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "debug", "socket-log"] => match method {
            "GET" => Ok(text_response(200, state.session.socket_log().render_text())),
            "DELETE" => {
                state.session.socket_log().clear();
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "debug", "replay-mode"] => match method {
            "GET" => Ok(json_response(
                200,
                &json!({ "mode": state.session.replay().mode().as_str() }),
            )),
            "PUT" => handle_put_replay_mode(state, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "debug", "call-log"] => match method {
            "GET" => Ok(text_response(
                200,
                state.session.call_trace().read_async().await,
            )),
            "DELETE" => {
                state.session.call_trace().clear_async().await;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "reactions"] => match method {
            "GET" => {
                let list: Vec<ReactionChoice> = REACTIONS
                    .iter()
                    .map(|r| ReactionChoice {
                        key: r.key,
                        emoji: r.emoji,
                    })
                    .collect();
                Ok(json_response(200, &list))
            }
            _ => Ok(empty_response(405)),
        },

        // ---- Conversations ----
        ["api", "threads"] => match method {
            "GET" => Ok(json_response(200, &state.session.list_threads())),
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id] => match method {
            "DELETE" => {
                let kind = parse_kind(kind)?;
                state.session.delete_thread(kind, id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "info"] => match method {
            "GET" => {
                let kind = parse_kind(kind)?;
                let info = state.session.thread_info(kind, id).await?;
                Ok(json_response(200, &info))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "read"] => match method {
            "POST" => {
                let kind = parse_kind(kind)?;
                state.session.mark_read(kind, id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "unread"] => match method {
            "POST" => {
                let kind = parse_kind(kind)?;
                state.session.mark_unread(kind, id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "pin"] => match method {
            "PUT" => handle_put_pin(state, kind, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "mute"] => match method {
            "PUT" => handle_put_mute(state, kind, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "typing"] => match method {
            "POST" => {
                let kind = parse_kind(kind)?;
                state.session.send_typing(kind, id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "presence"] => match method {
            "PUT" => handle_put_presence(state, &body),
            _ => Ok(empty_response(405)),
        },

        // ---- Messages, uploads, cards, links, search & stickers ----
        ["api", "threads", kind, id, "attachments"] => match method {
            "POST" => handle_upload_attachment(state, kind, id, query, body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "voice"] => match method {
            "POST" => handle_upload_voice(state, kind, id, query, content_type, body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "messages"] => match method {
            "GET" => handle_get_messages(state, kind, id, query).await,
            "POST" => handle_post_message(state, kind, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "card"] => match method {
            "POST" => handle_post_card(state, kind, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "link"] => match method {
            "POST" => handle_post_link(state, kind, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "stickers"] => match method {
            "POST" => handle_post_sticker(state, kind, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "messages", msg_id] => match method {
            "DELETE" => {
                let kind = parse_kind(kind)?;
                state.session.delete_message(kind, id, msg_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "messages", msg_id, "reaction"] => match method {
            "PUT" => handle_put_reaction(state, kind, id, msg_id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "messages", msg_id, "recall"] => match method {
            "POST" => {
                let kind = parse_kind(kind)?;
                state.session.recall_message(kind, id, msg_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "threads", kind, id, "messages", msg_id, "forward"] => match method {
            "POST" => handle_post_forward(state, kind, id, msg_id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "search"] => match method {
            "GET" => handle_get_search(state, query),
            _ => Ok(empty_response(405)),
        },
        ["api", "stickers"] => match method {
            "GET" => handle_get_stickers(state, query).await,
            _ => Ok(empty_response(405)),
        },

        // ---- Friends & users ----
        ["api", "friends"] => match method {
            "GET" => Ok(json_response(200, &state.session.friends())),
            _ => Ok(empty_response(405)),
        },
        ["api", "friends", user_id] => match method {
            "DELETE" => {
                state.session.remove_friend(user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "friends", user_id, "alias"] => match method {
            "PUT" => handle_put_friend_alias(state, user_id, &body).await,
            "DELETE" => {
                state.session.remove_friend_alias(user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "users", "find"] => match method {
            "GET" => handle_get_find_user(state, query).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "users", user_id, "block"] => match method {
            "POST" => {
                state.session.block_user(user_id).await?;
                Ok(empty_response(204))
            }
            "DELETE" => {
                state.session.unblock_user(user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "friend-requests"] => match method {
            "GET" => Ok(json_response(200, &state.session.friend_requests().await?)),
            "POST" => handle_post_friend_request(state, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "friend-requests", "sent"] => match method {
            "GET" => Ok(json_response(
                200,
                &state.session.sent_friend_requests().await?,
            )),
            "DELETE" => {
                state.session.undo_friend_request("sent").await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "friend-requests", user_id] => match method {
            "DELETE" => {
                state.session.undo_friend_request(user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "friend-requests", user_id, "accept"] => match method {
            "POST" => {
                state.session.accept_friend_request(user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "friend-requests", user_id, "reject"] => match method {
            "POST" => {
                state.session.reject_friend_request(user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },

        // ---- Groups, links, deputies, board & notes ----
        ["api", "groups"] => match method {
            "POST" => handle_post_create_group(state, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", "join-link"] => match method {
            "POST" => handle_post_join_group_link(state, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "link"] => match method {
            "GET" => Ok(json_response(200, &state.session.get_group_link(id).await?)),
            "POST" => Ok(json_response(
                200,
                &state.session.enable_group_link(id).await?,
            )),
            "DELETE" => {
                state.session.disable_group_link(id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "members"] => match method {
            "POST" => handle_post_add_group_members(state, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "members", user_id] => match method {
            "DELETE" => {
                state.session.remove_group_member(id, user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "deputies", user_id] => match method {
            "POST" => {
                state.session.add_group_deputy(id, user_id).await?;
                Ok(empty_response(204))
            }
            "DELETE" => {
                state.session.remove_group_deputy(id, user_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "board"] => match method {
            "GET" => handle_get_group_board(state, id, query).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "notes"] => match method {
            "POST" => handle_post_create_group_note(state, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "name"] => match method {
            "PUT" => handle_put_group_name(state, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "groups", id, "leave"] => match method {
            "POST" => handle_post_leave_group(state, id, &body).await,
            _ => Ok(empty_response(405)),
        },

        // ---- Polls ----
        ["api", "groups", id, "polls"] => match method {
            "POST" => handle_post_create_poll(state, id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "polls", poll_id] => match method {
            "GET" => {
                let poll_id = parse_poll_id(poll_id)?;
                Ok(json_response(200, &state.session.get_poll(poll_id).await?))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "polls", poll_id, "vote"] => match method {
            "POST" => handle_post_vote_poll(state, poll_id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "polls", poll_id, "options"] => match method {
            "POST" => handle_post_add_poll_options(state, poll_id, &body).await,
            _ => Ok(empty_response(405)),
        },
        ["api", "polls", poll_id, "lock"] => match method {
            "POST" => {
                let poll_id = parse_poll_id(poll_id)?;
                Ok(json_response(200, &state.session.lock_poll(poll_id).await?))
            }
            _ => Ok(empty_response(405)),
        },

        // ---- Calls (Track 1) ----
        ["api", "calls", "current"] => match method {
            "GET" => match calls.current_call() {
                Some(call) => Ok(json_response(200, &call)),
                None => Ok(json_response(404, &json!({ "error": "No active call" }))),
            },
            _ => Ok(empty_response(405)),
        },
        ["api", "calls", "start"] => match method {
            "POST" => {
                let val = parse_json_object_body(&body)?;
                let user_id = val.get("userId").and_then(|v| v.as_str()).unwrap_or("");
                let video = val.get("video").and_then(|v| v.as_bool()).unwrap_or(false);
                let call = calls.start_call(user_id, video, None, None).await?;
                Ok(json_response(200, &call))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "calls", "hangup"] => match method {
            "POST" => {
                calls.hang_up_call()?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "calls", "mute"] => match method {
            "POST" => {
                let val = parse_json_object_body(&body)?;
                let muted = val.get("muted").and_then(|v| v.as_bool()).unwrap_or(false);
                calls.set_call_muted(muted)?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "calls", "camera"] => match method {
            "POST" => {
                let val = parse_json_object_body(&body)?;
                let on = val.get("on").and_then(|v| v.as_bool()).unwrap_or(false);
                calls.set_call_camera(on)?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "calls", call_id, "decline"] => match method {
            "POST" => {
                calls.decline_call(call_id).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "calls", call_id, "answer"] => match method {
            "POST" => {
                let _ = calls.answer_call(call_id, None).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "calls", call_id, "answer-test"] => match method {
            "POST" => {
                let _ = calls.answer_call(call_id, Some(ANSWER_TEST_SECONDS)).await?;
                Ok(empty_response(204))
            }
            _ => Ok(empty_response(405)),
        },

        // ---- History sync (Track 2) ----
        ["api", "history-sync", "status"] => match method {
            "GET" => {
                let status = history_sync.get_status();
                let log = history_sync.get_log();
                let mut val = serde_json::to_value(&status).unwrap_or_else(|_| json!({}));
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("log".to_string(), json!(log));
                }
                Ok(json_response(200, &val))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "history-sync", "start"] => match method {
            "POST" => {
                let (was_active, status) = history_sync.start().await?;
                let code = if was_active { 200 } else { 202 };
                Ok(json_response(code, &status))
            }
            _ => Ok(empty_response(405)),
        },
        ["api", "history-sync", "cancel"] => match method {
            "POST" => {
                history_sync.cancel();
                Ok(json_response(200, &history_sync.get_status()))
            }
            _ => Ok(empty_response(405)),
        },

        _ => Ok(empty_response(404)),
    }
}

// ---- Individual handler helpers ----

fn handle_put_notify_window(state: &AppState, body: &Bytes) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let now = now_ms();
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
    Ok(json_response(
        200,
        &json!({ "triggerTs": w.trigger_ts, "expiresAt": w.expires_at }),
    ))
}

async fn handle_put_replay_mode(state: &AppState, body: &Bytes) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let mode = val
        .get("mode")
        .and_then(crate::replay::ReplayMode::parse)
        .ok_or_else(|| HttpError::bad_request("mode must be \"null\", \"newest\" or \"page\""))?;
    state.session.replay().set_mode(mode);
    state.session.replay().flush().await;
    Ok(json_response(200, &json!({ "mode": mode.as_str() })))
}

async fn handle_put_pin(
    state: &AppState,
    kind: &str,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
    let pinned = val
        .get("pinned")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| HttpError::bad_request("pinned must be a boolean"))?;
    state.session.set_pinned(kind, id, pinned).await?;
    Ok(empty_response(204))
}

async fn handle_put_mute(
    state: &AppState,
    kind: &str,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
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
    state.session.set_muted(kind, id, option).await?;
    Ok(empty_response(204))
}

fn handle_put_presence(state: &AppState, body: &Bytes) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
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
    Ok(empty_response(204))
}

async fn handle_upload_attachment(
    state: &AppState,
    kind: &str,
    id: &str,
    query: &HashMap<String, String>,
    body: Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
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
    let msg_ids = state.session.send_attachment(kind, id, file).await?;
    Ok(json_response(201, &json!({ "msgIds": msg_ids })))
}

async fn handle_upload_voice(
    state: &AppState,
    kind: &str,
    id: &str,
    query: &HashMap<String, String>,
    content_type: Option<&str>,
    body: Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    if body.is_empty() {
        return Err(HttpError::bad_request("Voice file is required"));
    }
    let ct = content_type.unwrap_or("");
    let (data, filename) = if ct
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        let boundary = extract_multipart_boundary(ct)
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
        .send_voice(kind, id, data, &filename)
        .await?;
    Ok(empty_response(201))
}

async fn handle_get_messages(
    state: &AppState,
    kind: &str,
    id: &str,
    query: &HashMap<String, String>,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
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
        .get_messages(kind, id, before, limit)
        .await?;
    Ok(json_response(
        200,
        &json!({ "thread": thread, "messages": messages }),
    ))
}

async fn handle_post_message(
    state: &AppState,
    kind: &str,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
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
        .send_text_with_ttl(kind, id, text, quote_id.as_deref(), mentions, ttl)
        .await?;
    Ok(json_response(201, &json!({ "msgId": msg_id })))
}

async fn handle_post_card(
    state: &AppState,
    kind: &str,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
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
        .send_card(kind, id, user_id, phone, ttl)
        .await?;
    Ok(empty_response(201))
}

async fn handle_post_link(
    state: &AppState,
    kind: &str,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
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
        .send_link(kind, id, link, message, ttl)
        .await?;
    Ok(empty_response(201))
}

async fn handle_post_sticker(
    state: &AppState,
    kind: &str,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
    let sticker_id = val
        .get("id")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .filter(|&n| n > 0)
        .ok_or_else(|| HttpError::bad_request("id must be a sticker id"))?;
    state.session.send_sticker(kind, id, sticker_id).await?;
    Ok(empty_response(201))
}

async fn handle_put_reaction(
    state: &AppState,
    kind: &str,
    id: &str,
    msg_id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
    let reaction = match val.get("reaction") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        _ => return Err(HttpError::bad_request("reaction must be a string or null")),
    };
    state
        .session
        .set_reaction(kind, id, msg_id, reaction)
        .await?;
    Ok(empty_response(204))
}

async fn handle_post_forward(
    state: &AppState,
    kind: &str,
    id: &str,
    msg_id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let kind = parse_kind(kind)?;
    let val = parse_json_body(body)?;
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
        .forward_message(kind, id, msg_id, &targets)
        .await?;
    Ok(empty_response(204))
}

fn handle_get_search(
    state: &AppState,
    query: &HashMap<String, String>,
) -> ApiResult<EmbeddedResponse> {
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
    Ok(json_response(200, &json!({ "results": results })))
}

async fn handle_get_stickers(
    state: &AppState,
    query: &HashMap<String, String>,
) -> ApiResult<EmbeddedResponse> {
    let q = query.get("q").map(|s| s.trim()).unwrap_or("");
    if q.is_empty() {
        return Err(HttpError::bad_request("q is required"));
    }
    let list = state.session.search_stickers(q).await?;
    Ok(json_response(200, &list))
}

async fn handle_put_friend_alias(
    state: &AppState,
    user_id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let alias = val
        .get("alias")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("alias is required"))?;
    state.session.set_friend_alias(user_id, alias).await?;
    Ok(empty_response(204))
}

async fn handle_get_find_user(
    state: &AppState,
    query: &HashMap<String, String>,
) -> ApiResult<EmbeddedResponse> {
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
    Ok(json_response(200, &user))
}

async fn handle_post_friend_request(
    state: &AppState,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
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
    Ok(empty_response(204))
}

async fn handle_post_create_group(
    state: &AppState,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let members = parse_id_list(val.get("members"), "members")?;
    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let (group_id, failed) = state.session.create_group(name, &members).await?;
    Ok(json_response(
        201,
        &json!({ "groupId": group_id, "failed": failed }),
    ))
}

async fn handle_post_join_group_link(
    state: &AppState,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let link = val
        .get("link")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HttpError::bad_request("link is required"))?;
    let group_id = state.session.join_group_link(link).await?;
    Ok(json_response(200, &json!({ "groupId": group_id })))
}

async fn handle_post_add_group_members(
    state: &AppState,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let members = parse_id_list(val.get("members"), "members")?;
    let failed = state.session.add_group_members(id, &members).await?;
    Ok(json_response(200, &json!({ "failed": failed })))
}

async fn handle_get_group_board(
    state: &AppState,
    id: &str,
    query: &HashMap<String, String>,
) -> ApiResult<EmbeddedResponse> {
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
    let items = state.session.get_group_board(id, page, count).await?;
    Ok(json_response(200, &json!({ "items": items })))
}

async fn handle_post_create_group_note(
    state: &AppState,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
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
    let item = state.session.create_group_note(id, title, pin).await?;
    Ok(json_response(201, &item))
}

async fn handle_put_group_name(
    state: &AppState,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HttpError::bad_request("name must be a string"))?;
    state.session.rename_group(id, name).await?;
    Ok(empty_response(204))
}

async fn handle_post_leave_group(
    state: &AppState,
    id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
    let silent = val.get("silent").and_then(|v| v.as_bool()) == Some(true);
    state.session.leave_group(id, silent).await?;
    Ok(empty_response(204))
}

async fn handle_post_create_poll(
    state: &AppState,
    group_id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let val = parse_json_body(body)?;
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
    let now = now_ms();
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
            } else if f <= now as f64 {
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
    let poll = state.session.create_poll(group_id, input).await?;
    Ok(json_response(201, &poll))
}

async fn handle_post_vote_poll(
    state: &AppState,
    poll_id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let poll_id = parse_poll_id(poll_id)?;
    let val = parse_json_body(body)?;
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
    Ok(json_response(
        200,
        &state.session.vote_poll(poll_id, &ids).await?,
    ))
}

async fn handle_post_add_poll_options(
    state: &AppState,
    poll_id: &str,
    body: &Bytes,
) -> ApiResult<EmbeddedResponse> {
    let poll_id = parse_poll_id(poll_id)?;
    let val = parse_json_body(body)?;
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
    Ok(json_response(
        200,
        &state.session.add_poll_options(poll_id, &cleaned).await?,
    ))
}

// ---- Shared parsing helpers ----

pub(crate) fn parse_json_body(body: &Bytes) -> ApiResult<Value> {
    if body.is_empty() || body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(json!({}));
    }
    serde_json::from_slice::<Value>(body)
        .map_err(|e| HttpError::bad_request(format!("Invalid JSON body: {e}")))
}

fn parse_json_object_body(body: &Bytes) -> ApiResult<Value> {
    if body.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let val: Value =
        serde_json::from_slice(body).map_err(|_| HttpError::bad_request("Invalid JSON body"))?;
    if !val.is_object() {
        return Err(HttpError::bad_request("Request body must be a JSON object"));
    }
    Ok(val)
}

pub(crate) fn parse_kind(kind: &str) -> ApiResult<ThreadKind> {
    kind.parse::<ThreadKind>()
        .map_err(|_| HttpError::bad_request(format!("Unknown thread kind: {kind}")))
}

pub(crate) fn parse_poll_id(value: &str) -> ApiResult<i64> {
    value
        .parse::<i64>()
        .ok()
        .filter(|&id| id > 0)
        .ok_or_else(|| HttpError::bad_request("Invalid poll id"))
}

pub(crate) fn parse_optional_ttl(val: Option<&Value>) -> ApiResult<Option<u64>> {
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

pub(crate) fn positive_usize(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<usize>().ok().filter(|&n| n > 0)
}

pub(crate) fn parse_id_list(val: Option<&Value>, field: &str) -> ApiResult<Vec<String>> {
    let arr = val
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| {
            HttpError::bad_request(format!("{field} must be a non-empty array of user ids"))
        })?;
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

pub(crate) fn parse_mentions(input: Option<&Value>, text: &str) -> ApiResult<Option<Vec<MentionInfo>>> {
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

pub(crate) fn clean_poll_options(val: Option<&Value>) -> Vec<String> {
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

// ---- Multipart voice parser ----

pub(crate) fn extract_multipart_boundary(content_type: &str) -> Option<String> {
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

pub(crate) fn find_multipart_file(
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

        if let Some(disp_line) = headers_text.lines().find(|line| {
            line.trim_start()
                .to_ascii_lowercase()
                .starts_with("content-disposition:")
        }) {
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
