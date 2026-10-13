use super::*;
use crate::{config::ServerConfig, routes, store::RegistrationMode};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request},
    routing::get,
};
use chrono::Utc;
use openmgmt_protocol::{
    AccountAuth, DeviceRegistrationRequest, SyncEntityType, SyncEvent, SyncOperation,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tower::ServiceExt;

fn register(store: &ServerStore, device: &str, account: Option<&str>) {
    store
        .register_device(
            &DeviceRegistrationRequest {
                protocol_version: "omgp/1".into(),
                device_id: device.into(),
                device_name: device.into(),
                previous_device_token: None,
                user_hint: None,
            },
            account
                .map(|id| RegistrationMode::AccountAuthenticated {
                    account_user_id: id.into(),
                    existing_device_token: None,
                })
                .unwrap_or(RegistrationMode::Open),
        )
        .unwrap();
}
fn entity_event(
    device: &str,
    id: &str,
    sequence: i64,
    title: &str,
    kind: SyncEntityType,
) -> SyncEvent {
    let now = Utc::now();
    let entity = if kind == SyncEntityType::Task {
        json!({"id":id,"project_id":"project-1","title":title,"status":"ready","priority":0,"pinned":false,"tags":[],"created_at":now,"updated_at":now})
    } else {
        json!({"id":id,"organization_id":"org-1","name":title,"slug":id,"project_type":"other","status":"active","priority":0,"created_at":now,"updated_at":now})
    };
    SyncEvent {
        event_id: format!("{device}-{sequence}"),
        device_id: device.into(),
        actor_user_id: None,
        target_user_id: None,
        workspace_id: None,
        sequence,
        entity_type: kind,
        entity_id: id.into(),
        operation: if sequence == 1 {
            SyncOperation::Created
        } else {
            SyncOperation::Updated
        },
        payload_json: json!({"entity":entity}),
        created_at: now,
        synced_at: None,
    }
}
fn fixture() -> ServerStore {
    let store = ServerStore::in_memory().unwrap();
    register(&store, "a", Some("user-a"));
    register(&store, "b", Some("user-b"));
    register(&store, "legacy", None);
    for (device, sequence, id, title) in [
        ("a", 1, "shared", "Réviser report"),
        ("a", 2, "second", "Zulu report"),
        ("b", 1, "shared", "Private other account"),
        ("legacy", 1, "legacy-task", "Unowned global data"),
    ] {
        store
            .push_events(
                &[entity_event(
                    device,
                    id,
                    sequence,
                    title,
                    SyncEntityType::Task,
                )],
                device,
            )
            .unwrap();
    }
    store
        .push_events(
            &[entity_event(
                "a",
                "project-1",
                3,
                "Research project",
                SyncEntityType::Project,
            )],
            "a",
        )
        .unwrap();
    store
}
#[test]
fn projections_are_account_scoped_and_ignore_unowned_global_events() {
    let store = fixture();
    let a = store.view_page("user-a", ViewQuery::default()).unwrap();
    let b = store.view_page("user-b", ViewQuery::default()).unwrap();
    assert_eq!(a.rows.len(), 2);
    assert_eq!(b.rows.len(), 1);
    assert_eq!(b.rows[0].title, "Private other account");
    assert!(store.view_detail("user-b", "task", "second").is_err());
    assert!(store.view_detail("user-a", "task", "legacy-task").is_err());
    assert_eq!(
        store
            .view_page(
                "user-a",
                ViewQuery {
                    kind: "project".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .rows
            .len(),
        1
    );
}
#[test]
fn server_normalizes_search_and_pages_with_authenticated_cursors() {
    let store = fixture();
    let page = store
        .view_page(
            "user-a",
            ViewQuery {
                q: Some(" RÉVISER ".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].id, "task_shared");
    let first = store
        .view_page(
            "user-a",
            ViewQuery {
                limit: 1,
                sort_id: "title_asc".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(first.has_more);
    let cursor = first.next_cursor.clone();
    let query = ViewQuery {
        limit: 1,
        sort_id: "title_asc".into(),
        cursor: cursor.clone(),
        ..Default::default()
    };
    let next = store.view_page("user-a", query.clone()).unwrap();
    assert_ne!(first.rows[0].id, next.rows[0].id);
    assert!(!next.has_more);
    assert!(store.view_page("user-b", query.clone()).is_err());
    assert!(
        store
            .view_page(
                "user-a",
                ViewQuery {
                    q: Some("different".into()),
                    ..query.clone()
                }
            )
            .is_err()
    );
    assert!(
        store
            .view_page(
                "user-a",
                ViewQuery {
                    cursor: cursor.map(|c| format!("{c}x")),
                    ..query
                }
            )
            .is_err()
    );
}
#[test]
fn accepted_updates_and_archives_change_materialized_rows_atomically() {
    let store = fixture();
    let first = store
        .view_page(
            "user-a",
            ViewQuery {
                limit: 1,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .push_events(
            &[entity_event(
                "a",
                "shared",
                4,
                "Updated title",
                SyncEntityType::Task,
            )],
            "a",
        )
        .unwrap();
    assert_eq!(
        store
            .view_detail("user-a", "task", "shared")
            .unwrap()
            .screen
            .title,
        "Updated title"
    );
    let failure = store
        .view_page(
            "user-a",
            ViewQuery {
                limit: 1,
                cursor: first.next_cursor,
                ..Default::default()
            },
        )
        .err()
        .unwrap();
    assert_eq!(failure.0, StatusCode::CONFLICT);
    let mut archive = entity_event("a", "shared", 5, "ignored", SyncEntityType::Task);
    archive.operation = SyncOperation::Archived;
    archive.payload_json = json!({"id":"shared","archived_at":Utc::now()});
    store.push_events(&[archive], "a").unwrap();
    assert_eq!(
        store
            .view_page("user-a", ViewQuery::default())
            .unwrap()
            .rows
            .len(),
        1
    );
    assert_eq!(
        store
            .view_page(
                "user-a",
                ViewQuery {
                    filter_id: "archived".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .rows[0]
            .id,
        "task_shared"
    );
    assert_eq!(
        store
            .view_detail("user-a", "task", "shared")
            .unwrap()
            .screen
            .subtitle,
        "Canceled"
    );
    let mut project_archive = entity_event("a", "project-1", 6, "ignored", SyncEntityType::Project);
    project_archive.operation = SyncOperation::Archived;
    project_archive.payload_json = json!({"id":"project-1","archived_at":Utc::now()});
    store.push_events(&[project_archive], "a").unwrap();
    assert_eq!(
        store
            .view_page(
                "user-a",
                ViewQuery {
                    kind: "project".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .rows
            .len(),
        0
    );
    assert_eq!(
        store
            .view_page(
                "user-a",
                ViewQuery {
                    kind: "project".into(),
                    filter_id: "archived".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .rows
            .len(),
        1
    );
    assert_eq!(
        store
            .view_detail("user-a", "project", "project-1")
            .unwrap()
            .screen
            .subtitle,
        "Archived"
    );
}
#[test]
fn malformed_new_snapshot_cannot_present_old_data_as_current() {
    let store = fixture();
    let mut invalid = entity_event("a", "shared", 4, "bad", SyncEntityType::Task);
    invalid.payload_json = json!({"entity":{"id":"wrong"}});
    store.push_events(&[invalid], "a").unwrap();
    assert!(store.view_detail("user-a", "task", "shared").is_err());
    assert_eq!(
        store
            .view_page("user-a", ViewQuery::default())
            .unwrap()
            .rows
            .len(),
        1
    );
}
#[test]
fn existing_events_backfill_and_legacy_claim_requires_real_proof() {
    let store = fixture();
    let connection = store.connection().unwrap();
    connection
        .execute(
            "DELETE FROM server_state WHERE key='mobile_projection_v1'",
            [],
        )
        .unwrap();
    connection
        .execute("DELETE FROM mobile_projection", [])
        .unwrap();
    drop(connection);
    store.migrate().unwrap();
    assert_eq!(
        store
            .view_page("user-a", ViewQuery::default())
            .unwrap()
            .rows
            .len(),
        2
    );
    let legacy = store
        .register_device(
            &DeviceRegistrationRequest {
                protocol_version: "omgp/1".into(),
                device_id: "legacy".into(),
                device_name: "legacy".into(),
                previous_device_token: None,
                user_hint: None,
            },
            RegistrationMode::Open,
        )
        .unwrap();
    store
        .register_device(
            &DeviceRegistrationRequest {
                protocol_version: "omgp/1".into(),
                device_id: "legacy".into(),
                device_name: "legacy".into(),
                previous_device_token: Some(legacy.device_token.clone()),
                user_hint: None,
            },
            RegistrationMode::AccountAuthenticated {
                account_user_id: "user-a".into(),
                existing_device_token: Some(legacy.device_token),
            },
        )
        .unwrap();
    assert_eq!(
        store.device_account_id("legacy").unwrap().as_deref(),
        Some("user-a")
    );
    assert_eq!(
        store
            .view_page("user-a", ViewQuery::default())
            .unwrap()
            .rows
            .len(),
        3
    );
    store
        .connection()
        .unwrap()
        .execute(
            "UPDATE server_devices SET account_id=NULL,device_token=NULL WHERE device_id='legacy'",
            [],
        )
        .unwrap();
    assert!(
        store
            .register_device(
                &DeviceRegistrationRequest {
                    protocol_version: "omgp/1".into(),
                    device_id: "legacy".into(),
                    device_name: "legacy".into(),
                    previous_device_token: None,
                    user_hint: None
                },
                RegistrationMode::AccountAuthenticated {
                    account_user_id: "attacker".into(),
                    existing_device_token: None
                }
            )
            .is_err()
    );
}
#[test]
fn unsupported_queries_and_page_bounds_are_rejected() {
    let store = fixture();
    for query in [
        ViewQuery {
            limit: 51,
            ..Default::default()
        },
        ViewQuery {
            kind: "organization".into(),
            ..Default::default()
        },
        ViewQuery {
            sort_id: "sql".into(),
            ..Default::default()
        },
        ViewQuery {
            filter_id: "other".into(),
            ..Default::default()
        },
        ViewQuery {
            q: Some("x".repeat(201)),
            ..Default::default()
        },
    ] {
        assert!(store.view_page("user-a", query).is_err());
    }
}
async fn issuer() -> String {
    async fn info(headers: HeaderMap) -> Response {
        match headers.get(header::AUTHORIZATION).and_then(|v|v.to_str().ok()) {
            Some("Bearer user-a")=>Json(json!({"id":"user-a","email":"a@example.com","email_verified":true,"scope":"openmgmt:tasks:read"})).into_response(),
            Some("Bearer user-b")=>Json(json!({"id":"user-b","email":"b@example.com","email_verified":true,"scope":"openmgmt:tasks:read"})).into_response(),
            Some("Bearer identity")=>Json(json!({"id":"user-a","email":"a@example.com","email_verified":true,"scope":"identity"})).into_response(),
            Some("Bearer unverified")=>Json(json!({"id":"user-a","email":"a@example.com","email_verified":false,"scope":"openmgmt:tasks:read"})).into_response(),
            _=>StatusCode::UNAUTHORIZED.into_response(),
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/oauth/userinfo", get(info)))
            .await
            .unwrap();
    });
    format!("http://{address}")
}
fn app(store: ServerStore, issuer: Option<&str>) -> Router {
    routes::router(AppState {
        config: Arc::new(ServerConfig {
            bind_addr: "127.0.0.1:0".into(),
            database_path: PathBuf::from(":memory:"),
            server_name: "test".into(),
            server_version: None,
            auth_issuer: issuer.map(str::to_owned),
        }),
        store,
        account_auth: issuer.map(|issuer| Arc::new(AccountAuth::new(issuer).unwrap())),
    })
}
async fn get_response(app: Router, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::get(path);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    if status == StatusCode::OK {
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
    }
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}
#[tokio::test]
async fn read_routes_require_configured_auth_verified_accounts_and_product_scope() {
    let issuer = issuer().await;
    let app = app(fixture(), Some(&issuer));
    for (token, status) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("invalid"), StatusCode::UNAUTHORIZED),
        (Some("identity"), StatusCode::FORBIDDEN),
        (Some("unverified"), StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            get_response(app.clone(), "/api/views/v1/overview", token)
                .await
                .0,
            status
        );
    }
    assert_eq!(
        get_response(
            super::tests::app(fixture(), None),
            "/api/views/v1/overview",
            Some("user-a")
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (status, overview) =
        get_response(app.clone(), "/api/views/v1/overview", Some("user-a")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(overview["service_id"], "openmgmt");
    assert_eq!(
        get_response(
            app.clone(),
            "/api/views/v1/items/task/second",
            Some("user-b")
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, view) =
        get_response(app, "/api/views/v1/items?kind=project", Some("user-a")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["rows"][0]["id"], "project_project-1");
}

#[tokio::test]
async fn summary_counts_are_complete_account_scoped_and_follow_archive_visibility() {
    let issuer = issuer().await;
    let store = fixture();
    for sequence in 4..=13 {
        let id = format!("extra-{sequence}");
        store
            .push_events(
                &[entity_event(
                    "a",
                    &id,
                    sequence,
                    "Additional task",
                    SyncEntityType::Task,
                )],
                "a",
            )
            .unwrap();
    }
    let app = app(store.clone(), Some(&issuer));
    let (_, overview) = get_response(app.clone(), "/api/views/v1/overview", Some("user-a")).await;
    assert_eq!(overview["metrics"][0]["id"], "task_count");
    assert_eq!(overview["metrics"][0]["value"], "12");
    assert_eq!(overview["metrics"][1]["value"], "1");
    let preview = overview["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|section| section["id"] == "tasks")
        .unwrap();
    assert_eq!(preview["items"].as_array().unwrap().len(), 5);
    let browse = overview["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|section| section["component"] == "action_rows")
        .unwrap();
    assert_eq!(browse["items"][0]["route"], "services/openmgmt/items");
    assert_eq!(browse["items"].as_array().unwrap().len(), 1);
    assert!(
        overview["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|metric| {
                metric["tone"] == "muted"
                    && metric["action"] == "open_openmgmt"
                    && metric["label"]
                        .as_str()
                        .is_some_and(|label| !label.is_empty())
            })
    );
    let (_, other) = get_response(app.clone(), "/api/views/v1/overview", Some("user-b")).await;
    assert_eq!(other["metrics"][0]["value"], "1");
    assert_eq!(other["metrics"][1]["value"], "0");

    for (id, sequence, kind) in [
        ("shared", 14, SyncEntityType::Task),
        ("project-1", 15, SyncEntityType::Project),
    ] {
        let mut archive = entity_event("a", id, sequence, "ignored", kind);
        archive.operation = SyncOperation::Archived;
        archive.payload_json = json!({"id":id,"archived_at":Utc::now()});
        store.push_events(&[archive], "a").unwrap();
    }
    let mut malformed = entity_event("a", "second", 16, "invalid", SyncEntityType::Task);
    malformed.payload_json = json!({"entity":{"id":"wrong"}});
    store.push_events(&[malformed], "a").unwrap();
    let (_, updated) = get_response(app.clone(), "/api/views/v1/overview", Some("user-a")).await;
    assert_eq!(updated["metrics"][0]["value"], "10");
    assert_eq!(updated["metrics"][1]["value"], "0");
    assert_eq!(updated["availability"], "partial");
    let (_, unchanged_other) = get_response(app, "/api/views/v1/overview", Some("user-b")).await;
    assert_eq!(unchanged_other["metrics"], other["metrics"]);
}
