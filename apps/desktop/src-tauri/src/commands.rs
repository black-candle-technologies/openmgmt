use openmgmt_core::{
    AppService, BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch,
    Project, ProjectPatch, SyncSettings, SyncSettingsPatch, SyncStatus, Task, TaskPatch,
};
use openmgmt_sync_client::SyncOnceResult;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

type CommandResult<T> = Result<T, String>;

fn core<T>(result: openmgmt_core::db::Result<T>) -> CommandResult<T> {
    result.map_err(|error| {
        tracing::error!(%error, "Tauri command failed");
        error.to_string()
    })
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
pub async fn sync_now(service: State<'_, AppService>) -> CommandResult<SyncOnceResult> {
    let database = service.database();
    openmgmt_sync_client::sync_once(&database)
        .await
        .map_err(|error| {
            tracing::error!(%error, "manual sync failed");
            error.to_string()
        })
}

#[tauri::command]
pub fn open_tv_board_window(app: AppHandle) -> CommandResult<()> {
    if let Some(window) = app.get_webview_window("tv-board") {
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "tv-board", WebviewUrl::App("index.html".into()))
        .initialization_script("window.__OPENMGMT_BOARD__ = true;")
        .title("OpenMgmt TV Board")
        .fullscreen(true)
        .decorations(false)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(())
}
