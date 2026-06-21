use crate::{
    db::{CoreError, Result},
    models::{
        BoardState, LocalAiChatMessage, LocalAiChatResponse, LocalAiConnectionResult, LocalAiModel,
        LocalAiModelListResult, LocalAiSettings, LocalAiWorkflowResponse, Project, Task,
        TaskWithContext,
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
            content: "You are OpenMgmt's local planning assistant. P1 is highest priority. Be concise and operational. Do not invent tasks not present in context unless asked. Prefer Schedule, Daily Operations, Board, Overdue, Blocked, and Due Soon.".into(),
        },
        LocalAiChatMessage {
            role: "user".into(),
            content: prompt,
        },
    ]
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
