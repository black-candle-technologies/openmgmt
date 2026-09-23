use std::path::PathBuf;

// The default auth issuer lives in openmgmt-protocol so the sync server and
// the MCP HTTP transport validate against the same issuer. Self-hosters
// point OPENMGMT_AUTH_ISSUER at their own issuer; an empty value disables
// account auth entirely (open registration, only sensible on loopback).
use openmgmt_protocol::DEFAULT_AUTH_ISSUER;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_DATABASE_PATH: &str = "data/openmgmt-server.sqlite";
const DEFAULT_SERVER_NAME: &str = "OpenMgmt Sync Server";

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub database_path: PathBuf,
    pub server_name: String,
    pub server_version: Option<String>,
    /// Base URL of the OAuth issuer for Black Candle account auth.
    /// `None` disables account auth (historical open registration).
    pub auth_issuer: Option<String>,
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
            auth_issuer: match std::env::var("OPENMGMT_AUTH_ISSUER") {
                Ok(value) if value.trim().is_empty() => None,
                Ok(value) => Some(value.trim().to_owned()),
                Err(_) => Some(DEFAULT_AUTH_ISSUER.into()),
            },
        }
    }

    /// Account auth is required whenever an issuer is configured.
    pub fn account_auth_required(&self) -> bool {
        self.auth_issuer.is_some()
    }
}
