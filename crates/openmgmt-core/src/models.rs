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
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
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
    pub description: Option<String>,
    pub project_type: Option<ProjectType>,
    pub status: Option<ProjectStatus>,
    pub priority: Option<i32>,
    pub deadline: Option<DateTime<Utc>>,
    pub repo_url: Option<String>,
    pub notes: Option<String>,
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
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<i32>,
    pub due_at: Option<DateTime<Utc>>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub estimated_minutes: Option<i32>,
    pub time_limit_minutes: Option<i32>,
    pub pinned: Option<bool>,
    pub blocked_reason: Option<String>,
    pub tags: Option<Vec<String>>,
}

fn default_priority() -> i32 {
    3
}
