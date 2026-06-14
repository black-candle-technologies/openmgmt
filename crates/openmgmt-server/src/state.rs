use crate::{config::ServerConfig, store::ServerStore};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServerConfig>,
    pub store: ServerStore,
}
