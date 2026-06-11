use chrono::{DateTime, Utc};
use openmgmt_core::{
    AppService, NewProject, NewTask, ProjectStatus, ProjectType, TaskPatch, TaskStatus,
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
        let mut tool_router = Self::tool_router();
        if !writes_enabled {
            for name in [
                "create_task",
                "update_task",
                "complete_task",
                "create_project",
            ] {
                tool_router.disable_route(name.to_owned());
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
            focus.sort_by(|a, b| b.urgency_score.cmp(&a.urgency_score));
            focus.truncate(8);
            serde_json::json!({ "generated_at": board.generated_at, "focus": focus, "board": board })
        }))
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
                description: input.description,
                status,
                priority: input.priority,
                due_at: input.due_at,
                scheduled_at: input.scheduled_at,
                estimated_minutes: input.estimated_minutes,
                time_limit_minutes: input.time_limit_minutes,
                pinned: input.pinned,
                blocked_reason: input.blocked_reason,
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
    use openmgmt_core::Database;

    #[test]
    fn write_tools_are_hidden_by_default() {
        let database = Database::in_memory().unwrap();
        let server = OpenMgmtMcp::new(AppService::new(database), false);
        assert!(server.tool_router.has_route("list_tasks"));
        assert!(!server.tool_router.has_route("create_task"));
        assert!(!server.tool_router.has_route("complete_task"));
    }

    #[test]
    fn write_tools_can_be_enabled_explicitly() {
        let database = Database::in_memory().unwrap();
        let server = OpenMgmtMcp::new(AppService::new(database), true);
        assert!(server.tool_router.has_route("create_task"));
        assert!(server.tool_router.has_route("create_project"));
    }
}
