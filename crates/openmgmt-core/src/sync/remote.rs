use super::{SyncEntityType, SyncOperation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteApplyStatus {
    Applied,
    AlreadyApplied,
    SkippedLocalEcho,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteApplyResult {
    pub event_id: String,
    pub entity_type: SyncEntityType,
    pub entity_id: String,
    pub operation: SyncOperation,
    pub status: RemoteApplyStatus,
    pub conflict_ids: Vec<String>,
    pub auto_resolved_conflict_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteApplyBatchResult {
    pub applied_count: usize,
    pub already_applied_count: usize,
    pub skipped_local_echo_count: usize,
    pub conflict_count: usize,
    pub auto_resolved_conflict_count: usize,
    pub results: Vec<RemoteApplyResult>,
}
