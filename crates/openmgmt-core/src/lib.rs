#[cfg(feature = "native")]
pub mod ai;
#[cfg(feature = "native")]
pub mod api_keys;
pub mod board;
#[cfg(feature = "native")]
pub mod commands;
#[cfg(feature = "native")]
pub mod db;
pub mod models;
pub mod scheduling;
pub mod scoring;
pub mod sync;

#[cfg(feature = "native")]
pub use api_keys::{API_KEY_PREFIX, ApiKey, ApiKeyScope, ApiKeyValidation, scopes_allow_write};
pub use board::build_board;
#[cfg(feature = "native")]
pub use commands::AppService;
#[cfg(feature = "native")]
pub use db::{Database, McpAuditRecord, McpAuditRow, default_database_path};
pub use models::*;
pub use scheduling::{generate_schedule_ics, next_recurrence_at};
pub use scoring::{ScoringWeights, score_task};
pub use sync::{
    RemoteApplyBatchResult, RemoteApplyResult, RemoteApplyStatus, SyncConnectionState,
    SyncEntityType, SyncEvent, SyncOperation, SyncSettings, SyncSettingsPatch, SyncStatus,
};
