use openmgmt_core::{
    AppService, BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch,
    Project, ProjectPatch, SyncSettings, SyncSettingsPatch, SyncStatus, Task, TaskPatch,
};
use openmgmt_sync_client::{SyncConnectionTestResult, SyncOnceResult};
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
pub fn seed_database(service: State<'_, AppService>) -> CommandResult<()> {
    core(service.seed_database())
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
}
