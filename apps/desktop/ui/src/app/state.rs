//! Shared application state, Tauri invoke plumbing, and small input helpers.
//!
//! Everything in this module is presentation-agnostic so that the components,
//! pages, forms, and board can all share a single source of truth.

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{
    BoardState, Organization, Project, ProjectStatus, ProjectType, RecurrenceRule, Task, TaskStatus,
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
    DailyOps,
    Organizations,
    Projects,
    Project(String),
    Tasks,
    Schedule,
    Board,
    Sync,
    Settings,
}

impl Page {
    pub fn title(&self) -> &'static str {
        match self {
            Page::Dashboard => "Dashboard",
            Page::DailyOps => "Daily Operations",
            Page::Organizations => "Organizations",
            Page::Projects => "Projects",
            Page::Project(_) => "Project",
            Page::Tasks => "Tasks",
            Page::Schedule => "Schedule",
            Page::Board => "Board",
            Page::Sync => "Sync",
            Page::Settings => "Settings",
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
    refresh_token: RwSignal<u64>,
    board_refresh_token: RwSignal<u64>,
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
            refresh_token: RwSignal::new(0),
            board_refresh_token: RwSignal::new(0),
        }
    }

    pub fn refresh(self) {
        spawn_local(async move {
            self.reload().await;
        });
    }

    pub async fn reload(self) {
        let token = self.refresh_token.get_untracked().wrapping_add(1);
        self.refresh_token.set(token);
        self.loading.set(true);
        match load_snapshot().await {
            Ok(snapshot) => {
                if self.refresh_token.get_untracked() != token {
                    return;
                }
                self.snapshot.set(snapshot);
                self.synced_at.set(Some(Utc::now()));
                self.error.set(None);
            }
            Err(error) => {
                if self.refresh_token.get_untracked() != token {
                    return;
                }
                self.fail("Refresh failed", error);
            }
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
        let token = self.board_refresh_token.get_untracked().wrapping_add(1);
        self.board_refresh_token.set(token);
        self.loading.set(true);
        web_sys::console::log_1(&JsValue::from_str("[board] get_board_state: requesting"));
        match invoke::<BoardState>("get_board_state", serde_json::json!({})).await {
            Ok(board) => {
                if self.board_refresh_token.get_untracked() != token {
                    return;
                }
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
                if self.board_refresh_token.get_untracked() != token {
                    return;
                }
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
    let query_mode = window.location().search().ok().is_some_and(|query| {
        web_sys::UrlSearchParams::new_with_str(&query).is_ok_and(|params| {
            params.get("board").as_deref() == Some("1")
                || params.get("mode").as_deref() == Some("board")
        })
    });
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
    let href = window
        .as_ref()
        .and_then(|window| window.location().href().ok())
        .unwrap_or_default();
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
        "[board] window.location.href = {href:?}"
    )));
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

/// Combine a `YYYY-MM-DD` date and `HH:MM` time (both local) into a UTC instant,
/// reusing the tested `datetime-local` → UTC bridge. Shared by the scheduling
/// surfaces (the Schedule page and the Tasks-page schedule modal).
pub fn combine_local(date: &str, time: &str) -> Result<DateTime<Utc>, String> {
    parse_datetime_local(format!("{date}T{time}"))?
        .ok_or_else(|| "Invalid date or time.".to_string())
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

// ---------------------------------------------------------------------------
// Local time/date display helpers
//
// Stored datetimes are UTC; these render them in the viewer's local timezone via
// the browser `Date`, mirroring how `datetime_local_value` / `parse_datetime_local`
// bridge the local `datetime-local` inputs. Shared by the schedule views and the
// scheduling indicators on task cards so times read consistently everywhere.
// ---------------------------------------------------------------------------

fn js_date(value: DateTime<Utc>) -> js_sys::Date {
    js_sys::Date::new(&JsValue::from_f64(value.timestamp_millis() as f64))
}

/// Local hour-of-day (0–23) for a UTC instant.
pub fn local_hour(value: DateTime<Utc>) -> u32 {
    js_date(value).get_hours()
}

/// Local `(year, month, day)` for a UTC instant.
pub fn local_ymd(value: DateTime<Utc>) -> (i32, u32, u32) {
    let date = js_date(value);
    (
        date.get_full_year() as i32,
        date.get_month() + 1,
        date.get_date(),
    )
}

/// Local `YYYY-MM-DD`, suitable for a `<input type="date">` value.
pub fn local_date_str(value: DateTime<Utc>) -> String {
    let (y, mo, d) = local_ymd(value);
    format!("{y:04}-{mo:02}-{d:02}")
}

/// Local `HH:MM` (24h), suitable for a `<input type="time">` value.
pub fn local_time_str(value: DateTime<Utc>) -> String {
    let date = js_date(value);
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

fn to_12h(hour24: u32) -> (u32, &'static str) {
    match hour24 % 24 {
        0 => (12, "AM"),
        12 => (12, "PM"),
        h if h < 12 => (h, "AM"),
        h => (h - 12, "PM"),
    }
}

/// Friendly local clock label, e.g. `2 PM` or `2:30 PM`.
pub fn fmt_time(value: DateTime<Utc>) -> String {
    let date = js_date(value);
    let (h12, ap) = to_12h(date.get_hours());
    let minutes = date.get_minutes();
    if minutes == 0 {
        format!("{h12} {ap}")
    } else {
        format!("{h12}:{minutes:02} {ap}")
    }
}

/// Local time range label, e.g. `2 PM – 3:30 PM`.
pub fn fmt_time_range(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    format!("{} – {}", fmt_time(start), fmt_time(end))
}

/// Local date + time label, e.g. `Jun 17, 2 PM`.
pub fn fmt_datetime(value: DateTime<Utc>) -> String {
    let (_, mo, d) = local_ymd(value);
    format!("{} {}, {}", month_short(mo), d, fmt_time(value))
}

/// Long local date label for the board clock, e.g. `Wednesday, June 19`.
///
/// Uses the browser `Date` so it reflects the viewer's system timezone, never
/// UTC (chrono's `.format()` on a `DateTime<Utc>` would render UTC).
pub fn fmt_clock_date(value: DateTime<Utc>) -> String {
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let date = js_date(value);
    let weekday = WEEKDAYS.get(date.get_day() as usize).copied().unwrap_or("");
    let month = MONTHS.get(date.get_month() as usize).copied().unwrap_or("");
    format!("{weekday}, {month} {}", date.get_date())
}

/// Local wall-clock label with seconds for the board clock, e.g. `2:30:45 PM`.
///
/// Renders in the viewer's system timezone via the browser `Date`, matching the
/// system clock rather than UTC.
pub fn fmt_clock_time(value: DateTime<Utc>) -> String {
    let date = js_date(value);
    let (h12, ap) = to_12h(date.get_hours());
    format!(
        "{h12}:{:02}:{:02} {ap}",
        date.get_minutes(),
        date.get_seconds()
    )
}

/// Label for a whole-hour timeline slot, e.g. `8 AM`.
pub fn hour_label(hour24: u32) -> String {
    let (h12, ap) = to_12h(hour24);
    format!("{h12} {ap}")
}

/// Three-letter month abbreviation for a 1-based month number.
pub fn month_short(month: u32) -> &'static str {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("")
}

/// Three-letter weekday abbreviation for a 0-based (Sunday) day-of-week.
pub fn weekday_short(weekday: u32) -> &'static str {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    DAYS.get(weekday as usize).copied().unwrap_or("")
}

/// Human label for a recurrence rule (`Once` for the `None` rule).
pub fn recurrence_label(rule: RecurrenceRule) -> &'static str {
    match rule {
        RecurrenceRule::None => "Once",
        RecurrenceRule::Daily => "Daily",
        RecurrenceRule::Weekdays => "Weekdays",
        RecurrenceRule::Weekly => "Weekly",
        RecurrenceRule::Monthly => "Monthly",
    }
}

pub fn confirmed(message: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.confirm_with_message(message).ok())
        .unwrap_or(false)
}

/// Native prompt with a default value. Returns `None` if the user cancels.
pub fn prompt_with_default(message: &str, default: &str) -> Option<String> {
    web_sys::window()
        .and_then(|window| {
            window
                .prompt_with_message_and_default(message, default)
                .ok()
                .flatten()
        })
        .map(|value| value.trim().to_owned())
}

/// Triggers a client-side file download of in-memory text by building a Blob,
/// an object URL, and a momentary `<a download>` click. Used by the data export
/// commands which return their payload as a string for the UI to persist.
pub fn download_text(filename: &str, content: &str) -> Result<(), String> {
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(content));
    let blob = web_sys::Blob::new_with_str_sequence(parts.as_ref())
        .map_err(|_| "Could not create file".to_string())?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|_| "Could not create download URL".to_string())?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("No document")?;
        let anchor = document
            .create_element("a")
            .map_err(|_| "Could not create link")?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "Could not create link")?;
        anchor.set_href(&url);
        anchor.set_download(filename);
        anchor.click();
        Ok::<(), String>(())
    })();
    let _ = web_sys::Url::revoke_object_url(&url);
    result
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
