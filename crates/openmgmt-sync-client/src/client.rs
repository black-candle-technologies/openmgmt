use crate::{
    SyncClientConfig, SyncClientError, SyncClientResult, SyncConnectionTestResult, SyncOnceResult,
    SyncPhase, SyncPhaseResult, http::OmgpHttpClient,
};
use openmgmt_core::{Database, SyncSettingsPatch};
use openmgmt_protocol::{
    AuthContext, DeviceRegistrationRequest, PROTOCOL_VERSION, SyncHelloRequest, SyncPullRequest,
    SyncPushRequest,
};
use std::collections::HashSet;

const SERVER_CHECKPOINT_KEY: &str = "sync.server_checkpoint";

pub struct OpenMgmtSyncClient {
    config: SyncClientConfig,
}

impl OpenMgmtSyncClient {
    pub fn new(config: SyncClientConfig) -> Self {
        Self { config }
    }

    pub async fn test_connection(
        &self,
        database: &Database,
    ) -> SyncClientResult<SyncConnectionTestResult> {
        let settings = database.get_sync_settings()?;
        if !settings.enabled {
            return Err(SyncClientError::Disabled);
        }
        let server_url = settings
            .server_url
            .clone()
            .ok_or(SyncClientError::NotConfigured)?;
        let device_id = database.get_or_create_device_id()?;
        let response = OmgpHttpClient::new(&server_url, self.config.timeout_seconds)?
            .hello(SyncHelloRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                client_name: "OpenMgmt Desktop".into(),
                client_version: Some(env!("CARGO_PKG_VERSION").into()),
                device_id: Some(device_id),
            })
            .await?;
        Ok(SyncConnectionTestResult {
            ok: true,
            server_url: Some(server_url),
            protocol_version: response.protocol_version,
            server_name: Some(response.server_name),
            server_version: response.server_version,
            message: Some("Server protocol is compatible.".into()),
        })
    }

    pub async fn sync_once(&self, database: &Database) -> SyncClientResult<SyncOnceResult> {
        let settings = database.get_sync_settings()?;
        if !settings.enabled {
            return Err(SyncClientError::Disabled);
        }
        let server_url = settings
            .server_url
            .clone()
            .ok_or(SyncClientError::NotConfigured)?;
        let device_id = database.get_or_create_device_id()?;
        database.record_sync_attempt_started()?;

        let result = self
            .sync_after_attempt(database, settings, server_url, device_id)
            .await;
        if let Err(error) = &result {
            if let Err(status_error) = database.record_sync_error(&error.to_string()) {
                tracing::error!(
                    error = %status_error,
                    sync_error = %error,
                    "failed to record sync error"
                );
            }
        }
        result
    }

    async fn sync_after_attempt(
        &self,
        database: &Database,
        mut settings: openmgmt_core::SyncSettings,
        server_url: String,
        device_id: String,
    ) -> SyncClientResult<SyncOnceResult> {
        let mut phases = vec![SyncPhaseResult::ok(
            SyncPhase::Settings,
            Some("Sync is enabled and configured.".into()),
        )];
        let http = OmgpHttpClient::new(&server_url, self.config.timeout_seconds)?;

        http.hello(SyncHelloRequest {
            protocol_version: PROTOCOL_VERSION.into(),
            client_name: "OpenMgmt Desktop".into(),
            client_version: Some(env!("CARGO_PKG_VERSION").into()),
            device_id: Some(device_id.clone()),
        })
        .await?;
        phases.push(SyncPhaseResult::ok(
            SyncPhase::Hello,
            Some("Server protocol is compatible.".into()),
        ));

        if settings.device_token.is_none() {
            let registration = http
                .register_device(DeviceRegistrationRequest {
                    protocol_version: PROTOCOL_VERSION.into(),
                    device_id: device_id.clone(),
                    device_name: settings.device_name.clone(),
                    user_hint: settings.user_id.clone(),
                })
                .await?;
            let token = registration.device_token.ok_or_else(|| {
                SyncClientError::Protocol(
                    "device registration succeeded without a device token".into(),
                )
            })?;
            settings = database.update_sync_settings(SyncSettingsPatch {
                account_id: Some(registration.account_id),
                user_id: Some(registration.user_id),
                device_token: Some(Some(token)),
                ..Default::default()
            })?;
            phases.push(SyncPhaseResult::ok(
                SyncPhase::DeviceRegistration,
                Some("Device registered.".into()),
            ));
        } else {
            phases.push(SyncPhaseResult::ok(
                SyncPhase::DeviceRegistration,
                Some("Existing device registration reused.".into()),
            ));
        }

        let auth = AuthContext {
            account_id: settings.account_id.clone(),
            user_id: settings.user_id.clone(),
            device_id,
            device_token: settings.device_token.clone(),
        };
        let checkpoint = database.get_sync_state(SERVER_CHECKPOINT_KEY)?;
        let events = database
            .list_unsynced_events()?
            .into_iter()
            .take(self.config.max_push_events)
            .collect::<Vec<_>>();
        let pushed_event_count = events.len();
        let pushed_event_ids = events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<HashSet<_>>();
        let push = http
            .push(SyncPushRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                auth: auth.clone(),
                base_checkpoint: checkpoint.clone(),
                events,
            })
            .await?;
        let mut seen_accepted_ids = HashSet::new();
        let accepted_event_ids = push
            .accepted_event_ids
            .into_iter()
            .filter(|event_id| pushed_event_ids.contains(event_id))
            .filter(|event_id| seen_accepted_ids.insert(event_id.clone()))
            .collect::<Vec<_>>();
        database.mark_sync_events_synced(&accepted_event_ids)?;
        let accepted_event_count = accepted_event_ids.len();
        let rejected_event_count = push.rejected_events.len();
        phases.push(SyncPhaseResult::ok(
            SyncPhase::Push,
            Some(if rejected_event_count == 0 {
                format!("Accepted {accepted_event_count} local events.")
            } else {
                format!(
                    "Accepted {accepted_event_count} local events; rejected {rejected_event_count}."
                )
            }),
        ));

        let pull = http
            .pull(SyncPullRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                auth: auth.clone(),
                after_checkpoint: checkpoint,
                limit: Some(self.config.pull_limit),
            })
            .await?;
        let pulled_event_count = pull.events.len();
        phases.push(SyncPhaseResult::ok(
            SyncPhase::Pull,
            Some(format!("Pulled {pulled_event_count} server events.")),
        ));

        let applied = database.apply_remote_sync_events(&pull.events)?;
        if !pull.server_checkpoint.is_empty() {
            database.set_sync_state(SERVER_CHECKPOINT_KEY, &pull.server_checkpoint)?;
        }
        phases.push(SyncPhaseResult::ok(
            SyncPhase::RemoteApply,
            Some(format!(
                "Applied {}; already applied {}; skipped local echoes {}.",
                applied.applied_count,
                applied.already_applied_count,
                applied.skipped_local_echo_count
            )),
        ));
        database.record_sync_success()?;
        phases.push(SyncPhaseResult::ok(
            SyncPhase::StatusUpdate,
            Some("Sync status recorded as successful.".into()),
        ));

        Ok(SyncOnceResult {
            pushed_event_count,
            accepted_event_count,
            rejected_event_count,
            pulled_event_count,
            applied_event_count: applied.applied_count,
            server_checkpoint: (!pull.server_checkpoint.is_empty())
                .then_some(pull.server_checkpoint),
            phases,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SyncClientConfig, SyncClientError};
    use axum::{Json, Router, extract::State, routing::post};
    use openmgmt_core::{
        Database, Organization, Project, ProjectStatus, ProjectType, SyncEntityType, SyncOperation,
        SyncSettingsPatch,
    };
    use openmgmt_protocol::{
        DeviceRegistrationRequest, DeviceRegistrationResponse, PROTOCOL_VERSION, SyncEvent,
        SyncHelloRequest, SyncHelloResponse, SyncPullRequest, SyncPullResponse, SyncPushRequest,
        SyncPushResponse,
    };
    use std::sync::{Arc, Mutex};

    const CHECKPOINT_KEY: &str = "sync.server_checkpoint";

    #[derive(Clone)]
    struct TestServerState {
        pulled_events: Vec<SyncEvent>,
        checkpoint: String,
        pushed_event_ids: Arc<Mutex<Vec<String>>>,
        duplicate_accepted_ids: bool,
    }

    async fn hello(Json(_): Json<SyncHelloRequest>) -> Json<SyncHelloResponse> {
        Json(SyncHelloResponse {
            protocol_version: PROTOCOL_VERSION.into(),
            server_name: "Test Server".into(),
            server_version: None,
            compatible: true,
            error: None,
        })
    }

    async fn register(
        Json(_): Json<DeviceRegistrationRequest>,
    ) -> Json<DeviceRegistrationResponse> {
        Json(DeviceRegistrationResponse {
            accepted: true,
            account_id: Some("account-1".into()),
            user_id: Some("user-1".into()),
            device_token: Some("token-1".into()),
            error: None,
        })
    }

    async fn push(
        State(state): State<TestServerState>,
        Json(request): Json<SyncPushRequest>,
    ) -> Json<SyncPushResponse> {
        let mut accepted_event_ids = request
            .events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        if state.duplicate_accepted_ids {
            accepted_event_ids.extend(accepted_event_ids.clone());
        }
        *state.pushed_event_ids.lock().unwrap() = accepted_event_ids.clone();
        Json(SyncPushResponse {
            accepted_event_ids,
            rejected_events: Vec::new(),
            server_checkpoint: state.checkpoint.clone(),
            error: None,
        })
    }

    async fn pull(
        State(state): State<TestServerState>,
        Json(_): Json<SyncPullRequest>,
    ) -> Json<SyncPullResponse> {
        Json(SyncPullResponse {
            events: state.pulled_events,
            server_checkpoint: state.checkpoint,
            has_more: false,
            error: None,
        })
    }

    async fn test_server(
        pulled_events: Vec<SyncEvent>,
        duplicate_accepted_ids: bool,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let pushed_event_ids = Arc::new(Mutex::new(Vec::new()));
        let state = TestServerState {
            pulled_events,
            checkpoint: "checkpoint-1".into(),
            pushed_event_ids: pushed_event_ids.clone(),
            duplicate_accepted_ids,
        };
        let app = Router::new()
            .route("/omgp/v1/hello", post(hello))
            .route("/omgp/v1/devices/register", post(register))
            .route("/omgp/v1/sync/push", post(push))
            .route("/omgp/v1/sync/pull", post(pull))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), pushed_event_ids)
    }

    fn configured_database(server_url: Option<String>) -> Database {
        let database = Database::in_memory().unwrap();
        database
            .update_sync_settings(SyncSettingsPatch {
                enabled: Some(true),
                server_url: Some(server_url),
                ..Default::default()
            })
            .unwrap();
        database
    }

    fn remote_organization_event(event_id: &str) -> SyncEvent {
        let now = chrono::Utc::now();
        let organization = Organization {
            id: "remote-org".into(),
            name: "Remote Organization".into(),
            slug: "remote-organization".into(),
            description: None,
            color: None,
            icon: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SyncEvent {
            event_id: event_id.into(),
            device_id: "other-device".into(),
            actor_user_id: None,
            target_user_id: None,
            workspace_id: None,
            sequence: 1,
            entity_type: SyncEntityType::Organization,
            entity_id: organization.id.clone(),
            operation: SyncOperation::Created,
            payload_json: serde_json::json!({"entity": organization}),
            created_at: now,
            synced_at: None,
        }
    }

    #[tokio::test]
    async fn disabled_sync_returns_disabled() {
        let database = Database::in_memory().unwrap();
        let error = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap_err();
        assert!(matches!(error, SyncClientError::Disabled));
        assert!(
            database
                .get_sync_status()
                .unwrap()
                .last_attempted_sync_at
                .is_none()
        );
    }

    #[tokio::test]
    async fn enabled_without_url_returns_not_configured() {
        let database = configured_database(None);
        let error = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap_err();
        assert!(matches!(error, SyncClientError::NotConfigured));
        assert!(
            database
                .get_sync_status()
                .unwrap()
                .last_attempted_sync_at
                .is_none()
        );
    }

    #[tokio::test]
    async fn connection_test_requires_configured_server_url() {
        let database = configured_database(None);
        let error = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .test_connection(&database)
            .await
            .unwrap_err();

        assert!(matches!(error, SyncClientError::NotConfigured));
    }

    #[tokio::test]
    async fn connection_test_returns_compatible_server_identity() {
        let (server_url, pushed) = test_server(Vec::new(), false).await;
        let database = configured_database(Some(server_url.clone()));

        let result = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .test_connection(&database)
            .await
            .unwrap();

        assert!(result.ok);
        assert_eq!(result.server_url.as_deref(), Some(server_url.as_str()));
        assert_eq!(result.protocol_version, PROTOCOL_VERSION);
        assert_eq!(result.server_name.as_deref(), Some("Test Server"));
        assert_eq!(result.server_version, None);
        assert!(pushed.lock().unwrap().is_empty());
        assert_eq!(database.get_sync_settings().unwrap().device_token, None);
    }

    #[tokio::test]
    async fn registration_push_and_empty_pull_complete_sync() {
        let (server_url, pushed) = test_server(Vec::new(), false).await;
        let database = configured_database(Some(server_url));
        database
            .append_sync_event(
                SyncEntityType::Task,
                "task-1",
                SyncOperation::Created,
                serde_json::json!({"entity": {"id": "task-1"}}),
            )
            .unwrap();

        let result = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap();

        assert_eq!(result.pushed_event_count, 1);
        assert_eq!(result.accepted_event_count, 1);
        assert_eq!(result.pulled_event_count, 0);
        assert!(database.list_unsynced_events().unwrap().is_empty());
        let settings = database.get_sync_settings().unwrap();
        assert_eq!(settings.device_token.as_deref(), Some("token-1"));
        assert_eq!(settings.account_id.as_deref(), Some("account-1"));
        assert_eq!(settings.user_id.as_deref(), Some("user-1"));
        assert_eq!(
            database.get_sync_state(CHECKPOINT_KEY).unwrap().as_deref(),
            Some("checkpoint-1")
        );
        assert_eq!(pushed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn remote_events_apply_and_advance_checkpoint() {
        let (server_url, _) =
            test_server(vec![remote_organization_event("remote-event")], false).await;
        let database = configured_database(Some(server_url));
        database
            .set_sync_state(CHECKPOINT_KEY, "checkpoint-0")
            .unwrap();

        let result = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap();

        assert_eq!(result.applied_event_count, 1);
        assert_eq!(
            database.get_sync_state(CHECKPOINT_KEY).unwrap().as_deref(),
            Some("checkpoint-1")
        );
        assert_eq!(database.list_organizations().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn replay_failure_does_not_advance_checkpoint_and_records_error() {
        let now = chrono::Utc::now();
        let project = Project {
            id: "remote-project".into(),
            organization_id: "missing-org".into(),
            name: "Remote Project".into(),
            slug: "remote-project".into(),
            description: None,
            project_type: ProjectType::Software,
            status: ProjectStatus::Active,
            priority: 3,
            deadline: None,
            repo_url: None,
            notes: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        let event = SyncEvent {
            event_id: "remote-project-event".into(),
            device_id: "other-device".into(),
            actor_user_id: None,
            target_user_id: None,
            workspace_id: None,
            sequence: 1,
            entity_type: SyncEntityType::Project,
            entity_id: project.id.clone(),
            operation: SyncOperation::Created,
            payload_json: serde_json::json!({"entity": project}),
            created_at: now,
            synced_at: None,
        };
        let (server_url, _) = test_server(vec![event], false).await;
        let database = configured_database(Some(server_url));
        database
            .set_sync_state(CHECKPOINT_KEY, "checkpoint-0")
            .unwrap();

        let error = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap_err();

        assert!(matches!(error, SyncClientError::Core(_)));
        assert_eq!(
            database.get_sync_state(CHECKPOINT_KEY).unwrap().as_deref(),
            Some("checkpoint-0")
        );
        assert_eq!(
            database
                .get_sync_status()
                .unwrap()
                .last_error
                .as_deref()
                .unwrap(),
            "local database error: validation error: cannot apply remote project event because organization is missing"
        );
    }

    #[tokio::test]
    async fn already_applied_remote_event_advances_checkpoint_without_reapplying() {
        let event = remote_organization_event("already-applied");
        let database = Database::in_memory().unwrap();
        database.apply_remote_sync_event(&event).unwrap();
        let (server_url, _) = test_server(vec![event], false).await;
        database
            .update_sync_settings(SyncSettingsPatch {
                enabled: Some(true),
                server_url: Some(Some(server_url)),
                ..Default::default()
            })
            .unwrap();

        let result = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap();

        assert_eq!(result.pulled_event_count, 1);
        assert_eq!(result.applied_event_count, 0);
        assert_eq!(
            database.get_sync_state(CHECKPOINT_KEY).unwrap().as_deref(),
            Some("checkpoint-1")
        );
        assert_eq!(database.list_organizations().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn duplicate_accepted_event_ids_are_idempotent() {
        let (server_url, _) = test_server(Vec::new(), true).await;
        let database = configured_database(Some(server_url));
        database
            .append_sync_event(
                SyncEntityType::Task,
                "task-1",
                SyncOperation::Created,
                serde_json::json!({"entity": {"id": "task-1"}}),
            )
            .unwrap();

        let result = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap();

        assert_eq!(result.pushed_event_count, 1);
        assert_eq!(result.accepted_event_count, 1);
        assert!(database.list_unsynced_events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn local_device_echoes_advance_checkpoint_without_remote_apply() {
        let database = Database::in_memory().unwrap();
        let device_id = database.get_or_create_device_id().unwrap();
        let (server_url, _) = test_server(
            vec![SyncEvent {
                event_id: "local-echo".into(),
                device_id,
                actor_user_id: None,
                target_user_id: None,
                workspace_id: None,
                sequence: 1,
                entity_type: SyncEntityType::Task,
                entity_id: "task-1".into(),
                operation: SyncOperation::Created,
                payload_json: serde_json::json!({"entity": {"id": "task-1"}}),
                created_at: chrono::Utc::now(),
                synced_at: None,
            }],
            false,
        )
        .await;
        database
            .update_sync_settings(SyncSettingsPatch {
                enabled: Some(true),
                server_url: Some(Some(server_url)),
                ..Default::default()
            })
            .unwrap();

        let result = OpenMgmtSyncClient::new(SyncClientConfig::default())
            .sync_once(&database)
            .await
            .unwrap();

        assert_eq!(result.pulled_event_count, 1);
        assert_eq!(result.applied_event_count, 0);
        assert_eq!(
            database.get_sync_state(CHECKPOINT_KEY).unwrap().as_deref(),
            Some("checkpoint-1")
        );
    }
}

pub async fn sync_once(database: &Database) -> SyncClientResult<SyncOnceResult> {
    OpenMgmtSyncClient::new(SyncClientConfig::default())
        .sync_once(database)
        .await
}

pub async fn test_connection(database: &Database) -> SyncClientResult<SyncConnectionTestResult> {
    OpenMgmtSyncClient::new(SyncClientConfig::default())
        .test_connection(database)
        .await
}
