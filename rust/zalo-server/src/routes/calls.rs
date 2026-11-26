//! Track 1 boundary (`feat/rust-calls`): call debug log, call REST endpoints, and `/ws/call-audio`.

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::routes::AppState;

const TRACK1_MSG: &str = "Calls are being ported in Track 1 (feat/rust-calls)";

/// Builds the Track 1 calls & call-log router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/debug/call-log", get(get_call_log).delete(clear_call_log))
        .route("/api/calls/current", get(current_call))
        .route("/api/calls/{callId}/decline", post(not_implemented))
        .route("/api/calls/{callId}/answer", post(not_implemented))
        .route("/api/calls/{callId}/answer-test", post(not_implemented))
        .route("/api/calls/start", post(not_implemented))
        .route("/api/calls/hangup", post(not_implemented))
        .route("/api/calls/mute", post(not_implemented))
        .route("/api/calls/camera", post(not_implemented))
        .route("/ws/call-audio", get(ws_call_audio))
}

async fn get_call_log(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.session.call_trace().read();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
}

async fn clear_call_log(State(state): State<AppState>) -> StatusCode {
    state.session.call_trace().clear();
    StatusCode::NO_CONTENT
}

async fn current_call() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "No active call" })))
}

async fn not_implemented() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": TRACK1_MSG })),
    )
}

async fn ws_call_audio(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(close_no_active_call)
}

async fn close_no_active_call(mut socket: WebSocket) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: "No active call".into(),
        })))
        .await;
}
