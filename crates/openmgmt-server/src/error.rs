use crate::store::StoreError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use openmgmt_protocol::ProtocolError;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self, "OpenMGMT server request failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ProtocolError::server_error("internal server error")),
        )
            .into_response()
    }
}
