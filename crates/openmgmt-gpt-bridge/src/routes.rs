use crate::{config::GptBridgeConfig, error::BridgeError, state::BridgeState};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, header},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use openmgmt_core::{
    AppService, BoardState, NewGptActionLog, NewOrganization, NewProject, NewTask, Organization,
    Project, ProjectStatus, ProjectType, ScoredTask, Task, TaskPatch, TaskQueryFilter, TaskSort,
    TaskSortField, TaskStatus, TaskWithContext,
};
use serde::{Deserialize, Serialize};
use std::{str::FromStr, sync::Arc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

const MAX_TASK_LIMIT: usize = 200;

pub fn router(state: BridgeState) -> Router {
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/openmgmt/summary", get(summary))
        .route(
            "/api/openmgmt/organizations",
            get(list_organizations).post(create_organization),
        )
        .route(
            "/api/openmgmt/projects",
            get(list_projects).post(create_project),
        )
        .route("/api/openmgmt/tasks", get(list_tasks).post(create_task))
        .route("/api/openmgmt/tasks/{id}", get(get_task).patch(update_task))
        .route("/api/openmgmt/tasks/{id}/complete", post(complete_task))
        .route("/api/openmgmt/tasks/{id}/start", post(start_task))
        .route("/api/openmgmt/tasks/{id}/block", post(block_task))
        .route("/api/openmgmt/board", get(board))
        .route("/api/openmgmt/today", get(today))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    if let Some(origin) = state.config.cors_origin.as_ref() {
        match HeaderValue::from_str(origin) {
            Ok(origin) => app.layer(
                CorsLayer::new()
                    .allow_origin(origin)
                    .allow_methods([Method::GET, Method::POST, Method::PATCH])
                    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
            ),
            Err(error) => {
                tracing::warn!(%origin, %error, "ignoring invalid OPENMGMT_GPT_CORS_ORIGIN");
                app
            }
        }
    } else {
        app
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    version: &'static str,
    write_enabled: bool,
}

async fn health(State(state): State<BridgeState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "openmgmt-gpt-bridge",
        version: env!("CARGO_PKG_VERSION"),
        write_enabled: state.config.write_enabled,
    })
}

#[derive(Debug, Serialize)]
struct OpenMgmtSummary {
    organization_count: usize,
    project_count: usize,
    open_task_count: usize,
    blocked_task_count: usize,
    overdue_task_count: usize,
    due_soon_task_count: usize,
    in_progress_task_count: usize,
    done_today_count: usize,
    board: BoardSummary,
}

#[derive(Debug, Serialize)]
struct BoardSummary {
    now: usize,
    next_up: usize,
    overdue: usize,
    due_soon: usize,
    waiting_blocked: usize,
    later_today: usize,
    done_today: usize,
}

async fn summary(
    State(state): State<BridgeState>,
    headers: HeaderMap,
) -> Result<Json<OpenMgmtSummary>, BridgeError> {
    require_auth(&headers, &state.config)?;
    let board = state.service.get_board_state()?;
    let tasks = state.service.query_tasks(
        TaskQueryFilter {
            include_done: Some(true),
            include_canceled: Some(true),
            ..Default::default()
        },
        None,
    )?;
    let now = Utc::now();
    Ok(Json(OpenMgmtSummary {
        organization_count: state.service.list_organizations()?.len(),
        project_count: state.service.list_projects()?.len(),
        open_task_count: tasks
            .iter()
            .filter(|item| !matches!(item.task.status, TaskStatus::Done | TaskStatus::Canceled))
            .count(),
        blocked_task_count: tasks
            .iter()
            .filter(|item| matches!(item.task.status, TaskStatus::Blocked | TaskStatus::Waiting))
            .count(),
        overdue_task_count: tasks
            .iter()
            .filter(|item| {
                !matches!(item.task.status, TaskStatus::Done | TaskStatus::Canceled)
                    && item.task.due_at.is_some_and(|due_at| due_at < now)
            })
            .count(),
        due_soon_task_count: board.due_soon.len(),
        in_progress_task_count: tasks
            .iter()
            .filter(|item| item.task.status == TaskStatus::InProgress)
            .count(),
        done_today_count: board.done_today.len(),
        board: board_summary(&board),
    }))
}

async fn list_organizations(
    State(state): State<BridgeState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Organization>>, BridgeError> {
    require_auth(&headers, &state.config)?;
    Ok(Json(state.service.list_organizations()?))
}

#[derive(Debug, Deserialize)]
struct CreateOrganizationRequest {
    name: String,
    slug: Option<String>,
    description: Option<String>,
    color: Option<String>,
    icon: Option<String>,
}

async fn create_organization(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Organization>, BridgeError> {
    let meta = WriteMeta::new(
        "create_organization",
        "organization",
        None,
        "POST",
        "/api/openmgmt/organizations",
    )
    .summary("create organization".into());
    require_write(&state, &headers, &meta)?;
    let request: CreateOrganizationRequest = parse_write_json(&state, &meta, &body)?;
    let meta = meta.summary(format!("name={}", summarize_text(&request.name)));
    if request.name.trim().is_empty() {
        log_write(&state, &meta.failure("name is required"));
        return Err(BridgeError::BadRequest("name is required".into()));
    }
    match state.service.create_organization(NewOrganization {
        name: request.name.trim().to_string(),
        slug: clean_optional(request.slug),
        description: clean_optional(request.description),
        color: clean_optional(request.color),
        icon: clean_optional(request.icon),
    }) {
        Ok(organization) => {
            log_write(&state, &meta.resource_id(organization.id.clone()).success());
            Ok(Json(organization))
        }
        Err(error) => {
            log_write(&state, &meta.failure(error.to_string()));
            Err(error.into())
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectQuery {
    organization_id: Option<String>,
    status: Option<String>,
    #[serde(rename = "type")]
    project_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProjectWithOrganization {
    project: Project,
    organization: Option<Organization>,
}

async fn list_projects(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Vec<ProjectWithOrganization>>, BridgeError> {
    require_auth(&headers, &state.config)?;
    let status = parse_optional::<ProjectStatus>(query.status.as_deref(), "status")?;
    let project_type = parse_optional::<ProjectType>(query.project_type.as_deref(), "type")?;
    let organizations = state.service.list_organizations()?;
    let projects = state
        .service
        .list_projects()?
        .into_iter()
        .filter(|project| {
            query
                .organization_id
                .as_ref()
                .is_none_or(|id| &project.organization_id == id)
        })
        .filter(|project| status.is_none_or(|status| project.status == status))
        .filter(|project| project_type.is_none_or(|kind| project.project_type == kind))
        .map(|project| {
            let organization = organizations
                .iter()
                .find(|item| item.id == project.organization_id)
                .cloned();
            ProjectWithOrganization {
                project,
                organization,
            }
        })
        .collect();
    Ok(Json(projects))
}

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    organization_id: String,
    name: String,
    slug: Option<String>,
    description: Option<String>,
    #[serde(rename = "type")]
    project_type: Option<ProjectType>,
    status: Option<ProjectStatus>,
    priority: Option<i32>,
    deadline: Option<DateTime<Utc>>,
    repo_url: Option<String>,
    notes: Option<String>,
}

async fn create_project(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Project>, BridgeError> {
    let meta = WriteMeta::new(
        "create_project",
        "project",
        None,
        "POST",
        "/api/openmgmt/projects",
    )
    .summary("create project".into());
    require_write(&state, &headers, &meta)?;
    let request: CreateProjectRequest = parse_write_json(&state, &meta, &body)?;
    let meta = meta.summary(format!("name={}", summarize_text(&request.name)));
    if request.name.trim().is_empty() {
        log_write(&state, &meta.failure("name is required"));
        return Err(BridgeError::BadRequest("name is required".into()));
    }
    if request.organization_id.trim().is_empty() {
        log_write(&state, &meta.failure("organization_id is required"));
        return Err(BridgeError::BadRequest(
            "organization_id is required".into(),
        ));
    }
    match state.service.create_project(NewProject {
        organization_id: request.organization_id.trim().to_string(),
        name: request.name.trim().to_string(),
        slug: clean_optional(request.slug),
        description: clean_optional(request.description),
        project_type: request.project_type.unwrap_or_default(),
        status: request.status.unwrap_or_default(),
        priority: request.priority.unwrap_or(3),
        deadline: request.deadline,
        repo_url: clean_optional(request.repo_url),
        notes: clean_optional(request.notes),
    }) {
        Ok(project) => {
            log_write(&state, &meta.resource_id(project.id.clone()).success());
            Ok(Json(project))
        }
        Err(error) => {
            log_write(&state, &meta.failure(error.to_string()));
            Err(error.into())
        }
    }
}

#[derive(Debug, Deserialize)]
struct TaskQuery {
    organization_id: Option<String>,
    project_id: Option<String>,
    status: Option<String>,
    priority: Option<i32>,
    tag: Option<String>,
    text: Option<String>,
    due_before: Option<DateTime<Utc>>,
    due_after: Option<DateTime<Utc>>,
    scheduled_before: Option<DateTime<Utc>>,
    scheduled_after: Option<DateTime<Utc>>,
    pinned: Option<bool>,
    blocked: Option<bool>,
    limit: Option<usize>,
}

async fn list_tasks(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Vec<TaskWithContext>>, BridgeError> {
    require_auth(&headers, &state.config)?;
    let limit = query_limit(query.limit);
    let mut tasks = state
        .service
        .query_tasks(task_filter(&query)?, Some(TaskSort::default()))?;
    tasks.truncate(limit);
    Ok(Json(tasks))
}

async fn get_task(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TaskWithContext>, BridgeError> {
    require_auth(&headers, &state.config)?;
    Ok(Json(task_with_context(&state.service, &id)?))
}

#[derive(Debug, Deserialize)]
struct CreateTaskRequest {
    project_id: String,
    title: String,
    description: Option<String>,
    status: Option<TaskStatus>,
    priority: Option<i32>,
    due_at: Option<DateTime<Utc>>,
    scheduled_at: Option<DateTime<Utc>>,
    estimated_minutes: Option<i32>,
    time_limit_minutes: Option<i32>,
    pinned: Option<bool>,
    tags: Option<Vec<String>>,
}

async fn create_task(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TaskWithContext>, BridgeError> {
    let meta = WriteMeta::new("create_task", "task", None, "POST", "/api/openmgmt/tasks")
        .summary("create task".into());
    require_write(&state, &headers, &meta)?;
    let request: CreateTaskRequest = parse_write_json(&state, &meta, &body)?;
    let meta = meta.summary(format!("title={}", summarize_text(&request.title)));
    if request.title.trim().is_empty() {
        log_write(&state, &meta.failure("title is required"));
        return Err(BridgeError::BadRequest("title is required".into()));
    }
    if let Err(error) = state.service.get_project(&request.project_id) {
        log_write(&state, &meta.failure(error.to_string()));
        return Err(error.into());
    }
    let result = state.service.create_task(NewTask {
        project_id: request.project_id,
        title: request.title.trim().to_string(),
        description: clean_optional(request.description),
        status: request.status.unwrap_or_default(),
        priority: request.priority.unwrap_or(3),
        due_at: request.due_at,
        scheduled_at: request.scheduled_at,
        estimated_minutes: request.estimated_minutes,
        time_limit_minutes: request.time_limit_minutes,
        pinned: request.pinned.unwrap_or(false),
        tags: request.tags.unwrap_or_default(),
    });
    match result {
        Ok(task) => {
            let meta = meta.resource_id(task.id.clone());
            log_write(&state, &meta.success());
            Ok(Json(task_with_context(&state.service, &task.id)?))
        }
        Err(error) => {
            log_write(&state, &meta.failure(error.to_string()));
            Err(error.into())
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct UpdateTaskRequest {
    title: Option<String>,
    description: Option<String>,
    status: Option<TaskStatus>,
    priority: Option<i32>,
    due_at: Option<DateTime<Utc>>,
    scheduled_at: Option<DateTime<Utc>>,
    estimated_minutes: Option<i32>,
    time_limit_minutes: Option<i32>,
    pinned: Option<bool>,
    blocked_reason: Option<String>,
    tags: Option<Vec<String>>,
}

async fn update_task(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<TaskWithContext>, BridgeError> {
    let meta = WriteMeta::new(
        "update_task",
        "task",
        Some(id.clone()),
        "PATCH",
        format!("/api/openmgmt/tasks/{id}"),
    )
    .summary("allowed task fields".into());
    require_write(&state, &headers, &meta)?;
    let request: UpdateTaskRequest = parse_write_json(&state, &meta, &body)?;
    let result = state.service.update_task(
        &id,
        TaskPatch {
            title: request.title.map(|value| value.trim().to_string()),
            description: request.description.map(|value| clean_optional(Some(value))),
            status: request.status,
            priority: request.priority,
            due_at: request.due_at.map(Some),
            scheduled_at: request.scheduled_at.map(Some),
            estimated_minutes: request.estimated_minutes.map(Some),
            time_limit_minutes: request.time_limit_minutes.map(Some),
            pinned: request.pinned,
            blocked_reason: request
                .blocked_reason
                .map(|value| clean_optional(Some(value))),
            tags: request.tags,
        },
    );
    match result {
        Ok(task) => {
            log_write(&state, &meta.success());
            Ok(Json(task_with_context(&state.service, &task.id)?))
        }
        Err(error) => {
            log_write(&state, &meta.failure(error.to_string()));
            Err(error.into())
        }
    }
}

async fn complete_task(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TaskWithContext>, BridgeError> {
    write_task_transition(
        state,
        headers,
        id,
        "complete_task",
        "POST",
        "complete task and stop active timer",
        |service, id| service.complete_task_with_timer(id),
    )
    .await
}

async fn start_task(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TaskWithContext>, BridgeError> {
    let meta = WriteMeta::new(
        "start_task",
        "task",
        Some(id.clone()),
        "POST",
        format!("/api/openmgmt/tasks/{id}/start"),
    )
    .summary("start task timer".into());
    require_write(&state, &headers, &meta)?;
    match state.service.start_task_timer(&id) {
        Ok(_) => {
            log_write(&state, &meta.success());
            Ok(Json(task_with_context(&state.service, &id)?))
        }
        Err(error) => {
            log_write(&state, &meta.failure(error.to_string()));
            Err(error.into())
        }
    }
}

#[derive(Debug, Deserialize)]
struct BlockTaskRequest {
    reason: String,
}

async fn block_task(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<TaskWithContext>, BridgeError> {
    let meta = WriteMeta::new(
        "block_task",
        "task",
        Some(id.clone()),
        "POST",
        format!("/api/openmgmt/tasks/{id}/block"),
    )
    .summary("block task".into());
    require_write(&state, &headers, &meta)?;
    let request: BlockTaskRequest = parse_write_json(&state, &meta, &body)?;
    let meta = meta.summary(format!("reason={}", summarize_text(&request.reason)));
    if request.reason.trim().is_empty() {
        log_write(&state, &meta.failure("reason is required"));
        return Err(BridgeError::BadRequest("reason is required".into()));
    }
    match state
        .service
        .block_task(&id, request.reason.trim().to_string())
    {
        Ok(task) => {
            log_write(&state, &meta.success());
            Ok(Json(task_with_context(&state.service, &task.id)?))
        }
        Err(error) => {
            log_write(&state, &meta.failure(error.to_string()));
            Err(error.into())
        }
    }
}

async fn board(
    State(state): State<BridgeState>,
    headers: HeaderMap,
) -> Result<Json<BoardState>, BridgeError> {
    require_auth(&headers, &state.config)?;
    Ok(Json(state.service.get_board_state()?))
}

#[derive(Debug, Serialize)]
struct TodayPlan {
    now: Vec<ScoredTask>,
    next_up: Vec<ScoredTask>,
    overdue: Vec<ScoredTask>,
    due_soon: Vec<ScoredTask>,
    blocked: Vec<ScoredTask>,
    in_progress: Vec<TaskWithContext>,
    pinned: Vec<TaskWithContext>,
    done_today: Vec<ScoredTask>,
    recommended_next_task: Option<TaskWithContext>,
}

async fn today(
    State(state): State<BridgeState>,
    headers: HeaderMap,
) -> Result<Json<TodayPlan>, BridgeError> {
    require_auth(&headers, &state.config)?;
    Ok(Json(today_plan(&state.service)?))
}

fn today_plan(service: &AppService) -> Result<TodayPlan, BridgeError> {
    let board = service.get_board_state()?;
    let in_progress = service.query_tasks(
        TaskQueryFilter {
            status: Some(vec![TaskStatus::InProgress]),
            ..Default::default()
        },
        Some(TaskSort::default()),
    )?;
    let pinned = service.query_tasks(
        TaskQueryFilter {
            pinned: Some(true),
            ..Default::default()
        },
        Some(TaskSort::default()),
    )?;
    let recommended_next_task = service
        .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?
        .into_iter()
        .next();
    Ok(TodayPlan {
        now: board.now,
        next_up: board.next_up,
        overdue: board.overdue,
        due_soon: board.due_soon,
        blocked: board.waiting_blocked,
        in_progress,
        pinned,
        done_today: board.done_today,
        recommended_next_task,
    })
}

async fn write_task_transition<F>(
    state: BridgeState,
    headers: HeaderMap,
    id: String,
    action: &'static str,
    method: &'static str,
    summary: &'static str,
    operation: F,
) -> Result<Json<TaskWithContext>, BridgeError>
where
    F: FnOnce(&AppService, &str) -> openmgmt_core::db::Result<Task>,
{
    let meta = WriteMeta::new(
        action,
        "task",
        Some(id.clone()),
        method,
        format!(
            "/api/openmgmt/tasks/{id}/{action}",
            action = action_name_suffix(action)
        ),
    )
    .summary(summary.into());
    require_write(&state, &headers, &meta)?;
    match operation(&state.service, &id) {
        Ok(task) => {
            log_write(&state, &meta.success());
            Ok(Json(task_with_context(&state.service, &task.id)?))
        }
        Err(error) => {
            log_write(&state, &meta.failure(error.to_string()));
            Err(error.into())
        }
    }
}

fn action_name_suffix(action: &str) -> &str {
    action.strip_suffix("_task").unwrap_or(action)
}

fn task_filter(query: &TaskQuery) -> Result<TaskQueryFilter, BridgeError> {
    let status = parse_optional::<TaskStatus>(query.status.as_deref(), "status")?;
    let status = match (status, query.blocked) {
        (Some(status), _) => Some(vec![status]),
        (None, Some(true)) => Some(vec![TaskStatus::Blocked, TaskStatus::Waiting]),
        (None, Some(false)) => Some(vec![
            TaskStatus::Inbox,
            TaskStatus::Backlog,
            TaskStatus::Scheduled,
            TaskStatus::Ready,
            TaskStatus::InProgress,
            TaskStatus::Done,
        ]),
        (None, None) => None,
    };
    let include_done = status
        .as_ref()
        .is_some_and(|statuses| statuses.contains(&TaskStatus::Done));
    let include_canceled = status
        .as_ref()
        .is_some_and(|statuses| statuses.contains(&TaskStatus::Canceled));
    Ok(TaskQueryFilter {
        organization_id: clean_optional(query.organization_id.clone()),
        project_id: clean_optional(query.project_id.clone()),
        status,
        priority: query.priority.map(|value| vec![value]),
        due_from: query.due_after,
        due_to: query.due_before,
        scheduled_from: query.scheduled_after,
        scheduled_to: query.scheduled_before,
        pinned: query.pinned,
        tags: clean_optional(query.tag.clone()).map(|tag| vec![tag]),
        text: clean_optional(query.text.clone()),
        include_done: Some(include_done),
        include_canceled: Some(include_canceled),
    })
}

fn task_with_context(service: &AppService, id: &str) -> Result<TaskWithContext, BridgeError> {
    service
        .query_tasks(
            TaskQueryFilter {
                include_done: Some(true),
                include_canceled: Some(true),
                ..Default::default()
            },
            Some(TaskSort {
                field: TaskSortField::Urgency,
                descending: true,
            }),
        )?
        .into_iter()
        .find(|item| item.task.id == id)
        .ok_or(openmgmt_core::db::CoreError::NotFound("task").into())
}

fn board_summary(board: &BoardState) -> BoardSummary {
    BoardSummary {
        now: board.now.len(),
        next_up: board.next_up.len(),
        overdue: board.overdue.len(),
        due_soon: board.due_soon.len(),
        waiting_blocked: board.waiting_blocked.len(),
        later_today: board.later_today.len(),
        done_today: board.done_today.len(),
    }
}

fn require_auth(headers: &HeaderMap, config: &GptBridgeConfig) -> Result<(), BridgeError> {
    match headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    {
        Some(token) if constant_time_eq(token.as_bytes(), config.api_token.as_bytes()) => Ok(()),
        _ => Err(BridgeError::Unauthorized),
    }
}

/// Compare two byte strings without short-circuiting on the first mismatch, so
/// bearer-token validation does not leak length or content through timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (lhs, rhs) in a.iter().zip(b.iter()) {
        diff |= lhs ^ rhs;
    }
    diff == 0
}

fn require_write(
    state: &BridgeState,
    headers: &HeaderMap,
    meta: &WriteMeta,
) -> Result<(), BridgeError> {
    if let Err(error) = require_auth(headers, &state.config) {
        log_write(state, &meta.failure("unauthorized"));
        return Err(error);
    }
    if !state.config.write_enabled {
        log_write(state, &meta.failure("write mode disabled"));
        return Err(BridgeError::WriteDisabled);
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct WriteMeta {
    action: String,
    resource_type: String,
    resource_id: Option<String>,
    method: String,
    path: String,
    request_summary: String,
}

impl WriteMeta {
    fn new(
        action: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: Option<String>,
        method: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            action: action.into(),
            resource_type: resource_type.into(),
            resource_id,
            method: method.into(),
            path: path.into(),
            request_summary: String::new(),
        }
    }

    fn summary(mut self, value: String) -> Self {
        self.request_summary = value;
        self
    }

    fn resource_id(mut self, value: String) -> Self {
        self.resource_id = Some(value);
        self
    }

    fn success(&self) -> NewGptActionLog {
        self.log(true, None)
    }

    fn failure(&self, error: impl Into<String>) -> NewGptActionLog {
        self.log(false, Some(error.into()))
    }

    fn log(&self, success: bool, error_message: Option<String>) -> NewGptActionLog {
        NewGptActionLog {
            action: self.action.clone(),
            resource_type: self.resource_type.clone(),
            resource_id: self.resource_id.clone(),
            method: self.method.clone(),
            path: self.path.clone(),
            request_summary: self.request_summary.clone(),
            success,
            error_message,
        }
    }
}

fn log_write(state: &BridgeState, input: &NewGptActionLog) {
    if let Err(error) = state.service.record_gpt_action(input.clone()) {
        tracing::error!(%error, action = %input.action, "failed to record GPT action");
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn summarize_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() > 80 {
        format!("{}...", &trimmed[..80])
    } else {
        trimmed.to_string()
    }
}

fn parse_optional<T>(value: Option<&str>, field: &str) -> Result<Option<T>, BridgeError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|error| BridgeError::BadRequest(format!("invalid {field}: {error}")))
        })
        .transpose()
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, BridgeError> {
    serde_json::from_slice(body)
        .map_err(|error| BridgeError::BadRequest(format!("invalid JSON body: {error}")))
}

/// Parse a write request body, recording a failed action-log row when the body
/// is malformed so rejected writes are observable.
fn parse_write_json<T: serde::de::DeserializeOwned>(
    state: &BridgeState,
    meta: &WriteMeta,
    body: &[u8],
) -> Result<T, BridgeError> {
    match parse_json(body) {
        Ok(value) => Ok(value),
        Err(error) => {
            log_write(state, &meta.failure(error.to_string()));
            Err(error)
        }
    }
}

fn query_limit(value: Option<usize>) -> usize {
    value.unwrap_or(MAX_TASK_LIMIT).clamp(1, MAX_TASK_LIMIT)
}

pub fn test_state(
    service: AppService,
    api_token: impl Into<String>,
    write_enabled: bool,
) -> BridgeState {
    BridgeState {
        config: Arc::new(GptBridgeConfig {
            api_token: api_token.into(),
            write_enabled,
            bind_addr: "127.0.0.1:0".into(),
            database_path: ":memory:".into(),
            cors_origin: None,
        }),
        service,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use openmgmt_core::{Database, NewOrganization, NewProject};
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    const TOKEN: &str = "test-token";

    fn app(write_enabled: bool) -> (Router, AppService) {
        let database = Database::in_memory().unwrap();
        let service = AppService::new(database);
        (
            router(test_state(service.clone(), TOKEN, write_enabled)),
            service,
        )
    }

    async fn request(app: Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (status, value)
    }

    async fn get_json<T: DeserializeOwned>(app: Router, path: &str) -> (StatusCode, T) {
        let response = app
            .oneshot(
                Request::get(path)
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    fn auth_post(path: &str, body: Value) -> Request<Body> {
        Request::post(path)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    fn create_project(service: &AppService) -> String {
        let organization = service
            .create_organization(NewOrganization {
                name: "Test Org".into(),
                slug: None,
                description: None,
                color: None,
                icon: None,
            })
            .unwrap();
        service
            .create_project(NewProject {
                organization_id: organization.id,
                name: "Test Project".into(),
                slug: None,
                description: None,
                project_type: ProjectType::Software,
                status: ProjectStatus::Active,
                priority: 3,
                deadline: None,
                repo_url: None,
                notes: None,
            })
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn missing_bearer_token_returns_401() {
        let (app, _) = app(false);
        let (status, _) = request(
            app,
            Request::get("/api/openmgmt/summary")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_bearer_token_returns_401() {
        let (app, _) = app(false);
        let (status, _) = request(
            app,
            Request::get("/api/openmgmt/summary")
                .header("authorization", "Bearer bad-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_token_can_call_read_endpoint() {
        let (app, _) = app(false);
        let (status, body) = get_json::<Value>(app, "/api/openmgmt/summary").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["organization_count"], 0);
    }

    #[tokio::test]
    async fn writes_return_403_when_disabled() {
        let (app, service) = app(false);
        let project_id = create_project(&service);
        let (status, _) = request(
            app,
            auth_post(
                "/api/openmgmt/tasks",
                json!({"project_id": project_id, "title": "New task"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn malformed_write_body_still_returns_403_when_disabled() {
        let (app, _) = app(false);
        let (status, _) = request(app, auth_post("/api/openmgmt/tasks", json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn writes_are_allowed_when_enabled_and_action_log_is_written() {
        let (app, service) = app(true);
        let project_id = create_project(&service);
        let (status, body) = request(
            app,
            auth_post(
                "/api/openmgmt/tasks",
                json!({"project_id": project_id, "title": "New task", "priority": 2}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["task"]["title"], "New task");
        let logs = service.list_gpt_action_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "create_task");
        assert!(logs[0].success);
    }

    #[tokio::test]
    async fn empty_db_summary_and_board_are_zero_or_empty() {
        let (app, _) = app(false);
        let (status, summary) = get_json::<Value>(app.clone(), "/api/openmgmt/summary").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(summary["organization_count"], 0);
        assert_eq!(summary["project_count"], 0);
        assert_eq!(summary["open_task_count"], 0);

        let (status, board) = get_json::<Value>(app, "/api/openmgmt/board").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(board["now"].as_array().unwrap().len(), 0);
        assert_eq!(board["next_up"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn empty_db_today_returns_empty_state() {
        let (app, _) = app(false);
        let (status, today) = get_json::<Value>(app, "/api/openmgmt/today").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(today["now"].as_array().unwrap().len(), 0);
        assert_eq!(today["overdue"].as_array().unwrap().len(), 0);
        assert_eq!(today["in_progress"].as_array().unwrap().len(), 0);
        assert!(today["recommended_next_task"].is_null());
    }

    #[tokio::test]
    async fn create_organization_works_when_writes_enabled_and_is_logged() {
        let (app, service) = app(true);
        let (status, body) = request(
            app,
            auth_post("/api/openmgmt/organizations", json!({"name": "Acme"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], "Acme");
        let logs = service.list_gpt_action_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "create_organization");
        assert!(logs[0].success);
        assert!(logs[0].resource_id.is_some());
    }

    #[tokio::test]
    async fn create_organization_blocked_when_writes_disabled() {
        let (app, service) = app(false);
        let (status, _) = request(
            app,
            auth_post("/api/openmgmt/organizations", json!({"name": "Acme"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let logs = service.list_gpt_action_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "create_organization");
        assert!(!logs[0].success);
    }

    #[tokio::test]
    async fn create_project_works_when_writes_enabled_and_is_logged() {
        let (app, service) = app(true);
        let organization = service
            .create_organization(NewOrganization {
                name: "Acme".into(),
                slug: None,
                description: None,
                color: None,
                icon: None,
            })
            .unwrap();
        let (status, body) = request(
            app,
            auth_post(
                "/api/openmgmt/projects",
                json!({"organization_id": organization.id, "name": "Launch", "priority": 2}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], "Launch");
        assert_eq!(body["priority"], 2);
        let logs = service.list_gpt_action_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "create_project");
        assert!(logs[0].success);
    }

    #[tokio::test]
    async fn create_project_blocked_when_writes_disabled() {
        let (app, _) = app(false);
        let (status, _) = request(
            app,
            auth_post(
                "/api/openmgmt/projects",
                json!({"organization_id": "missing", "name": "Launch"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_task_with_unknown_project_returns_404() {
        let (app, service) = app(true);
        let (status, _) = request(
            app,
            auth_post(
                "/api/openmgmt/tasks",
                json!({"project_id": "does-not-exist", "title": "Orphan"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let logs = service.list_gpt_action_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert!(!logs[0].success);
    }

    #[tokio::test]
    async fn create_task_with_invalid_priority_returns_400() {
        let (app, service) = app(true);
        let project_id = create_project(&service);
        let (status, _) = request(
            app,
            auth_post(
                "/api/openmgmt/tasks",
                json!({"project_id": project_id, "title": "Bad", "priority": 9}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn malformed_write_body_is_logged_when_enabled() {
        let (app, service) = app(true);
        let request_body = Request::post("/api/openmgmt/tasks")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from("{not json"))
            .unwrap();
        let (status, _) = request(app, request_body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let logs = service.list_gpt_action_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert!(!logs[0].success);
    }

    #[tokio::test]
    async fn openapi_includes_org_and_project_create() {
        let openapi = include_str!("../../../docs/gpt-action/openapi.yaml");
        assert!(openapi.contains("operationId: createOrganization"));
        assert!(openapi.contains("operationId: createProject"));
    }

    #[tokio::test]
    async fn today_recommends_p1_above_p5() {
        let (app, service) = app(true);
        let project_id = create_project(&service);
        service
            .create_task(NewTask {
                project_id: project_id.clone(),
                title: "P5 work".into(),
                description: None,
                status: TaskStatus::Ready,
                priority: 5,
                due_at: None,
                scheduled_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                tags: Vec::new(),
            })
            .unwrap();
        service
            .create_task(NewTask {
                project_id,
                title: "P1 work".into(),
                description: None,
                status: TaskStatus::Ready,
                priority: 1,
                due_at: None,
                scheduled_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                tags: Vec::new(),
            })
            .unwrap();

        let (status, body) = get_json::<Value>(app, "/api/openmgmt/today").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["recommended_next_task"]["task"]["title"], "P1 work");
    }

    #[test]
    fn openapi_includes_bearer_auth_and_no_destructive_paths() {
        let openapi = include_str!("../../../docs/gpt-action/openapi.yaml");
        assert!(openapi.contains("bearerAuth"));
        assert!(openapi.contains("operationId: createTask"));
        assert!(!openapi.contains("operationId: delete"));
        assert!(!openapi.contains("/archive"));
    }
}
