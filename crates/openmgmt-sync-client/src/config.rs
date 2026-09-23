use crate::oauth::OAuthConfig;
use openmgmt_protocol::DEFAULT_SYNC_EVENT_LIMIT;

#[derive(Debug, Clone)]
pub struct SyncClientConfig {
    pub timeout_seconds: u64,
    pub max_push_events: usize,
    pub pull_limit: u32,
    /// Black Candle access token for device registration, from the native
    /// app OAuth flow. Required when the server enables account auth.
    pub bearer_token: Option<String>,
    /// Native OAuth sign-in configuration. When set and no `bearer_token`
    /// is configured, the client falls back to the keychain-held token
    /// from a previous interactive sign-in.
    pub oauth: Option<OAuthConfig>,
}

impl Default for SyncClientConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 15,
            max_push_events: 500,
            pull_limit: DEFAULT_SYNC_EVENT_LIMIT,
            bearer_token: None,
            oauth: None,
        }
    }
}
