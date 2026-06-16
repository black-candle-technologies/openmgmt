use crate::{error::ServerError, state::AppState, store::parse_checkpoint};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use openmgmt_protocol::{
    DEFAULT_SYNC_EVENT_LIMIT, DeviceRegistrationRequest, DeviceRegistrationResponse,
    MAX_SYNC_EVENT_LIMIT, PROTOCOL_VERSION, ProtocolError, ProtocolErrorCode, SyncHelloRequest,
    SyncHelloResponse, SyncPullRequest, SyncPullResponse, SyncPushRequest, SyncPushResponse,
    is_compatible_protocol_version,
};
use serde::Serialize;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/omgp/v1/hello", post(hello))
        .route("/omgp/v1/devices/register", post(register_device))
        .route("/omgp/v1/sync/push", post(sync_push))
        .route("/omgp/v1/sync/pull", post(sync_pull))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    protocol_version: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "openmgmt-server",
        protocol_version: PROTOCOL_VERSION,
    })
}

async fn hello(
    State(state): State<AppState>,
    Json(request): Json<SyncHelloRequest>,
) -> Json<SyncHelloResponse> {
    let compatible = is_compatible_protocol_version(&request.protocol_version);
    Json(SyncHelloResponse {
        protocol_version: PROTOCOL_VERSION.into(),
        server_name: state.config.server_name.clone(),
        server_version: state.config.server_version.clone(),
        compatible,
        error: (!compatible)
            .then(|| ProtocolError::incompatible_version(&request.protocol_version)),
    })
}

async fn register_device(
    State(state): State<AppState>,
    Json(request): Json<DeviceRegistrationRequest>,
) -> Result<Json<DeviceRegistrationResponse>, ServerError> {
    if !is_compatible_protocol_version(&request.protocol_version) {
        return Ok(Json(DeviceRegistrationResponse {
            accepted: false,
            account_id: None,
            user_id: None,
            device_token: None,
            error: Some(ProtocolError::incompatible_version(
                &request.protocol_version,
            )),
        }));
    }
    let device = state.store.register_device(&request)?;
    Ok(Json(DeviceRegistrationResponse {
        accepted: true,
        account_id: device.account_id,
        user_id: device.user_id,
        device_token: Some(device.device_token),
        error: None,
    }))
}

async fn sync_push(
    State(state): State<AppState>,
    Json(request): Json<SyncPushRequest>,
) -> Result<Json<SyncPushResponse>, ServerError> {
    if !is_compatible_protocol_version(&request.protocol_version) {
        return Ok(Json(push_error(ProtocolError::incompatible_version(
            &request.protocol_version,
        ))));
    }
    if !state.store.authenticate(&request.auth)? {
        return Ok(Json(push_error(unauthorized())));
    }
    let result = state.store.push_events(&request.events)?;
    Ok(Json(SyncPushResponse {
        accepted_event_ids: result.accepted_event_ids,
        rejected_events: result.rejected_events,
        server_checkpoint: result.server_checkpoint,
        error: None,
    }))
}

async fn sync_pull(
    State(state): State<AppState>,
    Json(request): Json<SyncPullRequest>,
) -> Result<Json<SyncPullResponse>, ServerError> {
    if !is_compatible_protocol_version(&request.protocol_version) {
        return Ok(Json(pull_error(ProtocolError::incompatible_version(
            &request.protocol_version,
        ))));
    }
    if !state.store.authenticate(&request.auth)? {
        return Ok(Json(pull_error(unauthorized())));
    }
    let after = match request.after_checkpoint.as_deref() {
        Some(value) => match parse_checkpoint(value) {
            Some(checkpoint) => Some(checkpoint),
            None => {
                return Ok(Json(pull_error(ProtocolError::invalid_request(
                    "invalid checkpoint",
                ))));
            }
        },
        None => None,
    };
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SYNC_EVENT_LIMIT)
        .clamp(1, MAX_SYNC_EVENT_LIMIT);
    let page = state.store.pull_events(after, limit)?;
    Ok(Json(SyncPullResponse {
        events: page.events,
        server_checkpoint: page.server_checkpoint,
        has_more: page.has_more,
        error: None,
    }))
}

fn unauthorized() -> ProtocolError {
    ProtocolError::new(
        ProtocolErrorCode::Unauthorized,
        "device is not registered or token is invalid",
        false,
    )
}

fn push_error(error: ProtocolError) -> SyncPushResponse {
    SyncPushResponse {
        accepted_event_ids: Vec::new(),
        rejected_events: Vec::new(),
        server_checkpoint: String::new(),
        error: Some(error),
    }
}

fn pull_error(error: ProtocolError) -> SyncPullResponse {
    SyncPullResponse {
        events: Vec::new(),
        server_checkpoint: String::new(),
        has_more: false,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::ServerConfig, store::ServerStore};
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use chrono::{TimeZone, Utc};
    use openmgmt_protocol::{
        AuthContext, DeviceRegistrationResponse, SyncEntityType, SyncEvent, SyncOperation,
    };
    use serde::{Serialize, de::DeserializeOwned};
    use serde_json::json;
    use std::{path::PathBuf, sync::Arc};
    use tower::ServiceExt;

    fn app() -> Router {
        router(AppState {
            config: Arc::new(ServerConfig {
                bind_addr: "127.0.0.1:0".into(),
                database_path: PathBuf::from(":memory:"),
                server_name: "Test Server".into(),
                server_version: Some("0.1.0".into()),
            }),
            store: ServerStore::in_memory().unwrap(),
        })
    }

    fn event(event_id: &str, sequence: i64) -> SyncEvent {
        SyncEvent {
            event_id: event_id.into(),
            device_id: "device-1".into(),
            actor_user_id: None,
            target_user_id: None,
            workspace_id: None,
            sequence,
            entity_type: SyncEntityType::Task,
            entity_id: format!("task-{sequence}"),
            operation: SyncOperation::Created,
            payload_json: json!({"entity": {"id": format!("task-{sequence}")}}),
            created_at: Utc
                .with_ymd_and_hms(2026, 6, 13, 12, 0, sequence as u32)
                .unwrap(),
            synced_at: None,
        }
    }

    async fn post<T: Serialize, R: DeserializeOwned>(
        app: Router,
        path: &str,
        payload: &T,
    ) -> (StatusCode, R) {
        let response = app
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn health_reports_server_and_protocol() {
        let response = app()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(health["ok"], true);
        assert_eq!(health["service"], "openmgmt-server");
        assert_eq!(health["protocol_version"], PROTOCOL_VERSION);
    }

    async fn register(app: Router) -> DeviceRegistrationResponse {
        post(
            app,
            "/omgp/v1/devices/register",
            &DeviceRegistrationRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                device_id: "device-1".into(),
                device_name: "Desktop".into(),
                user_hint: None,
            },
        )
        .await
        .1
    }

    fn auth(token: Option<String>) -> openmgmt_protocol::AuthContext {
        AuthContext {
            account_id: None,
            user_id: None,
            device_id: "device-1".into(),
            device_token: token,
        }
    }

    #[tokio::test]
    async fn hello_accepts_current_and_rejects_unknown_versions() {
        let (_, compatible): (_, SyncHelloResponse) = post(
            app(),
            "/omgp/v1/hello",
            &SyncHelloRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                client_name: "test".into(),
                client_version: None,
                device_id: None,
            },
        )
        .await;
        assert!(compatible.compatible);

        let (_, incompatible): (_, SyncHelloResponse) = post(
            app(),
            "/omgp/v1/hello",
            &SyncHelloRequest {
                protocol_version: "omgp/2".into(),
                client_name: "test".into(),
                client_version: None,
                device_id: None,
            },
        )
        .await;
        assert!(!incompatible.compatible);
        assert_eq!(
            incompatible.error.unwrap().code,
            ProtocolErrorCode::IncompatibleVersion
        );
    }

    #[tokio::test]
    async fn push_requires_registered_device() {
        let (_, response): (_, SyncPushResponse) = post(
            app(),
            "/omgp/v1/sync/push",
            &SyncPushRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                auth: auth(None),
                base_checkpoint: None,
                events: vec![event("event-1", 1)],
            },
        )
        .await;
        assert_eq!(
            response.error.unwrap().code,
            ProtocolErrorCode::Unauthorized
        );
    }

    #[tokio::test]
    async fn push_is_idempotent_and_rejects_sequence_conflicts() {
        let app = app();
        let registration = register(app.clone()).await;
        let request = SyncPushRequest {
            protocol_version: PROTOCOL_VERSION.into(),
            auth: auth(registration.device_token),
            base_checkpoint: None,
            events: vec![event("event-1", 1)],
        };
        let (_, first): (_, SyncPushResponse) =
            post(app.clone(), "/omgp/v1/sync/push", &request).await;
        let (_, duplicate): (_, SyncPushResponse) =
            post(app.clone(), "/omgp/v1/sync/push", &request).await;
        assert_eq!(first.accepted_event_ids, vec!["event-1"]);
        assert_eq!(duplicate.accepted_event_ids, vec!["event-1"]);

        let mut conflict = request;
        conflict.events = vec![event("event-2", 1)];
        let (_, conflict): (_, SyncPushResponse) = post(app, "/omgp/v1/sync/push", &conflict).await;
        assert_eq!(conflict.rejected_events.len(), 1);
        assert_eq!(
            conflict.rejected_events[0].error.code,
            ProtocolErrorCode::Conflict
        );
    }

    #[tokio::test]
    async fn pull_returns_events_after_checkpoint_and_respects_limit() {
        let app = app();
        let registration = register(app.clone()).await;
        let auth = auth(registration.device_token);
        let (_, pushed): (_, SyncPushResponse) = post(
            app.clone(),
            "/omgp/v1/sync/push",
            &SyncPushRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                auth: auth.clone(),
                base_checkpoint: None,
                events: vec![event("event-1", 1), event("event-2", 2)],
            },
        )
        .await;
        assert_eq!(pushed.accepted_event_ids.len(), 2);

        let (_, first): (_, SyncPullResponse) = post(
            app.clone(),
            "/omgp/v1/sync/pull",
            &SyncPullRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                auth: auth.clone(),
                after_checkpoint: None,
                limit: Some(1),
            },
        )
        .await;
        assert_eq!(first.events.len(), 1);
        assert!(first.has_more);

        let (_, second): (_, SyncPullResponse) = post(
            app,
            "/omgp/v1/sync/pull",
            &SyncPullRequest {
                protocol_version: PROTOCOL_VERSION.into(),
                auth,
                after_checkpoint: Some(first.server_checkpoint),
                limit: Some(10),
            },
        )
        .await;
        assert_eq!(second.events.len(), 1);
        assert_eq!(second.events[0].event_id, "event-2");
        assert!(!second.has_more);
    }
}
