use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! sync_string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                let value = match self {
                    $(Self::$variant => $value),+
                };
                formatter.write_str(value)
            }
        }

        impl FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(format!("invalid {}: {value}", stringify!($name))),
                }
            }
        }
    };
}

sync_string_enum!(SyncEntityType {
    Organization => "organization",
    Project => "project",
    Task => "task",
});

sync_string_enum!(SyncOperation {
    Created => "created",
    Updated => "updated",
    Archived => "archived",
    Transitioned => "transitioned",
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncEvent {
    pub event_id: String,
    pub device_id: String,
    pub actor_user_id: Option<String>,
    pub target_user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub sequence: i64,
    pub entity_type: SyncEntityType,
    pub entity_id: String,
    pub operation: SyncOperation,
    pub payload_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub synced_at: Option<DateTime<Utc>>,
}
