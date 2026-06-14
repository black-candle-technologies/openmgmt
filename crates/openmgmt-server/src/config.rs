use std::path::PathBuf;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_DATABASE_PATH: &str = "data/openmgmt-server.sqlite";
const DEFAULT_SERVER_NAME: &str = "OpenMgmt Sync Server";

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub database_path: PathBuf,
    pub server_name: String,
    pub server_version: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            bind_addr: std::env::var("OPENMGMT_SERVER_BIND_ADDR")
                .unwrap_or_else(|_| DEFAULT_BIND_ADDR.into()),
            database_path: std::env::var_os("OPENMGMT_SERVER_DATABASE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_DATABASE_PATH)),
            server_name: std::env::var("OPENMGMT_SERVER_NAME")
                .unwrap_or_else(|_| DEFAULT_SERVER_NAME.into()),
            server_version: Some(env!("CARGO_PKG_VERSION").into()),
        }
    }
}
