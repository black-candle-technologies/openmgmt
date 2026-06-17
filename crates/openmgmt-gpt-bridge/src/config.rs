use openmgmt_core::default_database_path;
use std::path::PathBuf;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8790";

#[derive(Debug, Clone)]
pub struct GptBridgeConfig {
    pub api_token: String,
    pub write_enabled: bool,
    pub bind_addr: String,
    pub database_path: PathBuf,
    pub cors_origin: Option<String>,
}

impl GptBridgeConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_token = std::env::var("OPENMGMT_GPT_API_TOKEN")
            .map_err(|_| anyhow::anyhow!("OPENMGMT_GPT_API_TOKEN is required"))?;
        if api_token.trim().is_empty() {
            anyhow::bail!("OPENMGMT_GPT_API_TOKEN cannot be empty");
        }
        Ok(Self {
            api_token,
            write_enabled: env_bool("OPENMGMT_GPT_WRITE_ENABLED", false),
            bind_addr: std::env::var("OPENMGMT_GPT_BIND")
                .unwrap_or_else(|_| DEFAULT_BIND_ADDR.into()),
            database_path: std::env::var_os("OPENMGMT_DATABASE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(default_database_path),
            cors_origin: std::env::var("OPENMGMT_GPT_CORS_ORIGIN")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        })
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}
