//! Core Zalỏ server library: configuration, errors, logging, wire types, persistent `ChatStore`,
//! `ZaloSession`, and Axum HTTP/WebSocket server.

pub mod config;
pub mod errors;
pub mod http;
pub mod log;
pub mod routes;
pub mod session;
pub mod store;
pub mod types;

use std::future::Future;
use std::sync::Arc;

use tokio::net::TcpListener;
use tracing::info;

use crate::config::{is_loopback_host, Config};
use crate::session::ZaloSession;
use crate::store::ChatStore;

/// Installs the rustls ring crypto provider (no-op if already installed), opens `ChatStore`,
/// starts `ZaloSession`, binds a TCP listener on `(config.host, config.port)`, and serves HTTP/WS
/// until `shutdown` resolves. Flushes `ChatStore` cleanly on exit.
pub async fn run_server(
    config: Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !is_loopback_host(&config.host) && config.access_token.is_none() {
        return Err(format!(
            "HOST={} is not loopback; set ACCESS_TOKEN to expose the server.",
            config.host
        )
        .into());
    }
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;
    serve_on_listener(config, listener, shutdown).await
}

/// Serves the Zalỏ HTTP/WebSocket application on a pre-bound [`TcpListener`] (useful for tests
/// binding an ephemeral `port = 0` and inspecting `listener.local_addr()`).
pub async fn serve_on_listener(
    config: Config,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let local_addr = listener.local_addr()?;
    let store = ChatStore::open(&config.data_dir);
    let session = Arc::new(ZaloSession::new(
        store.clone(),
        config.data_dir.clone(),
        config.read_receipts,
    ));

    let router = http::build_router(Arc::new(config), Arc::clone(&session));
    info!("Zalỏ server listening on http://{local_addr}");

    session.start();

    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await;

    session.shutdown();
    store.flush();

    serve_result.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
}
