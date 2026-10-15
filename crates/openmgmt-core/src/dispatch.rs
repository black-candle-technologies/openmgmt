//! Dispatch metadata is local-only: older sync peers replace entire task/project
//! payloads. Dedicated local columns keep those peers from erasing agent sessions.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Dispatched,
    Running,
    NeedsInput,
    PrOpened,
    Done,
    Failed,
}
impl AgentState {
    pub fn from_api(value: &str) -> Self {
        match value {
            "running" | "in_progress" => Self::Running,
            "needs_input" | "waiting_for_input" => Self::NeedsInput,
            "pr_opened" => Self::PrOpened,
            "done" | "completed" => Self::Done,
            "failed" | "canceled" | "cancelled" => Self::Failed,
            _ => Self::Dispatched,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Dispatched => "Dispatched",
            Self::Running => "Running",
            Self::NeedsInput => "Needs input",
            Self::PrOpened => "PR opened",
            Self::Done => "Done",
            Self::Failed => "Failed",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAgent {
    pub state: AgentState,
    pub dispatch_id: String,
    pub pr_url: Option<String>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBinding {
    pub owner: String,
    pub name: String,
    pub default_branch: String,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_mapping_is_forward_compatible() {
        for (wire, expected) in [
            ("queued", AgentState::Dispatched),
            ("future_state", AgentState::Dispatched),
            ("running", AgentState::Running),
            ("needs_input", AgentState::NeedsInput),
            ("pr_opened", AgentState::PrOpened),
            ("completed", AgentState::Done),
            ("failed", AgentState::Failed),
            ("cancelled", AgentState::Failed),
        ] {
            assert_eq!(AgentState::from_api(wire), expected);
        }
    }
}
