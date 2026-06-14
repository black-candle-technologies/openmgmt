use crate::{
    DeviceRegistrationRequest, DeviceRegistrationResponse, ProtocolError, SyncHelloRequest,
    SyncHelloResponse, SyncPullRequest, SyncPullResponse, SyncPushRequest, SyncPushResponse,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ProtocolMessage {
    HelloRequest(SyncHelloRequest),
    HelloResponse(SyncHelloResponse),
    DeviceRegistrationRequest(DeviceRegistrationRequest),
    DeviceRegistrationResponse(DeviceRegistrationResponse),
    SyncPushRequest(SyncPushRequest),
    SyncPushResponse(SyncPushResponse),
    SyncPullRequest(SyncPullRequest),
    SyncPullResponse(SyncPullResponse),
    Error(ProtocolError),
}
