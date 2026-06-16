use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SYNC_ENABLED_KEY: &str = "sync.enabled";
pub const SYNC_SERVER_URL_KEY: &str = "sync.server_url";
pub const SYNC_DEVICE_NAME_KEY: &str = "sync.device_name";
pub const SYNC_ACCOUNT_ID_KEY: &str = "sync.account_id";
pub const SYNC_USER_ID_KEY: &str = "sync.user_id";
pub const SYNC_DEVICE_TOKEN_KEY: &str = "sync.device_token";
pub const SYNC_LAST_SUCCESSFUL_AT_KEY: &str = "sync.last_successful_sync_at";
pub const SYNC_LAST_ATTEMPTED_AT_KEY: &str = "sync.last_attempted_sync_at";
pub const SYNC_LAST_ERROR_KEY: &str = "sync.last_error";

pub const DEFAULT_DEVICE_NAME: &str = "Local device";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettings {
    pub enabled: bool,
    pub server_url: Option<String>,
    pub device_name: String,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub device_token: Option<String>,
    pub last_successful_sync_at: Option<DateTime<Utc>>,
    pub last_attempted_sync_at: Option<DateTime<Utc>>,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            server_url: None,
            device_name: DEFAULT_DEVICE_NAME.into(),
            account_id: None,
            user_id: None,
            device_token: None,
            last_successful_sync_at: None,
            last_attempted_sync_at: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettingsPatch {
    pub enabled: Option<bool>,
    pub server_url: Option<Option<String>>,
    pub device_name: Option<String>,
    pub account_id: Option<Option<String>>,
    pub user_id: Option<Option<String>>,
    pub device_token: Option<Option<String>>,
}
