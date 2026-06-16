use crate::ProtocolError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistrationRequest {
    pub protocol_version: String,
    pub device_id: String,
    pub device_name: String,
    pub user_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistrationResponse {
    pub accepted: bool,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub device_token: Option<String>,
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContext {
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub device_id: String,
    pub device_token: Option<String>,
}
