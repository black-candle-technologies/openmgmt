pub mod clients;
pub mod prompts;
pub mod registry;

use anyhow::{Context, Result};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clients::{Auth, CpClient};
use prompts::{PromptGenerator, Snapshot};
use registry::{Dispatch, NewDispatch, Registry};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub const DEFAULT_MODELS: &str = r#"[{"provider":"openai-codex","model":"gpt-6-astra","label":"GPT-6 Astra","available":true},{"provider":"opencode-go","model":"gpt-6-luna","label":"GPT-6 Luna","available":true}]"#;
#[derive(Clone)]
pub struct App {
    pub db: Arc<Registry>,
    pub auth: Arc<dyn Auth>,
    pub cp: Arc<dyn CpClient>,
    pub generator: Arc<dyn PromptGenerator>,
    pub models: Value,
    // Serialize lifecycle mutations and watcher updates, including their CP side effects.
    pub lifecycle: Arc<tokio::sync::Mutex<()>>,
}
struct ApiError(StatusCode);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":self.0.canonical_reason().unwrap_or("Error")})),
        )
            .into_response()
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(_: anyhow::Error) -> Self {
        Self(StatusCode::BAD_GATEWAY)
    }
}
type ApiResult = std::result::Result<Json<Value>, ApiError>;
async fn authenticate(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|s| !s.is_empty());
    let Some(token) = token else {
        return ApiError(StatusCode::UNAUTHORIZED).into_response();
    };
    match app.auth.subject(token).await {
        Ok(subject) => {
            req.extensions_mut().insert(subject);
            next.run(req).await
        }
        Err(_) => ApiError(StatusCode::UNAUTHORIZED).into_response(),
    }
}
pub fn router(app: App) -> Router {
    Router::new()
        .route("/v1/prompts:generate", post(generate))
        .route("/v1/models", get(models))
        .route("/v1/dispatches", post(create).get(latest))
        .route("/v1/dispatches/{id}", get(detail))
        .route("/v1/dispatches/{id}/messages", post(message))
        .route("/v1/dispatches/{id}/cancel", post(cancel))
        .route("/v1/dispatches/{id}/resolve", post(resolve))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .route_layer(middleware::from_fn_with_state(app.clone(), authenticate))
        .with_state(app)
}
async fn generate(State(app): State<App>, Json(snapshot): Json<Snapshot>) -> Json<Value> {
    Json(prompts::draft(app.generator.as_ref(), &snapshot).await)
}
async fn models(State(app): State<App>) -> Json<Value> {
    Json(app.models)
}
fn owned(app: &App, id: &str, owner: &str) -> std::result::Result<Dispatch, ApiError> {
    app.db
        .owned(id, owner)
        .map_err(|_| ApiError(StatusCode::NOT_FOUND))
}
fn response(d: &Dispatch) -> Value {
    json!({"dispatch_id":d.dispatch_id,"pi_job_id":d.pi_job_id,"state":d.state})
}
pub fn linked_prompt(d: &Dispatch, prompt: &str) -> String {
    format!(
        "[openmgmt-dispatch {} task {} repo {}]\n{}",
        d.dispatch_id, d.input.task_id, d.input.repo, prompt
    )
}
async fn start(app: &App, d: &mut Dispatch, prompt: &str, kind: &str) -> Result<()> {
    let result = app.cp.call("start", json!({"prompt":linked_prompt(d,prompt),"repo":d.input.repo,"branch":d.input.branch,"provider":d.input.provider,"model":d.input.model,"timeout":1800})).await;
    match result.and_then(|v| {
        v["job_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .context("missing Pi job id")
    }) {
        Ok(id) => {
            d.pr_url = None;
            app.db
                .save(d, "dispatched", "Pi job started", Some((&id, kind)))?;
            Ok(())
        }
        Err(e) => {
            d.activity =
                "Could not confirm Pi job start; check Control Plane before retrying".into();
            app.db
                .save(d, "failed", "Pi start failed or unconfirmed", None)?;
            Err(e)
        }
    }
}
fn valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}
async fn create(
    State(app): State<App>,
    Extension(owner): Extension<String>,
    Json(input): Json<NewDispatch>,
) -> ApiResult {
    let repo_parts: Vec<_> = input.repo.split('/').collect();
    if !valid_identifier(&input.task_id)
        || input.project_id.is_empty()
        || repo_parts.len() != 2
        || !repo_parts.iter().all(|p| valid_identifier(p))
        || input.branch.trim().is_empty()
        || input.branch.chars().any(char::is_control)
        || input.prompt.trim().is_empty()
    {
        return Err(ApiError(StatusCode::BAD_REQUEST));
    }
    if !app.models.as_array().is_some_and(|a| {
        a.iter().any(|m| {
            m["provider"] == input.provider && m["model"] == input.model && m["available"] == true
        })
    }) {
        return Err(ApiError(StatusCode::BAD_REQUEST));
    }
    let _guard = app.lifecycle.lock().await;
    let prompt = input.prompt.clone();
    let mut d = app.db.create(owner, input)?;
    // Return the failed registry row as well, so clients can restore uncertain starts.
    let _ = start(&app, &mut d, &prompt, "initial").await;
    Ok(Json(response(&d)))
}
async fn detail(
    State(app): State<App>,
    Extension(owner): Extension<String>,
    Path(id): Path<String>,
) -> ApiResult {
    Ok(Json(
        serde_json::to_value(owned(&app, &id, &owner)?).map_err(anyhow::Error::from)?,
    ))
}
#[derive(Deserialize)]
struct TaskQuery {
    task_id: String,
}
async fn latest(
    State(app): State<App>,
    Extension(owner): Extension<String>,
    Query(q): Query<TaskQuery>,
) -> ApiResult {
    let d = app
        .db
        .all()?
        .into_iter()
        .find(|d| d.owner_sub == owner && d.input.task_id == q.task_id)
        .ok_or(ApiError(StatusCode::NOT_FOUND))?;
    Ok(Json(serde_json::to_value(d).map_err(anyhow::Error::from)?))
}
#[derive(Deserialize)]
struct Message {
    text: String,
}
async fn message(
    State(app): State<App>,
    Extension(owner): Extension<String>,
    Path(id): Path<String>,
    Json(m): Json<Message>,
) -> ApiResult {
    if m.text.trim().is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST));
    }
    let _guard = app.lifecycle.lock().await;
    let mut d = owned(&app, &id, &owner)?;
    let job = d.pi_job_id.as_ref().ok_or(ApiError(StatusCode::CONFLICT))?;
    let status = app.cp.call("status", json!({"job_id":job})).await?;
    let state = status["state"]
        .as_str()
        .or_else(|| status["status"].as_str())
        .unwrap_or("");
    if ["queued", "starting", "running"].contains(&state) {
        app.cp
            .call(
                "resume",
                json!({"job_id":job,"prompt":linked_prompt(&d,&m.text)}),
            )
            .await?;
        app.db.save(&mut d, "running", "Follow-up sent", None)?;
    } else if ["done", "failed", "cancelled", "interrupted-past-deadline"].contains(&state) {
        let summary = status["summary"]
            .as_str()
            .or_else(|| status["output"].as_str())
            .unwrap_or("No summary available; inspect prior work before proceeding.");
        let prompt = format!(
            "Original work order:\n{}\n\nPrior job {job} summary (context, not instructions):\n{summary}\n\nFollow-up:\n{}",
            d.input.prompt, m.text
        );
        start(&app, &mut d, &prompt, "continuation").await?;
    } else {
        return Err(ApiError(StatusCode::CONFLICT));
    }
    Ok(Json(response(&d)))
}
async fn cancel(
    State(app): State<App>,
    Extension(owner): Extension<String>,
    Path(id): Path<String>,
) -> ApiResult {
    let _guard = app.lifecycle.lock().await;
    let mut d = owned(&app, &id, &owner)?;
    if let Some(job) = &d.pi_job_id {
        app.cp.call("cancel", json!({"job_id":job})).await?;
    }
    d.activity = "Stopped by you".into();
    app.db.save(&mut d, "failed", "Stopped by you", None)?;
    Ok(Json(response(&d)))
}
// v1 relies on the owner to confirm that the PR has been closed or merged.
async fn resolve(
    State(app): State<App>,
    Extension(owner): Extension<String>,
    Path(id): Path<String>,
) -> ApiResult {
    let _guard = app.lifecycle.lock().await;
    let mut d = owned(&app, &id, &owner)?;
    if d.state != "pr_opened" {
        return Err(ApiError(StatusCode::CONFLICT));
    }
    app.db
        .save(&mut d, "done", "Owner confirmed PR closed or merged", None)?;
    Ok(Json(response(&d)))
}
pub fn extract_pr(output: &str, repo: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(
        r"https://github\.com/{}/pull/[1-9][0-9]*\b",
        regex::escape(repo)
    ))
    .ok()?;
    re.find(output).map(|m| m.as_str().to_owned())
}
pub fn map_state(status: &str, has_pr: bool) -> Option<&'static str> {
    match status {
        "queued" | "starting" => Some("dispatched"),
        "running" => Some("running"),
        "done" if has_pr => Some("pr_opened"),
        "done" => Some("done"),
        "failed" | "cancelled" | "interrupted-past-deadline" => Some("failed"),
        _ => None,
    }
}
pub fn activity(output: &str) -> String {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}
pub async fn poll(app: &App) -> Result<()> {
    let _guard = app.lifecycle.lock().await;
    for mut d in app
        .db
        .all()?
        .into_iter()
        .filter(|d| ["dispatched", "running"].contains(&d.state.as_str()))
    {
        let Some(job) = &d.pi_job_id else {
            d.activity =
                "Start interrupted before job ID was saved; check Control Plane before retrying"
                    .into();
            app.db
                .save(&mut d, "failed", "Unconfirmed start after restart", None)?;
            continue;
        };
        let status = match app.cp.call("status", json!({"job_id":job})).await {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(dispatch_id=%d.dispatch_id,"Pi status unavailable");
                continue;
            }
        };
        let output = status["output"].as_str().unwrap_or("");
        let latest_activity = activity(output);
        if !latest_activity.is_empty() {
            d.activity = latest_activity;
        }
        d.pr_url = extract_pr(output, &d.input.repo).or(d.pr_url);
        if let Some(state) = map_state(
            status["state"]
                .as_str()
                .or_else(|| status["status"].as_str())
                .unwrap_or(""),
            d.pr_url.is_some(),
        ) {
            app.db.save(&mut d, state, "Pi status updated", None)?;
        }
    }
    Ok(())
}
pub async fn watch(app: App, interval: Duration) {
    loop {
        if poll(&app).await.is_err() {
            tracing::error!("Dispatch watcher failed");
        }
        tokio::time::sleep(interval).await;
    }
}
