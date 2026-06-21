use crate::{
    db::{CoreError, Result},
    models::{
        BoardState, LocalAiAccessMode, LocalAiChatMessage, LocalAiChatResponse,
        LocalAiConnectionResult, LocalAiModel, LocalAiModelListResult, LocalAiNativeToolCall,
        LocalAiSettings, LocalAiToolDefinition, LocalAiWorkflowResponse, Project, Task,
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
/// Reasonable `num_ctx` fallback when we can't detect a model's context length.
pub const DEFAULT_CONTEXT_LENGTH: u64 = 4096;
pub const LOCAL_AI_SYSTEM_PROMPT: &str = "You are OpenMgmt's local command assistant. P1 is highest priority. Use OpenMgmt context. Be concise and operational. Never claim to have changed data unless a tool was executed. For writes, propose an action instead of pretending to perform it. Prefer known tool names.";

/// Per-request knobs, modeled on Zed's Ollama options (`num_ctx`, `keep_alive`,
/// thinking, native tools). All optional so plain chat sends a minimal body.
#[derive(Debug, Default, Clone)]
pub struct ChatOptions {
    /// Native Ollama `tools` array. Only set when the model supports tools and
    /// the turn actually wants tool use (never for pure chat or write planning).
    pub tools: Option<Value>,
    /// `num_ctx` override (user setting or detected model context length).
    pub num_ctx: Option<u64>,
    /// Ask a thinking-capable model to think; we keep the thinking out of the
    /// visible answer either way (see [`strip_thinking`]).
    pub think: bool,
}

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

    /// Discover installed models (`/api/tags`) and enrich each with its
    /// capability record (`/api/show`). Like Zed we detect tools/vision/thinking
    /// and context length; embedding models are flagged (the UI demotes them).
    ///
    // ponytail: /api/show runs sequentially (gentlest on Ollama, no extra dep);
    // switch to bounded-concurrent fetches via `futures` only if model lists grow large.
    pub async fn list_models(&self) -> LocalAiModelListResult {
        let value = match self
            .client
            .get(self.endpoint("/api/tags"))
            .send()
            .await
            .and_then(|response| response.error_for_status())
        {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => value,
                Err(error) => {
                    return LocalAiModelListResult {
                        connected: false,
                        models: Vec::new(),
                        error: Some(format!("malformed Ollama model list response: {error}")),
                    };
                }
            },
            Err(error) => {
                return LocalAiModelListResult {
                    connected: false,
                    models: Vec::new(),
                    error: Some(format!("Ollama unavailable: {error}")),
                };
            }
        };
        let mut models = parse_ollama_tags(&value);
        for model in &mut models {
            if let Ok(show) = self.show_model(&model.name).await {
                enrich_model_from_show(model, &show);
            }
        }
        LocalAiModelListResult {
            connected: true,
            models,
            error: None,
        }
    }

    /// Fetch one model's `/api/show` detail payload (capabilities, model_info,
    /// parameters). Errors are swallowed by the caller so one bad model never
    /// breaks the whole list.
    pub async fn show_model(&self, name: &str) -> Result<Value> {
        self.client
            .post(self.endpoint("/api/show"))
            .json(&json!({ "model": name }))
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|error| CoreError::Validation(format!("Ollama show failed: {error}")))?
            .json::<Value>()
            .await
            .map_err(|error| {
                CoreError::Validation(format!("malformed Ollama show response: {error}"))
            })
    }

    pub async fn chat(
        &self,
        settings: &LocalAiSettings,
        model: Option<String>,
        messages: Vec<LocalAiChatMessage>,
        options: ChatOptions,
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
        let mut opts = serde_json::Map::new();
        if let Some(temperature) = settings.temperature {
            opts.insert("temperature".into(), json!(temperature));
        }
        if let Some(num_ctx) = options.num_ctx {
            opts.insert("num_ctx".into(), json!(num_ctx));
        }
        if !opts.is_empty() {
            body["options"] = Value::Object(opts);
        }
        // Only send a tools array when the caller decided the model supports it.
        if let Some(tools) = options.tools {
            body["tools"] = tools;
        }
        if options.think {
            body["think"] = json!(true);
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
            let detail = |key: &str| {
                details
                    .as_ref()
                    .and_then(|details| details.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            let family = detail("family");
            Some(LocalAiModel {
                display_name: Some(name.clone()),
                size: item.get("size").and_then(Value::as_i64),
                modified_at: item
                    .get("modified_at")
                    .and_then(Value::as_str)
                    .and_then(parse_time),
                digest: item
                    .get("digest")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                parameter_size: detail("parameter_size"),
                quantization_level: detail("quantization_level"),
                context_length: None,
                supports_tools: false,
                supports_vision: false,
                supports_thinking: false,
                is_embedding_model: is_embedding_model(&name, family.as_deref()),
                family,
                details,
                installed: true,
                name,
            })
        })
        .collect()
}

/// Fold an `/api/show` payload into a model record: capabilities (`tools`,
/// `vision`, `thinking`), context length, and parameter/quantization detail.
pub fn enrich_model_from_show(model: &mut LocalAiModel, show: &Value) {
    let capabilities = show
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let has = |name: &str| capabilities.iter().any(|cap| cap == name);
    model.supports_tools = has("tools");
    model.supports_vision = has("vision");
    model.supports_thinking = has("thinking") || has("reasoning");
    if has("embedding") || has("embed") {
        model.is_embedding_model = true;
    }
    if let Some(context_length) = parse_context_length(show) {
        model.context_length = Some(context_length);
    }
    if let Some(details) = show.get("details") {
        if model.parameter_size.is_none() {
            model.parameter_size = details
                .get("parameter_size")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if model.quantization_level.is_none() {
            model.quantization_level = details
                .get("quantization_level")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if model.family.is_none() {
            model.family = details
                .get("family")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
}

/// Detect a model's context length, Zed-style: prefer the architecture-specific
/// `model_info` key (`<arch>.context_length`), then fall back to a `num_ctx`
/// line in the `parameters` block.
pub fn parse_context_length(show: &Value) -> Option<u64> {
    if let Some(info) = show.get("model_info").and_then(Value::as_object) {
        let arch = info
            .get("general.architecture")
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(value) = info
            .get(&format!("{arch}.context_length"))
            .or_else(|| {
                info.iter()
                    .find(|(key, _)| key.ends_with(".context_length"))
                    .map(|(_, value)| value)
            })
            .and_then(Value::as_u64)
        {
            return Some(value);
        }
    }
    // `parameters` is a free-form text block: "num_ctx    8192".
    show.get("parameters")
        .and_then(Value::as_str)
        .and_then(|text| {
            text.lines()
                .filter_map(|line| line.trim().strip_prefix("num_ctx"))
                .filter_map(|rest| rest.split_whitespace().next())
                .find_map(|value| value.parse::<u64>().ok())
        })
}

/// Embedding-only models can't chat. Ollama tags don't flag this directly, so we
/// match the conventional `embed`/`bge` naming (and a `bert` family) until
/// `/api/show` capabilities confirm it.
pub fn is_embedding_model(name: &str, family: Option<&str>) -> bool {
    let name = name.to_lowercase();
    name.contains("embed")
        || name.contains("bge")
        || name.starts_with("nomic-embed")
        || family.is_some_and(|family| family.eq_ignore_ascii_case("bert"))
}

pub fn parse_ollama_chat_response(value: &Value) -> Result<LocalAiChatResponse> {
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_OLLAMA_MODEL)
        .to_owned();
    let message = value.get("message");
    // Content is optional: a tool-only turn may carry no text, and a thinking
    // model may put reasoning in `thinking` (or inline `<think>…</think>` which
    // we strip) rather than `content`.
    let content = message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(strip_thinking)
        .unwrap_or_default();
    let tool_calls = message
        .map(parse_native_tool_calls)
        .unwrap_or_default()
        .into_iter()
        .map(|call| LocalAiNativeToolCall {
            name: call.tool_name,
            arguments: call.arguments,
        })
        .collect();
    Ok(LocalAiChatResponse {
        model,
        content,
        total_duration: value.get("total_duration").and_then(Value::as_i64),
        load_duration: value.get("load_duration").and_then(Value::as_i64),
        prompt_eval_count: value.get("prompt_eval_count").and_then(Value::as_i64),
        eval_count: value.get("eval_count").and_then(Value::as_i64),
        tool_calls,
        done_reason: value
            .get("done_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Remove `<think>…</think>` reasoning blocks (and a dangling unclosed one) so a
/// thinking model's chain-of-thought never leaks into the visible answer.
pub fn strip_thinking(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(open) = rest.find("<think>") {
        out.push_str(&rest[..open]);
        match rest[open..].find("</think>") {
            Some(close) => rest = &rest[open + close + "</think>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

/// Parse native Ollama `message.tool_calls[].function {name, arguments}` into our
/// internal [`ParsedToolCall`] shape. `arguments` may arrive as an object or as
/// a JSON string.
pub fn parse_native_tool_calls(message: &Value) -> Vec<ParsedToolCall> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| {
            let function = call.get("function").unwrap_or(call);
            let tool_name = function
                .get("name")
                .and_then(Value::as_str)?
                .trim()
                .to_owned();
            if tool_name.is_empty() {
                return None;
            }
            let arguments = match function.get("arguments") {
                Some(Value::String(text)) => {
                    serde_json::from_str(text).unwrap_or_else(|_| json!({}))
                }
                Some(value) => value.clone(),
                None => json!({}),
            };
            Some(ParsedToolCall {
                tool_name,
                arguments,
            })
        })
        .collect()
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
// Conversational agent runtime.
//
// A user turn is first classified (chat / read / write / clarify). Pure chat
// never sees the tool manifest and never calls tools; reads answer from injected
// context; writes go through a complete-plan builder that is validated and then
// access-gated. Everything here is pure and unit-tested; execution, the access
// gate, and Ollama I/O live in `commands.rs`.
// ---------------------------------------------------------------------------

/// One tool the model asked to run, before validation/resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    pub tool_name: String,
    pub arguments: Value,
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

// --- Turn router -----------------------------------------------------------

/// What kind of turn the user took. The router is deterministic so a tiny local
/// model can never turn a greeting into a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAiTurnIntent {
    /// Conversational; no workspace data, no tools.
    Chat,
    /// A question about the workspace; read context, no mutation.
    Read,
    /// A request to change the workspace; plan-first, access-gated.
    Write,
    /// A write-ish request too vague to plan; ask for specifics.
    Clarify,
}

/// Classify a user message. Deterministic guardrails first (pure chat can never
/// be misread as a tool action), then write verbs, then read questions.
pub fn classify_turn(message: &str) -> LocalAiTurnIntent {
    let raw = message.trim();
    if raw.is_empty() {
        return LocalAiTurnIntent::Chat;
    }
    let lower = raw.to_lowercase();

    // 1) Pure-chat guardrails win outright — "say the words clear schedule" is
    //    chat, never a clear_task_schedule call.
    if is_pure_chat(&lower) {
        return LocalAiTurnIntent::Chat;
    }

    // 2) Write intent: a write verb plus an identifiable target.
    if contains_write_verb(&lower) {
        let has_target = contains_target_noun(&lower)
            || has_quoted(raw)
            || lower.contains("called ")
            || lower.contains("named ");
        return if has_target {
            LocalAiTurnIntent::Write
        } else {
            LocalAiTurnIntent::Clarify
        };
    }

    // 3) Read: a question about the workspace.
    if is_read_question(&lower) {
        return LocalAiTurnIntent::Read;
    }

    // 4) Anything else stays conversational.
    LocalAiTurnIntent::Chat
}

fn is_pure_chat(lower: &str) -> bool {
    const EXACT: &[&str] = &[
        "hello",
        "hi",
        "hey",
        "yo",
        "thanks",
        "thank you",
        "ty",
        "ok",
        "okay",
        "k",
        "nevermind",
        "never mind",
        "test",
        "testing",
        "cool",
        "nice",
        "lol",
        "hmm",
        "sup",
    ];
    if EXACT.contains(&lower) {
        return true;
    }
    const PREFIX: &[&str] = &[
        "say ",
        "repeat ",
        "echo ",
        "what are you",
        "who are you",
        "what can you do",
        "what do you do",
        "what is openmgmt",
        "hello ",
        "hi ",
        "hey ",
        "thanks",
        "thank you",
        "good morning",
        "good afternoon",
        "good evening",
        "how are you",
    ];
    PREFIX.iter().any(|prefix| lower.starts_with(prefix))
}

fn contains_write_verb(lower: &str) -> bool {
    // Whole-word verbs (so "increate"/"unblocked" don't false-match wrongly).
    const VERBS: &[&str] = &[
        "create",
        "make",
        "add",
        "new",
        "update",
        "change",
        "rename",
        "edit",
        "set",
        "schedule",
        "reschedule",
        "start",
        "begin",
        "block",
        "unblock",
        "complete",
        "finish",
        "cancel",
        "archive",
        "pause",
        "resume",
        "stop",
        "assign",
        "mark",
        "move",
    ];
    let words = lower
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    VERBS.iter().any(|verb| words.contains(verb))
        || lower.contains("clear schedule")
        || lower.contains("clear the schedule")
}

fn contains_target_noun(lower: &str) -> bool {
    const NOUNS: &[&str] = &[
        "organization",
        "organizations",
        "organisation",
        "org",
        "orgs",
        "project",
        "projects",
        "task",
        "tasks",
        "schedule",
        "timer",
        "priority",
        "deadline",
        "board",
        "subtask",
    ];
    let words = lower
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    NOUNS.iter().any(|noun| words.contains(noun))
}

fn has_quoted(raw: &str) -> bool {
    raw.matches('"').count() >= 2 || raw.matches('\'').count() >= 2 || raw.matches('`').count() >= 2
}

fn is_read_question(lower: &str) -> bool {
    if lower.ends_with('?') {
        return true;
    }
    const STARTS: &[&str] = &[
        "what",
        "which",
        "when",
        "where",
        "who",
        "how many",
        "how much",
        "do i",
        "is there",
        "are there",
        "show",
        "list",
        "summarize",
        "summarise",
        "tell me",
        "give me",
        "find",
        "search",
    ];
    const CONTAINS: &[&str] = &[
        "work on next",
        "next task",
        "should i",
        "what's next",
        "whats next",
        "overdue",
        "my tasks",
        "my schedule",
        "my projects",
        "my day",
        "in progress",
        "this week",
    ];
    STARTS.iter().any(|start| lower.starts_with(start))
        || CONTAINS.iter().any(|needle| lower.contains(needle))
}

/// Extract the literal a `say`/`repeat`/`echo` turn wants echoed back, so pure
/// chat is deterministic ("say the words something" → "something").
pub fn parse_say_command(message: &str) -> Option<String> {
    let trimmed = message.trim();
    let lower = trimmed.to_lowercase();
    for verb in ["say", "repeat", "echo"] {
        let Some(rest_lower) = lower.strip_prefix(verb) else {
            continue;
        };
        if !rest_lower.is_empty() && !rest_lower.starts_with(char::is_whitespace) {
            continue; // "saying", "sayonara" — not a say command.
        }
        let rest = strip_say_filler(trimmed[verb.len()..].trim_start());
        let echoed = unquote(rest).trim().to_owned();
        if !echoed.is_empty() {
            return Some(echoed);
        }
    }
    None
}

/// Drop a leading "the words"/"the phrase"/"that" filler from a say command.
fn strip_say_filler(rest: &str) -> &str {
    const FILLERS: &[&str] = &[
        "the words",
        "the word",
        "the phrase",
        "the following",
        "the text",
        "that",
        "this",
    ];
    let lower = rest.to_lowercase();
    for filler in FILLERS {
        if let Some(after) = lower.strip_prefix(filler)
            && after
                .chars()
                .next()
                .is_none_or(|character| character.is_whitespace() || character == '"')
        {
            return rest[filler.len()..].trim_start();
        }
    }
    rest
}

fn unquote(text: &str) -> &str {
    let text = text.trim();
    for quote in ['"', '\'', '`'] {
        if let Some(inner) = text.strip_prefix(quote)
            && let Some(inner) = inner.strip_suffix(quote)
        {
            return inner;
        }
    }
    text
}

// --- Prompts ---------------------------------------------------------------

/// Pure-chat persona: no tools, no JSON, no workspace dump.
pub fn chat_system_prompt() -> String {
    "You are OpenMgmt Local AI, a friendly local assistant for the user's OpenMgmt workspace. \
     Reply conversationally in plain language. Do NOT output JSON, code, or tool calls. \
     If the user asks you to say or repeat something, just say it back. Keep replies short."
        .into()
}

/// Read persona: answer a workspace question from injected context.
pub fn read_system_prompt() -> String {
    "You are OpenMgmt Local AI, a local assistant for the user's OpenMgmt workspace. \
     Answer the user's question using the workspace context provided. P1 is the highest \
     priority, P5 the lowest. Be concise and operational. Reply in plain language — no JSON, \
     no tool calls. If the context doesn't contain the answer, say so plainly."
        .into()
}

/// Write planner: ask for a complete, ordered `action_plan` in one JSON object.
pub fn write_planning_prompt(manifest: &str) -> String {
    format!(
        "You are OpenMgmt Local AI's planner. The user wants to change their OpenMgmt workspace. \
Produce a COMPLETE, ordered plan of typed tool calls that fulfills the WHOLE request at once. \
P1 is the highest priority.\n\n\
WRITE TOOLS:\n{manifest}\n\n\
Reply with EXACTLY ONE JSON object — no prose, no code fence:\n\
{{\"type\":\"action_plan\",\"summary\":\"<one short sentence>\",\"steps\":[{{\"tool_name\":\"<tool>\",\"arguments\":{{...}}}}]}}\n\n\
Rules:\n\
- Include EVERY action the request needs, in order. Org, project, and task ⇒ three steps.\n\
- Create prerequisites first; reference things you create by name with *_selector args, e.g. \"project_selector\":\"Website\".\n\
- Use \"organization_selector\":\"any\" when the org doesn't matter.\n\
- Use ONLY the tools listed above; never invent tools or fields."
    )
}

/// Recent user/assistant history mapped into chat messages (tool/system rows
/// skipped — pure chat and reads don't replay tool transcripts).
fn history_messages(
    history: &[crate::models::LocalAiChatMessageRecord],
    limit: usize,
) -> Vec<LocalAiChatMessage> {
    use crate::models::LocalAiChatRole;
    let start = history.len().saturating_sub(limit);
    history[start..]
        .iter()
        .filter_map(|record| {
            let role = match record.role {
                LocalAiChatRole::User => "user",
                LocalAiChatRole::Assistant => "assistant",
                LocalAiChatRole::Tool | LocalAiChatRole::System => return None,
            };
            Some(LocalAiChatMessage {
                role: role.into(),
                content: record.content.clone(),
            })
        })
        .collect()
}

/// Seed a pure-chat exchange: chat persona + recent conversation.
pub fn build_chat_messages(
    history: &[crate::models::LocalAiChatMessageRecord],
    limit: usize,
) -> Vec<LocalAiChatMessage> {
    let mut messages = vec![LocalAiChatMessage {
        role: "system".into(),
        content: chat_system_prompt(),
    }];
    messages.extend(history_messages(history, limit));
    messages
}

/// Seed a read exchange: read persona + workspace context + recent conversation.
pub fn build_read_messages(
    context: &str,
    history: &[crate::models::LocalAiChatMessageRecord],
    limit: usize,
) -> Vec<LocalAiChatMessage> {
    let mut messages = vec![LocalAiChatMessage {
        role: "system".into(),
        content: format!(
            "{}\n\nWorkspace context:\n{}",
            read_system_prompt(),
            context
        ),
    }];
    messages.extend(history_messages(history, limit));
    messages
}

// --- Action plans ----------------------------------------------------------

/// One step of a write plan: a typed tool plus its arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedStep {
    pub tool_name: String,
    pub arguments: Value,
}

/// A complete write plan the model (or our deterministic extractor) produced.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionPlan {
    pub summary: String,
    pub steps: Vec<PlannedStep>,
}

/// Parse an `action_plan` JSON object (pure, fenced, or embedded in prose).
/// Returns `None` when there's no usable `steps` array.
pub fn parse_action_plan(content: &str) -> Option<ActionPlan> {
    let json = extract_json_object(content)?;
    let value: Value = serde_json::from_str(&json).ok()?;
    let steps = value
        .get("steps")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(parse_planned_step)
        .collect::<Vec<_>>();
    if steps.is_empty() {
        return None;
    }
    Some(ActionPlan {
        summary: value
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        steps,
    })
}

fn parse_planned_step(value: &Value) -> Option<PlannedStep> {
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
    Some(PlannedStep {
        tool_name,
        arguments: value
            .get("arguments")
            .or_else(|| value.get("args"))
            .or_else(|| value.get("parameters"))
            .cloned()
            .unwrap_or_else(|| json!({})),
    })
}

/// Validate that every step targets a known write tool — rejects unknown tools
/// and stray reads from a write plan.
pub fn plan_is_executable(steps: &[PlannedStep]) -> bool {
    !steps.is_empty()
        && steps
            .iter()
            .all(|step| local_ai_tool(&step.tool_name).is_some_and(|tool| tool.write))
}

/// Deterministically build a single task **status change** (complete / start /
/// cancel / block / unblock / clear schedule) when the request names a task, so
/// these common one-shot writes never depend on a flaky small model. Returns an
/// empty plan when the verb or the task target is unclear (the model planner
/// then handles it — e.g. updates, or scheduling with a specific time).
pub fn plan_action_from_message(message: &str) -> Vec<PlannedStep> {
    let lower = message.to_lowercase();
    // Create requests are handled by `plan_from_message`; never here.
    if looks_like_create(&lower) {
        return Vec::new();
    }
    let words = lower
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let has = |verb: &str| words.contains(&verb);

    let tool = if lower.contains("clear schedule") || lower.contains("clear the schedule") {
        "clear_task_schedule"
    } else if has("unblock") {
        "unblock_task"
    } else if has("complete") || has("finish") || has("done") {
        "complete_task"
    } else if has("cancel") {
        "cancel_task"
    } else if has("block") {
        "block_task"
    } else if has("start") || has("begin") {
        "start_task"
    } else {
        return Vec::new();
    };

    match extract_task_target(message) {
        Some(target) => vec![PlannedStep {
            tool_name: tool.into(),
            arguments: json!({ "task_selector": target }),
        }],
        None => Vec::new(),
    }
}

/// Find the task a status-change request is about: a `task called/named NAME`,
/// a quoted name, or the words right after a `task` keyword.
fn extract_task_target(message: &str) -> Option<String> {
    if let Some(name) = extract_entity_name(message, &["task"]) {
        return Some(name);
    }
    for quote in ['"', '\'', '`'] {
        if let Some(after) = message.split_once(quote).map(|(_, rest)| rest)
            && let Some(name) = after.split(quote).next()
            && !name.trim().is_empty()
        {
            return Some(name.trim().to_owned());
        }
    }
    // Words right after a standalone "task" keyword, minus a leading article.
    let lower = message.to_lowercase();
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("task") {
        let at = from + offset;
        let after = at + "task".len();
        let before_boundary = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
        let after_boundary = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_boundary && after_boundary {
            let mut rest = message[after..].trim_start();
            for article in ["the ", "a ", "an ", "my "] {
                if let Some(stripped) = rest.strip_prefix(article) {
                    rest = stripped.trim_start();
                }
            }
            let name = take_name(rest);
            if !name.is_empty() {
                return Some(name);
            }
        }
        from = after;
    }
    None
}

/// Deterministically build a create-chain plan straight from the request, so the
/// canonical "organization X → project Y → task Z" never depends on a flaky
/// small model. Returns an empty plan when the message isn't a create request.
pub fn plan_from_message(message: &str) -> Vec<PlannedStep> {
    if !looks_like_create(message) {
        return Vec::new();
    }
    let organization = extract_entity_name(message, &["organization", "organisation", "org"]);
    let project = extract_entity_name(message, &["project"]);
    let task = extract_entity_name(message, &["task"]);

    let mut steps = Vec::new();
    if let Some(name) = &organization {
        steps.push(PlannedStep {
            tool_name: "create_organization".into(),
            arguments: json!({ "name": name }),
        });
    }
    if let Some(name) = &project {
        // "under that" → depend on the org we just created, else any org.
        let organization_selector = organization.clone().unwrap_or_else(|| "any".into());
        steps.push(PlannedStep {
            tool_name: "create_project".into(),
            arguments: json!({ "name": name, "organization_selector": organization_selector }),
        });
    }
    if let Some(name) = &task {
        let project_selector = project.clone().unwrap_or_else(|| "any".into());
        steps.push(PlannedStep {
            tool_name: "create_task".into(),
            arguments: json!({ "title": name, "project_selector": project_selector }),
        });
    }
    steps
}

/// For a create request, names mentioned by the user that the plan doesn't
/// cover with a matching create step. Drives the planner's one repair pass.
pub fn plan_covers_request(message: &str, steps: &[PlannedStep]) -> Vec<String> {
    if !looks_like_create(message) {
        return Vec::new();
    }
    let mut missing = Vec::new();
    let mut require = |name: Option<String>, tool: &str, key: &str| {
        if let Some(name) = name {
            let covered = steps.iter().any(|step| {
                step.tool_name == tool
                    && step
                        .arguments
                        .get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(&name))
            });
            if !covered {
                missing.push(name);
            }
        }
    };
    require(
        extract_entity_name(message, &["organization", "organisation", "org"]),
        "create_organization",
        "name",
    );
    require(
        extract_entity_name(message, &["project"]),
        "create_project",
        "name",
    );
    require(
        extract_entity_name(message, &["task"]),
        "create_task",
        "title",
    );
    missing
}

fn looks_like_create(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "create", "make ", "add ", "new ", "set up", "setup", "spin up",
    ]
    .iter()
    .any(|verb| lower.contains(verb))
}

/// Find an entity's name from "`<keyword>` … called/named `<NAME>`".
fn extract_entity_name(message: &str, keywords: &[&str]) -> Option<String> {
    let lower = message.to_lowercase();
    let bytes = lower.as_bytes();
    for keyword in keywords {
        let mut from = 0;
        while let Some(offset) = lower[from..].find(keyword) {
            let at = from + offset;
            let after = at + keyword.len();
            let before_boundary = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
            let after_boundary = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
            if before_boundary
                && after_boundary
                && let Some(name) = name_after(&lower, message, after)
            {
                return Some(name);
            }
            from = after;
        }
    }
    None
}

fn name_after(lower: &str, original: &str, from: usize) -> Option<String> {
    const MARKERS: &[&str] = &[" called ", " named ", " titled ", " labeled ", " labelled "];
    let (marker_at, marker_len) = MARKERS
        .iter()
        .filter_map(|marker| {
            lower[from..]
                .find(marker)
                .map(|at| (from + at, marker.len()))
        })
        .min_by_key(|(at, _)| *at)?;
    let name = take_name(&original[marker_at + marker_len..]);
    (!name.is_empty()).then_some(name)
}

/// Take a name after a "called"/"named" marker: a quoted span, or words up to a
/// clause boundary ("and"/"then"/"under"/punctuation).
fn take_name(slice: &str) -> String {
    let slice = slice.trim_start();
    for quote in ['"', '\'', '`'] {
        if let Some(rest) = slice.strip_prefix(quote)
            && let Some(end) = rest.find(quote)
        {
            return rest[..end].trim().to_owned();
        }
    }
    let lower = slice.to_lowercase();
    let mut end = slice.len();
    for stop in [
        " and ", " then ", " under ", " with ", " that ", " in ", " for ",
    ] {
        if let Some(at) = lower.find(stop) {
            end = end.min(at);
        }
    }
    for (index, character) in slice.char_indices() {
        if matches!(character, ',' | '.' | ';' | ':' | '\n') {
            end = end.min(index);
            break;
        }
    }
    slice[..end]
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '`'))
        .to_owned()
}

// --- Native Ollama tools (capability-gated) --------------------------------

/// Whether to attach a native Ollama `tools` array for this turn. Never for pure
/// chat or write planning (writes are plan-first); reads may use tools when the
/// selected model reports tool support.
pub fn should_send_native_tools(intent: LocalAiTurnIntent, supports_tools: bool) -> bool {
    matches!(intent, LocalAiTurnIntent::Read) && supports_tools
}

/// Build an Ollama-native `tools` array from typed tool definitions.
pub fn native_tool_schema(tools: &[LocalAiToolDefinition]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": {
                            "type": "object",
                            "properties": tool.input_schema,
                        },
                    },
                })
            })
            .collect(),
    )
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

/// A compact, model-friendly listing of the WRITE tools and their argument
/// shapes — the only tools the planner is allowed to use.
pub fn write_tool_manifest_text() -> String {
    local_ai_tool_registry()
        .iter()
        .filter(|tool| tool.write)
        .map(|tool| {
            format!(
                "- {}: {} args={}",
                tool.name, tool.description, tool.input_schema
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    // --- router ------------------------------------------------------------

    #[test]
    fn router_keeps_pure_chat_away_from_tools() {
        use LocalAiTurnIntent::*;
        assert_eq!(classify_turn("say the words something"), Chat);
        assert_eq!(classify_turn("say the words \"clear schedule\""), Chat);
        assert_eq!(classify_turn("hello"), Chat);
        assert_eq!(classify_turn("thanks!"), Chat);
        assert_eq!(classify_turn("what can you do?"), Chat);
        assert_eq!(classify_turn("what are you"), Chat);
    }

    #[test]
    fn router_detects_read_and_write_and_clarify() {
        use LocalAiTurnIntent::*;
        assert_eq!(classify_turn("what should I work on next?"), Read);
        assert_eq!(classify_turn("list my overdue tasks"), Read);
        assert_eq!(classify_turn("create a task called Draft brief"), Write);
        assert_eq!(
            classify_turn("create org A then project B then task C"),
            Write
        );
        assert_eq!(classify_turn("complete task Draft brief"), Write);
        assert_eq!(classify_turn("schedule task Draft brief tomorrow"), Write);
        // Write verb with no target/object → ask for specifics.
        assert_eq!(classify_turn("update"), Clarify);
        assert_eq!(classify_turn("change it"), Clarify);
    }

    #[test]
    fn say_command_echoes_payload() {
        assert_eq!(
            parse_say_command("say the words something").as_deref(),
            Some("something")
        );
        assert_eq!(
            parse_say_command("say \"something\"").as_deref(),
            Some("something")
        );
        assert_eq!(
            parse_say_command("repeat the phrase clear schedule").as_deref(),
            Some("clear schedule")
        );
        assert_eq!(parse_say_command("create a task"), None);
    }

    // --- action plans ------------------------------------------------------

    #[test]
    fn parses_action_plan_plain_and_fenced() {
        let plan = parse_action_plan(
            r#"{"type":"action_plan","summary":"x","steps":[{"tool_name":"create_task","arguments":{"title":"X"}}]}"#,
        )
        .unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].tool_name, "create_task");
        assert_eq!(plan.steps[0].arguments["title"], "X");

        let fenced = parse_action_plan(
            "Here:\n```json\n{\"steps\":[{\"tool_name\":\"complete_task\",\"arguments\":{\"task_selector\":\"Y\"}}]}\n```",
        )
        .unwrap();
        assert_eq!(fenced.steps[0].tool_name, "complete_task");

        // Malformed / no steps → None.
        assert_eq!(parse_action_plan("not json"), None);
        assert_eq!(parse_action_plan(r#"{"summary":"x"}"#), None);
    }

    #[test]
    fn plan_executable_rejects_unknown_and_read_tools() {
        assert!(plan_is_executable(&[PlannedStep {
            tool_name: "create_task".into(),
            arguments: json!({ "title": "X" }),
        }]));
        // Unknown tool.
        assert!(!plan_is_executable(&[PlannedStep {
            tool_name: "drop_database".into(),
            arguments: json!({}),
        }]));
        // Read tool in a write plan.
        assert!(!plan_is_executable(&[PlannedStep {
            tool_name: "list_tasks".into(),
            arguments: json!({}),
        }]));
        assert!(!plan_is_executable(&[]));
    }

    #[test]
    fn deterministic_plan_builds_full_create_chain() {
        let steps = plan_from_message(
            "create an organization called localtest, and then a project under that called localproject and then a task under that called localtask",
        );
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].tool_name, "create_organization");
        assert_eq!(steps[0].arguments["name"], "localtest");
        assert_eq!(steps[1].tool_name, "create_project");
        assert_eq!(steps[1].arguments["name"], "localproject");
        // "under that" resolves the project to the org we just created.
        assert_eq!(steps[1].arguments["organization_selector"], "localtest");
        assert_eq!(steps[2].tool_name, "create_task");
        assert_eq!(steps[2].arguments["title"], "localtask");
        assert_eq!(steps[2].arguments["project_selector"], "localproject");
    }

    #[test]
    fn deterministic_plan_ignores_non_create_requests() {
        assert!(plan_from_message("complete the task called Draft brief").is_empty());
        assert!(plan_from_message("what tasks are overdue?").is_empty());
    }

    #[test]
    fn deterministic_action_plan_handles_status_changes() {
        let cases = [
            (
                "complete the task called Draft brief",
                "complete_task",
                "Draft brief",
            ),
            ("complete task Draft", "complete_task", "Draft"),
            ("start task Onboarding", "start_task", "Onboarding"),
            ("cancel the task \"Old idea\"", "cancel_task", "Old idea"),
            ("unblock task Release", "unblock_task", "Release"),
            (
                "clear schedule for task Standup",
                "clear_task_schedule",
                "Standup",
            ),
        ];
        for (message, tool, target) in cases {
            let steps = plan_action_from_message(message);
            assert_eq!(steps.len(), 1, "{message}");
            assert_eq!(steps[0].tool_name, tool, "{message}");
            assert_eq!(steps[0].arguments["task_selector"], target, "{message}");
        }
        // Create requests and scheduling-with-a-time stay out of this path.
        assert!(plan_action_from_message("create a task called X").is_empty());
        assert!(plan_action_from_message("what should I do next?").is_empty());
    }

    #[test]
    fn coverage_validator_flags_missing_steps() {
        let message = "create an org called Acme, a project called Web, and a task called Draft";
        // An incomplete plan (missing the task) is reported.
        let partial = vec![
            PlannedStep {
                tool_name: "create_organization".into(),
                arguments: json!({ "name": "Acme" }),
            },
            PlannedStep {
                tool_name: "create_project".into(),
                arguments: json!({ "name": "Web" }),
            },
        ];
        assert_eq!(
            plan_covers_request(message, &partial),
            vec!["Draft".to_string()]
        );
        // The deterministic plan covers everything.
        assert!(plan_covers_request(message, &plan_from_message(message)).is_empty());
    }

    // --- capability detection ---------------------------------------------

    #[test]
    fn tags_parser_flags_embedding_models() {
        let models = parse_ollama_tags(&json!({
            "models": [
                { "name": "qwen3:1.7b", "details": { "family": "qwen3", "parameter_size": "1.7B", "quantization_level": "Q4_K_M" } },
                { "name": "nomic-embed-text:latest", "details": { "family": "nomic-bert" } },
            ]
        }));
        assert_eq!(models.len(), 2);
        assert!(!models[0].is_embedding_model);
        assert_eq!(models[0].parameter_size.as_deref(), Some("1.7B"));
        assert_eq!(models[0].quantization_level.as_deref(), Some("Q4_K_M"));
        assert!(models[1].is_embedding_model);
    }

    #[test]
    fn show_enrichment_detects_capabilities_and_context() {
        let mut model = parse_ollama_tags(&json!({ "models": [{ "name": "qwen3:1.7b" }] }))
            .pop()
            .unwrap();
        enrich_model_from_show(
            &mut model,
            &json!({
                "capabilities": ["completion", "tools", "thinking"],
                "model_info": { "general.architecture": "qwen3", "qwen3.context_length": 40960 },
            }),
        );
        assert!(model.supports_tools);
        assert!(model.supports_thinking);
        assert!(!model.supports_vision);
        assert_eq!(model.context_length, Some(40960));
    }

    #[test]
    fn context_length_falls_back_to_num_ctx_parameters() {
        let from_params =
            parse_context_length(&json!({ "parameters": "stop \"<|end|>\"\nnum_ctx    8192\n" }));
        assert_eq!(from_params, Some(8192));
        // Nothing detectable → None (caller uses DEFAULT_CONTEXT_LENGTH).
        assert_eq!(parse_context_length(&json!({})), None);
    }

    // --- native tool wiring ------------------------------------------------

    #[test]
    fn native_tools_gated_by_intent_and_capability() {
        use LocalAiTurnIntent::*;
        // Pure chat never gets tools, even on a tool-capable model.
        assert!(!should_send_native_tools(Chat, true));
        // Writes are plan-first, never native tools.
        assert!(!should_send_native_tools(Write, true));
        // Reads use tools only when the model supports them.
        assert!(should_send_native_tools(Read, true));
        assert!(!should_send_native_tools(Read, false));
    }

    #[test]
    fn parses_native_tool_calls_and_empty_content() {
        let response = parse_ollama_chat_response(&json!({
            "model": "qwen3:1.7b",
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    { "function": { "name": "create_task", "arguments": { "title": "X" } } }
                ]
            },
            "prompt_eval_count": 11,
            "eval_count": 7,
            "done_reason": "stop"
        }))
        .unwrap();
        assert_eq!(response.content, "");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "create_task");
        assert_eq!(response.tool_calls[0].arguments["title"], "X");
        assert_eq!(response.prompt_eval_count, Some(11));
        assert_eq!(response.eval_count, Some(7));
        assert_eq!(response.done_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn strips_thinking_blocks_from_content() {
        assert_eq!(
            strip_thinking("<think>reasoning…</think>something"),
            "something"
        );
        assert_eq!(
            strip_thinking("answer <think>aside</think> here"),
            "answer  here"
        );
        // Unterminated think block is dropped entirely.
        assert_eq!(strip_thinking("<think>still going"), "");
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
            context_window: None,
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
