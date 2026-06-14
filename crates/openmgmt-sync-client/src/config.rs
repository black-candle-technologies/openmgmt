use openmgmt_protocol::DEFAULT_SYNC_EVENT_LIMIT;

#[derive(Debug, Clone)]
pub struct SyncClientConfig {
    pub timeout_seconds: u64,
    pub max_push_events: usize,
    pub pull_limit: u32,
}

impl Default for SyncClientConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 15,
            max_push_events: 500,
            pull_limit: DEFAULT_SYNC_EVENT_LIMIT,
        }
    }
}
