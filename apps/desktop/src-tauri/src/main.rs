// Hide the console window for release builds; keep it in dev for log output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use openmgmt_core::{AppService, Database, default_database_path};
use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};
use tracing_subscriber::EnvFilter;

/// Starts the Tauri desktop shell with the resolved SQLite database.
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let database_path = desktop_database_path().expect("failed to resolve OpenMgmt database path");
    let database = Database::open(database_path).expect("failed to open OpenMgmt database");

    tauri::Builder::default()
        .manage(AppService::new(database))
        .manage(commands::SyncRuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_organizations,
            commands::create_organization,
            commands::update_organization,
            commands::archive_organization,
            commands::list_projects,
            commands::create_project,
            commands::get_project,
            commands::update_project,
            commands::archive_project,
            commands::list_tasks,
            commands::create_task,
            commands::get_task,
            commands::update_task,
            commands::cancel_task,
            commands::get_board_state,
            commands::get_schedule_today,
            commands::get_schedule_week,
            commands::get_schedule_for_day,
            commands::get_unscheduled_tasks,
            commands::get_overdue_tasks,
            commands::auto_start_due_scheduled_tasks,
            commands::schedule_task,
            commands::reschedule_task,
            commands::clear_task_schedule,
            commands::list_schedule_conflicts,
            commands::suggest_next_time_block,
            commands::suggest_tasks_for_time_window,
            commands::complete_scheduled_block,
            commands::skip_scheduled_block,
            commands::hold_scheduled_block,
            commands::generate_schedule_ics,
            commands::start_task,
            commands::complete_task,
            commands::start_task_timer,
            commands::pause_task_timer,
            commands::resume_task_timer,
            commands::stop_task_timer,
            commands::complete_task_with_timer,
            commands::list_task_timer_sessions,
            commands::get_active_timer_session,
            commands::block_task,
            commands::unblock_task,
            commands::list_saved_task_views,
            commands::get_saved_task_view,
            commands::create_saved_task_view,
            commands::update_saved_task_view,
            commands::archive_saved_task_view,
            commands::query_tasks,
            commands::get_scoring_settings,
            commands::update_scoring_settings,
            commands::reset_scoring_settings,
            commands::export_tasks_json,
            commands::export_tasks_csv,
            commands::export_all_json,
            commands::backup_sqlite_database,
            commands::get_sync_settings,
            commands::update_sync_settings,
            commands::get_sync_status,
            commands::sync_now,
            commands::test_sync_connection,
            commands::clear_sync_error,
            commands::open_tv_board_window,
            commands::close_tv_board_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running OpenMgmt");
}

/// Returns the SQLite database path for the desktop shell.
///
/// Debug builds and explicit overrides keep the development path. Packaged
/// Windows release builds use per-user local app data.
fn desktop_database_path() -> Result<PathBuf, String> {
    if env::var_os("OPENMGMT_DATABASE_PATH").is_some()
        || cfg!(debug_assertions)
        || !cfg!(target_os = "windows")
    {
        return Ok(default_database_path());
    }

    installed_database_path()
}

/// Returns the installed Windows database path from per-user app data.
fn installed_database_path() -> Result<PathBuf, String> {
    installed_database_path_from_env(env::var_os("LOCALAPPDATA"), env::var_os("APPDATA"))
}

/// Chooses local app data first, falling back to roaming app data.
fn installed_database_path_from_env(
    local_app_data: Option<OsString>,
    roaming_app_data: Option<OsString>,
) -> Result<PathBuf, String> {
    let base = local_app_data
        .or(roaming_app_data)
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA or APPDATA must be set for installed OpenMgmt".to_string())?;

    Ok(installed_database_path_from_base(&base))
}

/// Appends the OpenMgmt SQLite filename to an app data base directory.
fn installed_database_path_from_base(base: &Path) -> PathBuf {
    base.join("OpenMgmt").join("openmgmt.sqlite")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the installed path stays under the OpenMgmt app data folder.
    #[test]
    fn installed_database_path_uses_openmgmt_app_data_folder() {
        let base = tempfile::tempdir().unwrap();

        assert_eq!(
            installed_database_path_from_base(base.path()),
            base.path().join("OpenMgmt").join("openmgmt.sqlite")
        );
    }

    /// Verifies local app data wins over roaming app data for SQLite storage.
    #[test]
    fn installed_database_path_prefers_local_app_data() {
        let local = PathBuf::from("local");
        let roaming = PathBuf::from("roaming");

        assert_eq!(
            installed_database_path_from_env(Some(local.clone().into()), Some(roaming.into()))
                .unwrap(),
            local.join("OpenMgmt").join("openmgmt.sqlite")
        );
    }

    /// Verifies a fresh installed database is created and has no user records.
    #[test]
    fn installed_database_parent_is_created_and_starts_empty() {
        let base = tempfile::tempdir().unwrap();
        let path = installed_database_path_from_base(base.path());
        let parent = path.parent().unwrap();

        assert!(!parent.exists());

        let database = Database::open(&path).unwrap();

        assert!(parent.exists());
        assert!(database.list_organizations().unwrap().is_empty());
        assert!(database.list_projects().unwrap().is_empty());
        assert!(database.list_tasks().unwrap().is_empty());
    }
}
