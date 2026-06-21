use super::SyncEntityType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! conflict_string_enum {
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

conflict_string_enum!(SyncConflictKind {
    LocalUnsyncedChangeVsRemoteUpdate => "local_unsynced_change_vs_remote_update",
    ArchiveVsUpdate => "archive_vs_update",
    TerminalStatusProtected => "terminal_status_protected",
    RestoreRequiresExplicitEvent => "restore_requires_explicit_event",
    UnsupportedMergeStrategy => "unsupported_merge_strategy",
});

conflict_string_enum!(SyncConflictPolicyAction {
    AppliedRemote => "applied_remote",
    KeptLocal => "kept_local",
    RecordedOnly => "recorded_only",
    AutoResolved => "auto_resolved",
});

conflict_string_enum!(SyncConflictResolutionStatus {
    Open => "open",
    AutoResolved => "auto_resolved",
    Ignored => "ignored",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConflict {
    pub conflict_id: String,
    pub remote_event_id: String,
    pub local_device_id: String,
    pub entity_type: SyncEntityType,
    pub entity_id: String,
    pub conflict_kind: SyncConflictKind,
    pub policy_action: SyncConflictPolicyAction,
    pub local_snapshot_json: Option<serde_json::Value>,
    pub remote_snapshot_json: Option<serde_json::Value>,
    pub resolution_status: SyncConflictResolutionStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}
