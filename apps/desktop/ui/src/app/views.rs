//! Saved-view presets and the translation layer that turns the Tasks page's
//! plain-string filter controls into a backend `TaskQueryFilter` + `TaskSort`.
//!
//! The core seeds system saved views whose `filter_json` uses human tokens
//! ("today", "due_soon") that are intentionally *not* a `TaskQueryFilter`. The
//! UI owns resolving those tokens against the live clock, so this module is that
//! resolver — keeping all query construction in one tested-by-eye place.

use chrono::{DateTime, Duration, Utc};
use openmgmt_core::{TaskQueryFilter, TaskSort, TaskSortField};

/// Plain-string mirror of the Tasks filter controls. Held in a single signal so
/// the query effect has exactly one dependency and saved-view presets can set
/// every control at once.
#[derive(Clone, Default, PartialEq)]
pub struct TaskFilterState {
    pub organization_id: String,
    pub project_id: String,
    pub status: String,
    pub tag: String,
    pub priority: String,
    /// "", "overdue", "today", "soon", "week".
    pub due_window: String,
    pub pinned_only: bool,
    pub include_done: bool,
    pub text: String,
    pub sort_field: String,
    pub sort_desc: bool,
}

impl TaskFilterState {
    /// The default Tasks view: everything active, most urgent first.
    pub fn all_active() -> Self {
        Self {
            sort_field: "urgency".into(),
            sort_desc: true,
            ..Default::default()
        }
    }
}

/// The nine system saved views, used as a fallback strip before the database is
/// loaded and to give each known slug a stable label.
pub fn default_view_presets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("all-tasks", "All Tasks"),
        ("today", "Today"),
        ("mvp", "MVP"),
        ("launch", "Launch"),
        ("bugs", "Bugs"),
        ("blocked", "Blocked"),
        ("due-soon", "Due Soon"),
        ("in-progress", "In Progress"),
        ("pinned", "Pinned"),
    ]
}

/// Resolve a saved-view slug into a concrete control state. Unknown slugs fall
/// back to the all-active view (custom views beyond the system set are a known
/// limitation until a full saved-view editor exists).
pub fn preset_for_slug(slug: &str) -> TaskFilterState {
    let mut state = TaskFilterState::all_active();
    match slug {
        "all-tasks" => state.include_done = true,
        "today" => state.due_window = "today".into(),
        "mvp" => {
            state.tag = "mvp".into();
            state.sort_field = "priority".into();
        }
        "launch" => {
            state.tag = "launch".into();
            state.sort_field = "due_at".into();
            state.sort_desc = false;
        }
        "bugs" => {
            state.tag = "bug".into();
            state.sort_field = "priority".into();
        }
        "blocked" => {
            state.status = "blocked".into();
            state.sort_field = "updated_at".into();
        }
        "due-soon" => {
            state.due_window = "soon".into();
            state.sort_field = "due_at".into();
            state.sort_desc = false;
        }
        "in-progress" => state.status = "in_progress".into(),
        "pinned" => state.pinned_only = true,
        _ => {}
    }
    state
}

/// Build the backend query from the current control state, resolving relative
/// due windows against `now`.
pub fn build_query(state: &TaskFilterState, now: DateTime<Utc>) -> (TaskQueryFilter, TaskSort) {
    let mut filter = TaskQueryFilter::default();
    if !state.organization_id.is_empty() {
        filter.organization_id = Some(state.organization_id.clone());
    }
    if !state.project_id.is_empty() {
        filter.project_id = Some(state.project_id.clone());
    }
    if let Ok(status) = state.status.parse() {
        filter.status = Some(vec![status]);
    }
    if !state.tag.is_empty() {
        filter.tags = Some(vec![state.tag.clone()]);
    }
    if let Ok(priority) = state.priority.parse::<i32>() {
        filter.priority = Some(vec![priority]);
    }
    match state.due_window.as_str() {
        "overdue" => filter.due_to = Some(now),
        "today" => filter.due_to = Some(end_of_day(now)),
        "soon" => filter.due_to = Some(now + Duration::hours(24)),
        "week" => filter.due_to = Some(now + Duration::days(7)),
        _ => {}
    }
    if state.pinned_only {
        filter.pinned = Some(true);
    }
    filter.include_done = Some(state.include_done);
    let text = state.text.trim();
    if !text.is_empty() {
        filter.text = Some(text.to_owned());
    }
    let field = state
        .sort_field
        .parse::<TaskSortField>()
        .unwrap_or(TaskSortField::Urgency);
    (
        filter,
        TaskSort {
            field,
            descending: state.sort_desc,
        },
    )
}

/// End of the current UTC day, matching the board's UTC date bucketing.
fn end_of_day(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .and_hms_opt(23, 59, 59)
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
        .unwrap_or(now)
}
