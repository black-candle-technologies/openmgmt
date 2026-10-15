use anyhow::{Result, bail};
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use openmgmt_dispatch::{clients::*, prompts::*, registry::*, *};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Reject;
#[async_trait]
impl Auth for Reject {
    async fn subject(&self, _: &str) -> Result<String> {
        bail!("invalid")
    }
}
struct Accept;
#[async_trait]
impl Auth for Accept {
    async fn subject(&self, token: &str) -> Result<String> {
        Ok(token.into())
    }
}
struct FailedGenerator;
#[async_trait]
impl PromptGenerator for FailedGenerator {
    async fn generate(&self, _: &Snapshot) -> Result<String> {
        bail!("offline")
    }
}
fn app(auth: Arc<dyn Auth>, cp: Arc<FakeCpClient>) -> App {
    App {
        db: Arc::new(Registry::open(":memory:").unwrap()),
        auth,
        cp,
        generator: Arc::new(FailedGenerator),
        models: serde_json::from_str(DEFAULT_MODELS).unwrap(),
        lifecycle: Arc::new(tokio::sync::Mutex::new(())),
    }
}
fn input() -> NewDispatch {
    serde_json::from_value(json!({"task_id":"task-1","project_id":"project-1","repo":"owner/repo","branch":"feat/test","prompt":"Fix the bug","provider":"openai-codex","model":"gpt-6-astra"})).unwrap()
}
#[test]
fn state_mapping() {
    for (s, want) in [
        ("queued", "dispatched"),
        ("starting", "dispatched"),
        ("running", "running"),
        ("done", "done"),
        ("failed", "failed"),
        ("cancelled", "failed"),
        ("interrupted-past-deadline", "failed"),
    ] {
        assert_eq!(map_state(s, false), Some(want));
    }
    assert_eq!(map_state("done", true), Some("pr_opened"));
    assert_eq!(map_state("unknown", false), None);
}
#[test]
fn pr_extraction_and_activity() {
    assert_eq!(
        extract_pr("PR: (https://github.com/owner/repo/pull/42).", "owner/repo").as_deref(),
        Some("https://github.com/owner/repo/pull/42")
    );
    assert_eq!(
        extract_pr("https://github.com/other/repo/pull/1", "owner/repo"),
        None
    );
    assert_eq!(
        extract_pr("https://github.com/owner/repo/pull/nope", "owner/repo"),
        None
    );
    assert_eq!(activity("first\n last  \n\n"), "last");
    assert_eq!(activity(&"é".repeat(250)).chars().count(), 200);
}
#[tokio::test]
async fn template_content_and_fallback() {
    let s: Snapshot = serde_json::from_value(json!({"title":"Fix sync","description":"No duplicate tasks","project_name":"OpenMGMT","organization":"BCT","priority":"high","due":"2026-11-01","tags":["bug"],"repo":"owner/repo","default_branch":"master"})).unwrap();
    let text = template(&s);
    for required in [
        "Fix sync",
        "No duplicate tasks",
        "OpenMGMT",
        "BCT",
        "high",
        "2026-11-01",
        "bug",
        "owner/repo",
        "master",
        "given branch",
        "open a PR",
        "never merge",
        "questions",
        "Acceptance criteria",
    ] {
        assert!(text.contains(required), "{required}");
    }
    let v = draft(&FailedGenerator, &s).await;
    assert_eq!(v["source"], "template");
    assert_eq!(v["prompt"], text);
}
#[test]
fn registry_crud_events_and_ownership() {
    let db = Registry::open(":memory:").unwrap();
    let mut d = db.create("alice".into(), input()).unwrap();
    db.save(&mut d, "dispatched", "started", Some(("job-1", "initial")))
        .unwrap();
    db.save(&mut d, "running", "running", None).unwrap();
    db.save(&mut d, "running", "unchanged", None).unwrap();
    db.save(&mut d, "done", "done", None).unwrap();
    db.save(
        &mut d,
        "dispatched",
        "continued",
        Some(("job-2", "continuation")),
    )
    .unwrap();
    let saved = db.owned(&d.dispatch_id, "alice").unwrap();
    assert_eq!(saved.events.len(), 5);
    assert_eq!(saved.pi_job_id.as_deref(), Some("job-2"));
    assert!(db.owned(&d.dispatch_id, "bob").is_err());
    assert_eq!(db.all().unwrap().len(), 1);
}
#[tokio::test]
async fn auth_rejection() {
    let router = router(app(Arc::new(Reject), Arc::new(FakeCpClient::default())));
    for token in [None, Some("Bearer invalid")] {
        let mut req = Request::builder().uri("/v1/models");
        if let Some(token) = token {
            req = req.header("authorization", token);
        }
        let res = router
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}
async fn request(
    app: App,
    method: &str,
    path: &str,
    owner: &str,
    body: Value,
) -> (StatusCode, Value) {
    let r = router(app)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {owner}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn lifecycle_start_poll_continue_cancel_and_isolation() {
    let cp = Arc::new(FakeCpClient::default());
    cp.replies.lock().unwrap().extend([
        json!({"job_id":"job-1"}),
        json!({"state":"done","output":"Opened https://github.com/owner/repo/pull/7"}),
        json!({"state":"done","summary":"Implemented sync"}),
        json!({"job_id":"job-2"}),
        json!({"ok":true}),
    ]);
    let app = app(Arc::new(Accept), cp.clone());
    let (status, v) = request(
        app.clone(),
        "POST",
        "/v1/dispatches",
        "alice",
        serde_json::to_value(input()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = v["dispatch_id"].as_str().unwrap();
    poll(&app).await.unwrap();
    assert_eq!(app.db.owned(id, "alice").unwrap().state, "pr_opened");
    assert_eq!(
        request(
            app.clone(),
            "GET",
            &format!("/v1/dispatches/{id}"),
            "bob",
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            &format!("/v1/dispatches/{id}/messages"),
            "alice",
            json!({"text":"Add tests"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            &format!("/v1/dispatches/{id}/cancel"),
            "alice",
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    let d = app.db.owned(id, "alice").unwrap();
    assert_eq!(d.state, "failed");
    assert_eq!(d.activity, "Stopped by you");
    let calls = cp.calls.lock().unwrap();
    assert_eq!(calls[0].1["timeout"], 1800);
    assert!(calls[0].1["prompt"].as_str().unwrap().starts_with(&format!(
        "[openmgmt-dispatch {id} task task-1 repo owner/repo]\n"
    )));
    assert!(
        calls[3].1["prompt"]
            .as_str()
            .unwrap()
            .contains("Implemented sync")
    );
    assert_eq!(calls[3].1["branch"], "feat/test");
}
#[tokio::test]
async fn authd_userinfo_scope_and_subject() {
    let server = axum::Router::new().route(
        "/oauth/userinfo",
        axum::routing::get(|| async { axum::Json(json!({"sub":"subject","scope":"profile"})) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, server).await.unwrap();
    });
    let mut auth = Authd {
        client: http_client().unwrap(),
        base: format!("http://{addr}"),
        require_scope: false,
    };
    assert_eq!(auth.subject("test-placeholder").await.unwrap(), "subject");
    auth.require_scope = true;
    assert!(auth.subject("test-placeholder").await.is_err());
    handle.abort();
}

#[tokio::test]
async fn live_resume_and_manual_resolution() {
    let cp = Arc::new(FakeCpClient::default());
    cp.replies.lock().unwrap().extend([
        json!({"status":"running"}),
        json!({"ok":true}),
        json!({"status":"done","output":"https://github.com/owner/repo/pull/9"}),
    ]);
    let app = app(Arc::new(Accept), cp.clone());
    let mut d = app.db.create("alice".into(), input()).unwrap();
    app.db
        .save(&mut d, "dispatched", "started", Some(("job-1", "initial")))
        .unwrap();
    let path = format!("/v1/dispatches/{}", d.dispatch_id);
    assert_eq!(
        request(
            app.clone(),
            "POST",
            &format!("{path}/resolve"),
            "alice",
            json!({})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            &format!("{path}/messages"),
            "alice",
            json!({"text":"Also cover Android"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(cp.calls.lock().unwrap()[1].0, "resume");
    poll(&app).await.unwrap();
    assert_eq!(
        request(
            app.clone(),
            "POST",
            &format!("{path}/resolve"),
            "bob",
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            &format!("{path}/resolve"),
            "alice",
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(app.db.owned(&d.dispatch_id, "alice").unwrap().state, "done");
}

#[tokio::test]
async fn failed_start_and_restart_recovery() {
    let app = app(Arc::new(Accept), Arc::new(FakeCpClient::default()));
    let (_, v) = request(
        app.clone(),
        "POST",
        "/v1/dispatches",
        "alice",
        serde_json::to_value(input()).unwrap(),
    )
    .await;
    assert_eq!(v["state"], "failed");
    assert!(v["pi_job_id"].is_null());
    let d = app.db.create("alice".into(), input()).unwrap();
    poll(&app).await.unwrap();
    let recovered = app.db.owned(&d.dispatch_id, "alice").unwrap();
    assert_eq!(recovered.state, "failed");
    assert!(recovered.activity.contains("check Control Plane"));
    let (_, latest) = request(
        app.clone(),
        "GET",
        "/v1/dispatches?task_id=task-1",
        "alice",
        json!(null),
    )
    .await;
    assert_eq!(latest["dispatch_id"], d.dispatch_id);
}

#[test]
fn cp_requires_https_without_url_credentials() {
    assert!(HttpCpClient::new("http://example.invalid", String::new()).is_err());
    assert!(HttpCpClient::new("https://user@example.invalid", String::new()).is_err());
}

#[test]
fn registry_persists_all_job_ids() {
    let path = std::env::temp_dir().join(format!("dispatch-test-{}.sqlite", uuid::Uuid::new_v4()));
    let path_string = path.to_str().unwrap();
    let id;
    {
        let db = Registry::open(path_string).unwrap();
        let mut d = db.create("alice".into(), input()).unwrap();
        db.save(&mut d, "dispatched", "initial", Some(("job-1", "initial")))
            .unwrap();
        db.save(
            &mut d,
            "dispatched",
            "continued",
            Some(("job-2", "continuation")),
        )
        .unwrap();
        id = d.dispatch_id;
    }
    {
        let db = Registry::open(path_string).unwrap();
        assert_eq!(
            db.owned(&id, "alice").unwrap().pi_job_id.as_deref(),
            Some("job-2")
        );
        let c = rusqlite::Connection::open(&path).unwrap();
        let count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM dispatch_jobs WHERE dispatch_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }
    std::fs::remove_file(path).unwrap();
}
