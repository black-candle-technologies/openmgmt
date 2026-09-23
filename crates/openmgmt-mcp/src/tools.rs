use chrono::{DateTime, Utc};
use openmgmt_core::{
    AppService, NewProject, NewTask, ProjectStatus, ProjectType, TaskPatch, TaskQueryFilter,
    TaskSort, TaskSortField, TaskStatus,
    ai::{self, enforce_ai_tool_permission},
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;

#[derive(Clone)]
pub struct OpenMgmtMcp {
    service: AppService,
    tool_router: ToolRouter<Self>,
    writes_enabled: bool,
}

impl OpenMgmtMcp {
    pub fn new(service: AppService, writes_enabled: bool) -> Self {
        Self::build(service, writes_enabled, false)
    }

    /// Remote (HTTP) serving mode. Applies the same #15 permission model as
    /// [`Self::new`], plus the remote invariant: destructive tools are never
    /// exposed over the network, even if the persisted AI settings would
    /// otherwise allow them.
    pub fn new_remote(service: AppService, writes_enabled: bool) -> Self {
        Self::build(service, writes_enabled, true)
    }

    fn build(service: AppService, writes_enabled: bool, remote: bool) -> Self {
        // The core AI permission model is the single source of truth for which
        // tools are exposed. The per-launcher env gate (`writes_enabled`) feeds
        // into it as the `mcp_writes_enabled` flag; the persisted
        // `AiSettings` (read/write/destructive toggles) come from the database
        // so the desktop app's AI settings govern MCP too.
        let settings = service.get_ai_settings().unwrap_or_else(|error| {
            tracing::warn!("failed to load AI settings, falling back to defaults: {error}");
            openmgmt_core::models::AiSettings::default()
        });
        let mut tool_router = Self::tool_router();
        for tool in ai::ai_tool_registry() {
            let check = enforce_ai_tool_permission(&settings, &tool, writes_enabled);
            // Destructive tools must never be exposed remotely, regardless of
            // the persisted setting.
            let denied = !check.allowed || (remote && tool.destructive);
            if denied {
                tracing::debug!(
                    "disabling MCP tool {}: {}",
                    tool.name,
                    check
                        .reason
                        .as_deref()
                        .unwrap_or("denied for remote serving")
                );
                tool_router.disable_route(tool.name);
            }
        }
        Self {
            service,
            tool_router,
            writes_enabled,
        }
    }

    fn json<T: serde::Serialize>(&self, result: openmgmt_core::db::Result<T>) -> String {
        match result {
            Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|e| e.to_string()),
            Err(error) => serde_json::json!({ "error": error.to_string() }).to_string(),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IdInput {
    #[schemars(description = "OpenMgmt UUID")]
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListProjectsInput {
    organization_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListTasksInput {
    project_id: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct QueryTasksInput {
    organization_id: Option<String>,
    project_id: Option<String>,
    status: Option<Vec<String>>,
    priority: Option<Vec<i32>>,
    due_from: Option<DateTime<Utc>>,
    due_to: Option<DateTime<Utc>>,
    scheduled_from: Option<DateTime<Utc>>,
    scheduled_to: Option<DateTime<Utc>>,
    pinned: Option<bool>,
    tags: Option<Vec<String>>,
    text: Option<String>,
    include_done: Option<bool>,
    include_canceled: Option<bool>,
    sort_field: Option<String>,
    sort_descending: Option<bool>,
}

impl QueryTasksInput {
    fn into_filter(self) -> Result<TaskQueryFilter, String> {
        let status = self
            .status
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.parse::<TaskStatus>())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TaskQueryFilter {
            organization_id: self.organization_id,
            project_id: self.project_id,
            status: if status.is_empty() {
                None
            } else {
                Some(status)
            },
            priority: self.priority,
            due_from: self.due_from,
            due_to: self.due_to,
            scheduled_from: self.scheduled_from,
            scheduled_to: self.scheduled_to,
            pinned: self.pinned,
            tags: self.tags,
            text: self.text,
            include_done: self.include_done,
            include_canceled: self.include_canceled,
        })
    }

    fn into_sort(self) -> Result<Option<TaskSort>, String> {
        match self.sort_field {
            None => Ok(None),
            Some(field) => {
                let field = field.parse::<TaskSortField>()?;
                Ok(Some(TaskSort {
                    field,
                    descending: self.sort_descending.unwrap_or(false),
                }))
            }
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateTaskInput {
    project_id: String,
    title: String,
    description: Option<String>,
    status: Option<String>,
    priority: Option<i32>,
    due_at: Option<DateTime<Utc>>,
    scheduled_at: Option<DateTime<Utc>>,
    estimated_minutes: Option<i32>,
    time_limit_minutes: Option<i32>,
    pinned: Option<bool>,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UpdateTaskInput {
    id: String,
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
    priority: Option<i32>,
    due_at: Option<DateTime<Utc>>,
    scheduled_at: Option<DateTime<Utc>>,
    estimated_minutes: Option<i32>,
    time_limit_minutes: Option<i32>,
    pinned: Option<bool>,
    blocked_reason: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateProjectInput {
    organization_id: String,
    name: String,
    description: Option<String>,
    project_type: Option<String>,
    priority: Option<i32>,
    deadline: Option<DateTime<Utc>>,
    repo_url: Option<String>,
    notes: Option<String>,
}

#[tool_router]
impl OpenMgmtMcp {
    #[tool(description = "List active OpenMgmt organizations")]
    fn list_organizations(&self) -> String {
        self.json(self.service.list_organizations())
    }

    #[tool(description = "List active projects, optionally filtered by organization ID")]
    fn list_projects(&self, Parameters(input): Parameters<ListProjectsInput>) -> String {
        self.json(self.service.list_projects().map(|projects| {
            projects
                .into_iter()
                .filter(|project| {
                    input
                        .organization_id
                        .as_ref()
                        .is_none_or(|id| project.organization_id == *id)
                })
                .collect::<Vec<_>>()
        }))
    }

    #[tool(description = "Get one OpenMgmt project by ID")]
    fn get_project(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.get_project(&input.id))
    }

    #[tool(description = "List tasks, optionally filtered by project ID or status")]
    fn list_tasks(&self, Parameters(input): Parameters<ListTasksInput>) -> String {
        self.json(self.service.list_tasks().map(|tasks| {
            tasks
                .into_iter()
                .filter(|task| {
                    input
                        .project_id
                        .as_ref()
                        .is_none_or(|id| task.project_id == *id)
                        && input
                            .status
                            .as_ref()
                            .is_none_or(|status| task.status.to_string() == *status)
                })
                .collect::<Vec<_>>()
        }))
    }

    #[tool(description = "Get one OpenMgmt task by ID")]
    fn get_task(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.get_task(&input.id))
    }

    #[tool(
        description = "Query tasks with filters (status, priority, due window, tags, text) and sorting"
    )]
    fn query_tasks(&self, Parameters(input): Parameters<QueryTasksInput>) -> String {
        let filter = match input.clone().into_filter() {
            Ok(filter) => filter,
            Err(error) => return serde_json::json!({ "error": error }).to_string(),
        };
        let sort = match input.into_sort() {
            Ok(sort) => sort,
            Err(error) => return serde_json::json!({ "error": error }).to_string(),
        };
        self.json(self.service.query_tasks(filter, sort))
    }

    #[tool(description = "Get the current scored ER-board state")]
    fn get_board_state(&self) -> String {
        self.json(self.service.get_board_state())
    }

    #[tool(description = "Get today's highest urgency work and complete board")]
    fn get_today_plan(&self) -> String {
        self.json(self.service.get_board_state().map(|board| {
            let mut focus = board
                .now
                .iter()
                .chain(&board.overdue)
                .chain(&board.due_soon)
                .chain(&board.next_up)
                .cloned()
                .collect::<Vec<_>>();
            focus.sort_by_key(|task| std::cmp::Reverse(task.urgency_score));
            focus.truncate(8);
            serde_json::json!({ "generated_at": board.generated_at, "focus": focus, "board": board })
        }))
    }

    #[tool(description = "List saved task views")]
    fn list_saved_task_views(&self) -> String {
        self.json(self.service.list_saved_task_views())
    }

    #[tool(description = "List timer sessions for one task")]
    fn list_timer_sessions(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.list_task_timer_sessions(&input.id))
    }

    #[tool(description = "Get the current scoring/urgency settings")]
    fn get_scoring_settings(&self) -> String {
        self.json(self.service.get_scoring_settings())
    }

    #[tool(
        description = "Deterministic summary of one project: status counts, overdue/blocked, suggested next task"
    )]
    fn summarize_project(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(ai::summarize_project(&self.service, &input.id))
    }

    #[tool(description = "Group stale, blocked, and overdue tasks for review")]
    fn triage_backlog(&self) -> String {
        self.json(ai::triage_backlog(&self.service))
    }

    #[tool(description = "Plan today from board state")]
    fn plan_today(&self) -> String {
        self.json(ai::plan_today(&self.service))
    }

    #[tool(description = "Suggest the highest urgency next task")]
    fn suggest_next_task(&self) -> String {
        self.json(ai::suggest_next_task(&self.service))
    }

    #[tool(description = "Create a task. Available only when MCP writes are enabled")]
    fn create_task(&self, Parameters(input): Parameters<CreateTaskInput>) -> String {
        let status = input
            .status
            .as_deref()
            .unwrap_or("inbox")
            .parse::<TaskStatus>();
        self.json(
            status
                .map_err(openmgmt_core::db::CoreError::Validation)
                .and_then(|status| {
                    self.service.create_task(NewTask {
                        project_id: input.project_id,
                        title: input.title,
                        description: input.description,
                        status,
                        priority: input.priority.unwrap_or(3),
                        due_at: input.due_at,
                        scheduled_at: input.scheduled_at,
                        estimated_minutes: input.estimated_minutes,
                        time_limit_minutes: input.time_limit_minutes,
                        pinned: input.pinned.unwrap_or(false),
                        tags: input.tags.unwrap_or_default(),
                    })
                }),
        )
    }

    #[tool(description = "Update a task. Available only when MCP writes are enabled")]
    fn update_task(&self, Parameters(input): Parameters<UpdateTaskInput>) -> String {
        let status = match input.status {
            Some(value) => match value.parse::<TaskStatus>() {
                Ok(status) => Some(status),
                Err(error) => return serde_json::json!({"error": error}).to_string(),
            },
            None => None,
        };
        self.json(self.service.update_task(
            &input.id,
            TaskPatch {
                title: input.title,
                description: input.description.map(Some),
                status,
                priority: input.priority,
                due_at: input.due_at.map(Some),
                scheduled_at: input.scheduled_at.map(Some),
                estimated_minutes: input.estimated_minutes.map(Some),
                time_limit_minutes: input.time_limit_minutes.map(Some),
                pinned: input.pinned,
                blocked_reason: input.blocked_reason.map(Some),
                tags: input.tags,
            },
        ))
    }

    #[tool(description = "Complete a task. Available only when MCP writes are enabled")]
    fn complete_task(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.complete_task(&input.id))
    }

    #[tool(description = "Create a project. Available only when MCP writes are enabled")]
    fn create_project(&self, Parameters(input): Parameters<CreateProjectInput>) -> String {
        let project_type = input
            .project_type
            .as_deref()
            .unwrap_or("other")
            .parse::<ProjectType>();
        self.json(
            project_type
                .map_err(openmgmt_core::db::CoreError::Validation)
                .and_then(|project_type| {
                    self.service.create_project(NewProject {
                        organization_id: input.organization_id,
                        name: input.name,
                        slug: None,
                        description: input.description,
                        project_type,
                        status: ProjectStatus::Active,
                        priority: input.priority.unwrap_or(3),
                        deadline: input.deadline,
                        repo_url: input.repo_url,
                        notes: input.notes,
                    })
                }),
        )
    }

    #[tool(description = "Start the timer for a task. Available only when MCP writes are enabled")]
    fn start_task_timer(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.start_task_timer(&input.id))
    }

    #[tool(
        description = "Pause the running timer for a task. Available only when MCP writes are enabled"
    )]
    fn pause_task_timer(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.pause_task_timer(&input.id))
    }

    #[tool(
        description = "Resume a paused timer for a task. Available only when MCP writes are enabled"
    )]
    fn resume_task_timer(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.resume_task_timer(&input.id))
    }

    #[tool(description = "Stop the timer for a task. Available only when MCP writes are enabled")]
    fn stop_task_timer(&self, Parameters(input): Parameters<IdInput>) -> String {
        self.json(self.service.stop_task_timer(&input.id))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for OpenMgmtMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            if self.writes_enabled {
                "OpenMgmt local project management. Read and write tools are enabled."
            } else {
                "OpenMgmt local project management. Write tools are disabled."
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmgmt_core::{AiSettingsPatch, Database};

    #[test]
    fn write_tools_are_hidden_by_default() {
        let database = Database::in_memory().unwrap();
        let server = OpenMgmtMcp::new(AppService::new(database), false);
        assert!(server.tool_router.has_route("list_tasks"));
        assert!(server.tool_router.has_route("query_tasks"));
        assert!(server.tool_router.has_route("triage_backlog"));
        assert!(!server.tool_router.has_route("create_task"));
        assert!(!server.tool_router.has_route("complete_task"));
        assert!(!server.tool_router.has_route("start_task_timer"));
    }

    #[test]
    fn write_tools_can_be_enabled_explicitly() {
        let database = Database::in_memory().unwrap();
        let server = OpenMgmtMcp::new(AppService::new(database), true);
        assert!(server.tool_router.has_route("create_task"));
        assert!(server.tool_router.has_route("create_project"));
        assert!(server.tool_router.has_route("start_task_timer"));
        assert!(server.tool_router.has_route("stop_task_timer"));
    }

    #[test]
    fn ai_settings_write_toggle_gates_write_tools() {
        let database = Database::in_memory().unwrap();
        let service = AppService::new(database);
        service
            .update_ai_settings(AiSettingsPatch {
                write_enabled: Some(false),
                ..Default::default()
            })
            .unwrap();
        // The env gate alone is not enough: the persisted setting wins.
        let server = OpenMgmtMcp::new(service, true);
        assert!(server.tool_router.has_route("list_tasks"));
        assert!(!server.tool_router.has_route("create_task"));
        assert!(!server.tool_router.has_route("start_task_timer"));
    }

    #[test]
    fn ai_settings_read_toggle_hides_read_tools() {
        let database = Database::in_memory().unwrap();
        let service = AppService::new(database);
        service
            .update_ai_settings(AiSettingsPatch {
                read_enabled: Some(false),
                ..Default::default()
            })
            .unwrap();
        let server = OpenMgmtMcp::new(service, true);
        assert!(!server.tool_router.has_route("list_tasks"));
        assert!(!server.tool_router.has_route("query_tasks"));
        assert!(!server.tool_router.has_route("triage_backlog"));
    }

    #[test]
    fn remote_mode_never_exposes_destructive_tools() {
        // The remote constructor applies the same #15 permission model as
        // the local one, plus the remote invariant: destructive tools are
        // never exposed over the network, even if the persisted settings
        // would allow them.
        let database = Database::in_memory().unwrap();
        let local = OpenMgmtMcp::new(AppService::new(database), true);
        let database = Database::in_memory().unwrap();
        let remote = OpenMgmtMcp::new_remote(AppService::new(database), true);
        for tool in ai::ai_tool_registry() {
            let expected = !tool.destructive && local.tool_router.has_route(tool.name.as_str());
            assert_eq!(
                remote.tool_router.has_route(tool.name.as_str()),
                expected,
                "remote exposure of {}",
                tool.name
            );
        }
    }
}
