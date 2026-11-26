//! Track 2 boundary (`feat/rust-history-sync`): phone history-sync REST endpoints.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::routes::AppState;
use crate::types::HistorySyncStatus;

const TRACK2_MSG: &str = "History sync is being ported in Track 2 (feat/rust-history-sync)";

/// Builds the Track 2 history-sync router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/history-sync/status", get(status))
        .route("/api/history-sync/start", post(not_implemented))
        .route("/api/history-sync/cancel", post(not_implemented))
}

async fn status() -> impl IntoResponse {
    let idle = HistorySyncStatus::default();
    let mut val = serde_json::to_value(&idle).unwrap_or_else(|_| json!({}));
    if let Some(obj) = val.as_object_mut() {
        obj.entry("log".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    (StatusCode::OK, Json(val))
}

async fn not_implemented() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": TRACK2_MSG })),
    )
}
