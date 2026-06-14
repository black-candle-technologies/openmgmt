use crate::{AuthContext, ProtocolError};
pub use openmgmt_core::sync::{SyncEntityType, SyncEvent, SyncOperation};
use serde::{Deserialize, Serialize};

pub const DEFAULT_SYNC_EVENT_LIMIT: u32 = 500;
pub const MAX_SYNC_EVENT_LIMIT: u32 = 2_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncHelloRequest {
    pub protocol_version: String,
    pub client_name: String,
    pub client_version: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncHelloResponse {
    pub protocol_version: String,
    pub server_name: String,
    pub server_version: Option<String>,
    pub compatible: bool,
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPushRequest {
    pub protocol_version: String,
    pub auth: AuthContext,
    pub base_checkpoint: Option<String>,
    pub events: Vec<SyncEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPushResponse {
    pub accepted_event_ids: Vec<String>,
    pub rejected_events: Vec<RejectedSyncEvent>,
    pub server_checkpoint: String,
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPullRequest {
    pub protocol_version: String,
    pub auth: AuthContext,
    pub after_checkpoint: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPullResponse {
    pub events: Vec<SyncEvent>,
    pub server_checkpoint: String,
    pub has_more: bool,
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedSyncEvent {
    pub event_id: String,
    pub error: ProtocolError,
}
