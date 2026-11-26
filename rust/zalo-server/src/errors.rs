//! Client-safe HTTP errors with status codes and Axum response conversion.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// An error whose message is safe to show the client, with the HTTP status to use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct HttpError {
    pub status: u16,
    pub message: String,
}

impl HttpError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(401, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, message)
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(503, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let code = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (code, Json(json!({ "error": self.message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, HttpError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn http_error_serializes_as_json_error_response() {
        let resp = HttpError::new(400, "Invalid thread kind").into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val, json!({ "error": "Invalid thread kind" }));
    }

    #[tokio::test]
    async fn invalid_status_code_falls_back_to_500() {
        let resp = HttpError::new(9999, "boom").into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
