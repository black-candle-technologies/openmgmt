use openmgmt_core::SyncConnectionState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
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
    pub server_checkpoint: Option<String>,
    pub phases: Vec<SyncPhaseResult>,
}

pub fn status_label(state: SyncConnectionState) -> &'static str {
    match state {
        SyncConnectionState::Disabled => "Sync disabled",
        SyncConnectionState::NotConfigured => "Not configured",
        SyncConnectionState::Ready => "Ready",
        SyncConnectionState::Syncing => "Syncing",
        SyncConnectionState::Error => "Error",
    }
}

pub fn server_url_hint(server_url: &str) -> Option<&'static str> {
    let server_url = server_url.trim();
    if server_url.is_empty()
        || server_url.starts_with("http://")
        || server_url.starts_with("https://")
    {
        None
    } else {
        Some("Server URL should start with http:// or https://")
    }
}

pub fn sync_result_summary(result: &SyncOnceResult) -> String {
    let checkpoint = result
        .server_checkpoint
        .as_deref()
        .map(|value| format!(" Checkpoint: {value}."))
        .unwrap_or_default();
    format!(
        "Pushed {}, accepted {}, rejected {}, pulled {}, applied {}.{}",
        result.pushed_event_count,
        result.accepted_event_count,
        result.rejected_event_count,
        result.pulled_event_count,
        result.applied_event_count,
        checkpoint
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_states_have_user_facing_labels() {
        assert_eq!(status_label(SyncConnectionState::Disabled), "Sync disabled");
        assert_eq!(
            status_label(SyncConnectionState::NotConfigured),
            "Not configured"
        );
        assert_eq!(status_label(SyncConnectionState::Ready), "Ready");
        assert_eq!(status_label(SyncConnectionState::Syncing), "Syncing");
        assert_eq!(status_label(SyncConnectionState::Error), "Error");
    }

    #[test]
    fn url_hint_accepts_http_and_https_including_local_servers() {
        assert_eq!(server_url_hint(""), None);
        assert_eq!(server_url_hint("http://127.0.0.1:8787"), None);
        assert_eq!(server_url_hint("https://sync.example.com"), None);
        assert_eq!(
            server_url_hint("sync.example.com"),
            Some("Server URL should start with http:// or https://")
        );
    }

    #[test]
    fn sync_result_summary_includes_all_counts_and_checkpoint() {
        let result = SyncOnceResult {
            pushed_event_count: 5,
            accepted_event_count: 4,
            rejected_event_count: 1,
            pulled_event_count: 3,
            applied_event_count: 2,
            server_checkpoint: Some("checkpoint-7".into()),
            phases: vec![],
        };

        assert_eq!(
            sync_result_summary(&result),
            "Pushed 5, accepted 4, rejected 1, pulled 3, applied 2. Checkpoint: checkpoint-7."
        );
    }
}
