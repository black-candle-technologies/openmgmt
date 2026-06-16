use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const LOCAL_DEVICE_ID_KEY: &str = "local_device_id";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncDevice {
    pub device_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
}
