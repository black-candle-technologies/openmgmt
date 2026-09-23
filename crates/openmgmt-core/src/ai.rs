use crate::{
    commands::AppService,
    db::CoreError,
    models::{
        AiProvider, AiProviderKind, AiSettings, AiToolAccess, AiToolMetadata, AiToolPermission,
        AiToolPermissionCheck, BacklogTriage, ProjectSummary, ScoredTask, TaskQueryFilter,
        TaskStatus, TaskWithContext, TodayPlan,
    },
};
use chrono::{Duration, Utc};
use serde_json::json;

pub fn ai_tool_registry() -> Vec<AiToolMetadata> {
    [
        read_tool("list_organizations", "List active OpenMgmt organizations"),
        read_tool("list_projects", "List active projects"),
        read_tool("get_project", "Get one project by id"),
        read_tool("query_tasks", "Query tasks with filters and sorting"),
        read_tool(
            "list_tasks",
            "List tasks with simple project/status filters",
        ),
        read_tool("get_task", "Get one task by id"),
        read_tool("get_board_state", "Get the scored ER board"),
        read_tool("get_today_plan", "Get today's deterministic focus plan"),
        read_tool("list_saved_task_views", "List saved task views"),
        read_tool("list_timer_sessions", "List timer sessions for a task"),
        read_tool("get_scoring_settings", "Get task scoring settings"),
        write_tool("create_task", "Create a task"),
        write_tool("update_task", "Update a task"),
        write_tool("complete_task", "Complete a task"),
        write_tool("start_task_timer", "Start a task timer"),
        write_tool("pause_task_timer", "Pause a task timer"),
        write_tool("resume_task_timer", "Resume a task timer"),
        write_tool("stop_task_timer", "Stop a task timer"),
        write_tool("create_project", "Create a project"),
        read_tool(
            "summarize_project",
            "Summarize one project deterministically",
        ),
        read_tool("triage_backlog", "Group stale, blocked, and overdue tasks"),
        read_tool("plan_today", "Plan today from board state"),
        read_tool("suggest_next_task", "Suggest the highest urgency next task"),
    ]
    .into_iter()
    .collect()
}

pub fn ai_tool_metadata(name: &str) -> Option<AiToolMetadata> {
    ai_tool_registry()
        .into_iter()
        .find(|tool| tool.name == name)
}

pub fn enforce_ai_tool_permission(
    settings: &AiSettings,
    tool: &AiToolMetadata,
    mcp_writes_enabled: bool,
) -> AiToolPermissionCheck {
    let denied = |reason: &str| AiToolPermissionCheck {
        tool: tool.clone(),
        allowed: false,
        reason: Some(reason.into()),
    };

    if tool.destructive && !settings.destructive_tools_enabled {
        return denied("destructive AI tools are disabled");
    }
    match tool.access {
        AiToolAccess::Read if !settings.read_enabled => {
            return denied("AI read access is disabled");
        }
        AiToolAccess::Write if !settings.write_enabled => {
            return denied("AI write access is disabled");
        }
        AiToolAccess::Write if !mcp_writes_enabled => {
            return denied("MCP write access is disabled");
        }
        _ => {}
    }

    AiToolPermissionCheck {
        tool: tool.clone(),
        allowed: true,
        reason: None,
    }
}

pub fn provider_is_local(provider: &AiProvider) -> bool {
    provider.local_only
        || matches!(
            provider.kind,
            AiProviderKind::LocalOpenAiCompatible
                | AiProviderKind::Ollama
                | AiProviderKind::LmStudio
        )
        || provider.base_url.as_deref().is_some_and(is_local_base_url)
}

pub fn provider_allowed_by_local_only(settings: &AiSettings, provider: &AiProvider) -> bool {
    !settings.local_only_mode || provider_is_local(provider)
}

fn read_tool(name: &str, description: &str) -> AiToolMetadata {
    tool(
        name,
        description,
        AiToolAccess::Read,
        false,
        AiToolPermission::ReadData,
    )
}

fn write_tool(name: &str, description: &str) -> AiToolMetadata {
    tool(
        name,
        description,
        AiToolAccess::Write,
        false,
        AiToolPermission::WriteData,
    )
}

fn tool(
    name: &str,
    description: &str,
    access: AiToolAccess,
    destructive: bool,
    required_permission: AiToolPermission,
) -> AiToolMetadata {
    AiToolMetadata {
        name: name.into(),
        description: description.into(),
        access,
        destructive,
        required_permission,
        input_schema: json!({
            "type": "object",
            "additionalProperties": true,
            "description": "placeholder schema; provider adapters will map concrete schemas later"
        }),
    }
}

fn is_local_base_url(value: &str) -> bool {
    let rest = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"));
    let Some(rest) = rest else {
        return false;
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }

    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some(end) = bracketed.find(']') else {
            return false;
        };
        let host = &bracketed[..end];
        let remainder = &bracketed[end + 1..];
        if !remainder.is_empty() && !remainder.starts_with(':') {
            return false;
        }
        (host, remainder.strip_prefix(':'))
    } else {
        let mut parts = authority.splitn(2, ':');
        let host = parts.next().unwrap_or_default();
        (host, parts.next())
    };
    if let Some(port) = port
        && (port.is_empty() || !port.chars().all(|character| character.is_ascii_digit()))
    {
        return false;
    }

    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
}

/// Tasks untouched for this long count as stale in `triage_backlog`.
const STALE_AFTER: Duration = Duration::days(14);

fn open_tasks(service: &AppService) -> Result<Vec<TaskWithContext>, CoreError> {
    service.query_tasks(
        TaskQueryFilter {
            organization_id: None,
            project_id: None,
            status: None,
            priority: None,
            due_from: None,
            due_to: None,
            scheduled_from: None,
            scheduled_to: None,
            pinned: None,
            tags: None,
            text: None,
            include_done: Some(false),
            include_canceled: Some(false),
        },
        None,
    )
}

/// Group stale, blocked, and overdue open tasks for review.
/// Deterministic: pure function of the current task list.
pub fn triage_backlog(service: &AppService) -> Result<BacklogTriage, CoreError> {
    let now = Utc::now();
    let stale_cutoff = now - STALE_AFTER;
    let mut stale = Vec::new();
    let mut blocked = Vec::new();
    let mut overdue = Vec::new();
    for task in open_tasks(service)? {
        let is_blocked = task.task.status == TaskStatus::Blocked
            || task.task.status == TaskStatus::Waiting
            || task.task.blocked_reason.is_some();
        let is_overdue = task.task.due_at.is_some_and(|due_at| due_at < now);
        let is_stale = task.task.updated_at < stale_cutoff && !is_blocked;
        if is_stale {
            stale.push(task.clone());
        }
        if is_blocked {
            blocked.push(task.clone());
        }
        if is_overdue {
            overdue.push(task.clone());
        }
    }
    // Most urgent first within each group.
    stale.sort_by_key(|task| task.task.updated_at);
    blocked.sort_by_key(|task| task.task.updated_at);
    overdue.sort_by_key(|task| task.task.due_at);
    Ok(BacklogTriage {
        generated_at: now,
        stale,
        blocked,
        overdue,
    })
}

/// Highest-urgency next task across the board.
/// Deterministic: max `urgency_score` over now/overdue/due-soon/next-up.
pub fn suggest_next_task(service: &AppService) -> Result<Option<ScoredTask>, CoreError> {
    let board = service.get_board_state()?;
    Ok(board
        .now
        .iter()
        .chain(&board.overdue)
        .chain(&board.due_soon)
        .chain(&board.next_up)
        .max_by_key(|task| task.urgency_score)
        .cloned())
}

/// Today's deterministic focus plan: top tasks by urgency plus counts.
/// Deterministic: pure function of the board state.
pub fn plan_today(service: &AppService) -> Result<TodayPlan, CoreError> {
    let board = service.get_board_state()?;
    let mut focus: Vec<ScoredTask> = board
        .now
        .iter()
        .chain(&board.overdue)
        .chain(&board.due_soon)
        .chain(&board.next_up)
        .cloned()
        .collect();
    focus.sort_by_key(|task| std::cmp::Reverse(task.urgency_score));
    focus.truncate(8);
    Ok(TodayPlan {
        generated_at: board.generated_at,
        focus,
        overdue_count: board.overdue.len(),
        due_soon_count: board.due_soon.len(),
    })
}

/// Deterministic per-project summary: status counts, overdue/blocked
/// counts, and the suggested next task within the project.
pub fn summarize_project(
    service: &AppService,
    project_id: &str,
) -> Result<ProjectSummary, CoreError> {
    let project = service.get_project(project_id)?;
    let tasks = service.query_tasks(
        TaskQueryFilter {
            organization_id: None,
            project_id: Some(project_id.into()),
            status: None,
            priority: None,
            due_from: None,
            due_to: None,
            scheduled_from: None,
            scheduled_to: None,
            pinned: None,
            tags: None,
            text: None,
            include_done: Some(true),
            include_canceled: Some(true),
        },
        None,
    )?;
    let now = Utc::now();
    let mut by_status = std::collections::BTreeMap::new();
    let mut overdue_count = 0;
    let mut blocked_count = 0;
    for task in &tasks {
        *by_status.entry(task.task.status.to_string()).or_insert(0) += 1;
        if task.task.status != TaskStatus::Done
            && task.task.status != TaskStatus::Canceled
            && task.task.due_at.is_some_and(|due_at| due_at < now)
        {
            overdue_count += 1;
        }
        if task.task.status == TaskStatus::Blocked || task.task.blocked_reason.is_some() {
            blocked_count += 1;
        }
    }
    let board = service.get_board_state()?;
    let suggested_next = board
        .now
        .iter()
        .chain(&board.overdue)
        .chain(&board.due_soon)
        .chain(&board.next_up)
        .filter(|task| task.context.task.project_id == project_id)
        .max_by_key(|task| task.urgency_score)
        .cloned();
    Ok(ProjectSummary {
        project_id: project.id,
        project_name: project.name,
        total_tasks: tasks.len(),
        by_status,
        overdue_count,
        blocked_count,
        suggested_next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn settings() -> AiSettings {
        let now = Utc::now();
        AiSettings {
            read_enabled: true,
            write_enabled: false,
            destructive_tools_enabled: false,
            default_provider_id: None,
            default_model_id: None,
            local_only_mode: false,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn registry_classifies_read_and_write_tools() {
        let registry = ai_tool_registry();
        let query = registry
            .iter()
            .find(|tool| tool.name == "query_tasks")
            .unwrap();
        let create = registry
            .iter()
            .find(|tool| tool.name == "create_task")
            .unwrap();

        assert_eq!(query.access, AiToolAccess::Read);
        assert_eq!(create.access, AiToolAccess::Write);
        assert!(!registry.iter().any(|tool| tool.destructive));
    }

    #[test]
    fn write_and_destructive_permissions_are_blocked_by_default() {
        let mut write = ai_tool_metadata("create_task").unwrap();
        let check = enforce_ai_tool_permission(&settings(), &write, true);
        assert!(!check.allowed);

        write.destructive = true;
        let mut permissive = settings();
        permissive.write_enabled = true;
        let check = enforce_ai_tool_permission(&permissive, &write, true);
        assert!(!check.allowed);
    }

    #[test]
    fn local_base_url_requires_exact_local_host() {
        for url in [
            "http://localhost",
            "https://localhost:11434/v1",
            "http://127.0.0.1:1234",
            "https://127.0.0.1/path",
            "http://[::1]:8080/v1",
            "https://0.0.0.0:8000",
        ] {
            assert!(is_local_base_url(url), "{url} should be local");
        }

        for url in [
            "http://localhost.evil.com",
            "http://127.0.0.1.evil.com",
            "ftp://localhost",
            "http://[::1]evil",
            "http://[::1.evil]",
            "not a url",
        ] {
            assert!(!is_local_base_url(url), "{url} should not be local");
        }
    }
}
