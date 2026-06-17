use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use openmgmt_core::db::CoreError;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("missing or invalid bearer token")]
    Unauthorized,
    #[error("GPT write mode is disabled")]
    WriteDisabled,
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("invalid request: {0}")]
    BadRequest(String),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::WriteDisabled => StatusCode::FORBIDDEN,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Core(CoreError::NotFound(_)) => StatusCode::NOT_FOUND,
            Self::Core(CoreError::Validation(_)) => StatusCode::BAD_REQUEST,
            Self::Core(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "GPT bridge request failed");
        }
        (
            status,
            Json(ErrorBody {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}
