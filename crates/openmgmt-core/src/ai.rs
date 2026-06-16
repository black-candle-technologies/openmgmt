use crate::models::{
    AiProvider, AiProviderKind, AiSettings, AiToolAccess, AiToolMetadata, AiToolPermission,
    AiToolPermissionCheck,
};
use serde_json::json;

pub fn ai_tool_registry() -> Vec<AiToolMetadata> {
    [
        read_tool("list_organizations", "List active OpenMgmt organizations"),
        read_tool("list_projects", "List active projects"),
        read_tool("get_project", "Get one project by id"),
        read_tool("query_tasks", "Query tasks with filters and sorting"),
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
    value.starts_with("http://localhost")
        || value.starts_with("http://127.0.0.1")
        || value.starts_with("http://[::1]")
        || value.starts_with("http://0.0.0.0")
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
}
