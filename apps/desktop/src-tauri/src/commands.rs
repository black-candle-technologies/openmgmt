use chrono::{DateTime, Utc};
use openmgmt_core::{
    AppService, BoardState, CalendarBlock, NewOrganization, NewProject, NewSavedTaskView, NewTask,
    Organization, OrganizationPatch, Project, ProjectPatch, SavedTaskView, SavedTaskViewPatch,
    ScheduleConflict, ScheduleTaskInput, ScheduledBlockCompletion, ScoringSettings,
    ScoringSettingsPatch, SyncSettings, SyncSettingsPatch, SyncStatus, Task, TaskPatch,
    TaskQueryFilter, TaskSort, TaskTimerSession, TaskWithContext, TimeBlockSuggestion,
};
use openmgmt_sync_client::{SyncConnectionTestResult, SyncOnceResult};
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::{Mutex, MutexGuard};

type CommandResult<T> = Result<T, String>;

fn core<T>(result: openmgmt_core::db::Result<T>) -> CommandResult<T> {
    result.map_err(|error| {
        tracing::error!(%error, "Tauri command failed");
        error.to_string()
    })
}

#[derive(Default)]
pub struct SyncRuntimeState {
    syncing: Mutex<()>,
}

impl SyncRuntimeState {
    fn try_start(&self) -> CommandResult<MutexGuard<'_, ()>> {
        self.syncing
            .try_lock()
            .map_err(|_| "sync is already running".into())
    }
}

#[tauri::command]
pub fn list_organizations(service: State<'_, AppService>) -> CommandResult<Vec<Organization>> {
    core(service.list_organizations())
}

#[tauri::command]
pub fn create_organization(
    service: State<'_, AppService>,
    input: NewOrganization,
) -> CommandResult<Organization> {
    core(service.create_organization(input))
}

#[tauri::command]
pub fn update_organization(
    service: State<'_, AppService>,
    id: String,
    patch: OrganizationPatch,
) -> CommandResult<Organization> {
    core(service.update_organization(&id, patch))
}

#[tauri::command]
pub fn archive_organization(service: State<'_, AppService>, id: String) -> CommandResult<()> {
    core(service.archive_organization(&id))
}

#[tauri::command]
pub fn list_projects(service: State<'_, AppService>) -> CommandResult<Vec<Project>> {
    core(service.list_projects())
}

#[tauri::command]
pub fn create_project(service: State<'_, AppService>, input: NewProject) -> CommandResult<Project> {
    core(service.create_project(input))
}

#[tauri::command]
pub fn get_project(service: State<'_, AppService>, id: String) -> CommandResult<Project> {
    core(service.get_project(&id))
}

#[tauri::command]
pub fn update_project(
    service: State<'_, AppService>,
    id: String,
    patch: ProjectPatch,
) -> CommandResult<Project> {
    core(service.update_project(&id, patch))
}

#[tauri::command]
pub fn archive_project(service: State<'_, AppService>, id: String) -> CommandResult<()> {
    core(service.archive_project(&id))
}

#[tauri::command]
pub fn list_tasks(service: State<'_, AppService>) -> CommandResult<Vec<Task>> {
    core(service.list_tasks())
}

#[tauri::command]
pub fn create_task(service: State<'_, AppService>, input: NewTask) -> CommandResult<Task> {
    core(service.create_task(input))
}

#[tauri::command]
pub fn get_task(service: State<'_, AppService>, id: String) -> CommandResult<Task> {
    core(service.get_task(&id))
}

#[tauri::command]
pub fn update_task(
    service: State<'_, AppService>,
    id: String,
    patch: TaskPatch,
) -> CommandResult<Task> {
    core(service.update_task(&id, patch))
}

#[tauri::command]
pub fn cancel_task(service: State<'_, AppService>, id: String) -> CommandResult<Task> {
    core(service.cancel_task(&id))
}

#[tauri::command]
pub fn start_task(service: State<'_, AppService>, id: String) -> CommandResult<Task> {
    core(service.start_task(&id))
}

#[tauri::command]
pub fn complete_task(service: State<'_, AppService>, id: String) -> CommandResult<Task> {
    core(service.complete_task(&id))
}

#[tauri::command]
pub fn start_task_timer(
    service: State<'_, AppService>,
    task_id: String,
) -> CommandResult<TaskTimerSession> {
    core(service.start_task_timer(&task_id))
}

#[tauri::command]
pub fn pause_task_timer(
    service: State<'_, AppService>,
    task_id: String,
) -> CommandResult<TaskTimerSession> {
    core(service.pause_task_timer(&task_id))
}

#[tauri::command]
pub fn resume_task_timer(
    service: State<'_, AppService>,
    task_id: String,
) -> CommandResult<TaskTimerSession> {
    core(service.resume_task_timer(&task_id))
}

#[tauri::command]
pub fn stop_task_timer(
    service: State<'_, AppService>,
    task_id: String,
) -> CommandResult<TaskTimerSession> {
    core(service.stop_task_timer(&task_id))
}

#[tauri::command]
pub fn complete_task_with_timer(
    service: State<'_, AppService>,
    task_id: String,
) -> CommandResult<Task> {
    core(service.complete_task_with_timer(&task_id))
}

#[tauri::command]
pub fn list_task_timer_sessions(
    service: State<'_, AppService>,
    task_id: String,
) -> CommandResult<Vec<TaskTimerSession>> {
    core(service.list_task_timer_sessions(&task_id))
}

#[tauri::command]
pub fn get_active_timer_session(
    service: State<'_, AppService>,
    task_id: String,
) -> CommandResult<Option<TaskTimerSession>> {
    core(service.get_active_timer_session(&task_id))
}

#[tauri::command]
pub fn block_task(
    service: State<'_, AppService>,
    id: String,
    reason: String,
) -> CommandResult<Task> {
    core(service.block_task(&id, reason))
}

#[tauri::command]
pub fn unblock_task(service: State<'_, AppService>, id: String) -> CommandResult<Task> {
    core(service.unblock_task(&id))
}

#[tauri::command]
pub fn get_board_state(service: State<'_, AppService>) -> CommandResult<BoardState> {
    core(service.get_board_state())
}

#[tauri::command]
pub fn get_schedule_today(service: State<'_, AppService>) -> CommandResult<Vec<TaskWithContext>> {
    core(service.get_schedule_today())
}

#[tauri::command]
pub fn get_schedule_week(service: State<'_, AppService>) -> CommandResult<Vec<TaskWithContext>> {
    core(service.get_schedule_week())
}

#[tauri::command]
pub fn get_schedule_for_day(
    service: State<'_, AppService>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> CommandResult<Vec<TaskWithContext>> {
    core(service.get_schedule_for_day(start, end))
}

#[tauri::command]
pub fn get_unscheduled_tasks(
    service: State<'_, AppService>,
) -> CommandResult<Vec<TaskWithContext>> {
    core(service.get_unscheduled_tasks())
}

#[tauri::command]
pub fn get_overdue_tasks(service: State<'_, AppService>) -> CommandResult<Vec<TaskWithContext>> {
    core(service.get_overdue_tasks())
}

#[tauri::command]
pub fn auto_start_due_scheduled_tasks(service: State<'_, AppService>) -> CommandResult<Vec<Task>> {
    core(service.auto_start_due_scheduled_tasks())
}

#[tauri::command]
pub fn schedule_task(
    service: State<'_, AppService>,
    task_id: String,
    input: ScheduleTaskInput,
) -> CommandResult<CalendarBlock> {
    core(service.schedule_task(&task_id, input))
}

#[tauri::command]
pub fn reschedule_task(
    service: State<'_, AppService>,
    task_id: String,
    input: ScheduleTaskInput,
) -> CommandResult<CalendarBlock> {
    core(service.reschedule_task(&task_id, input))
}

#[tauri::command]
pub fn clear_task_schedule(service: State<'_, AppService>, task_id: String) -> CommandResult<Task> {
    core(service.clear_task_schedule(&task_id))
}

#[tauri::command]
pub fn list_schedule_conflicts(
    service: State<'_, AppService>,
) -> CommandResult<Vec<ScheduleConflict>> {
    core(service.list_schedule_conflicts())
}

#[tauri::command]
pub fn suggest_next_time_block(
    service: State<'_, AppService>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    duration_minutes: i64,
) -> CommandResult<Option<TimeBlockSuggestion>> {
    core(service.suggest_next_time_block(window_start, window_end, duration_minutes))
}

#[tauri::command]
pub fn suggest_tasks_for_time_window(
    service: State<'_, AppService>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> CommandResult<Vec<TaskWithContext>> {
    core(service.suggest_tasks_for_time_window(window_start, window_end))
}

#[tauri::command]
pub fn complete_scheduled_block(
    service: State<'_, AppService>,
    block_id: String,
) -> CommandResult<ScheduledBlockCompletion> {
    core(service.complete_scheduled_block(&block_id))
}

#[tauri::command]
pub fn skip_scheduled_block(
    service: State<'_, AppService>,
    block_id: String,
) -> CommandResult<CalendarBlock> {
    core(service.skip_scheduled_block(&block_id))
}

#[tauri::command]
pub fn generate_schedule_ics(service: State<'_, AppService>) -> CommandResult<String> {
    core(service.generate_schedule_ics())
}

#[tauri::command]
pub fn list_saved_task_views(service: State<'_, AppService>) -> CommandResult<Vec<SavedTaskView>> {
    core(service.list_saved_task_views())
}

#[tauri::command]
pub fn get_saved_task_view(
    service: State<'_, AppService>,
    id: String,
) -> CommandResult<SavedTaskView> {
    core(service.get_saved_task_view(&id))
}

#[tauri::command]
pub fn create_saved_task_view(
    service: State<'_, AppService>,
    input: NewSavedTaskView,
) -> CommandResult<SavedTaskView> {
    core(service.create_saved_task_view(input))
}

#[tauri::command]
pub fn update_saved_task_view(
    service: State<'_, AppService>,
    id: String,
    patch: SavedTaskViewPatch,
) -> CommandResult<SavedTaskView> {
    core(service.update_saved_task_view(&id, patch))
}

#[tauri::command]
pub fn archive_saved_task_view(service: State<'_, AppService>, id: String) -> CommandResult<()> {
    core(service.archive_saved_task_view(&id))
}

#[tauri::command]
pub fn query_tasks(
    service: State<'_, AppService>,
    filter: TaskQueryFilter,
    sort: Option<TaskSort>,
) -> CommandResult<Vec<TaskWithContext>> {
    core(service.query_tasks(filter, sort))
}

#[tauri::command]
pub fn get_scoring_settings(service: State<'_, AppService>) -> CommandResult<ScoringSettings> {
    core(service.get_scoring_settings())
}

#[tauri::command]
pub fn update_scoring_settings(
    service: State<'_, AppService>,
    patch: ScoringSettingsPatch,
) -> CommandResult<ScoringSettings> {
    core(service.update_scoring_settings(patch))
}

#[tauri::command]
pub fn reset_scoring_settings(service: State<'_, AppService>) -> CommandResult<ScoringSettings> {
    core(service.reset_scoring_settings())
}

#[tauri::command]
pub fn export_tasks_json(service: State<'_, AppService>) -> CommandResult<String> {
    core(service.export_tasks_json())
}

#[tauri::command]
pub fn export_tasks_csv(service: State<'_, AppService>) -> CommandResult<String> {
    core(service.export_tasks_csv())
}

#[tauri::command]
pub fn export_all_json(service: State<'_, AppService>) -> CommandResult<String> {
    core(service.export_all_json())
}

#[tauri::command]
pub fn backup_sqlite_database(
    app: AppHandle,
    service: State<'_, AppService>,
    target_path: String,
) -> CommandResult<()> {
    let target_path = validate_backup_target(
        &target_path,
        &app.path()
            .app_data_dir()
            .map_err(|error| format!("could not resolve app data directory: {error}"))?
            .join("backups"),
    )?;
    core(
        service.backup_sqlite_database(
            target_path
                .to_str()
                .ok_or_else(|| "backup path must be valid UTF-8".to_string())?,
        ),
    )
}

fn validate_backup_target(target_path: &str, backup_dir: &Path) -> CommandResult<PathBuf> {
    let trimmed = target_path.trim();
    if trimmed.is_empty() {
        return Err("backup destination path is required".into());
    }
    let requested = Path::new(trimmed);
    if requested.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(
            "backup destination must be a safe filename under the app backup directory".into(),
        );
    }
    if requested.file_name().is_none() {
        return Err("backup destination must include a filename".into());
    }

    std::fs::create_dir_all(backup_dir)
        .map_err(|error| format!("could not create backup directory: {error}"))?;
    let canonical_backup_dir = backup_dir
        .canonicalize()
        .map_err(|error| format!("could not validate backup directory: {error}"))?;
    let candidate = backup_dir.join(requested);
    let parent = candidate
        .parent()
        .ok_or_else(|| "backup destination must include a parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create backup parent directory: {error}"))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|error| format!("could not validate backup parent directory: {error}"))?;
    if !canonical_parent.starts_with(&canonical_backup_dir) {
        return Err("backup destination escapes the app backup directory".into());
    }
    if candidate
        .symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err("backup destination cannot be a symlink".into());
    }
    Ok(candidate)
}

#[tauri::command]
pub fn get_sync_settings(service: State<'_, AppService>) -> CommandResult<SyncSettings> {
    core(service.get_sync_settings())
}

#[tauri::command]
pub fn update_sync_settings(
    service: State<'_, AppService>,
    patch: SyncSettingsPatch,
) -> CommandResult<SyncSettings> {
    core(service.update_sync_settings(patch))
}

#[tauri::command]
pub fn get_sync_status(service: State<'_, AppService>) -> CommandResult<SyncStatus> {
    core(service.get_sync_status())
}

#[tauri::command]
pub async fn sync_now(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntimeState>,
) -> CommandResult<SyncOnceResult> {
    let _guard = runtime.try_start()?;
    let database = service.database();
    openmgmt_sync_client::sync_once(&database)
        .await
        .map_err(|error| {
            tracing::error!(%error, "manual sync failed");
            error.to_string()
        })
}

#[tauri::command]
pub async fn test_sync_connection(
    service: State<'_, AppService>,
) -> CommandResult<SyncConnectionTestResult> {
    let database = service.database();
    openmgmt_sync_client::test_connection(&database)
        .await
        .map_err(|error| {
            tracing::error!(%error, "sync connection test failed");
            error.to_string()
        })
}

#[tauri::command]
pub fn clear_sync_error(service: State<'_, AppService>) -> CommandResult<SyncStatus> {
    core(service.clear_sync_error())
}

/// Resolves the URL the TV board webview should load.
///
/// The host/origin must match `devUrl` (dev) or the Tauri protocol (prod) so the
/// board window is recognised as an app URL and `window.__TAURI__` is available
/// — otherwise `get_board_state` cannot be invoked.
///
/// * Dev (`debug_assertions`): the Trunk dev server from `build.devUrl` with
///   `?board=1`. We use the same origin and root path the main window loads
///   (`/`), avoiding any reliance on the dev server serving `/index.html`.
/// * Prod: the packaged asset protocol (`tauri://localhost/index.html?board=1`).
fn board_target_url(app: &AppHandle) -> Url {
    if cfg!(debug_assertions) {
        let mut url = app.config().build.dev_url.clone().unwrap_or_else(|| {
            Url::parse("http://127.0.0.1:1420").expect("valid fallback dev url")
        });
        url.set_query(Some("board=1"));
        url
    } else {
        Url::parse("tauri://localhost/index.html?board=1").expect("valid prod board url")
    }
}

/// Opens (or recovers) the dedicated TV board window.
///
/// This MUST be an `async` command. Tauri runs synchronous commands on the
/// main/UI thread, and `WebviewWindowBuilder::build()` (like `navigate`/
/// `set_focus`) dispatches work to the main thread and blocks on a channel until
/// it runs — so calling it from a sync command on the main thread deadlocks: the
/// window is created but never finishes loading (it sits at `about:blank`, the
/// "white board"), and the command never returns. Async commands are spawned off
/// the main thread, letting the event loop service the dispatch and finish.
#[tauri::command]
pub async fn open_tv_board_window(app: AppHandle) -> CommandResult<()> {
    // The board always opens as a normal, movable, resizable, decorated window.
    // TODO: add an optional kiosk/fullscreen mode (e.g. a `kiosk: bool` arg)
    // that sets `.fullscreen(true).decorations(false)` for wall-mounted displays.
    let target = board_target_url(&app);
    let dev = cfg!(debug_assertions);
    tracing::info!(%target, dev, "open_tv_board_window: resolved board URL");

    if let Some(window) = app.get_webview_window("tv-board") {
        // Do NOT blindly refocus: a previously-opened board may be stale or
        // blank (e.g. the dev server was down when it first loaded). Navigate it
        // to a freshly resolved board URL so a broken window is reloaded into a
        // working one, and clear any stale kiosk/fullscreen/borderless state.
        tracing::info!("open_tv_board_window: existing tv-board found, navigating + refocusing");
        let _ = window.set_fullscreen(false);
        let _ = window.set_decorations(true);
        window.navigate(target).map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    // `?board=1` is the primary board-mode signal; the initialization script is
    // a fallback for environments that strip the query string.
    match WebviewWindowBuilder::new(&app, "tv-board", WebviewUrl::External(target))
        .initialization_script("window.__OPENMGMT_BOARD__ = true;")
        .title("OpenMgmt Board")
        .inner_size(1440.0, 900.0)
        .min_inner_size(960.0, 600.0)
        .resizable(true)
        .decorations(true)
        .fullscreen(false)
        .center()
        .build()
    {
        Ok(window) => {
            tracing::info!("open_tv_board_window: tv-board window built");
            // Make sure the freshly built window takes focus and paints.
            let _ = window.set_focus();
            Ok(())
        }
        Err(error) => {
            tracing::error!(%error, "open_tv_board_window: build failed");
            Err(error.to_string())
        }
    }
}

/// Closes the dedicated TV board window (the in-board "Close Board" button).
/// Closing only ever targets the `tv-board` label, never the main window.
///
/// Async for the same reason as [`open_tv_board_window`]: window operations
/// dispatch to and block on the main thread, so they must not run on it.
#[tauri::command]
pub async fn close_tv_board_window(app: AppHandle) -> CommandResult<()> {
    if let Some(window) = app.get_webview_window("tv-board") {
        window.close().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_runtime_rejects_overlapping_attempts() {
        let runtime = SyncRuntimeState::default();
        let _guard = runtime.try_start().expect("first sync should start");

        assert_eq!(runtime.try_start().unwrap_err(), "sync is already running");
    }

    #[test]
    fn backup_target_validation_rejects_unsafe_paths() {
        let temp = tempfile::tempdir().unwrap();
        let backup_dir = temp.path().join("backups");

        assert!(validate_backup_target("", &backup_dir).is_err());
        assert!(validate_backup_target("..\\escape.sqlite", &backup_dir).is_err());
        assert!(validate_backup_target("../escape.sqlite", &backup_dir).is_err());
        assert!(validate_backup_target("C:\\temp\\escape.sqlite", &backup_dir).is_err());
    }

    #[test]
    fn backup_target_validation_allows_safe_relative_paths() {
        let temp = tempfile::tempdir().unwrap();
        let backup_dir = temp.path().join("backups");

        let target = validate_backup_target("daily/openmgmt.sqlite", &backup_dir).unwrap();
        assert!(target.starts_with(&backup_dir));
        assert_eq!(
            target.file_name().and_then(|value| value.to_str()),
            Some("openmgmt.sqlite")
        );
    }
}
