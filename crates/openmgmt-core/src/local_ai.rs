use crate::{
    db::{CoreError, Result},
    models::{
        BoardState, LocalAiAccessMode, LocalAiChatMessage, LocalAiChatResponse,
        LocalAiConnectionResult, LocalAiModel, LocalAiModelListResult, LocalAiSettings,
        LocalAiToolDefinition, LocalAiWorkflowResponse, Project, Task, TaskWithContext,
    },
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{net::Ipv4Addr, time::Duration};

pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen3:1.7b";
pub const DEFAULT_OLLAMA_KEEP_ALIVE: &str = "5m";
pub const LOCAL_AI_SYSTEM_PROMPT: &str = "You are OpenMgmt's local command assistant. P1 is highest priority. Use OpenMgmt context. Be concise and operational. Never claim to have changed data unless a tool was executed. For writes, propose an action instead of pretending to perform it. Prefer known tool names.";

/// Hard cap on the agent's tool/think iterations per user turn, so a confused
/// local model can never loop forever.
pub const MAX_AGENT_STEPS: usize = 4;

pub struct OllamaClient {
    base_url: String,
    client: Client,
}

impl OllamaClient {
    pub fn new(settings: &LocalAiSettings, timeout_seconds: u64) -> Result<Self> {
        validate_local_ai_base_url(&settings.base_url, settings.allow_local_network)?;
        Ok(Self {
            base_url: settings.base_url.trim_end_matches('/').to_owned(),
            client: Client::builder()
                .timeout(Duration::from_secs(timeout_seconds))
                .build()
                .map_err(|error| {
                    CoreError::Validation(format!("could not create HTTP client: {error}"))
                })?,
        })
    }

    pub async fn test_connection(&self) -> LocalAiConnectionResult {
        match self
            .client
            .get(self.endpoint("/api/version"))
            .send()
            .await
            .and_then(|response| response.error_for_status())
        {
            Ok(response) => match response.json::<OllamaVersionResponse>().await {
                Ok(value) => LocalAiConnectionResult {
                    connected: true,
                    version: value.version,
                    error: None,
                },
                Err(error) => disconnected(format!("malformed Ollama version response: {error}")),
            },
            Err(error) => disconnected(format!("Ollama unavailable: {error}")),
        }
    }

    pub async fn list_models(&self) -> LocalAiModelListResult {
        match self
            .client
            .get(self.endpoint("/api/tags"))
            .send()
            .await
            .and_then(|response| response.error_for_status())
        {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => LocalAiModelListResult {
                    connected: true,
                    models: parse_ollama_tags(&value),
                    error: None,
                },
                Err(error) => LocalAiModelListResult {
                    connected: false,
                    models: Vec::new(),
                    error: Some(format!("malformed Ollama model list response: {error}")),
                },
            },
            Err(error) => LocalAiModelListResult {
                connected: false,
                models: Vec::new(),
                error: Some(format!("Ollama unavailable: {error}")),
            },
        }
    }

    pub async fn chat(
        &self,
        settings: &LocalAiSettings,
        model: Option<String>,
        messages: Vec<LocalAiChatMessage>,
    ) -> Result<LocalAiChatResponse> {
        let model = model
            .or_else(|| settings.default_model.clone())
            .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.into());
        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "keep_alive": settings.keep_alive.as_deref().unwrap_or(DEFAULT_OLLAMA_KEEP_ALIVE),
        });
        if let Some(temperature) = settings.temperature {
            body["options"] = json!({ "temperature": temperature });
        }
        let value = self
            .client
            .post(self.endpoint("/api/chat"))
            .json(&body)
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|error| CoreError::Validation(format!("Ollama chat failed: {error}")))?
            .json::<Value>()
            .await
            .map_err(|error| {
                CoreError::Validation(format!("malformed Ollama chat response: {error}"))
            })?;
        parse_ollama_chat_response(&value)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

pub fn validate_local_ai_base_url(value: &str, allow_local_network: bool) -> Result<()> {
    let trimmed = value.trim().trim_end_matches('/');
    let Some(rest) = trimmed.strip_prefix("http://") else {
        return Err(CoreError::Validation(
            "Ollama base URL must be an http:// local URL".into(),
        ));
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    if authority.is_empty() || authority.contains('@') {
        return Err(CoreError::Validation("invalid Ollama base URL".into()));
    }
    let (host, port) = authority
        .rsplit_once(':')
        .map(|(host, port)| (host, Some(port)))
        .unwrap_or((authority, None));
    if let Some(port) = port
        && (port.is_empty() || !port.chars().all(|character| character.is_ascii_digit()))
    {
        return Err(CoreError::Validation("invalid Ollama base URL port".into()));
    }
    if matches!(host, "localhost" | "127.0.0.1") {
        return Ok(());
    }
    if allow_local_network && is_private_ipv4(host) {
        return Ok(());
    }
    Err(CoreError::Validation(
        "Ollama base URL must point to localhost unless local-network access is enabled".into(),
    ))
}

pub fn parse_ollama_tags(value: &Value) -> Vec<LocalAiModel> {
    value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item
                .get("name")
                .or_else(|| item.get("model"))
                .and_then(Value::as_str)?
                .to_owned();
            let details = item.get("details").cloned();
            Some(LocalAiModel {
                display_name: Some(name.clone()),
                name,
                size: item.get("size").and_then(Value::as_i64),
                modified_at: item
                    .get("modified_at")
                    .and_then(Value::as_str)
                    .and_then(parse_time),
                digest: item
                    .get("digest")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                family: details
                    .as_ref()
                    .and_then(|details| details.get("family"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                details,
                installed: true,
            })
        })
        .collect()
}

pub fn parse_ollama_chat_response(value: &Value) -> Result<LocalAiChatResponse> {
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_OLLAMA_MODEL)
        .to_owned();
    let content = value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CoreError::Validation("Ollama chat response did not include assistant content".into())
        })?
        .to_owned();
    Ok(LocalAiChatResponse {
        model,
        content,
        total_duration: value.get("total_duration").and_then(Value::as_i64),
        load_duration: value.get("load_duration").and_then(Value::as_i64),
        prompt_eval_count: value.get("prompt_eval_count").and_then(Value::as_i64),
        eval_count: value.get("eval_count").and_then(Value::as_i64),
    })
}

pub fn prompt_messages(prompt: String) -> Vec<LocalAiChatMessage> {
    vec![
        LocalAiChatMessage {
            role: "system".into(),
            content: LOCAL_AI_SYSTEM_PROMPT.into(),
        },
        LocalAiChatMessage {
            role: "user".into(),
            content: prompt,
        },
    ]
}

pub fn chat_prompt_messages(context: String, user_message: String) -> Vec<LocalAiChatMessage> {
    vec![
        LocalAiChatMessage {
            role: "system".into(),
            content: LOCAL_AI_SYSTEM_PROMPT.into(),
        },
        LocalAiChatMessage {
            role: "user".into(),
            content: format!("{context}\n\nUser:\n{user_message}"),
        },
    ]
}

pub fn local_ai_tool_registry() -> Vec<LocalAiToolDefinition> {
    vec![
        // --- read tools -----------------------------------------------------
        read_tool(
            "get_workspace_summary",
            "Counts of organizations, projects, and active tasks.",
            json!({}),
        ),
        read_tool(
            "list_organizations",
            "List active organizations.",
            json!({}),
        ),
        read_tool("list_projects", "List active projects.", json!({})),
        read_tool("list_tasks", "List active tasks.", json!({})),
        read_tool("get_board", "Current board state (Now/Next/etc).", json!({})),
        read_tool(
            "get_daily_operations",
            "Board plus overdue and unscheduled tasks.",
            json!({}),
        ),
        read_tool("get_schedule_today", "Today's scheduled tasks.", json!({})),
        read_tool(
            "get_schedule_week",
            "This week's scheduled tasks.",
            json!({}),
        ),
        read_tool(
            "get_unscheduled_tasks",
            "Active tasks with no schedule.",
            json!({}),
        ),
        read_tool("get_overdue_tasks", "Tasks past their due date.", json!({})),
        read_tool(
            "get_task",
            "Get one task.",
            json!({ "task_id": "uuid", "task_selector": "title (fuzzy)" }),
        ),
        read_tool(
            "get_project",
            "Get one project.",
            json!({ "project_id": "uuid", "project_selector": "name (fuzzy)" }),
        ),
        read_tool("get_scoring_settings", "Urgency scoring weights.", json!({})),
        read_tool("get_sync_status", "Local-first sync status.", json!({})),
        // --- write tools ----------------------------------------------------
        write_tool(
            "create_organization",
            "Create an organization.",
            json!({ "name": "string (required)", "description": "string" }),
            false,
        )
        .with_examples(&["make an organization called Black Candle"]),
        write_tool(
            "update_organization",
            "Rename or re-describe an organization.",
            json!({ "organization_selector": "name or 'any'", "name": "string", "description": "string" }),
            false,
        ),
        write_tool(
            "archive_organization",
            "Archive (hide) an organization.",
            json!({ "organization_selector": "name" }),
            true,
        ),
        write_tool(
            "create_project",
            "Create a project under an organization.",
            json!({ "name": "string (required)", "organization_selector": "name or 'any'", "description": "string", "priority": "1-5" }),
            false,
        )
        .with_examples(&["create a project called Website under Black Candle"]),
        write_tool(
            "update_project",
            "Update a project's fields.",
            json!({ "project_selector": "name", "name": "string", "priority": "1-5", "status": "active|paused|completed" }),
            false,
        ),
        write_tool(
            "archive_project",
            "Archive (hide) a project.",
            json!({ "project_selector": "name" }),
            true,
        ),
        write_tool(
            "create_task",
            "Create a task in a project.",
            json!({ "title": "string (required)", "project_selector": "name or 'any'", "description": "string", "priority": "1-5" }),
            false,
        )
        .with_examples(&["add a task called do things to localtest"]),
        write_tool(
            "update_task",
            "Update a task's fields.",
            json!({ "task_selector": "title", "title": "string", "description": "string", "priority": "1-5" }),
            false,
        ),
        write_tool(
            "start_task",
            "Mark a task in progress.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "block_task",
            "Block a task with a reason.",
            json!({ "task_selector": "title", "reason": "string" }),
            false,
        ),
        write_tool(
            "unblock_task",
            "Return a blocked task to ready.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "complete_task",
            "Mark a task done.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "cancel_task",
            "Cancel a task.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "schedule_task",
            "Schedule a task into a time block (RFC3339 times).",
            json!({ "task_selector": "title", "start_at": "RFC3339", "end_at": "RFC3339" }),
            false,
        ),
        write_tool(
            "reschedule_task",
            "Move a task's time block (RFC3339 times).",
            json!({ "task_selector": "title", "start_at": "RFC3339", "end_at": "RFC3339" }),
            false,
        ),
        write_tool(
            "clear_task_schedule",
            "Remove a task's schedule.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "start_task_timer",
            "Start the work timer for a task.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "pause_task_timer",
            "Pause a task's timer.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "resume_task_timer",
            "Resume a task's timer.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "stop_task_timer",
            "Stop a task's timer.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "complete_task_with_timer",
            "Stop the timer and complete a task.",
            json!({ "task_selector": "title" }),
            false,
        ),
        write_tool(
            "update_scoring_settings",
            "Adjust urgency scoring weights.",
            json!({ "priority_weight": "int", "overdue_boost": "int", "due_soon_boost": "int" }),
            false,
        ),
        write_tool(
            "reset_scoring_settings",
            "Reset scoring weights to defaults.",
            json!({}),
            true,
        ),
    ]
}

pub fn local_ai_tool(name: &str) -> Option<LocalAiToolDefinition> {
    let needle = name.trim();
    local_ai_tool_registry()
        .into_iter()
        .find(|tool| tool.name == needle || tool.aliases.iter().any(|alias| alias == needle))
}

// ---------------------------------------------------------------------------
// Agent protocol: structured tool-calling over plain Ollama chat.
//
// Local models rarely support native function calling, so we instruct the model
// to emit one JSON object per turn and parse it defensively (fenced, loose, or
// surrounded by prose). Parsing lives here, pure and unit-tested; execution and
// the access gate live in `commands.rs`.
// ---------------------------------------------------------------------------

/// One tool the model asked to run, before validation/resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    pub tool_name: String,
    pub arguments: Value,
}

/// What the model decided this turn.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentDecision {
    Final {
        message: String,
    },
    ToolCalls {
        message: Option<String>,
        calls: Vec<ParsedToolCall>,
    },
}

/// What the access mode allows for a single tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDecision {
    Execute,
    Propose,
    Blocked,
}

/// The central access gate: read tools always run; write tools depend on mode.
pub fn tool_decision(access_mode: LocalAiAccessMode, write: bool) -> ToolDecision {
    match (access_mode, write) {
        (_, false) => ToolDecision::Execute,
        (LocalAiAccessMode::ReadOnly, true) => ToolDecision::Blocked,
        (LocalAiAccessMode::AskBeforeWrite, true) => ToolDecision::Propose,
        (LocalAiAccessMode::FullAccess, true) => ToolDecision::Execute,
    }
}

/// Parse the model's reply into a decision. Returns `None` when no usable JSON
/// is present (caller retries once, then falls back to plain text).
pub fn parse_agent_response(content: &str) -> Option<AgentDecision> {
    let json = extract_json_object(content)?;
    let value: Value = serde_json::from_str(&json).ok()?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|text| !text.trim().is_empty());

    let calls = value
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(parse_one_tool_call)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if kind == "final" {
        return Some(AgentDecision::Final {
            message: message.unwrap_or_default(),
        });
    }
    if !calls.is_empty() {
        return Some(AgentDecision::ToolCalls { message, calls });
    }
    // No tool calls: treat any message we found as a final answer.
    message.map(|message| AgentDecision::Final { message })
}

fn parse_one_tool_call(value: &Value) -> Option<ParsedToolCall> {
    let tool_name = value
        .get("tool_name")
        .or_else(|| value.get("name"))
        .or_else(|| value.get("tool"))
        .and_then(Value::as_str)?
        .trim()
        .to_owned();
    if tool_name.is_empty() {
        return None;
    }
    let arguments = value
        .get("arguments")
        .or_else(|| value.get("args"))
        .or_else(|| value.get("parameters"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    Some(ParsedToolCall {
        tool_name,
        arguments,
    })
}

/// Find the first balanced `{ ... }` object in arbitrary text. Handles pure
/// JSON, JSON wrapped in ```fences```, and JSON embedded in prose, and ignores
/// braces inside JSON strings.
fn extract_json_object(content: &str) -> Option<String> {
    let bytes = content.as_bytes();
    let start = content.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for index in start..bytes.len() {
        let character = bytes[index] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(content[start..=index].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// A compact, model-friendly listing of every tool and its argument shape.
pub fn build_tool_manifest_text() -> String {
    local_ai_tool_registry()
        .iter()
        .map(|tool| {
            let access = if tool.write { "write" } else { "read" };
            format!(
                "- {} [{access}]: {} args={}",
                tool.name, tool.description, tool.input_schema
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The agent system prompt: persona, access-mode rules, tool manifest, and the
/// strict JSON output contract.
pub fn agent_system_prompt(access_mode: LocalAiAccessMode, manifest: &str) -> String {
    let mode_line = match access_mode {
        LocalAiAccessMode::ReadOnly => {
            "ACCESS MODE: READ ONLY. You may call read tools. Do NOT call write tools. If the user asks to change data, return a final message saying you need write access (ask them to switch to Ask First or Full Access)."
        }
        LocalAiAccessMode::AskBeforeWrite => {
            "ACCESS MODE: ASK FIRST. Read tools run automatically. Call write tools normally; the app will show the user a confirmation card before anything changes."
        }
        LocalAiAccessMode::FullAccess => {
            "ACCESS MODE: FULL ACCESS. Read and write tools run automatically without confirmation."
        }
    };
    format!(
        "You are OpenMgmt's local assistant, embedded in the app. Talk like a helpful agent in a terminal. \
P1 is the highest priority, P5 the lowest. You can read and manage OpenMgmt data ONLY through the tools below. \
Never say you changed data unless a tool actually ran.\n\n\
{mode_line}\n\n\
TOOLS:\n{manifest}\n\n\
Reply with EXACTLY ONE JSON object and nothing else. Two shapes:\n\
1) {{\"type\":\"final\",\"message\":\"...\"}}\n\
2) {{\"type\":\"tool_calls\",\"message\":\"short note\",\"tool_calls\":[{{\"tool_name\":\"...\",\"arguments\":{{...}}}}]}}\n\n\
Rules: prefer *_selector args (a name, or \"any\") over ids. To act on something by name, pass e.g. \"project_selector\":\"Website\". \
Create prerequisites first (a project before its task). After tools run you will see their results; then reply with a final message. Keep messages short."
    )
}

/// Build the in-memory message list that seeds the agent loop: system prompt,
/// a workspace-context turn, then recent chat history (most recent `limit`).
pub fn build_agent_messages(
    system_prompt: String,
    context: &str,
    history: &[crate::models::LocalAiChatMessageRecord],
    limit: usize,
) -> Vec<LocalAiChatMessage> {
    use crate::models::LocalAiChatRole;
    let mut messages = vec![
        LocalAiChatMessage {
            role: "system".into(),
            content: system_prompt,
        },
        LocalAiChatMessage {
            role: "user".into(),
            content: format!("Workspace context (for reference):\n{context}"),
        },
        LocalAiChatMessage {
            role: "assistant".into(),
            content: "{\"type\":\"final\",\"message\":\"Ready.\"}".into(),
        },
    ];
    let start = history.len().saturating_sub(limit);
    for record in &history[start..] {
        let role = match record.role {
            LocalAiChatRole::User => "user",
            LocalAiChatRole::Assistant => "assistant",
            // Map tool results into the prompt as plain user-visible context;
            // not every local model understands a dedicated tool role.
            LocalAiChatRole::Tool => "user",
            LocalAiChatRole::System => continue,
        };
        let content = if matches!(record.role, LocalAiChatRole::Tool) {
            format!("Tool result: {}", record.content)
        } else {
            record.content.clone()
        };
        messages.push(LocalAiChatMessage {
            role: role.into(),
            content,
        });
    }
    messages
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    Plan,
    Board,
    Tasks(Option<String>),
    ScheduleToday,
    ScheduleWeek,
    Unscheduled,
    Models,
    UseModel(String),
    CreateTask(String),
    CompleteTask(String),
    StartTask(String),
}

pub fn parse_slash_command(input: &str) -> Result<Option<SlashCommand>> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }
    let without_slash = trimmed.trim_start_matches('/').trim();
    if without_slash.is_empty() {
        return Ok(Some(SlashCommand::Help));
    }
    let mut parts = without_slash.split_whitespace();
    let command = parts.next().unwrap_or_default();
    let rest = parts.collect::<Vec<_>>().join(" ");
    let parsed = match command {
        "help" => SlashCommand::Help,
        "plan" => SlashCommand::Plan,
        "board" => SlashCommand::Board,
        "tasks" => SlashCommand::Tasks((!rest.is_empty()).then_some(rest)),
        "schedule" if rest == "today" => SlashCommand::ScheduleToday,
        "schedule" if rest == "week" => SlashCommand::ScheduleWeek,
        "unscheduled" => SlashCommand::Unscheduled,
        "models" => SlashCommand::Models,
        "use" if !rest.is_empty() => SlashCommand::UseModel(rest),
        "create" if rest.strip_prefix("task ").is_some() => {
            let title = rest.trim_start_matches("task ").trim();
            if title.is_empty() {
                return Err(CoreError::Validation("task title is required".into()));
            }
            SlashCommand::CreateTask(title.into())
        }
        "complete" if rest.strip_prefix("task ").is_some() => {
            SlashCommand::CompleteTask(rest.trim_start_matches("task ").trim().into())
        }
        "start" if rest.strip_prefix("task ").is_some() => {
            SlashCommand::StartTask(rest.trim_start_matches("task ").trim().into())
        }
        _ => {
            return Err(CoreError::Validation(format!(
                "unknown local AI slash command: /{without_slash}"
            )));
        }
    };
    Ok(Some(parsed))
}

pub fn slash_help() -> String {
    [
        "Available commands:",
        "/help",
        "/plan",
        "/board",
        "/tasks",
        "/tasks blocked",
        "/tasks overdue",
        "/schedule today",
        "/schedule week",
        "/unscheduled",
        "/models",
        "/use <model>",
        "/create task <title>",
        "/complete task <id>",
        "/start task <id>",
    ]
    .join("\n")
}

pub fn compact_task_lines(tasks: &[TaskWithContext], limit: usize) -> String {
    tasks
        .iter()
        .take(limit)
        .map(|item| {
            format!(
                "- {} P{} score {} [{}] {} / {}",
                item.task.id,
                item.task.priority,
                item.urgency_score,
                item.task.status,
                item.project_name,
                item.task.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn plan_day_prompt(board: &BoardState, schedule: &[TaskWithContext]) -> String {
    format!(
        "Build a concise plan for today.\n\n{}\nSchedule:\n{}",
        board_context(board),
        task_lines(schedule)
    )
}

pub fn suggest_next_task_prompt(board: &BoardState, tasks: &[TaskWithContext]) -> String {
    format!(
        "Choose the next single task and explain briefly.\n\n{}\nCandidates:\n{}",
        board_context(board),
        task_lines(tasks)
    )
}

pub fn triage_tasks_prompt(
    overdue: &[TaskWithContext],
    blocked: &[TaskWithContext],
    due_soon: &[TaskWithContext],
    unscheduled: &[TaskWithContext],
) -> String {
    format!(
        "Prioritize this triage list. Keep it short.\nOverdue:\n{}\nBlocked:\n{}\nDue soon:\n{}\nUnscheduled:\n{}",
        task_lines(overdue),
        task_lines(blocked),
        task_lines(due_soon),
        task_lines(unscheduled)
    )
}

pub fn summarize_project_prompt(project: &Project, tasks: &[Task]) -> String {
    let tasks = tasks
        .iter()
        .map(|task| {
            format!(
                "- P{} [{}] {}{}",
                task.priority,
                task.status,
                task.title,
                task.description
                    .as_deref()
                    .map(|value| format!(": {value}"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Summarize this project with status, risks, and next actions.\nProject: {} [{} P{}]\nNotes: {}\nTasks:\n{}",
        project.name,
        project.status,
        project.priority,
        project.notes.as_deref().unwrap_or(""),
        tasks
    )
}

pub fn rewrite_task_prompt(task: &Task, instruction: &str) -> String {
    format!(
        "Rewrite this task description only. Return suggested text, not JSON.\nTask: {}\nCurrent description: {}\nInstruction: {}",
        task.title,
        task.description.as_deref().unwrap_or(""),
        instruction
    )
}

pub fn workflow_response(
    chat: LocalAiChatResponse,
    fallback_used: bool,
    fallback_task: Option<TaskWithContext>,
) -> LocalAiWorkflowResponse {
    LocalAiWorkflowResponse {
        content: chat.content,
        model: Some(chat.model),
        fallback_used,
        fallback_task,
        error: None,
    }
}

pub fn workflow_error(error: impl ToString) -> LocalAiWorkflowResponse {
    LocalAiWorkflowResponse {
        content: String::new(),
        model: None,
        fallback_used: false,
        fallback_task: None,
        error: Some(error.to_string()),
    }
}

fn disconnected(error: String) -> LocalAiConnectionResult {
    LocalAiConnectionResult {
        connected: false,
        version: None,
        error: Some(error),
    }
}

fn read_tool(name: &str, description: &str, input_schema: Value) -> LocalAiToolDefinition {
    LocalAiToolDefinition {
        name: name.into(),
        description: description.into(),
        write: false,
        destructive: false,
        input_schema,
        examples: Vec::new(),
        aliases: Vec::new(),
    }
}

fn write_tool(
    name: &str,
    description: &str,
    input_schema: Value,
    destructive: bool,
) -> LocalAiToolDefinition {
    LocalAiToolDefinition {
        name: name.into(),
        description: description.into(),
        write: true,
        destructive,
        input_schema,
        examples: Vec::new(),
        aliases: Vec::new(),
    }
}

impl LocalAiToolDefinition {
    fn with_examples(mut self, examples: &[&str]) -> Self {
        self.examples = examples.iter().map(|value| value.to_string()).collect();
        self
    }
}

fn board_context(board: &BoardState) -> String {
    format!(
        "Board now:\n{}\nNext up:\n{}\nOverdue:\n{}\nBlocked/waiting:\n{}\nDue soon:\n{}",
        scored_lines(&board.now),
        scored_lines(&board.next_up),
        scored_lines(&board.overdue),
        scored_lines(&board.waiting_blocked),
        scored_lines(&board.due_soon)
    )
}

fn scored_lines(tasks: &[crate::models::ScoredTask]) -> String {
    tasks
        .iter()
        .take(8)
        .map(|item| {
            format!(
                "- P{} score {} [{}] {} / {}",
                item.context.task.priority,
                item.urgency_score,
                item.context.task.status,
                item.context.project_name,
                item.context.task.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn task_lines(tasks: &[TaskWithContext]) -> String {
    tasks
        .iter()
        .take(20)
        .map(|item| {
            format!(
                "- P{} score {} [{}] {} / {}",
                item.task.priority,
                item.urgency_score,
                item.task.status,
                item.project_name,
                item.task.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .ok()
}

fn is_private_ipv4(host: &str) -> bool {
    host.parse::<Ipv4Addr>().is_ok_and(|ip| {
        ip.is_private() || ip.is_loopback() || ip.octets()[0] == 169 && ip.octets()[1] == 254
    })
}

#[derive(Deserialize)]
struct OllamaVersionResponse {
    version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn invalid_base_url_rejected() {
        assert!(validate_local_ai_base_url("https://api.openai.com", false).is_err());
        assert!(validate_local_ai_base_url("http://example.com:11434", false).is_err());
        assert!(validate_local_ai_base_url(DEFAULT_OLLAMA_BASE_URL, false).is_ok());
        assert!(validate_local_ai_base_url("http://192.168.1.2:11434", false).is_err());
        assert!(validate_local_ai_base_url("http://192.168.1.2:11434", true).is_ok());
    }

    #[test]
    fn model_list_parser_handles_sample_response() {
        let models = parse_ollama_tags(&json!({
            "models": [{
                "name": "qwen3:1.7b",
                "modified_at": "2026-01-02T03:04:05Z",
                "size": 123,
                "digest": "abc",
                "details": { "family": "qwen3" }
            }]
        }));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "qwen3:1.7b");
        assert_eq!(models[0].family.as_deref(), Some("qwen3"));
        assert_eq!(models[0].size, Some(123));
    }

    #[test]
    fn chat_response_parser_handles_sample_response() {
        let response = parse_ollama_chat_response(&json!({
            "model": "qwen3:1.7b",
            "message": { "role": "assistant", "content": "Ready." },
            "total_duration": 5,
            "eval_count": 2
        }))
        .unwrap();
        assert_eq!(response.model, "qwen3:1.7b");
        assert_eq!(response.content, "Ready.");
        assert_eq!(response.eval_count, Some(2));
    }

    #[test]
    fn prompt_building_includes_p1_instruction() {
        let messages = prompt_messages("Plan.".into());
        assert!(messages[0].content.contains("P1 is highest priority"));
    }

    #[test]
    fn slash_parser_handles_supported_commands() {
        assert!(matches!(
            parse_slash_command("/help").unwrap(),
            Some(SlashCommand::Help)
        ));
        assert!(matches!(
            parse_slash_command("/board").unwrap(),
            Some(SlashCommand::Board)
        ));
        assert_eq!(
            parse_slash_command("/tasks blocked").unwrap(),
            Some(SlashCommand::Tasks(Some("blocked".into())))
        );
        assert_eq!(
            parse_slash_command("/use llama3.2:3b").unwrap(),
            Some(SlashCommand::UseModel("llama3.2:3b".into()))
        );
        assert_eq!(
            parse_slash_command("/create task Draft brief").unwrap(),
            Some(SlashCommand::CreateTask("Draft brief".into()))
        );
        assert_eq!(
            parse_slash_command("/complete task task-1").unwrap(),
            Some(SlashCommand::CompleteTask("task-1".into()))
        );
        assert_eq!(
            parse_slash_command("/start task task-1").unwrap(),
            Some(SlashCommand::StartTask("task-1".into()))
        );
        assert!(matches!(
            parse_slash_command("/models").unwrap(),
            Some(SlashCommand::Models)
        ));
    }

    #[test]
    fn tool_registry_exposes_only_safe_typed_tools() {
        let tools = local_ai_tool_registry();
        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "create_task" && tool.write)
        );
        // No raw shell / sql / filesystem / delete escape hatches.
        assert!(!tools.iter().any(|tool| {
            tool.name.contains("delete")
                || tool.name.contains("shell")
                || tool.name.contains("sql")
                || tool.name.contains("exec")
                || tool.name.contains("file")
        }));
        // Archive/reset are real tools, but must be flagged destructive.
        for name in [
            "archive_organization",
            "archive_project",
            "reset_scoring_settings",
        ] {
            assert!(
                tools
                    .iter()
                    .any(|tool| tool.name == name && tool.write && tool.destructive),
                "{name} should be a destructive write tool"
            );
        }
    }

    #[test]
    fn gate_enforces_access_modes() {
        use LocalAiAccessMode::*;
        // Reads always execute.
        assert_eq!(tool_decision(ReadOnly, false), ToolDecision::Execute);
        assert_eq!(tool_decision(FullAccess, false), ToolDecision::Execute);
        // Writes depend on the mode.
        assert_eq!(tool_decision(ReadOnly, true), ToolDecision::Blocked);
        assert_eq!(tool_decision(AskBeforeWrite, true), ToolDecision::Propose);
        assert_eq!(tool_decision(FullAccess, true), ToolDecision::Execute);
    }

    #[test]
    fn parses_final_tool_fenced_and_loose_json() {
        // Final answer.
        let final_decision = parse_agent_response(r#"{"type":"final","message":"hi there"}"#);
        assert_eq!(
            final_decision,
            Some(AgentDecision::Final {
                message: "hi there".into()
            })
        );

        // Tool calls.
        let tools = parse_agent_response(
            r#"{"type":"tool_calls","message":"ok","tool_calls":[{"tool_name":"create_project","arguments":{"name":"localtest","organization_selector":"any"}}]}"#,
        )
        .unwrap();
        match tools {
            AgentDecision::ToolCalls { message, calls } => {
                assert_eq!(message.as_deref(), Some("ok"));
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].tool_name, "create_project");
                assert_eq!(calls[0].arguments["name"], "localtest");
            }
            other => panic!("expected tool calls, got {other:?}"),
        }

        // Fenced JSON wrapped in prose.
        let fenced = parse_agent_response(
            "Sure, here you go:\n```json\n{\"type\":\"final\",\"message\":\"done\"}\n```\nThanks!",
        );
        assert_eq!(
            fenced,
            Some(AgentDecision::Final {
                message: "done".into()
            })
        );

        // No JSON at all -> None (caller falls back to plain text).
        assert_eq!(parse_agent_response("I will create that for you."), None);

        // Tool call missing a name is dropped; with none usable -> None.
        assert_eq!(
            parse_agent_response(r#"{"type":"tool_calls","tool_calls":[{"arguments":{}}]}"#),
            None
        );
    }

    #[tokio::test]
    async fn ollama_unavailable_returns_clean_error() {
        let now = Utc::now();
        let settings = LocalAiSettings {
            id: "default".into(),
            enabled: true,
            provider: "ollama".into(),
            base_url: "http://127.0.0.1:9".into(),
            default_model: Some(DEFAULT_OLLAMA_MODEL.into()),
            keep_alive: Some(DEFAULT_OLLAMA_KEEP_ALIVE.into()),
            temperature: None,
            allow_local_network: false,
            last_connected_at: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        };
        let result = OllamaClient::new(&settings, 1)
            .unwrap()
            .test_connection()
            .await;
        assert!(!result.connected);
        assert!(result.error.unwrap().contains("Ollama unavailable"));
    }
}
