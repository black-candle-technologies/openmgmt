use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let value = match self { $(Self::$variant => $value),+ };
                f.write_str(value)
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(format!("invalid {}: {value}", stringify!($name))),
                }
            }
        }
    };
}

string_enum!(ProjectType {
    Software => "software",
    Writing => "writing",
    Business => "business",
    FilmStory => "film_story",
    MarketingPr => "marketing_pr",
    Research => "research",
    Operations => "operations",
    Personal => "personal",
    Other => "other",
});

string_enum!(ProjectStatus {
    Active => "active",
    Paused => "paused",
    Completed => "completed",
    Archived => "archived",
});

string_enum!(TaskStatus {
    Inbox => "inbox",
    Backlog => "backlog",
    Scheduled => "scheduled",
    Ready => "ready",
    InProgress => "in_progress",
    Blocked => "blocked",
    Waiting => "waiting",
    Done => "done",
    Canceled => "canceled",
});

string_enum!(TaskSortField {
    Urgency => "urgency",
    Priority => "priority",
    DueAt => "due_at",
    Status => "status",
    Project => "project",
    Organization => "organization",
    CreatedAt => "created_at",
    UpdatedAt => "updated_at",
    Tag => "tag",
});

string_enum!(RecurrenceRule {
    None => "none",
    Daily => "daily",
    Weekdays => "weekdays",
    Weekly => "weekly",
    Monthly => "monthly",
});

string_enum!(CalendarBlockSource {
    OpenMgmt => "openmgmt",
    ImportedIcs => "imported_ics",
    GoogleCalendarFuture => "google_calendar_future",
    OutlookFuture => "outlook_future",
});

string_enum!(CalendarBlockStatus {
    Planned => "planned",
    Completed => "completed",
    Skipped => "skipped",
    Moved => "moved",
    Canceled => "canceled",
});

string_enum!(LocalAiChatRole {
    User => "user",
    Assistant => "assistant",
    System => "system",
    Tool => "tool",
});

string_enum!(LocalAiToolCallStatus {
    Proposed => "proposed",
    Confirmed => "confirmed",
    Executed => "executed",
    Failed => "failed",
    Canceled => "canceled",
});

string_enum!(LocalAiContextScope {
    Minimal => "minimal",
    Daily => "daily",
    Project => "project",
    Task => "task",
    Schedule => "schedule",
    FullSummary => "full_summary",
});

string_enum!(AiProviderKind {
    OpenAi => "openai",
    Anthropic => "anthropic",
    LocalOpenAiCompatible => "local_openai_compatible",
    Ollama => "ollama",
    LmStudio => "lm_studio",
    CustomOpenAiCompatible => "custom_openai_compatible",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiToolAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiToolPermission {
    ReadData,
    WriteData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub kind: AiProviderKind,
    pub base_url: Option<String>,
    pub api_key_ref: Option<String>,
    pub default_model: Option<String>,
    pub enabled: bool,
    pub local_only: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub read_enabled: bool,
    pub write_enabled: bool,
    pub destructive_tools_enabled: bool,
    pub default_provider_id: Option<String>,
    pub default_model_id: Option<String>,
    pub local_only_mode: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiToolMetadata {
    pub name: String,
    pub description: String,
    pub access: AiToolAccess,
    pub destructive: bool,
    pub required_permission: AiToolPermission,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiToolPermissionCheck {
    pub tool: AiToolMetadata,
    pub allowed: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiSettings {
    pub id: String,
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub default_model: Option<String>,
    pub keep_alive: Option<String>,
    pub temperature: Option<f32>,
    pub allow_local_network: bool,
    pub last_connected_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalAiSettingsPatch {
    pub enabled: Option<bool>,
    pub base_url: Option<String>,
    pub default_model: Option<Option<String>>,
    pub keep_alive: Option<Option<String>>,
    pub temperature: Option<Option<f32>>,
    pub allow_local_network: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiModel {
    pub name: String,
    pub display_name: Option<String>,
    pub size: Option<i64>,
    pub modified_at: Option<DateTime<Utc>>,
    pub digest: Option<String>,
    pub family: Option<String>,
    pub details: Option<serde_json::Value>,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiConnectionResult {
    pub connected: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiModelListResult {
    pub connected: bool,
    pub models: Vec<LocalAiModel>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiChatResponse {
    pub model: String,
    pub content: String,
    pub total_duration: Option<i64>,
    pub load_duration: Option<i64>,
    pub prompt_eval_count: Option<i64>,
    pub eval_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiWorkflowResponse {
    pub content: String,
    pub model: Option<String>,
    pub fallback_used: bool,
    pub fallback_task: Option<TaskWithContext>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiChatSession {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiChatMessageRecord {
    pub id: String,
    pub session_id: String,
    pub role: LocalAiChatRole,
    pub content: String,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
    pub metadata_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiToolCall {
    pub id: String,
    pub session_id: String,
    pub message_id: Option<String>,
    pub tool_name: String,
    pub arguments_json: serde_json::Value,
    pub result_json: Option<serde_json::Value>,
    pub status: LocalAiToolCallStatus,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiToolDefinition {
    pub name: String,
    pub description: String,
    pub write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendLocalAiChatMessageInput {
    pub session_id: Option<String>,
    pub message: String,
    pub model: Option<String>,
    pub context_scope: Option<LocalAiContextScope>,
    #[serde(default)]
    pub allow_write_proposals: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiChatTurn {
    pub session: LocalAiChatSession,
    pub messages: Vec<LocalAiChatMessageRecord>,
    pub proposed_tool_calls: Vec<LocalAiToolCall>,
    pub assistant_output: Option<String>,
}

impl Default for LocalAiContextScope {
    fn default() -> Self {
        Self::Daily
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub project_type: ProjectType,
    pub status: ProjectStatus,
    pub priority: i32,
    pub deadline: Option<DateTime<Utc>>,
    pub repo_url: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: i32,
    pub due_at: Option<DateTime<Utc>>,
    pub scheduled_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scheduled_start_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scheduled_end_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub deadline_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reminder_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub recurrence_rule: Option<RecurrenceRule>,
    #[serde(default)]
    pub recurrence_anchor_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub recurrence_timezone: Option<String>,
    #[serde(default)]
    pub calendar_block_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub estimated_minutes: Option<i32>,
    pub time_limit_minutes: Option<i32>,
    pub pinned: bool,
    pub blocked_reason: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarBlock {
    pub id: String,
    pub task_id: Option<String>,
    pub project_id: Option<String>,
    pub organization_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub timezone: Option<String>,
    pub source: CalendarBlockSource,
    pub external_id: Option<String>,
    pub status: CalendarBlockStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleTaskInput {
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub timezone: Option<String>,
    pub reminder_at: Option<DateTime<Utc>>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub recurrence_rule: Option<RecurrenceRule>,
    pub recurrence_anchor_at: Option<DateTime<Utc>>,
    pub recurrence_timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConflict {
    pub first: CalendarBlock,
    pub second: CalendarBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeBlockSuggestion {
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub duration_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledBlockCompletion {
    pub block: CalendarBlock,
    pub task: Option<Task>,
    pub next_occurrence_task: Option<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskContext {
    #[serde(flatten)]
    pub task: Task,
    pub project_name: String,
    pub project_type: ProjectType,
    pub project_status: ProjectStatus,
    pub project_priority: i32,
    pub organization_name: String,
    pub organization_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredTask {
    #[serde(flatten)]
    pub context: TaskContext,
    pub urgency_score: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTimerSession {
    pub id: String,
    pub task_id: String,
    pub started_at: DateTime<Utc>,
    pub paused_at: Option<DateTime<Utc>>,
    pub resumed_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i64>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTimerInfo {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub paused_at: Option<DateTime<Utc>>,
    pub resumed_at: Option<DateTime<Utc>>,
    pub elapsed_seconds: i64,
    pub is_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskWithContext {
    pub task: Task,
    pub project_id: String,
    pub project_name: String,
    pub project_type: ProjectType,
    pub organization_id: String,
    pub organization_name: String,
    pub organization_color: Option<String>,
    pub organization_icon: Option<String>,
    pub urgency_score: i32,
    pub active_timer: Option<ActiveTimerInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskQueryFilter {
    pub organization_id: Option<String>,
    pub project_id: Option<String>,
    pub status: Option<Vec<TaskStatus>>,
    pub priority: Option<Vec<i32>>,
    pub due_from: Option<DateTime<Utc>>,
    pub due_to: Option<DateTime<Utc>>,
    pub scheduled_from: Option<DateTime<Utc>>,
    pub scheduled_to: Option<DateTime<Utc>>,
    pub pinned: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub text: Option<String>,
    pub include_done: Option<bool>,
    pub include_canceled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSort {
    pub field: TaskSortField,
    #[serde(default = "default_sort_descending")]
    pub descending: bool,
}

impl Default for TaskSort {
    fn default() -> Self {
        Self {
            field: TaskSortField::Urgency,
            descending: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTaskView {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub filter_json: serde_json::Value,
    pub sort_json: serde_json::Value,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSavedTaskView {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub filter_json: serde_json::Value,
    #[serde(default)]
    pub sort_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedTaskViewPatch {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<Option<String>>,
    pub filter_json: Option<serde_json::Value>,
    pub sort_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringSettings {
    pub id: String,
    pub priority_weight: i32,
    pub pinned_boost: i32,
    pub overdue_boost: i32,
    pub due_soon_boost: i32,
    pub in_progress_boost: i32,
    pub blocked_penalty: i32,
    pub waiting_penalty: i32,
    pub paused_project_penalty: i32,
    pub due_soon_window_hours: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScoringSettingsPatch {
    pub priority_weight: Option<i32>,
    pub pinned_boost: Option<i32>,
    pub overdue_boost: Option<i32>,
    pub due_soon_boost: Option<i32>,
    pub in_progress_boost: Option<i32>,
    pub blocked_penalty: Option<i32>,
    pub waiting_penalty: Option<i32>,
    pub paused_project_penalty: Option<i32>,
    pub due_soon_window_hours: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoardState {
    pub generated_at: DateTime<Utc>,
    pub now: Vec<ScoredTask>,
    pub next_up: Vec<ScoredTask>,
    pub due_soon: Vec<ScoredTask>,
    pub waiting_blocked: Vec<ScoredTask>,
    pub later_today: Vec<ScoredTask>,
    pub overdue: Vec<ScoredTask>,
    pub done_today: Vec<ScoredTask>,
}

impl Default for ProjectType {
    fn default() -> Self {
        Self::Other
    }
}

impl Default for ProjectStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Inbox
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewOrganization {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrganizationPatch {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub icon: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProject {
    pub organization_id: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub project_type: ProjectType,
    #[serde(default)]
    pub status: ProjectStatus,
    #[serde(default = "default_priority")]
    pub priority: i32,
    pub deadline: Option<DateTime<Utc>>,
    pub repo_url: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<Option<String>>,
    pub project_type: Option<ProjectType>,
    pub status: Option<ProjectStatus>,
    pub priority: Option<i32>,
    pub deadline: Option<Option<DateTime<Utc>>>,
    pub repo_url: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTask {
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default = "default_priority")]
    pub priority: i32,
    pub due_at: Option<DateTime<Utc>>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub estimated_minutes: Option<i32>,
    pub time_limit_minutes: Option<i32>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<i32>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub scheduled_at: Option<Option<DateTime<Utc>>>,
    pub estimated_minutes: Option<Option<i32>>,
    pub time_limit_minutes: Option<Option<i32>>,
    pub pinned: Option<bool>,
    pub blocked_reason: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
}

fn default_priority() -> i32 {
    3
}

fn default_sort_descending() -> bool {
    true
}
