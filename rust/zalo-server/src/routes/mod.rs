//! Modular Axum sub-routers isolating Track 1 (calls) and Track 2 (history sync) from core HTTP routes.

pub mod calls;
pub mod history_sync;

use std::sync::Arc;

use crate::config::Config;
use crate::session::ZaloSession;
use crate::store::ChatStore;

/// Shared state passed to all Axum route handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub session: Arc<ZaloSession>,
    pub store: ChatStore,
}
