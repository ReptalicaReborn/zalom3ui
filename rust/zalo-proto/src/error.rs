use thiserror::Error;

/// Unified error type for the `zalo-proto` crate.
#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("HTTP status {status}: {message}")]
    HttpStatus { status: u16, message: String },

    #[error("Crypto error: {0}")]
    Crypto(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Login error (code {0}): {1}")]
    Login(i64, String),

    #[error("Zalo API error (code {code}): {message}")]
    Api { code: i64, message: String },

    #[error("QR code expired")]
    QrExpired,

    #[error("QR code login declined on phone")]
    QrDeclined,

    #[error("QR code login aborted")]
    QrAborted,

    #[error("Attachment upload timed out waiting for file_done")]
    UploadTimeout,

    #[error("Invalid argument: {0}")]
    InvalidArg(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ProtoError>;
