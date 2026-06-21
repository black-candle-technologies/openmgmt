use crate::sync_runtime::{
    SYNC_ALREADY_RUNNING_ERROR, SyncReadiness, SyncRuntime, patch_changes_server_url,
    sync_readiness,
};
use openmgmt_core::{
    AppService, BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch,
    Project, ProjectPatch, SyncConflict, SyncSettings, SyncSettingsPatch, SyncStatus, Task,
    TaskPatch,
};
use openmgmt_sync_client::{SyncClientError, SyncConnectionTestResult, SyncOnceResult};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

type CommandResult<T> = Result<T, String>;

fn core<T>(result: openmgmt_core::db::Result<T>) -> CommandResult<T> {
    result.map_err(|error| {
        tracing::error!(%error, "Tauri command failed");
        error.to_string()
    })
}

fn core_mutation<T>(
    result: openmgmt_core::db::Result<T>,
    runtime: &SyncRuntime,
) -> CommandResult<T> {
    let value = core(result)?;
    runtime.trigger_mutation_sync();
    Ok(value)
}

fn map_sync_error(error: SyncClientError) -> String {
    if matches!(&error, SyncClientError::Other(message) if message == SYNC_ALREADY_RUNNING_ERROR) {
        SYNC_ALREADY_RUNNING_ERROR.into()
    } else {
        tracing::error!(%error, "manual sync failed");
        error.to_string()
    }
}

#[tauri::command]
pub fn list_organizations(service: State<'_, AppService>) -> CommandResult<Vec<Organization>> {
    core(service.list_organizations())
}

#[tauri::command]
pub fn create_organization(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    input: NewOrganization,
) -> CommandResult<Organization> {
    core_mutation(service.create_organization(input), &runtime)
}

#[tauri::command]
pub fn update_organization(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
    patch: OrganizationPatch,
) -> CommandResult<Organization> {
    core_mutation(service.update_organization(&id, patch), &runtime)
}

#[tauri::command]
pub fn archive_organization(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
) -> CommandResult<()> {
    core_mutation(service.archive_organization(&id), &runtime)
}

#[tauri::command]
pub fn list_projects(service: State<'_, AppService>) -> CommandResult<Vec<Project>> {
    core(service.list_projects())
}

#[tauri::command]
pub fn create_project(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    input: NewProject,
) -> CommandResult<Project> {
    core_mutation(service.create_project(input), &runtime)
}

#[tauri::command]
pub fn get_project(service: State<'_, AppService>, id: String) -> CommandResult<Project> {
    core(service.get_project(&id))
}

#[tauri::command]
pub fn update_project(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
    patch: ProjectPatch,
) -> CommandResult<Project> {
    core_mutation(service.update_project(&id, patch), &runtime)
}

#[tauri::command]
pub fn archive_project(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
) -> CommandResult<()> {
    core_mutation(service.archive_project(&id), &runtime)
}

#[tauri::command]
pub fn list_tasks(service: State<'_, AppService>) -> CommandResult<Vec<Task>> {
    core(service.list_tasks())
}

#[tauri::command]
pub fn create_task(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    input: NewTask,
) -> CommandResult<Task> {
    core_mutation(service.create_task(input), &runtime)
}

#[tauri::command]
pub fn get_task(service: State<'_, AppService>, id: String) -> CommandResult<Task> {
    core(service.get_task(&id))
}

#[tauri::command]
pub fn update_task(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
    patch: TaskPatch,
) -> CommandResult<Task> {
    core_mutation(service.update_task(&id, patch), &runtime)
}

#[tauri::command]
pub fn cancel_task(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
) -> CommandResult<Task> {
    core_mutation(service.cancel_task(&id), &runtime)
}

#[tauri::command]
pub fn start_task(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
) -> CommandResult<Task> {
    core_mutation(service.start_task(&id), &runtime)
}

#[tauri::command]
pub fn complete_task(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
) -> CommandResult<Task> {
    core_mutation(service.complete_task(&id), &runtime)
}

#[tauri::command]
pub fn block_task(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
    reason: String,
) -> CommandResult<Task> {
    core_mutation(service.block_task(&id, reason), &runtime)
}

#[tauri::command]
pub fn unblock_task(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    id: String,
) -> CommandResult<Task> {
    core_mutation(service.unblock_task(&id), &runtime)
}

#[tauri::command]
pub fn get_board_state(service: State<'_, AppService>) -> CommandResult<BoardState> {
    core(service.get_board_state())
}

#[tauri::command]
pub fn get_sync_settings(service: State<'_, AppService>) -> CommandResult<SyncSettings> {
    core(service.get_sync_settings())
}

#[tauri::command]
pub fn update_sync_settings(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
    patch: SyncSettingsPatch,
) -> CommandResult<SyncSettings> {
    let previous_server_url = if patch_changes_server_url(&patch) {
        Some(core(service.get_sync_settings())?.server_url)
    } else {
        None
    };
    let settings = core(service.update_sync_settings(patch))?;
    if previous_server_url.is_some_and(|previous| previous != settings.server_url) {
        runtime.reset_backoff();
    }
    if sync_readiness(&settings) == SyncReadiness::Ready {
        runtime.trigger_settings_sync();
    }
    Ok(settings)
}

#[tauri::command]
pub fn get_sync_status(
    service: State<'_, AppService>,
    runtime: State<'_, SyncRuntime>,
) -> CommandResult<SyncStatus> {
    let status = core(service.get_sync_status())?;
    Ok(runtime.with_runtime_status(status))
}

#[tauri::command]
pub async fn sync_now(runtime: State<'_, SyncRuntime>) -> CommandResult<SyncOnceResult> {
    runtime.sync_now().await.map_err(map_sync_error)
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

#[tauri::command]
pub fn get_sync_conflicts(service: State<'_, AppService>) -> CommandResult<Vec<SyncConflict>> {
    core(service.list_sync_conflicts())
}

#[tauri::command]
pub fn get_open_sync_conflicts(service: State<'_, AppService>) -> CommandResult<Vec<SyncConflict>> {
    core(service.list_open_sync_conflicts())
}

#[tauri::command]
pub fn ignore_sync_conflict(
    service: State<'_, AppService>,
    conflict_id: String,
) -> CommandResult<SyncConflict> {
    core(service.mark_sync_conflict_ignored(&conflict_id))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_runtime_error_string_for_overlap_is_stable() {
        let error = map_sync_error(SyncClientError::Other(SYNC_ALREADY_RUNNING_ERROR.into()));

        assert_eq!(error, SYNC_ALREADY_RUNNING_ERROR);
    }
}
