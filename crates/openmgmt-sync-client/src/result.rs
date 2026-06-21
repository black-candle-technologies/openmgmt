use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConnectionTestResult {
    pub ok: bool,
    pub server_url: Option<String>,
    pub protocol_version: String,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    Settings,
    Hello,
    DeviceRegistration,
    Push,
    Pull,
    RemoteApply,
    StatusUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPhaseResult {
    pub phase: SyncPhase,
    pub ok: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOnceResult {
    pub pushed_event_count: usize,
    pub accepted_event_count: usize,
    pub rejected_event_count: usize,
    pub pulled_event_count: usize,
    pub applied_event_count: usize,
    pub conflict_count: usize,
    pub auto_resolved_conflict_count: usize,
    pub server_checkpoint: Option<String>,
    pub phases: Vec<SyncPhaseResult>,
}

impl SyncPhaseResult {
    pub fn ok(phase: SyncPhase, message: impl Into<Option<String>>) -> Self {
        Self {
            phase,
            ok: true,
            message: message.into(),
        }
    }
}
