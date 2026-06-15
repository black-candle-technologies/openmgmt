//! Shared application state, Tauri invoke plumbing, and small input helpers.
//!
//! Everything in this module is presentation-agnostic so that the components,
//! pages, forms, and board can all share a single source of truth.

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{
    BoardState, Organization, Project, ProjectStatus, ProjectType, Task, TaskStatus,
};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

#[wasm_bindgen]
extern "C" {
    // `catch` so a missing bridge (`window.__TAURI__` undefined, e.g. when the
    // bundle is opened outside the Tauri webview) surfaces as a recoverable
    // `Err` instead of an uncaught JS exception that would strand the board on
    // "Updating…" with no visible error.
    #[wasm_bindgen(catch, js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    fn tauri_invoke(command: &str, args: JsValue) -> Result<js_sys::Promise, JsValue>;
}

/// Top-level workspace sections shown in the sidebar.
#[derive(Clone, PartialEq)]
pub enum Page {
    Dashboard,
    Organizations,
    Projects,
    Project(String),
    Tasks,
    Today,
    Board,
}

impl Page {
    pub fn title(&self) -> &'static str {
        match self {
            Page::Dashboard => "Dashboard",
            Page::Organizations => "Organizations",
            Page::Projects => "Projects",
            Page::Project(_) => "Project",
            Page::Tasks => "Tasks",
            Page::Today => "Today",
            Page::Board => "Board",
        }
    }
}

/// A drawer is a focused side panel used for record creation and editing so the
/// main views stay calm and free of always-open forms.
#[derive(Clone)]
pub enum Drawer {
    CreateOrganization,
    EditOrganization(Organization),
    CreateProject { organization_id: Option<String> },
    EditProject(Project),
    CreateTask { project_id: Option<String> },
    EditTask(Task),
}

/// Immutable snapshot of everything the local database currently holds.
#[derive(Clone, Default)]
pub struct Snapshot {
    pub organizations: Vec<Organization>,
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub board: BoardState,
}

impl Snapshot {
    pub fn project_name(&self, project_id: &str) -> Option<String> {
        self.projects
            .iter()
            .find(|project| project.id == project_id)
            .map(|project| project.name.clone())
    }
}

/// Shared reactive state. `Copy` so it can be handed to every component cheaply.
#[derive(Clone, Copy)]
pub struct AppState {
    pub snapshot: RwSignal<Snapshot>,
    pub error: RwSignal<Option<String>>,
    pub notice: RwSignal<Option<String>>,
    pub loading: RwSignal<bool>,
    pub drawer: RwSignal<Option<Drawer>>,
    /// Wall-clock time of the last successful data load. Used by the board to
    /// show a "last refreshed" timestamp without blanking existing data.
    pub synced_at: RwSignal<Option<DateTime<Utc>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            snapshot: RwSignal::new(Snapshot::default()),
            error: RwSignal::new(None),
            notice: RwSignal::new(None),
            loading: RwSignal::new(true),
            drawer: RwSignal::new(None),
            synced_at: RwSignal::new(None),
        }
    }

    pub fn refresh(self) {
        spawn_local(async move {
            self.reload().await;
        });
    }

    pub async fn reload(self) {
        self.loading.set(true);
        match load_snapshot().await {
            Ok(snapshot) => {
                self.snapshot.set(snapshot);
                self.synced_at.set(Some(Utc::now()));
                self.error.set(None);
            }
            Err(error) => self.fail("Refresh failed", error),
        }
        self.loading.set(false);
    }

    /// Board-only refresh: loads *only* `get_board_state`, leaving the rest of
    /// the snapshot untouched. The dedicated TV window uses this so it never
    /// depends on `list_organizations`/`list_projects`/`list_tasks` succeeding,
    /// and so an in-flight refresh never blanks the board that is already shown.
    pub fn refresh_board(self) {
        spawn_local(async move {
            self.reload_board().await;
        });
    }

    pub async fn reload_board(self) {
        self.loading.set(true);
        web_sys::console::log_1(&JsValue::from_str("[board] get_board_state: requesting"));
        match invoke::<BoardState>("get_board_state", serde_json::json!({})).await {
            Ok(board) => {
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[board] get_board_state: ok ({} task(s))",
                    board_total(&board)
                )));
                // Update only the board slice so a refresh never clears the
                // board that is currently on screen.
                self.snapshot.update(|snapshot| snapshot.board = board);
                self.synced_at.set(Some(Utc::now()));
                self.error.set(None);
            }
            Err(error) => {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "[board] get_board_state: FAILED — {error}"
                )));
                self.fail("Board refresh failed", error);
            }
        }
        self.loading.set(false);
    }

    pub fn fail(self, context: &str, error: String) {
        let message = format!("{context}: {error}");
        web_sys::console::error_1(&JsValue::from_str(&message));
        self.error.set(Some(message));
    }

    pub fn open_drawer(self, drawer: Drawer) {
        self.error.set(None);
        self.drawer.set(Some(drawer));
    }

    pub fn close_drawer(self) {
        self.drawer.set(None);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn invoke<T: DeserializeOwned>(command: &str, args: impl Serialize) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|error| error.to_string())?;
    let promise = tauri_invoke(command, args)
        .map_err(|_| "Tauri bridge unavailable (window.__TAURI__ is missing)".to_string())?;
    let value = JsFuture::from(promise).await.map_err(js_error_message)?;
    serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
}

async fn load_snapshot() -> Result<Snapshot, String> {
    Ok(Snapshot {
        organizations: invoke("list_organizations", serde_json::json!({})).await?,
        projects: invoke("list_projects", serde_json::json!({})).await?,
        tasks: invoke("list_tasks", serde_json::json!({})).await?,
        board: invoke("get_board_state", serde_json::json!({})).await?,
    })
}

/// Runs a mutation result, surfaces success/error feedback, and reloads on
/// success. Returns whether the action succeeded so callers (forms) can close a
/// drawer only when there is nothing to retry.
pub async fn finish_action<T>(
    state: AppState,
    result: Result<T, String>,
    success: &'static str,
    context: &'static str,
) -> bool {
    match result {
        Ok(_) => {
            state.notice.set(Some(success.into()));
            state.reload().await;
            true
        }
        Err(error) => {
            state.fail(context, error);
            false
        }
    }
}

fn js_error_message(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| {
        js_sys::JSON::stringify(&value)
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "Unknown Tauri invoke error".into())
    })
}

/// True when the current webview was opened as the dedicated TV board window.
///
/// Detection is layered so it is robust: the query string (`?board=1` or
/// `?mode=board`) is the primary signal, and the injected
/// `window.__OPENMGMT_BOARD__` global is a fallback for environments where the
/// query is stripped.
pub fn is_board_window() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let query_mode = window
        .location()
        .search()
        .ok()
        .is_some_and(|query| query.contains("board=1") || query.contains("mode=board"));
    let initialized_mode =
        js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("__OPENMGMT_BOARD__"))
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    query_mode || initialized_mode
}

/// One-time console diagnostics for the board window, so a blank/failed board
/// can be debugged straight from the webview console.
pub fn log_board_diagnostics() {
    let window = web_sys::window();
    let search = window
        .as_ref()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let has_tauri = window
        .as_ref()
        .map(|window| {
            js_sys::Reflect::has(window.as_ref(), &JsValue::from_str("__TAURI__")).unwrap_or(false)
        })
        .unwrap_or(false);
    web_sys::console::log_1(&JsValue::from_str("[board] board mode detected"));
    web_sys::console::log_1(&JsValue::from_str(&format!(
        "[board] window.location.search = {search:?}"
    )));
    web_sys::console::log_1(&JsValue::from_str(&format!(
        "[board] window.__TAURI__ present = {has_tauri}"
    )));
}

/// Total tasks across every board column (used for diagnostics logging).
fn board_total(board: &BoardState) -> usize {
    board.now.len()
        + board.next_up.len()
        + board.due_soon.len()
        + board.waiting_blocked.len()
        + board.later_today.len()
        + board.overdue.len()
        + board.done_today.len()
}

// ---------------------------------------------------------------------------
// Form input helpers
// ---------------------------------------------------------------------------

pub fn input_value(node: NodeRef<leptos::html::Input>) -> String {
    node.get().map(|input| input.value()).unwrap_or_default()
}

pub fn textarea_value(node: NodeRef<leptos::html::Textarea>) -> String {
    node.get().map(|input| input.value()).unwrap_or_default()
}

pub fn select_value(node: NodeRef<leptos::html::Select>) -> String {
    node.get().map(|input| input.value()).unwrap_or_default()
}

pub fn checkbox_value(node: NodeRef<leptos::html::Input>) -> bool {
    node.get().map(|input| input.checked()).unwrap_or(false)
}

pub fn optional_text(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub fn parse_i32(value: String) -> Option<i32> {
    value.trim().parse().ok()
}

pub fn parse_datetime_local(value: String) -> Result<Option<DateTime<Utc>>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let date = js_sys::Date::new(&JsValue::from_str(&value));
    let milliseconds = date.get_time();
    if milliseconds.is_nan() {
        return Err(format!("Invalid date and time: {value}"));
    }
    DateTime::from_timestamp_millis(milliseconds as i64)
        .map(Some)
        .ok_or_else(|| format!("Date is outside the supported range: {value}"))
}

pub fn datetime_local_value(value: Option<DateTime<Utc>>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let date = js_sys::Date::new(&JsValue::from_f64(value.timestamp_millis() as f64));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

pub fn confirmed(message: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.confirm_with_message(message).ok())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Select option catalogues
// ---------------------------------------------------------------------------

pub fn project_type_options() -> [(ProjectType, &'static str); 9] {
    [
        (ProjectType::Software, "Software"),
        (ProjectType::Writing, "Writing"),
        (ProjectType::Business, "Business"),
        (ProjectType::FilmStory, "Film / story"),
        (ProjectType::MarketingPr, "Marketing / PR"),
        (ProjectType::Research, "Research"),
        (ProjectType::Operations, "Operations"),
        (ProjectType::Personal, "Personal"),
        (ProjectType::Other, "Other"),
    ]
}

pub fn project_status_options() -> [(ProjectStatus, &'static str); 3] {
    [
        (ProjectStatus::Active, "Active"),
        (ProjectStatus::Paused, "Paused"),
        (ProjectStatus::Completed, "Completed"),
    ]
}

pub fn task_status_options() -> [(TaskStatus, &'static str); 8] {
    [
        (TaskStatus::Inbox, "Inbox"),
        (TaskStatus::Backlog, "Backlog"),
        (TaskStatus::Scheduled, "Scheduled"),
        (TaskStatus::Ready, "Ready"),
        (TaskStatus::InProgress, "In progress"),
        (TaskStatus::Blocked, "Blocked"),
        (TaskStatus::Waiting, "Waiting"),
        (TaskStatus::Done, "Done"),
    ]
}

/// Human label for a snake_case status/enum string.
pub fn humanize(value: &str) -> String {
    let mut chars = value.replace('_', " ");
    if let Some(first) = chars.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    chars
}
