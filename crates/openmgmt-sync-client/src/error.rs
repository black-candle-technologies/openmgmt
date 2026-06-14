pub type SyncClientResult<T> = Result<T, SyncClientError>;

#[derive(Debug, thiserror::Error)]
pub enum SyncClientError {
    #[error("sync is disabled")]
    Disabled,
    #[error("sync server URL is not configured")]
    NotConfigured,
    #[error("local database error: {0}")]
    Core(#[from] openmgmt_core::db::CoreError),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("remote events were pulled but remote apply is not implemented")]
    RemoteApplyUnavailable,
    #[error("unexpected sync error: {0}")]
    Other(String),
}
