use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorCode {
    IncompatibleVersion,
    InvalidRequest,
    Unauthorized,
    Forbidden,
    Conflict,
    PayloadTooLarge,
    RateLimited,
    ServerError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl ProtocolError {
    pub fn new(code: ProtocolErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorCode::InvalidRequest, message, false)
    }

    pub fn incompatible_version(version: impl AsRef<str>) -> Self {
        Self::new(
            ProtocolErrorCode::IncompatibleVersion,
            format!("incompatible protocol version: {}", version.as_ref()),
            false,
        )
    }

    pub fn server_error(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorCode::ServerError, message, true)
    }
}
