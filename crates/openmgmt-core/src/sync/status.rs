use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncConnectionState {
    Disabled,
    NotConfigured,
    Ready,
    Syncing,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub state: SyncConnectionState,
    pub enabled: bool,
    pub configured: bool,
    pub server_url: Option<String>,
    pub device_id: String,
    pub device_name: String,
    pub unsynced_event_count: i64,
    pub last_successful_sync_at: Option<DateTime<Utc>>,
    pub last_attempted_sync_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}
