use crate::{config::ServerConfig, store::ServerStore};
use openmgmt_protocol::AccountAuth;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServerConfig>,
    pub store: ServerStore,
    /// Validates Black Candle access tokens. `None` when account auth is
    /// disabled (open registration).
    pub account_auth: Option<Arc<AccountAuth>>,
}
