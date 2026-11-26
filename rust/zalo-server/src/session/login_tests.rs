//! Ports of `test/login.test.ts`, one Rust test per TS test, in the same order. The TS tests fake
//! `newZalo()` (login / loginQR); here `FakeLogin` plays that part.

use std::io;

use super::*;

/// A real "connection refused" error from the HTTP stack (nothing listens on port 1).
async fn refused_http_error() -> ProtoError {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    ProtoError::Http(client.get("http://127.0.0.1:1/").send().await.unwrap_err())
}

/// Stands in for the TS `fetchFailed("ENOTFOUND")`: a host that cannot be resolved.
fn dns_failure() -> ProtoError {
    ProtoError::Io(io::Error::other(
        "dns error: failed to lookup address information: Name or service not known",
    ))
}

#[tokio::test]
async fn describe_error_explains_fetch_failures() {
    assert_eq!(describe_proto_error(&dns_failure()), "no internet connection");
    assert_eq!(describe_proto_error(&refused_http_error().await), "connection refused");
    assert_eq!(
        describe_proto_error(&ProtoError::Io(io::Error::from(io::ErrorKind::TimedOut))),
        "connection timed out"
    );
    // Other errors keep their own text (the Rust error type adds a category prefix).
    assert!(describe_proto_error(&ProtoError::InvalidArg("boom".into())).contains("boom"));
}
